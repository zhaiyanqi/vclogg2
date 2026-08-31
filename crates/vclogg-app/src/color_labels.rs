use std::{ops::Range, sync::Arc};

use gpui::{Hsla, rgba};
use serde::{Deserialize, Serialize};
use vclogg_core::SearchMatcher;

const DEFAULT_COLOR_LABEL_ALPHA: u8 = 179;
const DEFAULT_COLOR_LABELS: [(&str, &str, u32); 10] = [
    ("天蓝", "Sky blue", 0x38bdf8),
    ("琥珀", "Amber", 0xf28e2b),
    ("珊瑚", "Coral", 0xe15759),
    ("青瓷", "Celadon", 0x76b7b2),
    ("草绿", "Grass green", 0x59a14f),
    ("金黄", "Golden yellow", 0xedc948),
    ("紫藤", "Wisteria", 0xb07aa1),
    ("樱粉", "Cherry pink", 0xff9da7),
    ("青柠", "Lime", 0xa3e635),
    ("靛紫", "Indigo", 0x6f63c2),
];

fn opaque_color_alpha() -> u8 {
    u8::MAX
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ColorLabel {
    pub id: String,
    pub name: String,
    pub color: u32,
    #[serde(default = "opaque_color_alpha")]
    pub alpha: u8,
}

impl ColorLabel {
    pub fn localized_name(&self) -> String {
        let Some(index) = self
            .id
            .strip_prefix("color-label-")
            .and_then(|value| value.parse::<usize>().ok())
            .and_then(|value| value.checked_sub(1))
        else {
            return self.name.clone();
        };
        let Some((chinese, english, _)) = DEFAULT_COLOR_LABELS.get(index).copied() else {
            return self.name.clone();
        };
        if self.name == chinese || self.name == english {
            crate::tr!(chinese, english).to_string()
        } else {
            self.name.clone()
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KeywordColorRule {
    pub label_id: Option<String>,
    pub keyword: String,
    pub color: u32,
    #[serde(default = "opaque_color_alpha")]
    pub alpha: u8,
    pub case_sensitive: bool,
    pub enabled: bool,
}

#[derive(Clone)]
pub struct ResolvedColorRule {
    matcher: SearchMatcher,
    color: u32,
    alpha: u8,
}

impl ResolvedColorRule {
    pub fn ranges(&self, text: &str) -> Vec<Range<usize>> {
        self.matcher.matching_ranges(text)
    }

    pub fn color(&self) -> Hsla {
        color_with_alpha(self.color, self.alpha)
    }
}

pub fn color_with_alpha(color: u32, alpha: u8) -> Hsla {
    rgba((color << 8) | u32::from(alpha)).into()
}

pub fn default_color_labels() -> Vec<ColorLabel> {
    DEFAULT_COLOR_LABELS
        .into_iter()
        .enumerate()
        .map(|(ix, (name, _, color))| ColorLabel {
            id: format!("color-label-{}", ix + 1),
            name: name.to_string(),
            color,
            alpha: DEFAULT_COLOR_LABEL_ALPHA,
        })
        .collect()
}

pub fn resolve_color_rules(
    rules: &[KeywordColorRule],
    labels: &[ColorLabel],
) -> Arc<[ResolvedColorRule]> {
    rules
        .iter()
        .filter(|rule| rule.enabled && !rule.keyword.is_empty())
        .filter_map(|rule| {
            let (color, alpha) = rule
                .label_id
                .as_deref()
                .and_then(|id| labels.iter().find(|label| label.id == id))
                .map_or((rule.color, rule.alpha), |label| (label.color, label.alpha));
            SearchMatcher::literal(&rule.keyword, rule.case_sensitive)
                .ok()
                .flatten()
                .map(|matcher| ResolvedColorRule {
                    matcher,
                    color,
                    alpha,
                })
        })
        .collect::<Vec<_>>()
        .into()
}

pub fn encode_rules(rules: &[KeywordColorRule]) -> String {
    serde_json::to_string(rules).unwrap_or_else(|_| "[]".to_string())
}

pub fn decode_rules(value: &str) -> Vec<KeywordColorRule> {
    if value.trim().is_empty() {
        return Vec::new();
    }
    serde_json::from_str(value).unwrap_or_default()
}
