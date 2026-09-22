//! Preparation for context; disk work stays on the worker.
use super::*;

impl Workspace {
    pub(super) fn ai_prepare_context(
        &self,
        scope: SharedScope,
        _call: &ToolCall,
        cx: &App,
    ) -> Result<Work> {
        let state = scope
            .lock()
            .map_err(|_| anyhow::anyhow!("Analysis state unavailable"))?;

        let tab = self.active_document();
        let region = match self.active_log_region {
            LogRegion::Body => "body",
            LogRegion::CurrentResults => "current_results",
            LogRegion::GlobalResults => "global_results",
        };
        let mut refs = Vec::new();
        let mut context_documents = state.documents.clone();
        let mut active_reference = None;
        if let Some(tab) = tab.and_then(|t| state.documents.get(&t.id).map(|d| (t, d))) {
            let (tab, doc) = tab;
            let table = if self.active_log_region == LogRegion::CurrentResults {
                &tab.result_table
            } else {
                &tab.log_table
            };
            let table = table.read(cx);
            if let Some(ix) = table.active_log_row()
                && let Some(LogRowKey::Row { source_row, .. }) = table.delegate().row_key(ix)
            {
                active_reference = Some(doc.reference(source_row));
            }
            for ix in table.viewport().visible_range().take(100) {
                if let Some(LogRowKey::Row { source_row, .. }) = table.delegate().row_key(ix) {
                    refs.push(doc.reference(source_row));
                }
            }
        }
        if self.active_log_region == LogRegion::GlobalResults {
            refs.clear();
            let table = self.global_table.read(cx);
            active_reference = None;
            if let Some(ix) = table.active_log_row()
                && let Some(LogRowKey::Row {
                    document_id,
                    source_row,
                }) = table.delegate().row_key(ix)
            {
                active_reference = self.ai_context_reference(
                    &mut context_documents,
                    &state.directory,
                    document_id,
                    source_row,
                );
            }
            for ix in table.viewport().visible_range().take(100) {
                if let Some(LogRowKey::Row {
                    document_id,
                    source_row,
                }) = table.delegate().row_key(ix)
                    && let Some(reference) = self.ai_context_reference(
                        &mut context_documents,
                        &state.directory,
                        document_id,
                        source_row,
                    )
                {
                    refs.push(reference);
                }
            }
        }
        let mut selected = self
            .active_document()
            .and_then(|t| state.documents.get(&t.id).map(|d| (t, d)))
            .map(|(t, d)| {
                t.selected_source_rows_compressed(cx)
                    .iter()
                    .take(100)
                    .map(|r| d.reference(r))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if self.active_log_region == LogRegion::GlobalResults {
            selected.clear();
            for (id, rows) in self.global_table.read(cx).delegate().selection_snapshot() {
                for row in rows.iter() {
                    if selected.len() == 100 {
                        break;
                    }
                    if let Some(reference) =
                        self.ai_context_reference(&mut context_documents, &state.directory, id, row)
                    {
                        selected.push(reference);
                    }
                }
            }
        }
        let active_document = active_reference
            .as_ref()
            .and_then(|r| context_documents.get(&r.document_id))
            .or_else(|| tab.and_then(|t| context_documents.get(&t.id)));
        let active_document_id = active_document.map(|d| d.id);
        let active_file = active_document
                    .map(|d| -> Result<Value> { Ok(json!({"document_id":d.id,"version":d.version,"name":d.document.file_name(),"path":d.absolute_path()?.display().to_string(),"line_count":d.document.source_line_count()})) }).transpose()?;
        let mut metadata = json!({"active_file":active_file,"active_region":region,"active_reference":active_reference,"selected_references":selected,"current_document_id":tab.map(|t|t.id),"send_document_id":state.current,"region":region,"selected":selected,"query":self.query.read(cx).value().to_string(),"case_sensitive":self.case_sensitive,"regex":self.regex,"results_visible":self.global_search.results_visible,"searching":self.search_tabs.running.is_some(),"directory":state.directory.directory.as_ref().map(|p|p.display().to_string()),"search_tab":self.active_search_tab_key().map(|(owner,id)|json!({"owner":format!("{owner:?}"),"id":id.0}))});
        metadata["directory_state"] = json!({
            "captured_for_run": true,
            "selected": state.directory.directory.is_some(),
            "include_subdirectories": state.directory.include_subdirectories,
            "include_hidden_directories": state.directory.include_hidden_directories,
            "file_type_filter_enabled": state.directory.file_type_filter_enabled,
            "file_type_patterns": state.directory.file_type_patterns,
        });
        metadata["search_tab"] = self.active_search_tab_key().and_then(|(owner, id)| {
            self.search_tabs.state(owner, id).map(|tab| json!({
                "owner": format!("{owner:?}"), "id": id.0, "revision": tab.revision,
                "completed": tab.saved.completed.is_some(),
                "needs_restore": tab.needs_restore,
                "running": self.search_tabs.running.as_ref().is_some_and(|(running_owner, running_id, _, _)| *running_owner == owner && *running_id == id),
            }))
        }).unwrap_or(Value::Null);
        let directory = state.directory.directory.clone();
        let explicit = state.explicit.clone();
        let docs = context_documents;
        let checked = refs
            .iter()
            .chain(selected.iter())
            .chain(active_reference.iter())
            .map(|r| r.document_id)
            .chain(active_document_id)
            .collect::<BTreeSet<_>>();
        drop(state);
        Ok(Box::new(move || {
            for id in &checked {
                let doc = &docs[id];
                doc.verify()?;
                if !doc.open && !explicit.contains(id) {
                    let root = approved_directory(
                        directory.as_deref().context("Select a directory first")?,
                    )?;
                    if !doc.document.path().canonicalize()?.starts_with(root) {
                        bail!("Visible result is outside this run's directory");
                    }
                }
            }

            let mut value = metadata;
            value["visible"] = json!(refs);
            value["visible_references"] = json!(refs);
            value["content_included"] = json!(false);
            let mut state = scope
                .lock()
                .map_err(|_| anyhow::anyhow!("Analysis unavailable"))?;
            if state.cancellation.is_cancelled() {
                bail!("Analysis stopped");
            }
            let snapshots = checked.iter().map(|id| docs[id].clone()).collect();
            for id in checked {
                state.documents.insert(id, docs[&id].clone());
            }
            Ok(Evidence::Context(value, snapshots))
        }))
    }
}
