use super::*;
use crate::update_check::{CheckGuard, CheckOutcome, Manifest, RELEASES_URL};
use gpui_kit::component::{dialog::Dialog, text::TextView};

struct UpdateCheckDialogState {
    outcome: Option<CheckOutcome>,
    dismissed: bool,
}

impl Workspace {
    pub(super) fn check_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(guard) = CheckGuard::acquire() else {
            window.notify_message(crate::tr!("正在检查更新…", "Checking for updates…"), cx);
            return;
        };
        let dialog_state = cx.new(|_| UpdateCheckDialogState {
            outcome: None,
            dismissed: false,
        });
        open_update_check_dialog(dialog_state.clone(), window, cx);
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
                _ = this.update_in(cx, |_, window, cx| {
                    // Closing the progress dialog must not reopen it or launch a browser later.
                    if dialog_state.read(cx).dismissed {
                        return;
                    }
                    let unavailable = matches!(&outcome, CheckOutcome::Unavailable);
                    dialog_state.update(cx, |state, cx| {
                        state.outcome = Some(outcome);
                        cx.notify();
                    });
                    window.refresh();
                    if unavailable {
                        cx.open_url(RELEASES_URL);
                    }
                });
            }),
            cx,
        );
    }
}

/// Keep one dialog alive while its content transitions from pending to the result.
fn open_update_check_dialog(
    state: Entity<UpdateCheckDialogState>,
    window: &mut Window,
    cx: &mut App,
) {
    window.open_dialog(cx, move |dialog, window, cx| {
        let cancel_state = state.clone();
        let dialog = dialog
            .title(crate::tr!("检查更新", "Check for updates"))
            .close_button(false)
            .on_cancel(move |_, _, cx| {
                cancel_state.update(cx, |state, _| state.dismissed = true);
                true
            });
        let outcome = state.read(cx).outcome.clone();
        match outcome {
            Some(CheckOutcome::Success {
                manifest,
                is_newer,
                cached,
            }) => release_notes_dialog(dialog, &manifest, is_newer, cached, window, cx),
            outcome => dialog
                .child(if outcome.is_none() {
                    crate::tr!(
                        "正在检查更新，请稍候…",
                        "Checking for updates. Please wait…"
                    )
                } else {
                    crate::tr!(
                        "暂时无法检查更新，已请求在浏览器打开 Releases 页面。",
                        "Unable to check for updates. Requested opening Releases in your browser."
                    )
                })
                .footer(
                    DialogFooter::new().child(crate::dialog_focus::dialog_cancel_action(
                        "update-check-close-action",
                        Button::new("update-check-close").label(crate::tr!("关闭", "Close")),
                        cx,
                    )),
                )
                .on_ok(|_, _, _| false),
        }
    });
}

/// The standard dialog owns scrolling, focus trapping and Escape dismissal.
fn release_notes_dialog(
    dialog: Dialog,
    manifest: &Manifest,
    is_newer: bool,
    cached: bool,
    window: &mut Window,
    cx: &mut App,
) -> Dialog {
    let version = manifest.release_version();
    let title = crate::tr_args!("版本 {version} 更新内容", "Release {version}");
    let current = crate::build_info::VERSION;
    let published = manifest.published_at();
    let details = crate::tr_args!(
        "当前版本：{current} · 发布日期：{published}",
        "Installed: {current} · Published: {published}"
    );
    let notes: SharedString = if manifest.release_notes().trim().is_empty() {
        crate::tr!(
            "此版本未提供更新说明，可打开下载页面查看。",
            "No release notes were provided. Open the download page for details."
        )
        .into()
    } else {
        manifest.release_notes().to_owned().into()
    };
    let url = manifest.download_url().to_owned();
    dialog
        .title(title)
        // Dialog sizing accepts pixels; resolve the rem and viewport sizes here.
        .w(window.rem_size() * 40.)
        .max_w(window.viewport_size().width * 0.9)
        .close_button(false)
        .child(
            v_flex()
                .gap_3()
                .child(if is_newer {
                    crate::tr!("发现新版本", "A new version is available")
                } else {
                    crate::tr!("未发现更新的稳定版本", "No newer stable release")
                })
                .child(details.clone())
                .when(cached, |this| {
                    this.child(crate::tr!(
                        "以下为 24 小时内缓存的检查结果",
                        "Showing a check cached within the last 24 hours"
                    ))
                })
                .child(
                    TextView::markdown("update-release-notes", notes.clone())
                        .selectable(true)
                        .scrollable(false),
                ),
        )
        .footer(
            DialogFooter::new()
                .child(crate::dialog_focus::dialog_cancel_action(
                    "update-notes-close-action",
                    Button::new("update-notes-close").label(crate::tr!("关闭", "Close")),
                    cx,
                ))
                .child(crate::dialog_focus::dialog_confirm_action(
                    "update-notes-download-action",
                    Button::new("update-notes-download")
                        .primary()
                        .label(crate::tr!("打开下载页面", "Open download page")),
                    cx,
                )),
        )
        .on_ok(move |_, _, cx| {
            cx.open_url(&url);
            true
        })
}
