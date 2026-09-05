use super::*;
use crate::search_feedback::SearchFeedback;

impl Workspace {
    pub(super) fn track_search_feedback(
        &mut self,
        target: SearchTarget,
        revision: u64,
        feedback: Arc<SearchFeedback>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.searches
            .update_feedback(target, revision, feedback.snapshot());
        let task = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let snapshot = feedback.snapshot();
                let current = this
                    .update_in(cx, |this, _, cx| {
                        if !this.searches.is_current(target, revision) {
                            return false;
                        }
                        if this.searches.update_feedback(target, revision, snapshot) {
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false);
                if !current {
                    break;
                }
            }
        });
        self.searches.set_feedback_task(task);
    }

    pub(super) fn render_search_feedback(&self, cx: &Context<Self>) -> impl IntoElement {
        let snapshot = self.searches.feedback().cloned().unwrap_or_default();
        let label = snapshot.percent.map_or_else(
            || crate::tr!("搜索中", "Searching").to_string(),
            |percent| {
                crate::tr_args!(
                    "搜索中 {percent}% · 已匹配 {} 行",
                    "Searching {percent}% · {} matching lines",
                    snapshot.matches
                )
            },
        );
        v_flex()
            .id("search-feedback")
            .w_full()
            .gap_1()
            .px_3()
            .py_1()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(label)
            .children(snapshot.previews.into_iter().map(|text| {
                div()
                    .w_full()
                    .truncate()
                    .child(crate::tr_args!("临时预览 · {text}", "Preview · {text}"))
            }))
    }
}
