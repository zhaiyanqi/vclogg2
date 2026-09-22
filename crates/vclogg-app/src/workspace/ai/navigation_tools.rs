//! Preparation for navigation; disk work stays on the worker.
use super::*;

impl Workspace {
    pub(super) fn ai_prepare_navigation(
        &self,
        scope: SharedScope,
        call: &ToolCall,
        _cx: &App,
    ) -> Result<Work> {
        let state = scope
            .lock()
            .map_err(|_| anyhow::anyhow!("Analysis state unavailable"))?;
        let args = &call.arguments;

        let (doc, _, _) = state.navigation_target(args)?;
        if doc.open {
            return Ok(Box::new(move || {
                doc.verify()?;
                Ok(Evidence::Json(json!({})))
            }));
        }
        let store = self.persistence.store.clone();
        let options = SearchPreparationOptions {
            case_sensitive: self.app_settings.default_case_sensitive,
            regex: self.app_settings.default_use_regex,
            max_results: self.app_settings.search_result_limit(),
        };
        let labels = self.color_labels.clone();
        let cancellation = state.cancellation.clone();
        Ok(Box::new(move || {
            doc.verify()?;
            let complete = LogDocument::open_cancellable(doc.document.path(), &cancellation)?
                .context("Analysis stopped")?;
            let prepared = super::document_tasks::prepare_document_in_range(
                doc.document.path(),
                Some(Arc::new(complete)),
                store.as_deref(),
                None,
                options,
                &labels,
                SearchRange::default(),
            )?;
            doc.verify()?;
            if !result_snapshot_matches_document(
                doc.document.path(),
                &doc.document,
                &prepared.document,
            ) {
                bail!("Source changed; refresh references");
            }
            Ok(Evidence::Open(Box::new(prepared)))
        }))
    }
}
