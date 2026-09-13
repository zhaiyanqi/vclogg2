use super::*;

pub(super) struct LogGroupDraft {
    pub id: String,
    pub name: Entity<InputState>,
    pub preset: Option<String>,
    pub example: String,
    pub rows: Vec<LogLevelDraft>,
    pub scroll: ScrollHandle,
    pub horizontal_scroll: ScrollHandle,
    pub preview_rule_id: Option<String>,
    pub resolved: std::sync::Arc<crate::color_labels::ResolvedLogLevelRules>,
    resolved_rules: Vec<LogLevelColorRule>,
    _subscription: Subscription,
}

impl ColorLabelsDialog {
    pub(super) fn push_group(
        &mut self,
        group: LogColoringGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::tr!("分组名称", "Group name"))
                .default_value(group.name)
                .context_menu(false)
        });
        let subscription = Self::subscribe_input(&name, cx);
        self.groups.push(LogGroupDraft {
            id: group.id,
            name,
            preset: group.preset,
            example: group.example,
            rows: Vec::new(),
            scroll: ScrollHandle::new(),
            horizontal_scroll: ScrollHandle::new(),
            preview_rule_id: None,
            resolved: Default::default(),
            resolved_rules: Vec::new(),
            _subscription: subscription,
        });
        self.selected_group = self.groups.len() - 1;
        for rule in group.rules {
            self.push_log_level_rule(rule, window, cx);
        }
    }

    fn add_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.push_group(
            LogColoringGroup {
                id: uuid::Uuid::new_v4().to_string(),
                name: crate::tr!("新分组", "New group").into(),
                preset: None,
                example: String::new(),
                rules: Vec::new(),
            },
            window,
            cx,
        );
        self.groups[self.selected_group]
            .name
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        cx.notify();
    }

    fn duplicate_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut group = self.group_snapshot(&self.groups[self.selected_group], cx);
        group.id = uuid::Uuid::new_v4().to_string();
        group.name = crate::tr_args!("{} 副本", "{} copy", group.name);
        // A copy remains associated with its sample format, but cannot reset another group.
        for rule in &mut group.rules {
            rule.id = uuid::Uuid::new_v4().to_string();
        }
        self.push_group(group, window, cx);
        self.refresh_rule_previews(cx);
        cx.notify();
    }

    fn delete_group(&mut self, cx: &mut Context<Self>) {
        if self.groups.len() <= 1 {
            return;
        }
        let removed = self.groups.remove(self.selected_group);
        self.selected_group = self.selected_group.min(self.groups.len() - 1);
        if self.active_group_id == removed.id {
            self.active_group_id = self.groups[0].id.clone();
            self.highlight_log_levels = false;
        }
        cx.notify();
    }

    fn restore_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.selected_group;
        let Some(mut preset) = crate::log_coloring::presets()
            .into_iter()
            .find(|preset| preset.preset == self.groups[selected].preset)
        else {
            return;
        };
        preset.id = self.groups[selected].id.clone();
        for rule in &mut preset.rules {
            rule.id = uuid::Uuid::new_v4().to_string();
        }
        preset.name = self.groups[selected].name.read(cx).value().to_string();
        self.push_group(preset, window, cx);
        let restored = self.groups.pop().expect("just added group");
        self.groups[selected] = restored;
        self.selected_group = selected;
        self.refresh_rule_previews(cx);
        cx.notify();
    }

    fn rule_snapshot(row: &LogLevelDraft, cx: &gpui::App) -> LogLevelColorRule {
        let (text_color, text_alpha) =
            picker_value(&row.text_color, String::new(), cx).unwrap_or((0, 255));
        let (background_color, background_alpha) =
            picker_value(&row.background_color, String::new(), cx).unwrap_or((0, 0));
        LogLevelColorRule {
            id: row.id.clone(),
            keyword: row.keyword.read(cx).value().trim().to_string(),
            match_kind: row.match_kind,
            keyword_only: row.keyword_only,
            text_color,
            text_alpha,
            background_color,
            background_alpha,
        }
    }

    fn group_snapshot(&self, group: &LogGroupDraft, cx: &gpui::App) -> LogColoringGroup {
        LogColoringGroup {
            id: group.id.clone(),
            name: group.name.read(cx).value().to_string(),
            preset: group.preset.clone(),
            example: group.example.clone(),
            rules: group
                .rows
                .iter()
                .map(|row| Self::rule_snapshot(row, cx))
                .collect(),
        }
    }

    pub(super) fn refresh_rule_previews(&mut self, cx: &gpui::App) {
        for group in &mut self.groups {
            let mut rules = Vec::with_capacity(group.rows.len());
            for row in &mut group.rows {
                let rule = Self::rule_snapshot(row, cx);
                if rule != row.resolved_config {
                    row.match_error = if row.match_kind == LogRuleMatch::Regex {
                        regex::Regex::new(&rule.keyword)
                            .err()
                            .map(|error| error.to_string())
                    } else {
                        None
                    };
                    row.resolved =
                        crate::color_labels::resolve_log_level_rules(std::slice::from_ref(&rule));
                    row.resolved_config = rule.clone();
                }
                rules.push(rule);
            }
            if rules != group.resolved_rules {
                group.resolved = crate::color_labels::resolve_log_level_rules(&rules);
                group.resolved_rules = rules;
            }
        }
    }

    pub(super) fn rule_example(&self, row: &LogLevelDraft, cx: &gpui::App) -> String {
        let group = &self.groups[self.selected_group];
        LogColoringGroup {
            id: group.id.clone(),
            name: String::new(),
            preset: group.preset.clone(),
            example: group.example.clone(),
            rules: Vec::new(),
        }
        .example_for(&Self::rule_snapshot(row, cx))
    }

    fn render_group_preview(&self, cx: &gpui::App) -> impl IntoElement {
        let group = &self.groups[self.selected_group];
        let row = group
            .preview_rule_id
            .as_ref()
            .and_then(|id| group.rows.iter().find(|row| &row.id == id))
            .or_else(|| group.rows.first());
        let text = row
            .map(|row| self.rule_example(row, cx))
            .unwrap_or_else(|| {
                crate::tr!(
                    "添加规则以预览日志着色",
                    "Add a rule to preview log coloring"
                )
                .into()
            });
        let style = group.resolved.matching_style(&text);
        let highlights = group
            .resolved
            .matching_keyword_ranges(&text)
            .into_iter()
            .map(|(range, style)| {
                (
                    range,
                    HighlightStyle {
                        color: Some(style.foreground),
                        background_color: Some(style.background),
                        ..Default::default()
                    },
                )
            })
            .collect::<Vec<_>>();
        v_flex()
            .flex_none()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .child(crate::tr!("分组预览", "Group preview")),
            )
            .child(
                div()
                    .min_w_0()
                    .p_2()
                    .text_sm()
                    .bg(cx.theme().background)
                    .text_color(cx.theme().foreground)
                    .when_some(style, |view, style| {
                        view.bg(style.background).text_color(style.foreground)
                    })
                    .child(StyledText::new(text).with_highlights(highlights)),
            )
    }

    pub(super) fn render_log_levels(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = &self.groups[self.selected_group];
        let editor = cx.entity();
        h_flex()
            .items_stretch()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .gap_3()
            .child(
                v_flex()
                    .w(rems(12.))
                    .min_w(rems(9.))
                    .max_w(rems(16.))
                    .flex_none()
                    .min_h_0()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                Button::new("coloring-add-group")
                                    .small()
                                    .disabled(self.saving)
                                    .icon(IconName::Plus)
                                    .label(crate::tr!("新增分组", "Add group"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.add_group(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("coloring-add-preset")
                                    .small()
                                    .disabled(self.saving)
                                    .label(crate::tr!("添加预置…", "Add preset…"))
                                    .dropdown_menu(move |mut menu, _, _| {
                                        for preset in crate::log_coloring::presets() {
                                            let editor = editor.clone();
                                            menu = menu.item(
                                                PopupMenuItem::new(preset.name.clone()).on_click(
                                                    move |_, window, cx| {
                                                        editor.update(cx, |this, cx| {
                                                            if this.saving {
                                                                return;
                                                            }
                                                            let mut group = preset.clone();
                                                            group.id =
                                                                uuid::Uuid::new_v4().to_string();
                                                            for rule in &mut group.rules {
                                                                rule.id = uuid::Uuid::new_v4()
                                                                    .to_string();
                                                            }
                                                            this.push_group(group, window, cx);
                                                            this.refresh_rule_previews(cx);
                                                            cx.notify();
                                                        });
                                                    },
                                                ),
                                            );
                                        }
                                        menu
                                    }),
                            ),
                    )
                    .child(
                        v_flex()
                            .id("coloring-groups")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.group_scroll)
                            .gap_1()
                            .children(self.groups.iter().map(|group| {
                                let id = group.id.clone();
                                crate::log_coloring_row::group_button(
                                    format!("coloring-group-{id}"),
                                    group.name.read(cx).value(),
                                    group.rows.len(),
                                    group.id == selected.id,
                                    self.highlight_log_levels && group.id == self.active_group_id,
                                    cx,
                                )
                                .tooltip(group.name.read(cx).value())
                                .disabled(self.saving)
                                .on_click(cx.listener(
                                    move |this, event, _, cx| {
                                        if !crate::log_coloring_row::is_activation(event) {
                                            return;
                                        }
                                        if let Some(ix) =
                                            this.groups.iter().position(|group| group.id == id)
                                        {
                                            this.selected_group = ix;
                                            cx.notify();
                                        }
                                    },
                                ))
                            }))
                            .map(|list| {
                                color_rule_list(
                                    list,
                                    "coloring-groups-scrollbar",
                                    &self.group_scroll,
                                )
                            }),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .gap_2()
                    .overflow_hidden()
                    .child(
                        h_flex()
                            // Input's focus ring extends outside its border; keep it inside the clip.
                            .p_1()
                            .flex_none()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                div()
                                    .w(rems(12.))
                                    .capture_any_mouse_down(
                                        crate::log_coloring_row::ignore_secondary_mouse_down,
                                    )
                                    .child(
                                        Input::new(&selected.name).small().disabled(self.saving),
                                    ),
                            )
                            .child(
                                Button::new("coloring-use-group")
                                    .small()
                                    .disabled(self.saving || selected.id == self.active_group_id)
                                    .label(if selected.id == self.active_group_id {
                                        crate::tr!("当前分组", "Current group")
                                    } else {
                                        crate::tr!("设为当前分组", "Use this group")
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.active_group_id =
                                            this.groups[this.selected_group].id.clone();
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("coloring-copy-group")
                                    .small()
                                    .disabled(self.saving)
                                    .label(crate::tr!("复制", "Duplicate"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.duplicate_group(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("coloring-reset-group")
                                    .small()
                                    .disabled(self.saving || selected.preset.is_none())
                                    .label(crate::tr!("恢复预置", "Reset preset"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.restore_group(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("coloring-delete-group")
                                    .small()
                                    .disabled(self.saving || self.groups.len() <= 1)
                                    .label(crate::tr!("删除分组", "Delete group"))
                                    .on_click(cx.listener(|this, _, _, cx| this.delete_group(cx))),
                            ),
                    )
                    .child(self.render_log_rules(cx))
                    .child(self.render_group_preview(cx)),
            )
    }
}
