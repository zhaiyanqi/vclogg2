use super::*;
use crate::update_check::{CheckGuard, CheckOutcome, Manifest, RELEASES_URL};
use gpui_kit::component::text::TextView;

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
                    CheckOutcome::Success {
                        manifest,
                        is_newer,
                        cached,
                    } => {
                        show_release_notes(manifest, is_newer, cached, window, cx);
                    }
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

/// The standard dialog owns scrolling, focus trapping and Escape dismissal.
fn show_release_notes(
    manifest: Manifest,
    is_newer: bool,
    cached: bool,
    window: &mut Window,
    cx: &mut App,
) {
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
    window.open_dialog(cx, move |dialog, _, cx| {
        let url = url.clone();
        dialog
            .title(title.clone())
            .w(rems(40.))
            .max_w(relative(0.9))
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
    });
}
