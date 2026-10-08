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
        let mut ranges = matcher.matching_ranges(text);
        ranges.sort_by_key(|range| (range.start, range.end));
        assert_eq!(ranges, expected, "query: {query}, text: {text}");
    }
}

#[test]
fn regex_highlights_preserve_regex_alternation_semantics() {
    let matcher = SearchMatcher::new(&SearchQuery {
        text: "abc|bcd".into(),
        regex: true,
        ..Default::default()
    })
    .unwrap()
    .unwrap();
    assert_eq!(matcher.matching_ranges("abcdefg"), vec![0..3]);
}
