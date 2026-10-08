use vclogg_core::{SearchMatcher, SearchQuery};

#[test]
fn literal_highlights_include_overlapping_keywords() {
    for (query, text, expected) in [
        ("abc|bcd", "abcdefg", vec![0..3, 1..4]),
        ("abc|abcdef", "abcdefg", vec![0..3, 0..6]),
        ("abcdef|bc", "abcdefg", vec![0..6, 1..3]),
        ("abc|bcd", "ABCD abc bcd", vec![0..3, 1..4, 5..8, 9..12]),
        ("中文|文日", "中文日志", vec![0..6, 3..9]),
        ("aba", "ababa", vec![0..3, 2..5]),
    ] {
        let matcher = SearchMatcher::new(&SearchQuery {
            text: query.into(),
            ..Default::default()
        })
        .unwrap()
        .unwrap();
        let mut ranges = matcher.highlight_ranges(text);
        ranges.sort_by_key(|range| (range.start, range.end));
        assert_eq!(ranges, expected, "query: {query}, text: {text}");
    }
}

#[test]
fn regex_highlights_include_overlapping_matches() {
    for (query, text, expected) in [
        ("abc|bcd", "abcdefg", vec![0..3, 1..4]),
        ("a.c|b.d", "abcdefg", vec![0..3, 1..4]),
        ("中文|文日", "中文日志", vec![0..6, 3..9]),
        ("aba", "ababa", vec![0..3, 2..5]),
        ("abc|bcd", "ABCD abc bcd", vec![0..3, 1..4, 5..8, 9..12]),
        ("^abc|^bcd", "abcdefg", std::iter::once(0..3).collect()),
        (r"abc|\bbcd", "abcdefg", std::iter::once(0..3).collect()),
        ("^|bcd|$", "abcdefg", std::iter::once(1..4).collect()),
        ("^|$", "中文", vec![]),
        ("abc", "", vec![]),
        (r"(?-u:\xB8)", "中", vec![]),
    ] {
        let matcher = SearchMatcher::new(&SearchQuery {
            text: query.into(),
            regex: true,
            ..Default::default()
        })
        .unwrap()
        .unwrap();
        assert_eq!(matcher.highlight_ranges(text), expected, "query: {query}");
    }
}

#[test]
fn quick_find_keeps_non_overlapping_occurrences() {
    let matcher = SearchMatcher::quick_find("aba", true, false, true)
        .unwrap()
        .unwrap();
    assert_eq!(matcher.matching_ranges("ababa"), vec![0..3]);
}

#[test]
fn display_highlights_do_not_change_regex_matching() {
    let matcher = SearchMatcher::new(&SearchQuery {
        text: "abc|bcd".into(),
        regex: true,
        ..Default::default()
    })
    .unwrap()
    .unwrap();
    assert_eq!(matcher.matching_ranges("abcdefg"), vec![0..3]);
    assert_eq!(matcher.highlight_ranges("abcdefg"), vec![0..3, 1..4]);
    assert_eq!(matcher.first_matching_range("abcdefg"), Some(0..3));
}
