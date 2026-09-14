use super::super::search_tabs::SearchTabOwner;
use super::*;
use crate::workspace_state::{QuickFindMatch, QuickFindTarget};

impl Workspace {
    pub(super) fn ai_search_draft(&self, document_id: u64, cx: &App) -> Result<Value> {
        let tab = self
            .documents
            .iter()
            .find(|tab| tab.id == document_id)
            .context("File closed")?;
        let owner = SearchTabOwner::File(document_id);
        let group = self.search_tabs.groups.get(&owner);
        let id = group.map(|g| g.active);
        let draft = if id.is_some_and(|id| self.search_tabs.installed == Some((owner, id))) {
            self.query.read(cx).value().to_string()
        } else {
            id.and_then(|id| self.search_tabs.state(owner, id))
                .map(|tab| tab.saved.draft.text.clone())
                .unwrap_or_else(|| tab.search_query.text.clone())
        };
        Ok(json!({"tab_id":id.map(|id|id.0),"query":draft}))
    }

    pub(super) fn ai_append_search(
        &mut self,
        document_id: u64,
        text: &str,
        expected: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value> {
        if &self.ai_search_draft(document_id, cx)? != expected {
            bail!("Search draft changed; inspect it before appending again");
        }
        let appended = append_search_text(expected["query"].as_str().unwrap_or_default(), text)?;
        let ix = self
            .documents
            .iter()
            .position(|tab| tab.id == document_id)
            .context("File closed")?;
        self.activate_tab(ix, window, cx);
        self.set_search_scope(SearchScope::CurrentFile, window, cx);
        self.query.update(cx, |input, cx| {
            input.set_value(appended.clone(), window, cx)
        });
        self.capture_active_search_tab(cx);
        self.persist_search_tabs(window, cx);
        self.query.focus_handle(cx).focus(window, cx);
        cx.notify();
        Ok(json!({"document_id":document_id,"query":appended,"executed":false}))
    }

    pub(super) fn ai_select_search_hit(
        &mut self,
        search: &SearchSnapshot,
        doc: &DocumentSnapshot,
        source_row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool> {
        let Some((owner, id, revision)) = search.tab else {
            return Ok(false);
        };
        if !self
            .search_tabs
            .state(owner, id)
            .is_some_and(|tab| tab.revision == revision && tab.saved.completed.is_some())
        {
            bail!("Search results expired; acquire a new result reference");
        }
        if let SearchTabOwner::File(document_id) = owner {
            let ix = self
                .documents
                .iter()
                .position(|tab| tab.id == document_id)
                .context("File closed")?;
            self.activate_tab(ix, window, cx);
        }
        self.set_search_scope(owner.scope(), window, cx);
        if self.search_tabs.installed != Some((owner, id)) {
            self.commit_search_tab_activation(owner, id, window, cx);
        }
        let (target, view_row) = match owner {
            SearchTabOwner::File(document_id) => {
                let tab = self
                    .documents
                    .iter()
                    .find(|tab| tab.id == document_id)
                    .context("File closed")?;
                let row = tab
                    .result_table
                    .read(cx)
                    .delegate()
                    .row_ix_for_key(LogRowKey::Row {
                        document_id,
                        source_row,
                    })
                    .context("Result view is preparing; retry navigation")?;
                (QuickFindTarget::Results(document_id), row)
            }
            _ => {
                let document_id = self
                    .global_search
                    .results
                    .iter()
                    .find(|(_, result)| {
                        result_snapshot_matches_document(
                            &result.path,
                            &result.document,
                            &doc.document,
                        )
                    })
                    .map(|(id, _)| *id)
                    .context("Result source changed")?;
                let Some(row) =
                    self.global_table
                        .read(cx)
                        .delegate()
                        .row_ix_for_key(LogRowKey::Row {
                            document_id,
                            source_row,
                        })
                else {
                    // A collapsed group has no visible result row. The caller
                    // can still navigate to the verified source line.
                    return Ok(false);
                };
                (QuickFindTarget::GlobalResults, row)
            }
        };
        self.apply_quick_find_match(
            QuickFindMatch {
                target,
                view_row,
                source_row,
            },
            cx,
        );
        self.remember_user_log_region(self.active_log_region);
        cx.notify();
        Ok(true)
    }
}

fn append_search_text(current: &str, text: &str) -> Result<String> {
    let text = text.trim();
    if text.is_empty() {
        bail!("Search text is empty");
    }
    let query = if current.trim().is_empty() {
        text.to_owned()
    } else {
        format!("{} {text}", current.trim_end())
    };
    if query.len() > 8192 {
        bail!("Combined search exceeds 8 KiB");
    }
    Ok(query)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn append_preserves_the_existing_query_and_bounds_text() {
        assert_eq!(
            append_search_text("ERROR", " timeout ").unwrap(),
            "ERROR timeout"
        );
        assert_eq!(append_search_text("", "网络异常").unwrap(), "网络异常");
        assert!(append_search_text("INFO", " ").is_err());
        assert!(append_search_text(&"a".repeat(8192), "b").is_err());
    }
}
