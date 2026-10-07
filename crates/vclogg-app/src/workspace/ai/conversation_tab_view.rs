use super::*;
use gpui_kit::component::tab::{Tab, TabBar};

impl AiPanel {
    pub(super) fn render_conversation_tabs(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let disabled = self.conversation_tabs_busy(cx);
        let selected = self
            .open_conversations
            .iter()
            .position(|id| id == &self.current_id(cx))
            .unwrap_or(0);
        let tabs = TabBar::new("ai-conversation-tabs")
            .w_full()
            .min_w_0()
            .h_8()
            .track_scroll(&self.conversation_tab_scroll)
            .selected_index(selected)
            .max_width(window.rem_size() * 12.)
            .children(self.open_conversations.iter().map(|id| {
                let title = self.conversation_tab_title(id, cx);
                let close_id = id.clone();
                let middle_id = id.clone();
                let menu_id = id.clone();
                let owner = cx.entity();
                Tab::new()
                    .aria_label(title.clone())
                    .disabled(disabled)
                    .on_aux_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        if event.is_middle_click() {
                            cx.stop_propagation();
                            this.close_conversation_tab(&middle_id, window, cx);
                        }
                    }))
                    .child(
                        div()
                            .id(format!("ai-tab-menu-{id}"))
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left_0()
                            .right_0()
                            .context_menu(move |menu, window, cx| {
                                let disabled = owner.read(cx).conversation_tabs_busy(cx);
                                let close_id = menu_id.clone();
                                let others_id = menu_id.clone();
                                let delete_id = menu_id.clone();
                                let rename_id = menu_id.clone();
                                let fork_id = menu_id.clone();
                                let export_id = menu_id.clone();
                                let stop_id = menu_id.clone();
                                let running = owner.read(cx).is_running(&menu_id, cx);
                                let flags = owner
                                    .read(cx)
                                    .flags
                                    .get(&menu_id)
                                    .copied()
                                    .unwrap_or_default();
                                let flags_busy = owner.read(cx).flags_loading
                                    || owner.read(cx).flag_saving.contains(&menu_id);
                                let pin_id = menu_id.clone();
                                let archive_id = menu_id.clone();
                                menu.item(
                                    PopupMenuItem::new(crate::tr!("新建会话", "New conversation"))
                                        .disabled(disabled)
                                        .on_click(window.listener_for(
                                            &owner,
                                            |this, _, window, cx| {
                                                this.new_conversation_tab(window, cx);
                                            },
                                        )),
                                )
                                .item(
                                    PopupMenuItem::new(crate::tr!("重命名…", "Rename…")).on_click(
                                        window.listener_for(&owner, move |this, _, window, cx| {
                                            this.rename_conversation(&rename_id, window, cx)
                                        }),
                                    ),
                                )
                                .item(
                                    PopupMenuItem::new(crate::tr!(
                                        "创建分支",
                                        "Branch conversation"
                                    ))
                                    .on_click(
                                        window.listener_for(&owner, move |this, _, window, cx| {
                                            this.fork_conversation(&fork_id, None, window, cx)
                                        }),
                                    ),
                                )
                                .item(
                                    PopupMenuItem::new(crate::tr!(
                                        "导出 Markdown…",
                                        "Export Markdown…"
                                    ))
                                    .on_click(
                                        window.listener_for(&owner, move |this, _, window, cx| {
                                            this.export_conversation(&export_id, window, cx)
                                        }),
                                    ),
                                )
                                .item(
                                    PopupMenuItem::new(crate::tr!("停止会话", "Stop conversation"))
                                        .disabled(!running)
                                        .on_click(window.listener_for(
                                            &owner,
                                            move |this, _, _, cx| {
                                                this.stop_conversation(&stop_id, cx)
                                            },
                                        )),
                                )
                                .item(
                                    PopupMenuItem::new(if flags.0 {
                                        crate::tr!("取消置顶", "Unpin")
                                    } else {
                                        crate::tr!("置顶", "Pin")
                                    })
                                    .disabled(flags_busy)
                                    .on_click(
                                        window.listener_for(&owner, move |this, _, window, cx| {
                                            this.set_flags(
                                                pin_id.clone(),
                                                !flags.0,
                                                flags.1,
                                                window,
                                                cx,
                                            )
                                        }),
                                    ),
                                )
                                .item(
                                    PopupMenuItem::new(if flags.1 {
                                        crate::tr!("取消归档", "Unarchive")
                                    } else {
                                        crate::tr!("归档", "Archive")
                                    })
                                    .disabled(flags_busy)
                                    .on_click(
                                        window.listener_for(&owner, move |this, _, window, cx| {
                                            this.set_flags(
                                                archive_id.clone(),
                                                flags.0,
                                                !flags.1,
                                                window,
                                                cx,
                                            )
                                        }),
                                    ),
                                )
                                .separator()
                                .item(
                                    PopupMenuItem::new(crate::tr!("关闭会话标签", "Close tab"))
                                        .disabled(disabled)
                                        .on_click(window.listener_for(
                                            &owner,
                                            move |this, _, window, cx| {
                                                this.close_conversation_tab(&close_id, window, cx);
                                            },
                                        )),
                                )
                                .item(
                                    PopupMenuItem::new(crate::tr!(
                                        "关闭其他标签",
                                        "Close other tabs"
                                    ))
                                    .disabled(
                                        disabled || owner.read(cx).open_conversations.len() <= 1,
                                    )
                                    .on_click(
                                        window.listener_for(&owner, move |this, _, window, cx| {
                                            this.close_other_conversation_tabs(
                                                &others_id, window, cx,
                                            );
                                        }),
                                    ),
                                )
                                .separator()
                                .item(
                                    PopupMenuItem::new(crate::tr!(
                                        "删除会话（需先停止）",
                                        "Delete conversation (stop first)"
                                    ))
                                    .disabled(disabled || running)
                                    .on_click(
                                        window.listener_for(&owner, move |this, _, window, cx| {
                                            this.delete_conversation_tab(
                                                delete_id.clone(),
                                                window,
                                                cx,
                                            );
                                        }),
                                    ),
                                )
                            }),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .line_height(relative(AI_LABEL_LINE_HEIGHT))
                            .child(title),
                    )
                    .suffix(
                        crate::button_accessibility::with_label(
                            Button::new(format!("ai-tab-close-{id}"))
                                .xsmall()
                                .ghost()
                                .icon(IconName::Close)
                                .tooltip(crate::tr!(
                                    "关闭标签，保留历史记录",
                                    "Close tab and keep history"
                                ))
                                .disabled(disabled),
                            crate::tr!("关闭会话标签", "Close conversation tab"),
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close_conversation_tab(&close_id, window, cx);
                                this.conversation_tab_focus.focus(window, cx);
                            },
                        )),
                    )
            }))
            .on_click(cx.listener(|this, index: &usize, window, cx| {
                if let Some(id) = this.open_conversations.get(*index).cloned() {
                    this.conversation_tab_focus.focus(window, cx);
                    this.load_conversation(id, window, cx);
                }
            }))
            .suffix(
                h_flex()
                    .gap_1()
                    .px_1()
                    .child(
                        crate::button_accessibility::with_label(
                            Button::new("ai-new-conversation")
                                .small()
                                .ghost()
                                .icon(IconName::Plus)
                                .tooltip(crate::tr!("新建会话", "New conversation"))
                                .disabled(disabled),
                            crate::tr!("新建会话", "New conversation"),
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.new_conversation_tab(window, cx)
                            }),
                        ),
                    )
                    .child(
                        Button::new("ai-conversation-history")
                            .small()
                            .ghost()
                            .text_label(crate::tr!("历史记录", "History"))
                            .tooltip(crate::tr!(
                                "打开未删除的会话",
                                "Open conversations from history"
                            ))
                            .disabled(disabled)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_history(window, cx)),
                            ),
                    ),
            );
        div()
            .id("ai-conversation-tab-region")
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .border_b_1()
            .border_color(cx.theme().border)
            .track_focus(&self.conversation_tab_focus)
            .tab_index(0)
            .capture_any_mouse_down(|event, window, _| {
                if event.button != MouseButton::Left {
                    window.prevent_default();
                }
            })
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                let delta = event.delta.pixel_delta(window.line_height());
                // Preserve native horizontal gestures; map ordinary wheel motion
                // onto the tab strip's horizontal axis, as document tabs do.
                if delta.y == px(0.) || delta.x.abs() > delta.y.abs() {
                    return;
                }
                let scroll = &this.conversation_tab_scroll;
                let max_x = scroll.max_offset().x.max(px(0.));
                if max_x == px(0.) {
                    return;
                }
                let current = scroll.offset();
                let next_x = (current.x + delta.y).clamp(-max_x, px(0.));
                if next_x != current.x {
                    scroll.set_offset(point(next_x, current.y));
                    cx.notify();
                }
                cx.stop_propagation();
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.conversation_tabs_busy(cx) || this.open_conversations.is_empty() {
                    return;
                }
                let count = this.open_conversations.len();
                let current = this
                    .open_conversations
                    .iter()
                    .position(|id| id == &this.current_id(cx))
                    .unwrap_or(0);
                let index = match event.keystroke.key.as_str() {
                    "left" => (current + count - 1) % count,
                    "right" => (current + 1) % count,
                    "home" => 0,
                    "end" => count - 1,
                    _ => return,
                };
                cx.stop_propagation();
                window.prevent_default();
                this.load_conversation(this.open_conversations[index].clone(), window, cx);
            }))
            .child(tabs)
            .into_any_element()
    }
}
