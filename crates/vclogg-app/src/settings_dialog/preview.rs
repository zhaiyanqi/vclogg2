//! A passive sample shared by the reading preferences and highlight editor.
use std::sync::Arc;

use super::*;
use crate::{
    color_labels::ResolvedLogLevelRules,
    keyword_match_style::KeywordMatchStyles,
    log_table::{log_line_height, log_line_number_cell, log_row_separator_overlay},
    selectable_log_text::{LogText, SelectableLogText},
    selection_style::ResolvedSelectionStyle,
    ui_theme,
};
use gpui_kit::{HighlightStyle, StyledText, combine_highlights, px, rems};

pub(crate) struct PreviewStyles {
    pub(crate) dark: bool,
    pub(crate) selection: ResolvedSelectionStyle,
    pub(crate) matches: KeywordMatchStyles,
    pub(crate) levels: Option<Arc<ResolvedLogLevelRules>>,
    pub(crate) label: Option<HighlightStyle>,
}

impl SettingsDialog {
    pub(super) fn render_log_preview(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(editor) = &self.highlight_editor else {
            return div().into_any_element();
        };
        let styles = editor
            .read(cx)
            .preview_styles(self.active_category == SettingsCategory::Highlight, cx);
        let dark = styles.dark;
        let colors = ui_theme::palette_for_mode(if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        });
        let draft = self.log_colors(if dark {
            LogColorTheme::Dark
        } else {
            LogColorTheme::Light
        });
        let color = |picker: &Entity<ColorPickerState>, custom, default| {
            picker
                .read(cx)
                .value()
                .filter(|_| custom)
                .map(|value| value.alpha(1.))
                .unwrap_or(default)
        };
        let text_color = color(&draft.text, draft.text_custom, colors.log_text);
        let background = color(
            &draft.background,
            draft.background_custom,
            colors.log_background,
        );
        let number_color = color(
            &self.line_number_text_color,
            self.line_number_text_color_custom,
            colors.line_number,
        );
        let number_bg = color(
            &self.line_number_background_color,
            self.line_number_background_color_custom,
            colors.line_number_background,
        );
        let size = self.font_size.read(cx).value().start().round() as u16;
        let height = log_line_height(
            size,
            self.line_spacing.read(cx).value().start().round() as u16,
        );
        let number_width = self.line_number_width.read(cx).value().start().round();
        let font = self
            .font_family
            .read(cx)
            .selected_value()
            .unwrap_or(&self.draft.log_font_family)
            .family_name()
            .map_or_else(
                || cx.theme().mono_font_family.clone(),
                |s| s.to_owned().into(),
            );
        let samples = [
            "10:24:01 INFO  gateway connected",
            "10:24:02 WARN  retry scheduled",
            "10:24:05 ERROR request timeout",
            "10:24:06 INFO  worker ready",
        ];
        let rows = samples
            .into_iter()
            .enumerate()
            .map(|(index, sample)| {
                let level = styles
                    .levels
                    .as_ref()
                    .and_then(|rules| rules.matching_style(sample));
                let rule_ranges = styles
                    .levels
                    .as_ref()
                    .into_iter()
                    .flat_map(|rules| rules.matching_keyword_ranges(sample))
                    .map(|(range, style)| {
                        (
                            range,
                            HighlightStyle {
                                color: Some(style.foreground),
                                background_color: Some(style.background),
                                ..Default::default()
                            },
                        )
                    });
                let terms = [
                    ("gateway", styles.label),
                    (
                        "timeout",
                        self.draft
                            .highlight_matches
                            .then(|| styles.matches.style(dark, false).resolve(dark, false)),
                    ),
                    (
                        "retry",
                        Some(styles.matches.style(dark, true).resolve(dark, true)),
                    ),
                ];
                let hits = terms.into_iter().filter_map(|(term, style)| {
                    let start = sample.find(term)?;
                    Some((start..start + term.len(), style?))
                });
                let selected = index == 1;
                let highlights = styles
                    .selection
                    .row_highlights(selected, combine_highlights(rule_ranges, hits).collect());
                let text = LogText::new(sample.into());
                let handle = self.preview_selections.handle(index, &text, window, cx);
                let styled =
                    StyledText::new(text.display().clone()).with_highlights(highlights.clone());
                let selected_text = sample
                    .find("worker")
                    .map_or(0..0, |start| start..start + "worker".len());
                let text = SelectableLogText::new(
                    handle,
                    index as u64,
                    text,
                    styled,
                    styles.selection.text.background,
                )
                .selection_style(styles.selection.text, highlights)
                .preview_range(selected_text);
                let cell = div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .px_2()
                    .when_some(level, |cell, level| {
                        cell.bg(level.background).text_color(level.foreground)
                    })
                    .when(selected, |cell| {
                        cell.when_some(styles.selection.row_foreground, |cell, color| {
                            cell.text_color(color)
                        })
                        .child(styles.selection.row_overlay(true, true, cx))
                    })
                    .child(text)
                    .when(
                        self.draft.default_show_row_separators && index + 1 < samples.len(),
                        |cell| cell.child(log_row_separator_overlay(false, cx)),
                    );
                h_flex()
                    .flex_none()
                    .h(height)
                    .line_height(height)
                    .when(self.draft.default_show_line_numbers, |row| {
                        row.child(
                            log_line_number_cell(
                                127 + index,
                                size,
                                height,
                                number_color,
                                number_bg,
                                self.draft.show_line_number_row_separators
                                    && index + 1 < samples.len(),
                                cx,
                            )
                            .w(px(number_width))
                            .h_full()
                            .flex_none(),
                        )
                    })
                    .child(cell)
            })
            .collect::<Vec<_>>();
        v_flex()
            .id("settings-log-preview")
            .mx_4()
            .flex_none()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::tr!("日志预览", "Log preview")),
            )
            .child(
                v_flex()
                    .id("settings-preview-lines")
                    .max_h(rems(9.))
                    .overflow_y_scrollbar()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .font_family(font)
                    .text_size(px(f32::from(size)))
                    .text_color(text_color)
                    .bg(background)
                    .children(rows),
            )
            .into_any_element()
    }
}
