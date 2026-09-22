//! Preparation for annotations; disk work stays on the worker.
use super::*;

impl Workspace {
    pub(super) fn ai_prepare_annotations(
        &self,
        scope: SharedScope,
        call: &ToolCall,
        _cx: &App,
    ) -> Result<Work> {
        let state = scope
            .lock()
            .map_err(|_| anyhow::anyhow!("Analysis state unavailable"))?;
        let args = &call.arguments;
        let value = match call.name.as_str() {
            "list_colors" => {
                json!({"colors":self.color_labels.iter().map(|l| json!({"id":l.id,"name":l.name})).collect::<Vec<_>>(),"text_mark_color":"neutral"})
            }
            "highlight_keyword" => {
                let doc = state.document(number(args, "document_id")?, args["version"].as_str())?;
                let tab = self
                    .documents
                    .iter()
                    .find(|tab| tab.id == doc.id)
                    .context("Open the file before highlighting")?;
                let keyword = text(args, "keyword")?.trim().to_owned();
                if keyword.is_empty() {
                    bail!("Keyword is empty");
                }
                let case_sensitive = args["case_sensitive"].as_bool().unwrap_or(false);
                let expected = tab.file.keyword_color_rules.clone();
                let mut rules = expected.clone();
                rules.retain(|r| !(r.keyword == keyword && r.case_sensitive == case_sensitive));
                let action = text(args, "action")?;
                if action == "set" {
                    let label = self
                        .color_labels
                        .iter()
                        .find(|l| Some(l.id.as_str()) == args["color_label_id"].as_str())
                        .context("Unknown color label")?;
                    rules.push(KeywordColorRule {
                        label_id: Some(label.id.clone()),
                        keyword: keyword.clone(),
                        color: label.background_color,
                        alpha: label.background_alpha,
                        case_sensitive,
                        enabled: true,
                    });
                }
                let labels = self.color_labels.clone();
                let last_color_label_id = self.last_color_label_id.clone();
                let outcome = if action == "set" {
                    ColorRuleOutcome::Applied
                } else {
                    ColorRuleOutcome::Removed
                };
                return Ok(Box::new(move || {
                    doc.verify()?;
                    let resolved = resolve_color_rules(&rules, &labels);
                    Ok(Evidence::Color(Box::new(PreparedColorRuleUpdate {
                        document_id: doc.id,
                        document: doc.document,
                        expected_rules: expected,
                        expected_labels: labels,
                        rules,
                        resolved: Some(resolved),
                        propagated_files: Vec::new(),
                        search_session: None,
                        last_color_label_id,
                        outcome,
                    })))
                }));
            }
            "text_mark" => {
                let (doc, row) = state.reference(&args["reference"])?;
                return Ok(Box::new(move || {
                    doc.verify()?;
                    let preview = doc
                        .document
                        .line_preview(row, crate::virtual_log_lines::DEFAULT_MAX_LINE_SOURCE_BYTES)
                        .context("Source unavailable")?;
                    Ok(Evidence::Json(json!({"source":preview.text()})))
                }));
            }
            "list_marks" => {
                let doc = state.document(number(args, "document_id")?, args["version"].as_str())?;
                let tab = self
                    .documents
                    .iter()
                    .find(|t| t.id == doc.id)
                    .context("Open this file to inspect marks")?;
                let start = args["start_line"].as_u64().unwrap_or(1).max(1) as usize - 1;
                let rows = tab
                    .file
                    .marked_rows
                    .iter()
                    .chain(tab.file.row_tags.source_rows())
                    .filter(|r| *r >= start)
                    .collect::<BTreeSet<_>>();
                let entries = rows.iter().take(100).map(|row| json!({"reference":doc.reference(*row),"marked":tab.file.marked_rows.contains(*row),"text_marks":tab.file.row_tags.row(*row).map(|(id,t)|json!({"id":id,"text":t.label})).collect::<Vec<_>>()})).collect::<Vec<_>>();
                json!({"marks":entries,"next_line":if rows.len()>100 {rows.iter().nth(100).map(|r|r+1)} else {None}})
            }
            _ => {
                drop(state);
                return self.ai_prepare_state_change(scope, call);
            }
        };
        let value = match call.name.as_str() {
            "list_colors" => object_page(value, "colors", args)?,
            _ => value,
        };
        Ok(Box::new(move || Ok(Evidence::Json(value))))
    }
}
