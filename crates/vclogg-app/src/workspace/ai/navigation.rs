use super::*;

impl AiPanel {
    pub(super) fn conversation_with_log_sources(&self) -> vclogg_ai::Conversation {
        let mut conversation = self.conversation.clone();
        for scope in self.reference_scopes.iter().chain(self.scope.iter()) {
            if let Ok(state) = scope.lock() {
                for doc in state.documents.values() {
                    if !conversation
                        .log_sources
                        .iter()
                        .any(|source| source.document_id == doc.id && source.version == doc.version)
                    {
                        conversation.log_sources.push(vclogg_ai::LogSource {
                            document_id: doc.id,
                            version: doc.version.clone(),
                            path: doc.document.path().to_path_buf(),
                        });
                    }
                }
            }
        }
        conversation
    }

    // Manual history navigation uses a path and line, not an expired agent capability.
    pub(super) fn open_historical_reference(
        &mut self,
        reference: &LogReference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let conversation = self.conversation_with_log_sources();
        let path = conversation
            .log_sources
            .iter()
            .find(|source| {
                source.document_id == reference.document_id && source.version == reference.version
            })
            .map(|source| source.path.clone())
            .or_else(|| {
                self.conversation
                    .messages
                    .iter()
                    .rev()
                    .find_map(|message| match message {
                        AgentMessage::Tool { result, .. } if !result.is_error => {
                            historical_path(&result.value, reference)
                        }
                        _ => None,
                    })
            });
        let Some(path) = path.filter(|path| path.is_absolute()) else {
            self.error = crate::tr!(
                "这条历史记录未保存源文件路径",
                "This history entry has no saved source file path"
            )
            .into();
            cx.notify();
            return;
        };
        let Some(row) = reference.line.checked_sub(1) else {
            return;
        };
        let workspace = self.workspace.clone();
        let setup = workspace
            .update(cx, |w, cx| -> Result<_> {
                if w.open_task.is_some() {
                    bail!("Wait for the current file operation to finish");
                }
                if let Some(ix) = w
                    .documents
                    .iter()
                    .position(|tab| paths_match(tab.document.path(), &path))
                {
                    if row >= w.documents[ix].document.source_line_count() {
                        bail!("Source line unavailable");
                    }
                    w.activate_tab(ix, window, cx);
                    if !w.activate_document_log_row_atomically(ix, row, window, cx) {
                        bail!("Could not navigate to the line");
                    }
                    // Drive the table's normal selection event as well as the retained cursor.
                    // Historical citations are activated from the AI panel, so the row must be
                    // visibly selected even though the pointer event originated elsewhere.
                    let local = w.documents[ix]
                        .document
                        .local_row(row)
                        .context("Source line unavailable")?;
                    w.documents[ix]
                        .log_table
                        .update(cx, |table, cx| table.set_active_log_row(local, cx));
                    w.selected_source_row = Some(row);
                    w.remember_user_log_region(LogRegion::Body);
                    w.log_viewer.focus_handle.focus(window, cx);
                    return Ok(None);
                }
                Ok(Some((
                    w.persistence.store.clone(),
                    w.color_labels.clone(),
                    SearchPreparationOptions {
                        case_sensitive: w.app_settings.default_case_sensitive,
                        regex: w.app_settings.default_use_regex,
                        max_results: w.app_settings.search_result_limit(),
                    },
                )))
            })
            .and_then(|result| result);
        self.error.clear();
        let (store, labels, options) = match setup {
            Ok(Some(setup)) => setup,
            Ok(None) => {
                cx.notify();
                return;
            }
            Err(error) => {
                self.error = error.to_string();
                cx.notify();
                return;
            }
        };
        self.ui_busy = true;
        self.ui_task = Some(cx.spawn_in(window, async move |this, cx| {
            let source = path.clone();
            let prepared = cx
                .background_spawn(async move {
                    let document = Arc::new(LogDocument::open(&source)?);
                    if row >= document.source_line_count() {
                        bail!("Source line unavailable");
                    }
                    super::super::document_tasks::prepare_document_in_range(
                        &source,
                        Some(document),
                        store.as_deref(),
                        None,
                        options,
                        &labels,
                        SearchRange::default(),
                    )
                })
                .await;
            let result = prepared.and_then(|prepared| {
                workspace
                    .update_in(cx, |w, window, cx| -> Result<()> {
                        if w.open_task.is_some() {
                            bail!("Wait for the current file operation to finish");
                        }
                        if prepared
                            .color_labels_snapshot
                            .as_ref()
                            .is_some_and(|labels| labels != &w.color_labels)
                        {
                            bail!("Color settings changed; retry navigation");
                        }
                        w.install_documents(
                            vec![(path.clone(), Ok(prepared))],
                            Some(&path),
                            &BTreeMap::new(),
                            None,
                            true,
                            window,
                            cx,
                        );
                        let ix = w
                            .documents
                            .iter()
                            .position(|tab| paths_match(tab.document.path(), &path))
                            .context("File could not be opened")?;
                        w.activate_tab(ix, window, cx);
                        if !w.activate_document_log_row_atomically(ix, row, window, cx) {
                            bail!("Could not navigate to the line");
                        }
                        let local = w.documents[ix]
                            .document
                            .local_row(row)
                            .context("Source line unavailable")?;
                        w.documents[ix]
                            .log_table
                            .update(cx, |table, cx| table.set_active_log_row(local, cx));
                        w.selected_source_row = Some(row);
                        w.remember_user_log_region(LogRegion::Body);
                        w.log_viewer.focus_handle.focus(window, cx);
                        Ok(())
                    })
                    .and_then(|result| result)
            });
            _ = this.update(cx, |this, cx| {
                this.ui_busy = false;
                if let Err(error) = result {
                    this.error = error.to_string();
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}

fn historical_path(value: &Value, reference: &LogReference) -> Option<PathBuf> {
    if value["document_id"].as_u64() == Some(reference.document_id)
        && value["version"].as_str() == Some(&reference.version)
        && let Some(path) = value["path"].as_str()
    {
        return Some(PathBuf::from(path));
    }
    match value {
        Value::Object(values) => values
            .values()
            .find_map(|value| historical_path(value, reference)),
        Value::Array(values) => values
            .iter()
            .find_map(|value| historical_path(value, reference)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_history_resolves_paths_by_document_and_version() {
        let reference = LogReference {
            document_id: 7,
            version: "old-run".into(),
            line: 2,
        };
        let files = json!({"files":[
            {"document_id":7,"version":"new-run","path":"/other.log"},
            {"document_id":7,"version":"old-run","path":"/source.log"}
        ]});
        assert_eq!(
            historical_path(&files, &reference),
            Some(PathBuf::from("/source.log"))
        );
        let missing = json!({"document_id":7,"version":"old-run","file":"source.log"});
        assert_eq!(historical_path(&missing, &reference), None);
    }
}
