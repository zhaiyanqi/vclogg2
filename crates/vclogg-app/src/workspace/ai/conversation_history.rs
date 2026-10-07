use super::*;
use gpui_kit::component::input::Input;

struct HistorySurface {
    panel: Entity<AiPanel>,
    _subscription: Subscription,
}
impl Render for HistorySurface {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.panel
            .update(cx, |panel, cx| panel.render_history(window, cx))
    }
}
impl AiPanel {
    pub(super) fn load_history_flags(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.active.read(cx).store.clone() else {
            self.flags_loading = false;
            return;
        };
        cx.spawn(async move |this, cx| {
            let flags = cx
                .background_spawn(async move { store.ai_conversation_flags() })
                .await;
            _ = this.update(cx, |this, cx| {
                this.flags_loading = false;
                match flags {
                    Ok(flags) => this.flags = flags,
                    Err(error) => this.error = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn set_flags(
        &mut self,
        id: String,
        pinned: bool,
        archived: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.flags_loading || !self.flag_saving.insert(id.clone()) {
            return;
        }
        let Some(store) = self.active.read(cx).store.clone() else {
            self.flag_saving.remove(&id);
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let saved_id = id.clone();
            let result = cx
                .background_spawn(async move {
                    store.set_ai_conversation_flags(&saved_id, pinned, archived)
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.flag_saving.remove(&id);
                match result {
                    Ok(()) => {
                        this.flags.insert(id.clone(), (pinned, archived));
                        if archived {
                            this.close_conversation_tab(&id, window, cx);
                        }
                        this.refresh_conversation_history(false, window, cx);
                    }
                    Err(error) => this.error = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn open_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_history {
            return;
        }
        self.show_history = true;
        self.refresh_conversation_history(false, window, cx);
        let panel = cx.entity();
        let surface = cx.new(|cx| HistorySurface {
            _subscription: cx.observe(&panel, |_, _, cx| cx.notify()),
            panel: panel.clone(),
        });
        window.open_dialog(cx, move |dialog, window, _| {
            let close = panel.clone();
            dialog
                .title(crate::tr!("会话", "Conversations"))
                .w(window.rem_size() * 36.)
                .child(surface.clone())
                .on_close(move |_, _, cx| close.update(cx, |this, _| this.show_history = false))
        });
    }
    fn render_history(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.conversation_history_items(cx);
        v_flex()
            .gap_2()
            .min_w_0()
            .child(Input::new(&self.history_input))
            .when(!self.error.is_empty(), |view| {
                view.child(div().text_xs().child(self.error.clone()))
            })
            .when(rows.is_empty(), |view| {
                view.child(div().text_xs().child(if self.history_loading {
                    crate::tr!("正在加载…", "Loading…")
                } else {
                    crate::tr!("没有匹配的会话", "No matching conversations")
                }))
            })
            .child(
                Button::new("ai-history-archived")
                    .small()
                    .ghost()
                    .selected(self.show_archived)
                    .text_label(crate::tr!("已归档", "Archived"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.show_archived = !this.show_archived;
                        this.refresh_conversation_history(false, window, cx);
                    })),
            )
            .child(
                div()
                    .id("ai-history-results")
                    .max_h_80()
                    .overflow_y_scroll()
                    .children(rows.into_iter().map(|row| {
                        let flags = self.flags.get(&row.id).copied().unwrap_or_default();
                        let open = row.id.clone();
                        let pin = row.id.clone();
                        let archive = row.id.clone();
                        h_flex()
                            .gap_1()
                            .min_w_0()
                            .child(
                                Button::new(format!("ai-history-open-{}", row.id))
                                    .ghost()
                                    .text_label(row.title)
                                    .flex_1()
                                    .min_w_0()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.load_conversation(open.clone(), window, cx);
                                        window.close_dialog(cx);
                                    })),
                            )
                            .child(
                                Button::new(format!("ai-history-pin-{}", row.id))
                                    .small()
                                    .ghost()
                                    .selected(flags.0)
                                    .text_label(crate::tr!("置顶", "Pin"))
                                    .disabled(
                                        self.flags_loading || self.flag_saving.contains(&row.id),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.set_flags(pin.clone(), !flags.0, flags.1, window, cx)
                                    })),
                            )
                            .child(
                                Button::new(format!("ai-history-archive-{}", row.id))
                                    .small()
                                    .ghost()
                                    .text_label(if flags.1 {
                                        crate::tr!("恢复", "Restore")
                                    } else {
                                        crate::tr!("归档", "Archive")
                                    })
                                    .disabled(
                                        self.flags_loading || self.flag_saving.contains(&row.id),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.set_flags(
                                            archive.clone(),
                                            flags.0,
                                            !flags.1,
                                            window,
                                            cx,
                                        )
                                    })),
                            )
                    })),
            )
            .when(self.more_history, |view| {
                view.child(
                    Button::new("ai-history-more")
                        .small()
                        .text_label(crate::tr!("加载更多", "Load more"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.refresh_conversation_history(true, window, cx)
                        })),
                )
            })
            .into_any_element()
    }
}
