use std::{collections::BTreeMap, ops::Range, sync::Arc};

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
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

#[derive(Clone, Default)]
pub struct ResolvedColorRules {
    case_sensitive: Option<CaseSensitiveColorRules>,
    fallback: Arc<[ResolvedColorRule]>,
    layers: Arc<[Arc<ResolvedColorRules>]>,
}

#[derive(Clone)]
struct CaseSensitiveColorRules {
    matcher: AhoCorasick,
    rules: Arc<[ResolvedColorRuleMetadata]>,
}

#[derive(Clone, Copy)]
struct ResolvedColorRuleMetadata {
    color: u32,
    alpha: u8,
    order: usize,
}

#[derive(Clone)]
struct ResolvedColorRule {
    matcher: SearchMatcher,
    metadata: ResolvedColorRuleMetadata,
}

impl ResolvedColorRules {
    pub fn layered(
        base: Arc<ResolvedColorRules>,
        overlay: Arc<ResolvedColorRules>,
    ) -> Arc<ResolvedColorRules> {
        Arc::new(Self {
            case_sensitive: None,
            fallback: Arc::default(),
            layers: Arc::from([base, overlay]),
        })
    }

    pub fn matching_ranges(&self, text: &str) -> Vec<(Range<usize>, Hsla, usize)> {
        if !self.layers.is_empty() {
            let mut ranges = Vec::new();
            let mut order_offset = 0_usize;
            for layer in self.layers.iter() {
                let mut layer_ranges = layer.matching_ranges(text);
                let next_offset = layer_ranges
                    .iter()
                    .map(|(_, _, order)| order.saturating_add(1))
                    .max()
                    .unwrap_or_default();
                for (_, _, order) in &mut layer_ranges {
                    *order = order.saturating_add(order_offset);
                }
                ranges.extend(layer_ranges);
                order_offset = order_offset.saturating_add(next_offset);
            }
            return ranges;
        }
        let mut ranges = Vec::new();
        if let Some(case_sensitive) = &self.case_sensitive {
            let mut next_allowed_start = BTreeMap::<usize, usize>::new();
            for matched in case_sensitive
                .matcher
                .find_overlapping_iter(text.as_bytes())
            {
                let pattern = matched.pattern().as_usize();
                if next_allowed_start
                    .get(&pattern)
                    .is_some_and(|next_start| matched.start() < *next_start)
                {
                    continue;
                }
                next_allowed_start.insert(pattern, matched.end());
                let metadata = case_sensitive.rules[pattern];
                ranges.push((
                    matched.start()..matched.end(),
                    color_with_alpha(metadata.color, metadata.alpha),
                    metadata.order,
                ));
            }
        }
        for rule in self.fallback.iter() {
            ranges.extend(rule.matcher.matching_ranges(text).into_iter().map(|range| {
                (
                    range,
                    color_with_alpha(rule.metadata.color, rule.metadata.alpha),
                    rule.metadata.order,
                )
            }));
        }
        ranges
    }

    #[cfg(test)]
    fn batched_pattern_count(&self) -> usize {
        self.case_sensitive
            .as_ref()
            .map_or(0, |rules| rules.rules.len())
    }

    #[cfg(test)]
    fn fallback_pattern_count(&self) -> usize {
        self.fallback.len()
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
) -> Arc<ResolvedColorRules> {
    let mut case_sensitive = BTreeMap::<String, ResolvedColorRuleMetadata>::new();
    let mut fallback = Vec::new();
    for (order, rule) in rules
        .iter()
        .enumerate()
        .filter(|(_, rule)| rule.enabled && !rule.keyword.is_empty())
    {
        let (color, alpha) = rule
            .label_id
            .as_deref()
            .and_then(|id| labels.iter().find(|label| label.id == id))
            .map_or((rule.color, rule.alpha), |label| (label.color, label.alpha));
        let metadata = ResolvedColorRuleMetadata {
            color,
            alpha,
            order,
        };
        if rule.case_sensitive {
            case_sensitive.insert(rule.keyword.clone(), metadata);
        } else if let Some(matcher) = SearchMatcher::literal(&rule.keyword, false).ok().flatten() {
            fallback.push(ResolvedColorRule { matcher, metadata });
        }
    }

    let case_sensitive = (!case_sensitive.is_empty()).then(|| {
        let (patterns, metadata): (Vec<_>, Vec<_>) = case_sensitive.into_iter().unzip();
        match AhoCorasickBuilder::new()
            .match_kind(MatchKind::Standard)
            .build(&patterns)
        {
            Ok(matcher) => Some(CaseSensitiveColorRules {
                matcher,
                rules: metadata.into(),
            }),
            Err(_) => {
                fallback.extend(patterns.into_iter().zip(metadata).filter_map(
                    |(keyword, metadata)| {
                        SearchMatcher::literal(&keyword, true)
                            .ok()
                            .flatten()
                            .map(|matcher| ResolvedColorRule { matcher, metadata })
                    },
                ));
                None
            }
        }
    });
    Arc::new(ResolvedColorRules {
        case_sensitive: case_sensitive.flatten(),
        fallback: fallback.into(),
        layers: Arc::default(),
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(keyword: impl Into<String>, case_sensitive: bool, color: u32) -> KeywordColorRule {
        KeywordColorRule {
            label_id: None,
            keyword: keyword.into(),
            color,
            alpha: u8::MAX,
            case_sensitive,
            enabled: true,
        }
    }

    #[test]
    fn case_sensitive_rules_share_one_multi_pattern_matcher() {
        let rules = (0..1_000)
            .map(|index| rule(format!("keyword-{index}"), true, index))
            .collect::<Vec<_>>();

        let resolved = resolve_color_rules(&rules, &[]);

        assert_eq!(resolved.batched_pattern_count(), 1_000);
        assert_eq!(resolved.fallback_pattern_count(), 0);
    }

    #[test]
    fn batched_literals_keep_per_pattern_non_overlapping_ranges() {
        let resolved = resolve_color_rules(
            &[rule("aa", true, 0xff0000), rule("A", false, 0x00ff00)],
            &[],
        );
        let ranges = resolved.matching_ranges("aaa A");

        assert!(
            ranges
                .iter()
                .any(|(range, _, order)| range == &(0..2) && *order == 0)
        );
        assert!(
            !ranges
                .iter()
                .any(|(range, _, order)| range == &(1..3) && *order == 0)
        );
        assert!(
            ranges
                .iter()
                .any(|(range, _, order)| range == &(4..5) && *order == 1)
        );
    }

    #[test]
    fn layered_rules_give_the_search_session_overlay_precedence() {
        let base = resolve_color_rules(&[rule("error", true, 0xff0000)], &[]);
        let overlay = resolve_color_rules(&[rule("error", true, 0x00ff00)], &[]);
        let layered = ResolvedColorRules::layered(base, overlay);
        let mut ranges = layered.matching_ranges("error");
        ranges.sort_by_key(|(_, _, order)| *order);

        assert_eq!(ranges.len(), 2);
        assert!(ranges[1].2 > ranges[0].2);
    }
}
