use std::collections::HashSet;

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

pub(crate) fn search_autocomplete_suggestions(
    history: &[String],
    filters: &[PredefinedFilter],
    query: &str,
    limit: usize,
) -> Vec<SearchSuggestion> {
    let normalized_needle = search_autocomplete_needle(query).to_lowercase();
    let mut suggestions = Vec::new();
    let mut seen = HashSet::new();

    let mut add_suggestion = |suggestion: SearchSuggestion| {
        if suggestion.value.is_empty()
            || seen.contains(&suggestion.value)
            || (!normalized_needle.is_empty()
                && !suggestion.value.to_lowercase().contains(&normalized_needle))
        {
            return;
        }
        seen.insert(suggestion.value.clone());
        suggestions.push(suggestion);
    };

    if normalized_needle.is_empty() {
        for value in history {
            add_suggestion(SearchSuggestion {
                value: value.clone(),
                source: SearchSuggestionSource::History,
            });
        }
        suggestions.truncate(limit);
        return suggestions;
    }

    for value in history {
        for alternative in split_top_level_regex_alternatives(value) {
            add_suggestion(SearchSuggestion {
                value: alternative,
                source: SearchSuggestionSource::History,
            });
        }
    }

    let mut complete_filter_expressions = Vec::new();
    for filter in filters {
        let alternatives = if filter.use_regex {
            split_top_level_regex_alternatives(&filter.value)
        } else {
            vec![filter.value.clone()]
        };
        for value in &alternatives {
            add_suggestion(SearchSuggestion {
                value: value.clone(),
                source: SearchSuggestionSource::PredefinedFilter {
                    name: filter.name.clone(),
                },
            });
        }
        if filter.use_regex && alternatives.len() > 1 {
            complete_filter_expressions.push(SearchSuggestion {
                value: filter.value.clone(),
                source: SearchSuggestionSource::PredefinedFilter {
                    name: filter.name.clone(),
                },
            });
        }
    }
    for suggestion in complete_filter_expressions {
        add_suggestion(suggestion);
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
