use super::*;

const MAX_ATTACHMENTS: usize = 100;
const MAX_ATTACHMENT_BYTES: usize = 48 * 1024;

#[derive(Clone)]
pub(in crate::workspace) struct DraftLog {
    pub(super) document: DocumentSnapshot,
    pub(super) source_row: usize,
    pub(super) preview: String,
}

impl Workspace {
    /// Capture source identities when the menu opens, before focus or tabs can change.
    pub(in crate::workspace) fn ai_attachment_targets(
        &self,
        region: LogRegion,
        cx: &App,
    ) -> Vec<DraftLog> {
        let mut targets = Vec::new();
        if region == LogRegion::GlobalResults {
            let table = self.global_table.read(cx);
            let mut selection = table.delegate().selection_snapshot();
            if selection.is_empty()
                && let Some(LogRowKey::Row {
                    document_id,
                    source_row,
                }) = table
                    .active_log_row()
                    .and_then(|ix| table.delegate().row_key(ix))
            {
                selection.entry(document_id).or_default().insert(source_row);
            }
            for (id, rows) in selection {
                let Some(result) = self.global_search.results.get(&id) else {
                    continue;
                };
                let open = self.documents.iter().find(|tab| {
                    result_snapshot_matches_document(&result.path, &result.document, &tab.document)
                });
                let document = DocumentSnapshot {
                    id: open.map_or_else(next_directory_id, |tab| tab.id),
                    version: uuid::Uuid::new_v4().to_string(),
                    document: open
                        .map_or_else(|| result.document.clone(), |tab| tab.document.clone()),
                    open: open.is_some(),
                };
                for source_row in rows.iter().take(MAX_ATTACHMENTS + 1 - targets.len()) {
                    targets.push(DraftLog {
                        document: document.clone(),
                        source_row,
                        preview: String::new(),
                    });
                }
                if targets.len() > MAX_ATTACHMENTS {
                    break;
                }
            }
        } else if let Some(tab) = self.active_document() {
            let table = if region == LogRegion::CurrentResults {
                &tab.result_table
            } else {
                &tab.log_table
            };
            let table = table.read(cx);
            let mut rows = table.delegate().selected_source_rows_compressed();
            if rows.is_empty()
                && let Some(row) = table
                    .active_log_row()
                    .and_then(|ix| table.delegate().source_row(ix))
            {
                rows.insert(row);
            }
            let document = DocumentSnapshot {
                id: tab.id,
                version: uuid::Uuid::new_v4().to_string(),
                document: tab.document.clone(),
                open: true,
            };
            for source_row in rows.iter().take(MAX_ATTACHMENTS + 1) {
                targets.push(DraftLog {
                    document: document.clone(),
                    source_row,
                    preview: String::new(),
                });
            }
        }
        targets
    }

    pub(in crate::workspace) fn add_logs_to_ai(
        &mut self,
        targets: Vec<DraftLog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel = self
            .sidebar
            .update(cx, |sidebar, cx| sidebar.show_ai(window, cx));
        panel.update(cx, |panel, cx| panel.attach_logs(targets, window, cx));
    }
}

