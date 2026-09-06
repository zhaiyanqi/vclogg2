use std::collections::BTreeMap;

use gpui::{App, FontWeight, Hsla, Pixels, Styled as _, px};
use gpui_component::ColorName;
use gpui_component::{ActiveTheme as _, Sizable as _, tag::Tag};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// File-owned annotations, indexed by source row so rendering never scans every tag.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RowTags(BTreeMap<usize, BTreeMap<String, RowTag>>);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct RowTag {
    pub(crate) source_row: usize,
    pub(crate) label: String,
    pub(crate) color: TagColor,
    #[serde(default)]
    pub(crate) style: TagStyle,
    /// Thousandths of the log font size, relative to the row's content origin.
    pub(crate) x: u32,
    pub(crate) y: u32,
    /// Bind the annotation to the decoded row preview, not just its line number.
    pub(crate) source_digest: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct TagStyle {
    pub(crate) font_size: Option<u16>,
    pub(crate) height: Option<u16>,
    pub(crate) text_color: Option<u32>,
    pub(crate) background_color: Option<u32>,
    #[serde(default)]
    pub(crate) transparency: u8,
    pub(crate) bold: bool,
    pub(crate) pill: bool,
}

impl TagStyle {
    pub(crate) fn opacity(&self) -> f32 {
        1. - f32::from(self.transparency.min(100)) / 100.
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct TagPreset {
    pub(crate) label: String,
    pub(crate) color: TagColor,
    #[serde(default)]
    pub(crate) style: TagStyle,
}

impl RowTag {
    pub(crate) fn preset(&self) -> TagPreset {
        TagPreset {
            label: self.label.clone(),
            color: self.color,
            style: self.style.clone(),
        }
    }
}

impl TagPreset {
    pub(crate) fn id(&self) -> String {
        // History identity follows the text; the latest use replaces its appearance.
        source_digest(&self.label)
    }

    pub(crate) fn is_valid(&self) -> bool {
        !self.label.trim().is_empty()
            && self.label.chars().count() <= 128
            && !self.label.contains(['\n', '\r'])
    }

    pub(crate) fn dimensions(&self, font: Pixels, row_height: Pixels) -> (Pixels, Pixels) {
        // Physical log geometry, shared by preview and row painting. The border is 1px per side.
        let requested_font = self
            .style
            .font_size
            .map_or(font * 0.85, |value| px(value as f32));
        let height = self
            .style
            .height
            .map_or(requested_font + px(4.), |value| px(value as f32))
            .clamp(px(3.), row_height.max(px(3.)));
        let text = requested_font.clamp(px(1.), (height - px(2.)).max(px(1.)));
        (text, height)
    }

    pub(crate) fn colors(&self, cx: &App) -> (Hsla, Hsla) {
        let palette = self.color.color_name();
        let background = if cx.theme().is_dark() {
            palette.scale(950).opacity(0.5)
        } else {
            palette.scale(50)
        };
        let text = palette.scale(if cx.theme().is_dark() { 300 } else { 600 });
        (
            self.style
                .text_color
                .map_or(text, |color| gpui::rgba(color).into()),
            self.style
                .background_color
                .map_or(background, |color| gpui::rgba(color).into()),
        )
    }

    pub(crate) fn render_tag(&self, font: Pixels, row_height: Pixels, cx: &App) -> Tag {
        let (text, height) = self.dimensions(font, row_height);
        let (foreground, background) = self.colors(cx);
        Tag::color(self.color.color_name())
            .small()
            .h(height)
            .py_0()
            .rounded(if self.style.pill {
                height / 2.
            } else {
                cx.theme().radius
            })
            .text_size(text)
            .font_family(cx.theme().font_family.clone())
            .font_weight(if self.style.bold {
                FontWeight::BOLD
            } else {
                FontWeight::NORMAL
            })
            .text_color(foreground)
            .bg(background)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TagColor {
    #[default]
    Neutral,
    Red,
    Orange,
    Amber,
    Yellow,
    Lime,
    Green,
    Emerald,
    Teal,
    Cyan,
    Sky,
    Blue,
    Indigo,
    Violet,
    Purple,
    Fuchsia,
    Pink,
    Rose,
}

impl TagColor {
    pub(crate) fn color_name(self) -> ColorName {
        match self {
            Self::Neutral => ColorName::Neutral,
            Self::Red => ColorName::Red,
            Self::Orange => ColorName::Orange,
            Self::Amber => ColorName::Amber,
            Self::Yellow => ColorName::Yellow,
            Self::Lime => ColorName::Lime,
            Self::Green => ColorName::Green,
            Self::Emerald => ColorName::Emerald,
            Self::Teal => ColorName::Teal,
            Self::Cyan => ColorName::Cyan,
            Self::Sky => ColorName::Sky,
            Self::Blue => ColorName::Blue,
            Self::Indigo => ColorName::Indigo,
            Self::Violet => ColorName::Violet,
            Self::Purple => ColorName::Purple,
            Self::Fuchsia => ColorName::Fuchsia,
            Self::Pink => ColorName::Pink,
            Self::Rose => ColorName::Rose,
        }
    }
}

impl RowTags {
    pub(crate) fn row(&self, row: usize) -> impl Iterator<Item = (&String, &RowTag)> {
        self.0.get(&row).into_iter().flat_map(|tags| tags.iter())
    }

    pub(crate) fn get(&self, row: usize, id: &str) -> Option<&RowTag> {
        self.0.get(&row)?.get(id)
    }

    pub(crate) fn insert(&mut self, id: String, tag: RowTag) {
        self.0.entry(tag.source_row).or_default().insert(id, tag);
    }

    pub(crate) fn remove(&mut self, row: usize, id: &str) -> Option<RowTag> {
        let tags = self.0.get_mut(&row)?;
        let removed = tags.remove(id);
        if tags.is_empty() {
            self.0.remove(&row);
        }
        removed
    }

    pub(crate) fn from_records(records: BTreeMap<String, String>) -> Self {
        let mut tags = Self::default();
        for (id, payload) in records {
            if let Ok(tag) = serde_json::from_str::<RowTag>(&payload)
                && !id.is_empty()
                && !tag.label.trim().is_empty()
                && tag.label.chars().count() <= 128
            {
                tags.insert(id, tag);
            }
        }
        tags
    }

    pub(crate) fn to_records(&self) -> anyhow::Result<BTreeMap<String, String>> {
        self.0
            .values()
            .flat_map(|tags| tags.iter())
            .map(|(id, tag)| Ok((id.clone(), serde_json::to_string(tag)?)))
            .collect()
    }
}

pub(crate) fn source_digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
