use std::collections::HashSet;

use pinyin::ToPinyinMulti;

use crate::predefined_filters::PredefinedFilter;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SearchSuggestionSource {
    History,
    PredefinedFilter { name: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SearchSuggestion {
    pub value: String,
    pub source: SearchSuggestionSource,
}

struct CompletionFragment {
    prefix: String,
    needle: String,
    has_leading_whitespace: bool,
}

fn top_level_alternation_indexes(pattern: &str) -> Vec<usize> {
    let mut indexes = Vec::new();
    let mut group_depth = 0usize;
    let mut in_character_class = false;
    let mut escaped = false;

    for (index, character) in pattern.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if character == '[' && !in_character_class {
            in_character_class = true;
            continue;
        }
        if character == ']' && in_character_class {
            in_character_class = false;
            continue;
        }
        if in_character_class {
            continue;
        }
        match character {
            '(' => group_depth += 1,
            ')' if group_depth > 0 => group_depth -= 1,
            '|' if group_depth == 0 => indexes.push(index),
            _ => {}
        }
    }

    indexes
}

pub(crate) fn split_top_level_regex_alternatives(pattern: &str) -> Vec<String> {
    let mut alternatives = Vec::new();
    let mut start = 0usize;
    for separator in top_level_alternation_indexes(pattern) {
        let value = pattern[start..separator].trim();
        if !value.is_empty() {
            alternatives.push(value.to_string());
        }
        start = separator + 1;
    }
    let value = pattern[start..].trim();
    if !value.is_empty() {
        alternatives.push(value.to_string());
    }
    alternatives
}

fn completion_fragment(query: &str) -> CompletionFragment {
    let fragment_start = top_level_alternation_indexes(query)
        .last()
        .map_or(0, |separator| separator + 1);
    let fragment = &query[fragment_start..];
    let leading_len = fragment.len() - fragment.trim_start().len();
    CompletionFragment {
        prefix: format!("{}{}", &query[..fragment_start], &fragment[..leading_len]),
        needle: fragment[leading_len..].trim_end().to_string(),
        has_leading_whitespace: leading_len > 0,
    }
}

pub(crate) fn search_autocomplete_needle(query: &str) -> String {
    completion_fragment(query).needle
}

struct SuggestionMatcher {
    needle: String,
    pinyin_needle: String,
}

impl SuggestionMatcher {
    fn new(query: &str) -> Self {
        let needle = search_autocomplete_needle(query).to_lowercase();
        let pinyin_needle: String = needle
            .chars()
            .filter(|character| !character.is_whitespace() && !matches!(character, '\'' | '-'))
            .collect();
        Self {
            needle,
            pinyin_needle,
        }
    }

    fn matches(&self, value: &str) -> bool {
        let value = value.to_lowercase();
        if value.contains(&self.needle) {
            return true;
        }
        let needle = &self.pinyin_needle;
        if needle.is_empty()
            || !needle.bytes().all(|byte| byte.is_ascii_alphabetic())
            || !value
                .chars()
                .any(|character| character.to_pinyin_multi().is_some())
        {
            return false;
        }

        // Track query offsets instead of enumerating all combinations of
        // pronunciations and syllable prefixes. Each Chinese character may
        // consume a full syllable, an initial, or any intermediate prefix.
        // Offset zero starts a substring at each character boundary.
        let mut offsets = vec![false; needle.len() + 1];
        let mut next = offsets.clone();
        for character in value.chars() {
            offsets[0] = true;
            next.fill(false);
            let mut consume = |syllable: &str, allow_prefix: bool| {
                for (offset, matched) in offsets.iter().enumerate().take(needle.len()) {
                    if !matched {
                        continue;
                    }
                    let remaining = &needle.as_bytes()[offset..];
                    let max_len = syllable.len().min(remaining.len());
                    for length in 1..=max_len {
                        if (allow_prefix || length == syllable.len())
                            && remaining[..length] == syllable.as_bytes()[..length]
                        {
                            next[offset + length] = true;
                        }
                    }
                }
            };
            if let Some(pronunciations) = character.to_pinyin_multi() {
                for pronunciation in pronunciations {
                    consume(pronunciation.plain(), true);
                }
            } else {
                let mut buffer = [0; 4];
                consume(character.encode_utf8(&mut buffer), false);
            }
            if next[needle.len()] {
                return true;
            }
            std::mem::swap(&mut offsets, &mut next);
        }
        false
    }
}

pub(crate) fn search_autocomplete_suggestions(
    history: &[String],
    filters: &[PredefinedFilter],
    query: &str,
    limit: usize,
) -> Vec<SearchSuggestion> {
    let matcher = SuggestionMatcher::new(query);
    let mut suggestions = Vec::new();
    let mut seen = HashSet::new();

    let mut add_suggestion = |suggestion: SearchSuggestion, name_matches: bool| {
        if suggestion.value.is_empty()
            || seen.contains(&suggestion.value)
            || (!name_matches && !matcher.matches(&suggestion.value))
        {
            return;
        }
        seen.insert(suggestion.value.clone());
        suggestions.push(suggestion);
    };

    if matcher.needle.is_empty() {
        for value in history {
            add_suggestion(
                SearchSuggestion {
                    value: value.clone(),
                    source: SearchSuggestionSource::History,
                },
                false,
            );
        }
        suggestions.truncate(limit);
        return suggestions;
    }

    for value in history {
        for alternative in split_top_level_regex_alternatives(value) {
            add_suggestion(
                SearchSuggestion {
                    value: alternative,
                    source: SearchSuggestionSource::History,
                },
                false,
            );
        }
    }

    let mut complete_filter_expressions = Vec::new();
    for filter in filters {
        let name_matches = matcher.matches(&filter.name);
        let alternatives = if filter.use_regex {
            split_top_level_regex_alternatives(&filter.value)
        } else {
            vec![filter.value.clone()]
        };
        for value in &alternatives {
            add_suggestion(
                SearchSuggestion {
                    value: value.clone(),
                    source: SearchSuggestionSource::PredefinedFilter {
                        name: filter.name.clone(),
                    },
                },
                name_matches,
            );
        }
        if filter.use_regex && alternatives.len() > 1 {
            complete_filter_expressions.push((
                SearchSuggestion {
                    value: filter.value.clone(),
                    source: SearchSuggestionSource::PredefinedFilter {
                        name: filter.name.clone(),
                    },
                },
                name_matches,
            ));
        }
    }
    for (suggestion, name_matches) in complete_filter_expressions {
        add_suggestion(suggestion, name_matches);
    }

    suggestions.truncate(limit);
    suggestions
}

pub(crate) fn apply_search_suggestion(query: &str, suggestion: &str) -> String {
    let fragment = completion_fragment(query);
    format!(
        "{}{}",
        fragment.prefix,
        if fragment.has_leading_whitespace {
            suggestion.trim_start()
        } else {
            suggestion
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(name: &str, value: &str, use_regex: bool) -> PredefinedFilter {
        let mut filter = PredefinedFilter::new(&[]);
        filter.name = name.into();
        filter.value = value.into();
        filter.use_regex = use_regex;
        filter
    }

    #[test]
    fn filter_name_matches_offer_keywords_and_complete_expression() {
        let filters = [filter("截图", "Screenshot|Capture", true)];
        for query in [
            "截图", "截", "jietu", "jt", "jie", "jiet", "jtu", "jitu", "jie tu", "jie'tu",
            "jie-tu", "JIETU", "tu",
        ] {
            let suggestions = search_autocomplete_suggestions(&[], &filters, query, 100);
            assert_eq!(
                suggestions
                    .iter()
                    .map(|item| item.value.as_str())
                    .collect::<Vec<_>>(),
                ["Screenshot", "Capture", "Screenshot|Capture"],
                "query={query}"
            );
            assert!(suggestions.iter().all(|item| item.source
                == SearchSuggestionSource::PredefinedFilter {
                    name: "截图".into()
                }));
        }
        for query in ["jietux", "tj", "ietu", "random", "---", ".*"] {
            assert!(
                search_autocomplete_suggestions(&[], &filters, query, 100).is_empty(),
                "query={query}"
            );
        }
    }

    #[test]
    fn pinyin_matches_chinese_history_and_filter_values() {
        let history = vec!["错误|截图完成".into()];
        let filters = [filter("Saved", "开始截图", false)];
        let suggestions = search_autocomplete_suggestions(&history, &filters, "jt", 100);
        assert_eq!(
            suggestions,
            vec![
                SearchSuggestion {
                    value: "截图完成".into(),
                    source: SearchSuggestionSource::History
                },
                SearchSuggestion {
                    value: "开始截图".into(),
                    source: SearchSuggestionSource::PredefinedFilter {
                        name: "Saved".into()
                    }
                },
            ]
        );
    }

    #[test]
    fn pinyin_supports_multiple_pronunciations_and_latin_text() {
        let filters = [filter("重庆截图API", "capture", false)];
        for query in ["chongqing", "zhongqing", "cqjt", "jietuapi", "JTAPI"] {
            assert_eq!(
                search_autocomplete_suggestions(&[], &filters, query, 100).len(),
                1,
                "query={query}"
            );
        }
        assert!(search_autocomplete_suggestions(&[], &filters, "jtxapi", 100).is_empty());
    }

    #[test]
    fn literal_matching_keeps_history_priority_deduplication_and_limit() {
        let history = vec!["Screenshot|Capture".into(), "Capture".into()];
        let filters = [filter("Screenshot tools", "Capture|Other", true)];
        let suggestions = search_autocomplete_suggestions(&history, &filters, "screenshot", 100);
        assert_eq!(
            suggestions
                .iter()
                .map(|item| item.value.as_str())
                .collect::<Vec<_>>(),
            ["Screenshot", "Capture", "Other", "Capture|Other"]
        );
        assert_eq!(suggestions[0].source, SearchSuggestionSource::History);
        assert_eq!(
            search_autocomplete_suggestions(&history, &filters, "screenshot", 1),
            suggestions[..1]
        );
        assert!(search_autocomplete_suggestions(&history, &filters, "screenshot", 0).is_empty());
    }

    #[test]
    fn empty_query_only_offers_unsplit_history() {
        let history = vec!["Screenshot|Capture".into()];
        let filters = [filter("截图", "capture", false)];
        let suggestions = search_autocomplete_suggestions(&history, &filters, "", 100);
        assert_eq!(
            suggestions,
            vec![SearchSuggestion {
                value: history[0].clone(),
                source: SearchSuggestionSource::History
            }]
        );
    }

    #[test]
    fn matching_history_and_filter_keywords_are_deduplicated() {
        let history = vec!["截图完成|截图完成".into()];
        let filters = [
            filter("截图", "截图完成", false),
            filter("截图操作", "截图完成", false),
        ];
        let suggestions = search_autocomplete_suggestions(&history, &filters, "jt", 100);
        assert_eq!(
            suggestions,
            vec![SearchSuggestion {
                value: "截图完成".into(),
                source: SearchSuggestionSource::History
            }]
        );
    }

    #[test]
    fn name_completion_replaces_only_the_last_top_level_fragment() {
        let filters = [filter("截图", "Screenshot|Capture", true)];
        let query = "error|(foo|bar)| jt";
        let suggestions = search_autocomplete_suggestions(&[], &filters, query, 100);
        assert_eq!(
            apply_search_suggestion(query, &suggestions[0].value),
            "error|(foo|bar)| Screenshot"
        );
        assert_eq!(
            apply_search_suggestion(query, &suggestions[2].value),
            "error|(foo|bar)| Screenshot|Capture"
        );
    }

    #[test]
    fn literal_filters_keep_pipes_and_regex_groups_intact() {
        for (value, use_regex) in [
            ("Screenshot|Capture", false),
            ("(Screenshot|Capture)", true),
        ] {
            let filters = [filter("截图", value, use_regex)];
            let suggestions = search_autocomplete_suggestions(&[], &filters, "jt", 100);
            assert_eq!(suggestions.len(), 1);
            assert_eq!(suggestions[0].value, value);
        }
        let filters = [filter("截图", "", false)];
        assert!(search_autocomplete_suggestions(&[], &filters, "jt", 100).is_empty());
    }

    #[test]
    fn regex_syntax_and_non_chinese_text_keep_literal_matching() {
        let history = vec![r"error\.code|[a|b]|(foo|bar)".into(), "screenshot".into()];
        for query in [r"\.", "[a|b]", "(foo|bar)"] {
            assert_eq!(
                search_autocomplete_suggestions(&history, &[], query, 100).len(),
                1
            );
        }
        assert!(search_autocomplete_suggestions(&history, &[], "screen shot", 100).is_empty());
    }
}
