use gpui_kit::component::input::{
    Enter, InputGroup, InputGroupAddon, InputGroupAddonAlignment, InputGroupControl, Textarea,
};

use super::*;

impl Workspace {
    pub(super) fn search_input_focus_handle(&self, cx: &App) -> FocusHandle {
        if self.search_input_multiline {
            self.query.focus_handle(cx)
        } else {
            self.single_line_query.focus_handle(cx)
        }
    }

    pub(super) fn sync_single_line_search(&self, window: &mut Window, cx: &mut Context<Self>) {
        // A single-line editor strips line breaks. Display spaces while retaining
        // the original query until the user edits the collapsed field.
        let value = self.query.read(cx).value().replace(['\r', '\n'], " ");
        if self.single_line_query.read(cx).value() != value {
            self.single_line_query.update(cx, |input, cx| {
                input.replace_all(value, window, cx);
            });
        }
    }

    pub(super) fn select_search_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_single_line_search(window, cx);
        if self.search_input_multiline {
            self.query
                .update(cx, |input, cx| input.select_all(window, cx));
        } else {
            self.single_line_query
                .update(cx, |input, cx| input.select_all(window, cx));
        }
    }

    pub(super) fn capture_search_input_navigation(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.search_input_multiline
            && self.search_autocomplete_mode != SearchAutocompleteMode::History
            && matches!(key, "up" | "down")
        {
            return;
        }
        if self.navigate_search_autocomplete_by_key(key, cx) {
            cx.stop_propagation();
        }
    }

    fn toggle_search_input_multiline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_search_query_from_input(window, cx);
        self.search_input_multiline = !self.search_input_multiline;
        self.close_search_autocomplete();
        self.sync_single_line_search(window, cx);
        self.search_input_focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    pub(super) fn sync_search_query_from_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.search_input_multiline {
            let value = self.single_line_query.read(cx).value();
            if self.query.read(cx).value().replace(['\r', '\n'], " ") != value {
                self.query
                    .update(cx, |query, cx| query.replace_all(value, window, cx));
            }
        }
    }

    pub(super) fn capture_search_input_enter(
        &mut self,
        action: &Enter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Consume submits here: an unhandled, propagated textarea Enter can
        // fall through to native text insertion. Only expanded Shift+Enter
        // reaches the editor to insert a newline.
        if !self.search_input_multiline || !action.shift {
            self.sync_search_query_from_input(window, cx);
            if !self.accept_active_search_suggestion(window, cx) {
                self.start_search(window, cx);
            }
            cx.stop_propagation();
        }
    }

    pub(super) fn render_search_input(
        &self,
        height: Pixels,
        font_size: Pixels,
        line_height: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let multiline = self.search_input_multiline;
        let toggle_label = if multiline {
            crate::tr!("切换为单行搜索框", "Switch to single-line search")
        } else {
            crate::tr!("切换为多行搜索框", "Switch to multiline search")
        };
        let history_open = self.search_autocomplete_mode == SearchAutocompleteMode::History;
        let history_empty = self.search_history.is_empty();
        let input_empty = self.query.read(cx).value().is_empty();
        // Both Small controls own 2px vertical padding. Put the extra toolbar
        // clearance on their shared frame so wrapping cannot change the inset.
        let extra_padding = ((height - line_height - px(6.)) / 2.).max(px(0.));
        let control: InputGroupControl = if multiline {
            Textarea::new(&self.query)
                .accessibility_id("search-query")
                .aria_label(crate::tr!("搜索", "Search"))
                .text_size(font_size)
                .line_height(line_height)
                .min_h_0()
                .py_0()
                .into()
        } else {
            Input::new(&self.single_line_query)
                .accessibility_id("search-query")
                .aria_label(crate::tr!("搜索", "Search"))
                .text_size(font_size)
                .line_height(line_height)
                .map(|input| gpui_kit::Styled::h(input, line_height + px(4.)))
                .py(px(2.))
                .into()
        };
        InputGroup::new("search-input-group")
            .small()
            .w_full()
            .h_auto()
            .min_h(height)
            .py(extra_padding)
            .input(control)
            .addon(
                InputGroupAddon::new("search-input-mode-addon").child(
                    Button::new("search-input-mode")
                        .ghost()
                        .xsmall()
                        .icon(if multiline {
                            Icon::new(gpui_kit::assets::IconName::TextWrap)
                        } else {
                            Icon::new(IconName::Search)
                        })
                        .selected(multiline)
                        .tooltip(toggle_label)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_search_input_multiline(window, cx);
                        })),
                ),
            )
            .addon(
                InputGroupAddon::new("search-input-actions")
                    .align(InputGroupAddonAlignment::InlineEnd)
                    .gap_0p5()
                    .when(!input_empty, |addon| {
                        addon.child(
                            Button::new("search-input-clear")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Close)
                                .tooltip(crate::tr!("清空搜索内容", "Clear search text"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.query.update(cx, |input, cx| {
                                        input.replace_all("", window, cx);
                                    });
                                    this.sync_single_line_search(window, cx);
                                    this.search_input_focus_handle(cx).focus(window, cx);
                                })),
                        )
                    })
                    .child(
                        Button::new("search-history")
                            .text()
                            .icon(IconName::ChevronDown)
                            .xsmall()
                            .selected(history_open)
                            .disabled(history_empty)
                            .tooltip(if history_empty {
                                crate::tr!("暂无搜索历史", "No search history")
                            } else if history_open {
                                crate::tr!("收起搜索历史", "Hide search history")
                            } else {
                                crate::tr!("显示搜索历史", "Show search history")
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_search_history_popup(window, cx);
                            })),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use gpui_kit::test::TestWindowExt as _;

    use super::*;

    struct SearchToolbar {
        workspace: Entity<Workspace>,
        _subscription: Subscription,
    }

    impl Render for SearchToolbar {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.workspace.update(cx, |workspace, cx| {
                workspace.render_search_bar(window, cx).into_any_element()
            })
        }
    }

    #[gpui_kit::test]
    fn multiline_toggle_wraps_without_changing_query_and_preserves_editing(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::actions::init(cx);
            Workspace::init_window_registry(cx);
            crate::notifications::init(cx);
            crate::app_icon::init(cx);
        });
        let mut workspace = None;
        let handle = cx.open_window(size(px(1400.), px(400.)), |window, cx| {
            let view = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
            workspace = Some(view.clone());
            let toolbar = cx.new(|cx| SearchToolbar {
                _subscription: cx.observe(&view, |_, _, cx| cx.notify()),
                workspace: view,
            });
            Root::new(toolbar, window, cx)
        });
        let workspace = workspace.unwrap();
        let long_query = "network_timeout|数据库连接失败|".repeat(12);
        cx.update_window(handle.into(), |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace
                    .query
                    .update(cx, |query, cx| query.set_value("搜索 error", window, cx));
                workspace.sync_single_line_search(window, cx);
            });
            window.render_frame(cx);
            for (height, font_size) in [(24, 10), (30, 13), (48, 20)] {
                workspace.update(cx, |workspace, cx| {
                    workspace.app_settings.search_toolbar_height = height;
                    workspace.app_settings.search_input_font_size = font_size;
                    cx.notify();
                });
                for _ in 0..3 {
                    window.render_frame(cx);
                }
                let frame = window.find("search-input-group").bounds();
                let origin = workspace
                    .read(cx)
                    .single_line_query
                    .read(cx)
                    .input_bounds()
                    .origin;
                window.click("search-input-mode", cx);
                for _ in 0..3 {
                    window.render_frame(cx);
                }
                assert_eq!(window.find("search-input-group").bounds(), frame);
                assert_eq!(
                    workspace.read(cx).query.read(cx).input_bounds().origin,
                    origin,
                    "height={height}, font_size={font_size}"
                );
                window.click("search-input-mode", cx);
            }
            workspace.update(cx, |workspace, cx| {
                workspace.app_settings = AppSettings::default();
                workspace
                    .query
                    .update(cx, |query, cx| query.set_value("", window, cx));
                workspace.sync_single_line_search(window, cx);
                cx.notify();
            });
            window.render_frame(cx);
            let collapsed = window.find("search-input-group").bounds();
            let single_text = workspace.read(cx).single_line_query.read(cx).input_bounds();
            window.click("search-input-mode", cx);
            for _ in 0..3 {
                window.render_frame(cx);
            }
            assert_eq!(window.find("search-input-group").bounds(), collapsed);
            assert_eq!(
                workspace.read(cx).query.read(cx).input_bounds().origin,
                single_text.origin
            );
            assert_eq!(
                workspace.read(cx).query.read(cx).input_bounds().size.width,
                single_text.size.width
            );
            window.click("search-input-mode", cx);
            assert!(!workspace.read(cx).search_input_multiline);
            window.click(
                ("input", workspace.read(cx).single_line_query.entity_id()),
                cx,
            );
            window.input(&long_query, cx);
            assert_eq!(
                workspace.read(cx).single_line_query.read(cx).value(),
                long_query
            );
            window.click("search-input-mode", cx);
            for _ in 0..3 {
                window.render_frame(cx);
            }
            let expanded = window.find("search-input-group").bounds();
            assert!(workspace.read(cx).search_input_multiline);
            assert!(expanded.size.height > collapsed.size.height);
            assert_eq!(expanded.size.width, collapsed.size.width);
            assert_eq!(workspace.read(cx).query.read(cx).value(), long_query);
            assert!(
                workspace
                    .read(cx)
                    .search_input_focus_handle(cx)
                    .is_focused(window)
            );
            // Content grows the field instead of introducing horizontal scrolling.
            window.press("secondary-end", cx);
            window.render_frame(cx);
            let query = workspace.read(cx).query.read(cx);
            assert_eq!(query.scroll_offset().x, px(0.));
            assert_eq!(query.scroll_offset().y, px(0.));
            window.press("enter", cx);
            assert_eq!(workspace.read(cx).query.read(cx).value(), long_query);
            window.press("shift-enter", cx);
            assert!(workspace.read(cx).query.read(cx).value().contains('\n'));
            let edited = workspace.read(cx).query.read(cx).value();
            window.click("search-input-mode", cx);
            assert!(!workspace.read(cx).search_input_multiline);
            assert_eq!(workspace.read(cx).query.read(cx).value(), edited);
            window.render_frame(cx);
            assert_eq!(
                workspace.read(cx).single_line_query.read(cx).value(),
                edited.replace('\n', " ")
            );
            window.press("secondary-z", cx);
            assert_eq!(
                workspace.read(cx).single_line_query.read(cx).value(),
                long_query
            );
            window.click("search-input-clear", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| assert!(workspace.read(cx).query.read(cx).value().is_empty()));
        cx.update_window(handle.into(), |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.search_history = vec!["saved expression".into()];
                cx.notify();
            });
            window.render_frame(cx);
            window.click("search-input-mode", cx);
            window.click("search-history", cx);
            assert!(workspace.read(cx).search_autocomplete_mode == SearchAutocompleteMode::History);
            window.press("down", cx);
            assert_eq!(workspace.read(cx).search_suggestion_ix, Some(0));
            window.press("enter", cx);
            assert_eq!(
                workspace.read(cx).query.read(cx).value(),
                "saved expression"
            );
            assert!(workspace.read(cx).search_input_multiline);
        })
        .unwrap();
    }
}
