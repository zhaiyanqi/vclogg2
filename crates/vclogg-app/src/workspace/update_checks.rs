use super::*;
use crate::update_check::{CheckGuard, CheckOutcome, Manifest, RELEASES_URL};
use gpui_kit::component::{dialog::Dialog, text::TextView};

struct UpdateCheckDialogState {
    outcome: Option<CheckOutcome>,
    dismissed: bool,
    expanded_versions: std::collections::HashSet<String>,
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
            expanded_versions: Default::default(),
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
                        if let CheckOutcome::Success { manifest, .. } = &outcome {
                            state.expanded_versions = manifest
                                .upgrade_releases()
                                .into_iter()
                                .take(3)
                                .map(|release| release.release_version().to_owned())
                                .collect();
                        }
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
            }) => release_notes_dialog(dialog, &manifest, is_newer, cached, &state, window, cx),
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
    state: &Entity<UpdateCheckDialogState>,
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
    let language = crate::i18n::current_language();
    let releases = manifest.upgrade_releases();
    let count = releases.len();
    let partial = manifest.history_is_partial();
    let summary = if partial {
        crate::tr_args!(
            "显示 {count} 个版本的部分更新记录",
            "Showing partial history for {count} releases"
        )
    } else if is_newer {
        crate::tr_args!(
            "跨越 {count} 个稳定版本",
            "Upgrading across {count} stable releases"
        )
    } else {
        crate::tr!("未发现更新的稳定版本", "No newer stable release").to_string()
    };
    let mut important = String::new();
    for release in &releases {
        let important_notes = release.important_notes(language);
        if !important_notes.trim().is_empty() {
            important.push_str(&format!(
                "## {}\n\n{}\n\n",
                release.release_version(),
                important_notes
            ));
        }
    }
    let expanded = state.read(cx).expanded_versions.clone();
    let entries: Vec<AnyElement> = releases
        .into_iter()
        .map(|release| {
            let version = release.release_version().to_owned();
            let is_open = expanded.contains(&version);
            let toggle_state = state.clone();
            let toggle_version = version.clone();
            let label = format!(
                "{} {} · {}",
                if is_open { "▾" } else { "▸" },
                version,
                release.published_at()
            );
            let localized_notes = release.release_notes(language);
            let notes: SharedString = if localized_notes.trim().is_empty() {
                crate::tr!(
                    "此版本暂无中文更新说明，请打开发布页面查看原文及迁移要求。",
                    "English release notes are unavailable. Open the release page for the original notes and migration requirements."
                )
                .into()
            } else {
                localized_notes.into()
            };
            v_flex()
                .gap_2()
                .child(
                    Button::new(SharedString::from(format!(
                        "update-release-toggle-{version}"
                    )))
                    .label(label)
                    .on_click(move |_, window, cx| {
                        toggle_state.update(cx, |state, cx| {
                            if !state.expanded_versions.remove(&toggle_version) {
                                state.expanded_versions.insert(toggle_version.clone());
                            }
                            cx.notify();
                        });
                        window.refresh();
                    }),
                )
                .when(is_open, |this| {
                    this.child(
                        TextView::markdown(
                            SharedString::from(format!("update-release-notes-{version}")),
                            notes,
                        )
                        .selectable(true)
                        .scrollable(false),
                    )
                })
                .into_any_element()
        })
        .collect();
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
                .child(summary)
                .child(details.clone())
                .when(cached, |this| {
                    this.child(crate::tr!(
                        "以下为 24 小时内缓存的检查结果",
                        "Showing a check cached within the last 24 hours"
                    ))
                })
                .when(partial, |this| this.child(crate::tr!(
                    "当前版本早于清单覆盖范围，以下不是完整升级记录。",
                    "Your version predates the available history; these notes do not cover the entire upgrade."
                )))
                .when(!important.is_empty(), |this| this
                    .child(crate::tr!("不兼容变更与迁移要求", "Breaking changes and migration requirements"))
                    .child(TextView::markdown("update-important-notes", important)
                        .selectable(true).scrollable(false)))
                .children(entries)
                .child(Button::new("update-all-releases")
                    .label(crate::tr!("查看完整发布历史", "View full release history"))
                    .on_click(|_, _, cx| cx.open_url(RELEASES_URL))),
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
