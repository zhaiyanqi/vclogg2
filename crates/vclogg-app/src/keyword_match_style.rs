//! Persisted keyword appearance, resolved only in the presentation layer.
use gpui::{FontStyle, FontWeight, HighlightStyle, UnderlineStyle, px};
use gpui_component::theme::{ThemeMode, try_parse_color};
use serde::{Deserialize, Serialize};

use crate::ui_theme;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct KeywordMatchStyles {
    pub(crate) light: KeywordMatchThemeStyle,
    pub(crate) dark: KeywordMatchThemeStyle,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct KeywordMatchThemeStyle {
    pub(crate) search: KeywordMatchStyle,
    pub(crate) quick_find: KeywordMatchStyle,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct KeywordMatchStyle {
    pub(crate) foreground: Option<String>,
    pub(crate) background: Option<String>,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    pub(crate) underline: bool,
}

impl KeywordMatchStyles {
    pub(crate) fn theme_mut(&mut self, dark: bool) -> &mut KeywordMatchThemeStyle {
        if dark {
            &mut self.dark
        } else {
            &mut self.light
        }
    }

    pub(crate) fn style(&self, dark: bool, quick_find: bool) -> &KeywordMatchStyle {
        let theme = if dark { &self.dark } else { &self.light };
        if quick_find {
            &theme.quick_find
        } else {
            &theme.search
        }
    }

    pub(crate) fn style_mut(&mut self, dark: bool, quick_find: bool) -> &mut KeywordMatchStyle {
        let theme = self.theme_mut(dark);
        if quick_find {
            &mut theme.quick_find
        } else {
            &mut theme.search
        }
    }
}

impl KeywordMatchStyle {
    pub(crate) fn resolve(&self, dark: bool, quick_find: bool) -> HighlightStyle {
        let palette = ui_theme::palette_for_mode(if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        });
        let (foreground, background) = if quick_find {
            (palette.quick_find_foreground, palette.quick_find)
        } else {
            (palette.search_match_foreground, palette.search_match)
        };
        let foreground = self
            .foreground
            .as_deref()
            .and_then(|s| try_parse_color(s).ok())
            .unwrap_or(foreground);
        let background = self
            .background
            .as_deref()
            .and_then(|s| try_parse_color(s).ok())
            .unwrap_or(background);
        HighlightStyle {
            color: Some(foreground),
            background_color: Some(background),
            font_weight: self.bold.then_some(FontWeight::BOLD),
            font_style: self.italic.then_some(FontStyle::Italic),
            underline: self.underline.then_some(UnderlineStyle {
                // Physical stroke thickness, matching the existing text-selection underline.
                thickness: px(1.),
                color: Some(foreground),
                wavy: false,
            }),
            ..Default::default()
        }
    }
}
