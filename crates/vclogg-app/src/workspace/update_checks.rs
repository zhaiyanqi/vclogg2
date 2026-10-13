use super::*;
use crate::update_check::{CheckGuard, CheckOutcome, RELEASES_URL};

impl Workspace {
    pub(super) fn check_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(guard) = CheckGuard::acquire() else {
            window.notify_message(crate::tr!("正在检查更新…", "Checking for updates…"), cx);
            return;
        };
        window.notify_message(crate::tr!("正在检查更新…", "Checking for updates…"), cx);
        let store = self.persistence.store.clone();
        self.persistence.state_tasks.push(
            cx.spawn_in(window, async move |this, cx| {
                let outcome = cx
                    .background_spawn(async move {
                        // Keep the process-wide guard through blocking I/O even if the window closes.
                        let outcome = crate::update_check::check(store.as_deref());
                        (outcome, guard)
                    })
                    .await;
                let (outcome, _guard) = outcome;
                _ = this.update_in(cx, |_, window, cx| match outcome {
                    CheckOutcome::Available(url) => cx.open_url(&url),
                    CheckOutcome::Current { cached } => window.notify_message(
                        if cached {
                            crate::tr!(
                                "最近一次检查未发现新版（24 小时内缓存）",
                                "No newer release at the last check (cached within 24 hours)"
                            )
                        } else {
                            crate::tr!("当前已是最新版本", "You’re up to date")
                        },
                        cx,
                    ),
                    CheckOutcome::Unavailable => {
                        window.notify_message(
                            crate::tr!(
                                "暂时无法检查更新，正在打开 Releases 页面",
                                "Unable to check for updates right now. Opening Releases"
                            ),
                            cx,
                        );
                        cx.open_url(RELEASES_URL);
                    }
                });
            }),
            cx,
        );
    }
}
