use super::*;

impl SettingsDialog {
    pub(super) fn render_reading(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_6()
            .pb_4()
            .child(self.render_typography(cx))
            .child(self.render_line_numbers(cx))
            .child(self.render_log_colors(cx))
            .child(self.render_scrolling(cx))
            .child(section(
                crate::tr!("双击选词", "Double-click word selection"),
                None,
                [self.setting_row(
                    "words",
                    crate::tr!("分词边界字符", "Word-boundary characters"),
                    Some(crate::tr!(
                        "空白和以下字符会断开单词，最多可设 256 个 Unicode 字符。",
                        "Whitespace and these characters end a word. Up to 256 Unicode characters."
                    )),
                    div().id("settings-word-boundary-characters").w_48().child(
                        Input::new(&self.word_boundary_characters)
                            .id("settings-word-boundary-input")
                            .w_full(),
                    ),
                    cx,
                )],
                cx,
            ))
            .into_any_element()
    }

    fn render_typography(&self, cx: &mut Context<Self>) -> AnyElement {
        let font = Select::new(&self.font_family)
            .small()
            .w_48()
            .disabled(self.font_loading_task.is_some())
            .search_placeholder(crate::tr!("搜索字体…", "Search fonts…"))
            .placeholder(crate::tr_args!(
                "{}（不可用）",
                "{} (unavailable)",
                self.draft
                    .log_font_family
                    .family_name()
                    .unwrap_or(cx.theme().mono_font_family.as_ref())
            ));
        let wrap = crate::tr!("新文件默认换行", "Wrap new files by default");
        section(
            crate::tr!("排版", "Typography"),
            None,
            [
                self.setting_row(
                    "font",
                    crate::tr!("日志字体", "Log font"),
                    Some(crate::tr!(
                        "用于正文、行号和搜索结果。",
                        "Used for log text, line numbers, and results."
                    )),
                    font,
                    cx,
                ),
                self.setting_row(
                    "size",
                    crate::tr!("日志字号", "Log font size"),
                    Some(crate::tr!(
                        "在日志区也可按住 Ctrl 滚动调节。",
                        "You can also hold Ctrl and scroll over a log."
                    )),
                    slider(&self.font_size, " px", cx),
                    cx,
                ),
                self.setting_row(
                    "spacing",
                    crate::tr!("日志行距", "Log line spacing"),
                    Some(crate::tr!(
                        "每行额外留白，独立于字号。",
                        "Extra space per line, independent of font size."
                    )),
                    slider(&self.line_spacing, " px", cx),
                    cx,
                ),
                self.setting_row(
                    "wrap",
                    wrap,
                    Some(crate::tr!(
                        "仅用于首次打开。已有文件恢复上次的换行状态。",
                        "Only on first open. Returning files restore their last wrap setting."
                    )),
                    Switch::new("settings-default-word-wrap")
                        .small()
                        .accessibility_label(wrap)
                        .checked(self.draft.default_word_wrap)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.draft.default_word_wrap = *checked;
                            Self::draft_changed(cx);
                        })),
                    cx,
                ),
            ],
            cx,
        )
        .into_any_element()
    }

    fn render_line_numbers(&self, cx: &mut Context<Self>) -> AnyElement {
        let numbers = crate::tr!("显示行号", "Show line numbers");
        let enabled = self.draft.default_show_line_numbers;
        let mut rows = vec![
            self.setting_row(
                "numbers",
                numbers,
                Some(crate::tr!(
                    "在正文和搜索结果中显示固定行号列。",
                    "Show a fixed gutter in logs and search results."
                )),
                Switch::new("settings-show-line-numbers")
                    .small()
                    .accessibility_label(numbers)
                    .checked(enabled)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.draft.default_show_line_numbers = *checked;
                        Self::draft_changed(cx);
                    })),
                cx,
            ),
        ];
        if enabled
            || ["number-width", "number-ink", "number-bg", "number-lines"]
                .into_iter()
                .any(|key| self.location.is(key))
        {
            rows.push(self.setting_row(
                "number-width",
                crate::tr!("行号栏宽度", "Line-number column width"),
                None,
                slider(&self.line_number_width, " px", cx),
                cx,
            ));
            let ink = crate::tr!("行号文字颜色", "Line-number text color");
            let bg = crate::tr!("行号背景色", "Line-number background");
            rows.push(self.setting_row(
                "number-ink",
                ink,
                None,
                self.reading_color(
                    &self.line_number_text_color,
                    self.line_number_text_color_custom,
                    ink,
                    "settings-reset-line-number-text-color",
                    Self::reset_line_number_text_color,
                    cx,
                ),
                cx,
            ));
            rows.push(self.setting_row(
                "number-bg",
                bg,
                None,
                self.reading_color(
                    &self.line_number_background_color,
                    self.line_number_background_color_custom,
                    bg,
                    "settings-reset-line-number-background-color",
                    Self::reset_line_number_background_color,
                    cx,
                ),
                cx,
            ));
            let separators = crate::tr!("显示行号行间分隔线", "Show line-number separators");
            rows.push(
                self.setting_row(
                    "number-lines",
                    separators,
                    None,
                    Switch::new("settings-line-number-row-separators")
                        .small()
                        .accessibility_label(separators)
                        .disabled(!enabled)
                        .checked(self.draft.show_line_number_row_separators)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.draft.show_line_number_row_separators = *checked;
                            Self::draft_changed(cx);
                        })),
                    cx,
                ),
            );
        }
        let separators = crate::tr!("日志分隔线", "Log separators");
        rows.push(
            self.setting_row(
                "row-lines",
                separators,
                Some(crate::tr!(
                    "在相邻日志正文行之间显示细线。",
                    "Draw a fine line between adjacent log rows."
                )),
                Switch::new("settings-row-separators")
                    .small()
                    .accessibility_label(separators)
                    .checked(self.draft.default_show_row_separators)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.draft.default_show_row_separators = *checked;
                        Self::draft_changed(cx);
                    })),
                cx,
            ),
        );
        section(
            crate::tr!("行号与分隔线", "Line numbers and separators"),
            None,
            rows,
            cx,
        )
        .into_any_element()
    }

    fn reading_color(
        &self,
        picker: &Entity<ColorPickerState>,
        custom: bool,
        label: &'static str,
        reset_id: &'static str,
        reset: fn(&mut Self, &mut Window, &mut Context<Self>),
        cx: &Context<Self>,
    ) -> gpui_kit::Div {
        let value = if custom {
            picker
                .read(cx)
                .value()
                .map(|color| color.alpha(1.).to_hex())
                .unwrap_or_else(|| crate::tr!("未选择", "Not selected").to_string())
        } else {
            crate::tr!("跟随主题", "Follow theme").to_owned()
        };
        h_flex()
            .gap_2()
            .justify_end()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(value),
            )
            .child(ColorPicker::new(picker).small().label(label))
            .child(
                Button::new(reset_id)
                    .small()
                    .ghost()
                    .label(crate::tr!("恢复默认", "Reset"))
                    .accessibility_label(crate::tr_args!("恢复{}默认值", "Reset {}", label))
                    .disabled(!custom)
                    .on_click(cx.listener(move |this, _, window, cx| reset(this, window, cx))),
            )
    }

    fn render_log_colors(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Self::current_log_color_theme(cx);
        let colors = self.log_colors(theme);
        let theme_name = match theme {
            LogColorTheme::Light => crate::tr!("晨雾浅色", "Morning mist"),
            LogColorTheme::Dark => crate::tr!("深海夜色", "Deep sea"),
        };
        let text = crate::tr!("日志文字颜色", "Log text color");
        let bg = crate::tr!("日志背景色", "Log background");
        section(
            crate::tr!("日志配色", "Log colors"),
            Some(crate::tr_args!(
                "当前主题为“{}”。浅色和深色配色分别保存。",
                "Editing “{}”. Light and dark colors are saved separately.",
                theme_name
            )),
            [
                self.setting_row(
                    "log-ink",
                    text,
                    None,
                    self.reading_color(
                        &colors.text,
                        colors.text_custom,
                        text,
                        "settings-reset-log-text-color",
                        Self::reset_log_text_color,
                        cx,
                    ),
                    cx,
                ),
                self.setting_row(
                    "log-bg",
                    bg,
                    None,
                    self.reading_color(
                        &colors.background,
                        colors.background_custom,
                        bg,
                        "settings-reset-log-background-color",
                        Self::reset_log_background_color,
                        cx,
                    ),
                    cx,
                ),
            ],
            cx,
        )
        .id("settings-log-colors-section")
        .into_any_element()
    }

    fn render_scrolling(&self, cx: &mut Context<Self>) -> AnyElement {
        let horizontal = crate::tr!("显示横向滚动条", "Show horizontal scrollbar");
        let by_line = crate::tr!("按完整日志行滚动", "Scroll by complete log lines");
        let mut rows = vec![
            self.setting_row(
                "horizontal",
                horizontal,
                None,
                Switch::new("settings-horizontal-scrollbar")
                    .small()
                    .accessibility_label(horizontal)
                    .checked(self.draft.show_horizontal_scrollbar)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.draft.show_horizontal_scrollbar = *checked;
                        Self::draft_changed(cx);
                    })),
                cx,
            ),
            self.setting_row(
                "line-scroll",
                by_line,
                None,
                Switch::new("settings-scroll-by-line")
                    .small()
                    .accessibility_label(by_line)
                    .checked(self.draft.scroll_by_line)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.draft.scroll_by_line = *checked;
                        Self::draft_changed(cx);
                    })),
                cx,
            ),
        ];
        if self.draft.scroll_by_line
            || self.location.is("scroll-count")
            || self.location.is("wrap-scroll")
        {
            rows.push(self.setting_row(
                "scroll-count",
                crate::tr!("每次滚动行数", "Lines per scroll"),
                Some(crate::tr!(
                    "每次滚轮输入移动 1–100 行。",
                    "Move 1–100 logical lines per wheel input."
                )),
                slider(&self.scroll_lines, "", cx),
                cx,
            ));
            let wrap = crate::tr!(
                "自动换行时仍按完整日志行滚动",
                "Scroll by complete log lines while wrapping"
            );
            rows.push(
                self.setting_row(
                    "wrap-scroll",
                    wrap,
                    None,
                    Switch::new("settings-scroll-by-line-word-wrap")
                        .small()
                        .accessibility_label(wrap)
                        .checked(self.draft.scroll_by_line_when_word_wrap)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.draft.scroll_by_line_when_word_wrap = *checked;
                            Self::draft_changed(cx);
                        })),
                    cx,
                ),
            );
        }
        if !self.draft.scroll_by_line
            || !self.draft.scroll_by_line_when_word_wrap
            || self.location.is("pixel")
        {
            rows.push(self.setting_row(
                "pixel",
                crate::tr!("像素滚动距离", "Pixel scroll distance"),
                Some(crate::tr!(
                    "按设备输入距离的 1%–400% 调节。",
                    "Scale the device scroll distance from 1% to 400%."
                )),
                slider(&self.scroll_percent, "%", cx),
                cx,
            ));
        }
        section(crate::tr!("滚动", "Scrolling"), None, rows, cx)
            .id("settings-scroll-section")
            .into_any_element()
    }
}
