//! Application notification policy and bounded, process-local history.

use std::{collections::VecDeque, sync::Arc};

use chrono::Local;
use gpui::{
    App, AppContext as _, Context, Entity, Global, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Task, Window, div, rems,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    v_flex,
};

use crate::state_store::StateStore;

const HISTORY_LIMIT: usize = 100;

struct NotificationRecord {
    id: u64,
    time: SharedString,
    message: SharedString,
}

pub(crate) struct NotificationCenter {
    enabled: bool,
    modified: bool,
    store: Option<Arc<StateStore>>,
    save_task: Option<Task<()>>,
    next_id: u64,
    records: VecDeque<NotificationRecord>,
}

struct NotificationRuntime(Entity<NotificationCenter>);
impl Global for NotificationRuntime {}

pub(crate) fn init(cx: &mut App) {
    let center = cx.new(|_| NotificationCenter {
        enabled: true,
        modified: false,
        store: None,
        save_task: None,
        next_id: 0,
        records: VecDeque::new(),
    });
    cx.set_global(NotificationRuntime(center));
}

pub(crate) fn center(cx: &App) -> Entity<NotificationCenter> {
    cx.global::<NotificationRuntime>().0.clone()
}

pub(crate) fn is_enabled(cx: &App) -> bool {
    center(cx).read(cx).enabled
}

/// Only the first completed bootstrap restores the preference. A user's earlier
/// toggle wins over a late database read, including reads from other windows.
pub(crate) fn restore(store: Arc<StateStore>, enabled: bool, window: &mut Window, cx: &mut App) {
    center(cx).update(cx, |center, cx| {
        if center.store.is_some() {
            return;
        }
        center.store = Some(store);
        if center.modified {
            center.save(window, cx);
        } else {
            center.enabled = enabled;
        }
        cx.notify();
    });
    if !is_enabled(cx) {
        clear_popups(window, cx);
    }
}

pub(crate) fn toggle(window: &mut Window, cx: &mut App) {
    center(cx).update(cx, |center, cx| {
        center.enabled = !center.enabled;
        center.modified = true;
        center.save(window, cx);
        cx.notify();
    });
    if !is_enabled(cx) {
        clear_popups(window, cx);
    }
    cx.refresh_windows();
}

fn clear_popups(window: &mut Window, cx: &mut App) {
    window.clear_notifications(cx);
    let current = window.window_handle();
    for handle in cx.windows().into_iter().filter(|handle| *handle != current) {
        _ = handle.update(cx, |_, window, cx| window.clear_notifications(cx));
    }
    cx.refresh_windows();
}

pub(crate) trait NotificationWindowExt {
    fn notify_message(&mut self, message: impl Into<SharedString>, cx: &mut App);
}

impl NotificationWindowExt for Window {
    fn notify_message(&mut self, message: impl Into<SharedString>, cx: &mut App) {
        let message = message.into();
        center(cx).update(cx, |center, cx| center.record(message.clone(), cx));
        if is_enabled(cx) {
            self.push_notification(message, cx);
        }
    }
}

impl NotificationCenter {
    fn record(&mut self, message: SharedString, cx: &mut Context<Self>) {
        self.next_id += 1;
        self.records.push_front(NotificationRecord {
            id: self.next_id,
            time: Local::now().format("%m-%d %H:%M:%S").to_string().into(),
            message,
        });
        self.records.truncate(HISTORY_LIMIT);
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let enabled = self.enabled;
        let previous = self.save_task.take();
        let window = window.window_handle();
        self.save_task = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let result = cx
                .background_spawn(async move { store.save_notifications_enabled(enabled) })
                .await;
            if let Err(error) = result {
                let message = crate::tr_args!(
                    "通知设置未能保存：{error}",
                    "Couldn’t save notification settings: {error}"
                );
                log::warn!("{message}");
                // The setting is already effective. Preserve failure feedback in
                // history even when the originating window has closed.
                _ = this.update(cx, |center, cx| center.record(message.clone().into(), cx));
                _ = window.update(cx, |_, window, cx| {
                    if is_enabled(cx) {
                        window.push_notification(message, cx);
                    }
                });
            }
        }));
    }
}

struct NotificationFooter {
    center: Entity<NotificationCenter>,
    _subscription: Subscription,
}

impl NotificationFooter {
    fn new(center: Entity<NotificationCenter>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&center, |_, _, cx| cx.notify());
        Self {
            center,
            _subscription: subscription,
        }
    }
}

impl Render for NotificationFooter {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        Button::new("clear-notifications")
            .small()
            .outline()
            .label(crate::tr!("清除通知", "Clear notifications"))
            .disabled(self.center.read(cx).records.is_empty())
            .on_click(cx.listener(|this, _, window, cx| {
                this.center.update(cx, |center, cx| {
                    center.records.clear();
                    cx.notify();
                });
                clear_popups(window, cx);
            }))
    }
}

pub(crate) fn button(window: &mut Window, cx: &mut App) -> Button {
    let tooltip = if is_enabled(cx) {
        crate::tr!("最近通知", "Recent notifications")
    } else {
        crate::tr!(
            "最近通知（通知已关闭）",
            "Recent notifications (notifications off)"
        )
    };
    crate::button_accessibility::with_label(
        Button::new("notification-history")
            .small()
            .ghost()
            .icon(IconName::Bell),
        crate::tr!("最近通知", "Recent notifications"),
    )
    .tooltip(tooltip)
    .selected(window.has_active_sheet(cx))
    .on_click(|_, window, cx| {
        if window.has_active_sheet(cx) {
            window.close_sheet(cx);
            window.refresh();
            return;
        }
        let center = center(cx);
        let footer = cx.new(|cx| NotificationFooter::new(center.clone(), cx));
        window.open_sheet(cx, move |sheet, _, _| {
            sheet
                .title(crate::tr!("最近通知", "Recent notifications"))
                .size(rems(26.))
                .max_w_full()
                .p_0()
                .on_close(|_, window, _| window.refresh())
                .child(center.clone())
                .footer(footer.clone())
        });
        window.refresh();
    })
}

impl Render for NotificationCenter {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let description = if self.enabled {
            crate::tr!(
                "本次运行的最近 100 条通知",
                "The latest 100 notifications from this session"
            )
        } else {
            crate::tr!(
                "通知已关闭，消息仍会保留在这里。",
                "Notifications are off. Messages are still saved here."
            )
        };
        let mut list = v_flex().w_full().min_w_0().flex_shrink_0().text_sm().child(
            div()
                .px_4()
                .py_3()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(description),
        );
        if self.records.is_empty() {
            list = list.child(
                div()
                    .px_4()
                    .py_6()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::tr!("暂无通知", "No notifications yet")),
            );
        }
        for record in &self.records {
            list = list.child(
                v_flex()
                    .id(("notification", record.id))
                    .w_full()
                    .min_w_0()
                    .flex_shrink_0()
                    .gap_1()
                    .px_4()
                    .py_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(record.time.clone()),
                    )
                    .child(
                        div()
                            .w_full()
                            .whitespace_normal()
                            .child(record.message.clone()),
                    ),
            );
        }
        list
    }
}