impl AiPanel {
    pub(super) fn attach_logs(
        &mut self,
        targets: Vec<DraftLog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.attachments_loading {
            return;
        }
        if targets.is_empty() {
            return;
        }
        if self.draft_logs.len() + targets.len() > MAX_ATTACHMENTS {
            self.error = crate::tr!(
                "一次最多附加 100 行日志，请缩小选择范围",
                "Attach up to 100 log lines; select fewer rows"
            )
            .into();
            cx.notify();
            return;
        }
        let conversation = self.conversation.id.clone();
        self.attachments_loading = true;
        self.attachment_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { read_attachments(targets) })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.attachments_loading = false;
                if this.conversation.id != conversation {
                    return;
                }
                match result {
                    Ok(logs) => {
                        let available = this
                            .workspace
                            .read_with(cx, |workspace, _| {
                                logs.iter().all(|log| {
                                    !log.document.open
                                        || workspace.documents.iter().any(|tab| {
                                            tab.id == log.document.id
                                                && Arc::ptr_eq(
                                                    &tab.document,
                                                    &log.document.document,
                                                )
                                        })
                                })
                            })
                            .unwrap_or(false);
                        if !available {
                            this.error = crate::tr!(
                                "日志已刷新或关闭，请重新选择",
                                "Log refreshed or closed; select it again"
                            )
                            .into();
                        } else {
                            for log in logs {
                                if !this.draft_logs.iter().any(|old| {
                                    old.source_row == log.source_row
                                        && paths_match(
                                            old.document.document.path(),
                                            log.document.document.path(),
                                        )
                                        && old
                                            .document
                                            .document
                                            .same_source_snapshot(&log.document.document)
                                }) {
                                    this.draft_logs.push(log);
                                }
                            }
                            this.error.clear();
                            this.focus(window, cx);
                        }
                    }
                    Err(error) => this.error = error.to_string(),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn message_with_attachments(
        &self,
        question: &str,
        scope: &SharedScope,
    ) -> Result<String> {
        let mut text = if question.is_empty() {
            crate::tr!(
                "分析这些日志，说明异常与依据。",
                "Analyze these logs and explain anomalies with evidence."
            )
            .to_owned()
        } else {
            question.to_owned()
        };
        let mut state = scope
            .lock()
            .map_err(|_| anyhow::anyhow!("Analysis unavailable"))?;
        for log in &self.draft_logs {
            let doc = if log.document.open {
                let doc = state.document(log.document.id, None)?;
                if !Arc::ptr_eq(&doc.document, &log.document.document) {
                    bail!("Attached log changed; select it again");
                }
                doc
            } else {
                state.explicit.insert(log.document.id);
                state
                    .documents
                    .insert(log.document.id, log.document.clone());
                log.document.clone()
            };
            let reference = doc.reference(log.source_row);
            // JSON escaping keeps embedded log instructions/fences inside a data string.
            text.push_str(&format!(
                "\n\n[{}:{}]({})\n```json\n{}\n```",
                doc.document.file_name().replace(['[', ']'], "_"),
                reference.line,
                reference.url(),
                serde_json::to_string(&json!({"reference":reference,"log_data":log.preview}))?
            ));
        }
        if text.len() > 64 * 1024 {
            bail!("Message and attached logs exceed 64 KiB; remove some attachments");
        }
        Ok(text)
    }
}

fn read_attachments(mut targets: Vec<DraftLog>) -> Result<Vec<DraftLog>> {
    let mut bytes = 0;
    for target in &mut targets {
        target.document.verify()?;
        let preview = target
            .document
            .document
            .line_preview(target.source_row, 8192)
            .context("Log line unavailable")?;
        target.preview = preview.text().to_owned();
        if preview.is_truncated() {
            target.preview.push_str(" … [truncated]");
        }
        bytes += target.preview.len();
        if bytes > MAX_ATTACHMENT_BYTES {
            bail!("Selected logs exceed 48 KiB; select fewer rows");
        }
        target.document.verify()?;
    }
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attachments_are_bounded_and_reject_changed_sources() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("logs.txt");
        std::fs::write(&path, format!("{}\n", "x".repeat(8192)).repeat(7)).unwrap();
        let document = DocumentSnapshot {
            id: 1,
            version: "v1".into(),
            document: Arc::new(LogDocument::open(&path).unwrap()),
            open: true,
        };
        let targets = (0..7)
            .map(|source_row| DraftLog {
                document: document.clone(),
                source_row,
                preview: String::new(),
            })
            .collect::<Vec<_>>();
        assert!(read_attachments(targets.clone()).is_err());
        assert_eq!(read_attachments(targets[..1].to_vec()).unwrap().len(), 1);
        std::fs::write(&path, "changed").unwrap();
        assert!(read_attachments(targets[..1].to_vec()).is_err());
    }
}
