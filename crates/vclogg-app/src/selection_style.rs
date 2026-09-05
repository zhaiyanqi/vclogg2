//! Application-owned selection appearance. Persisted values contain no GPUI types.
use gpui::{App, Div, Hsla, Pixels, Styled as _, div, prelude::FluentBuilder as _, px};
use gpui_component::{
    ActiveTheme as _,
    theme::{ThemeMode, try_parse_color},
};
use serde::{Deserialize, Serialize};

use crate::ui_theme;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SelectionStyles {
    pub(crate) light: SelectionThemeStyle,
    pub(crate) dark: SelectionThemeStyle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SelectionBorder {
    None,
    #[default]
    Horizontal,
    Frame,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SelectionWeight {
    #[default]
    Thin,
    Medium,
    Thick,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SelectionRadius {
    #[default]
    Square,
    Small,
    Medium,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SelectionThemeStyle {
    pub(crate) row_background: Option<String>,
    pub(crate) row_border_color: Option<String>,
    pub(crate) row_border: SelectionBorder,
    pub(crate) row_weight: SelectionWeight,
    pub(crate) row_radius: SelectionRadius,
    pub(crate) text_background: Option<String>,
    pub(crate) text_foreground: Option<String>,
    pub(crate) text_underline: bool,
    pub(crate) dim_inactive: bool,
    pub(crate) inactive_opacity: u8,
}

impl Default for SelectionThemeStyle {
    fn default() -> Self {
        Self {
            row_background: None,
            row_border_color: None,
            row_border: SelectionBorder::default(),
            row_weight: SelectionWeight::default(),
            row_radius: SelectionRadius::default(),
            text_background: None,
            text_foreground: None,
            text_underline: false,
            dim_inactive: false,
            inactive_opacity: 50,
        }
    }
}

impl SelectionStyles {
    pub(crate) fn theme(&self, dark: bool) -> &SelectionThemeStyle {
        if dark { &self.dark } else { &self.light }
    }

    pub(crate) fn theme_mut(&mut self, dark: bool) -> &mut SelectionThemeStyle {
        if dark {
            &mut self.dark
        } else {
            &mut self.light
        }
    }

    pub(crate) fn resolve(&self, active: bool, cx: &App) -> ResolvedSelectionStyle {
        let dark = ui_theme::is_dark(cx);
        self.theme(dark).resolve(dark, active)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ResolvedTextSelectionStyle {
    pub(crate) background: Hsla,
    pub(crate) foreground: Option<Hsla>,
    pub(crate) underline: bool,
    /// Keep the historical translucent overlay exactly until text styling is customized.
    pub(crate) legacy_overlay: bool,
    pub(crate) opacity: f32,
}

#[derive(Clone, Copy)]
pub(crate) struct ResolvedSelectionStyle {
    pub(crate) row_background: Hsla,
    border_color: Hsla,
    border: SelectionBorder,
    border_width: Pixels,
    radius: SelectionRadius,
    pub(crate) text: ResolvedTextSelectionStyle,
}

impl SelectionThemeStyle {
    pub(crate) fn resolve(&self, dark: bool, active: bool) -> ResolvedSelectionStyle {
        let colors = ui_theme::palette_for_mode(if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        });
        let opacity = if self.dim_inactive && !active {
            f32::from(self.inactive_opacity.clamp(10, 100)) / 100.
        } else {
            1.
        };
        let color = |custom: &Option<String>, default: Hsla| {
            custom
                .as_deref()
                .and_then(|s| try_parse_color(s).ok())
                .unwrap_or(default)
        };
        ResolvedSelectionStyle {
            row_background: color(&self.row_background, colors.row_selected).opacity(opacity),
            border_color: color(&self.row_border_color, colors.row_selected_border)
                .opacity(opacity),
            border: self.row_border,
            border_width: px(match self.row_weight {
                SelectionWeight::Thin => 1.,
                SelectionWeight::Medium => 2.,
                SelectionWeight::Thick => 3.,
            }),
            radius: self.row_radius,
            text: ResolvedTextSelectionStyle {
                background: color(
                    &self.text_background,
                    colors.primary.opacity(if dark { 0.34 } else { 0.26 }),
                )
                .opacity(opacity),
                foreground: self
                    .text_foreground
                    .as_deref()
                    .and_then(|s| try_parse_color(s).ok()),
                underline: self.text_underline,
                legacy_overlay: self.text_background.is_none()
                    && self.text_foreground.is_none()
                    && !self.text_underline,
                opacity,
            },
        }
    }
}

impl ResolvedSelectionStyle {
    /// Borders and fill are paint-only children, shared by actual rows and the preview.
    pub(crate) fn row_overlay(self, top: bool, bottom: bool, cx: &App) -> Div {
        let radius = match self.radius {
            SelectionRadius::Square => px(0.),
            SelectionRadius::Small => cx.theme().radius / 2.,
            SelectionRadius::Medium => cx.theme().radius,
        };
        div()
            .absolute()
            .inset_0()
            .bg(self.row_background)
            .when(top, |overlay| overlay.rounded_t(radius))
            .when(bottom, |overlay| overlay.rounded_b(radius))
            .when(self.border != SelectionBorder::None, |overlay| {
                overlay
                    .when(top, |overlay| overlay.border_t(self.border_width))
                    .when(bottom, |overlay| overlay.border_b(self.border_width))
                    .when(self.border == SelectionBorder::Frame, |overlay| {
                        overlay
                            .border_l(self.border_width)
                            .border_r(self.border_width)
                    })
                    .border_color(self.border_color)
            })
    }
}
