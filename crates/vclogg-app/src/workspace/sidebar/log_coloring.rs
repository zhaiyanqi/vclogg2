use super::*;
use gpui_component::switch::Switch;

impl SidebarState {
    pub(super) fn render_log_coloring(&self, cx: &mut Context<Self>) -> AnyElement {
        let scroll = self.scrolls[&SidebarPanelId::LogColoring].clone();
        let workspace = self.workspace.clone();
        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .child(
                h_flex().flex_none().px_3().py_2().gap_2().child(
                    Switch::new("sidebar-log-coloring-enabled")
                        .small()
                        .label(crate::tr!("启用着色", "Enable coloring"))
                        .checked(self.log_coloring_enabled)
                        .on_click(cx.listener(|this, enabled: &bool, window, cx| {
                            _ = this.workspace.update(cx, |workspace, cx| {
                                workspace.set_log_coloring_enabled(*enabled, window, cx);
                            });
                        })),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .px_3()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::tr!(
                        "适用于所有窗口 · 点击分组启用",
                        "All windows · Select a group to enable"
                    )),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .track_focus(&self.focus[&SidebarPanelId::LogColoring])
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        this.list_key(SidebarPanelId::LogColoring, event, window, cx);
                    }))
                    .child(
                        uniform_list(
                            "sidebar-log-coloring-groups",
                            self.log_coloring.groups.len(),
                            cx.processor(|this, range: Range<usize>, _, cx| {
                                range
                                    .map(|ix| {
                                        let group = &this.log_coloring.groups[ix];
                                        let id = group.id.clone();
                                        let selected =
                                            group.id == this.log_coloring.active_group_id;
                                        let focused = this
                                            .selected
                                            .get(&SidebarPanelId::LogColoring)
                                            .map_or(selected, |focused| focused == &id);
                                        crate::log_coloring_row::group_button(
                                            format!("sidebar-coloring-group-{id}"),
                                            group.name.clone(),
                                            group.rules.len(),
                                            focused,
                                            selected && this.log_coloring_enabled,
                                            cx,
                                        )
                                        .on_click(cx.listener(move |this, event, window, cx| {
                                            if !crate::log_coloring_row::is_activation(event) {
                                                return;
                                            }
                                            this.selected
                                                .insert(SidebarPanelId::LogColoring, id.clone());
                                            cx.notify();
                                            _ = this.workspace.update(cx, |workspace, cx| {
                                                workspace.select_log_coloring_group(
                                                    id.clone(),
                                                    window,
                                                    cx,
                                                );
                                            });
                                        }))
                                        .into_any_element()
                                    })
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .track_scroll(&scroll)
                        // Padding belongs inside the list's clipping bounds, not around the list.
                        .p_1()
                        .size_full(),
                    )
                    .vertical_scrollbar(&scroll),
            )
            .child(
                h_flex().flex_none().px_3().py_2().child(
                    Button::new("sidebar-configure-highlights")
                        .small()
                        .disabled(self.log_coloring_saving)
                        .label(crate::tr!("配置高亮…", "Highlight settings…"))
                        .on_click(move |_, window, cx| {
                            _ = workspace.update(cx, |workspace, cx| {
                                workspace.open_color_labels_dialog(window, cx)
                            });
                        }),
                ),
            )
            .into_any_element()
    }
}
