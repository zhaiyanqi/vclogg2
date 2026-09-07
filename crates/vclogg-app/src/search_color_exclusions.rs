use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};

use crate::{
    color_labels::{ColorLabel, KeywordColorRule, ResolvedColorRules, resolve_color_rules},
    path_identity::{
        PathMatchKey, decode_persisted_path, encode_persisted_path, path_match_key,
        path_match_map_get,
    },
};

/// File-specific removals from shared search colors. New assignments restore only their keywords.
#[derive(Clone, Default)]
pub(crate) struct SearchColorExclusions {
    keywords: BTreeMap<PathMatchKey, (String, BTreeSet<String>)>,
    resolved: BTreeMap<PathMatchKey, Arc<ResolvedColorRules>>,
}

impl SearchColorExclusions {
    pub(crate) fn restore(
        persisted: &BTreeMap<String, BTreeSet<String>>,
        rules: &[KeywordColorRule],
        labels: &[ColorLabel],
    ) -> Self {
        let mut exclusions = Self::default();
        for (encoded_path, keywords) in persisted {
            let path = decode_persisted_path(encoded_path);
            exclusions
                .keywords
                .entry(path_match_key(&path))
                .or_insert_with(|| (encoded_path.clone(), BTreeSet::new()))
                .1
                .extend(keywords.iter().cloned());
        }
        exclusions.refresh(rules, labels);
        exclusions
    }

    pub(crate) fn persisted(&self) -> BTreeMap<String, BTreeSet<String>> {
        self.keywords.values().cloned().collect()
    }

    pub(crate) fn clear_file(&mut self, path: &Path, rules: &[KeywordColorRule]) {
        if rules.is_empty() {
            return;
        }
        let key = path_match_key(path);
        self.keywords.insert(
            key.clone(),
            (
                encode_persisted_path(path),
                rules.iter().map(|rule| rule.keyword.clone()).collect(),
            ),
        );
        self.resolved.insert(key, Arc::default());
    }

    pub(crate) fn include_keywords(&mut self, keywords: &BTreeSet<String>) {
        self.keywords.retain(|_, (_, excluded)| {
            excluded.retain(|keyword| !keywords.contains(keyword));
            !excluded.is_empty()
        });
    }

    pub(crate) fn excludes(&self, path: &Path, keyword: &str) -> bool {
        path_match_map_get(&self.keywords, path)
            .is_some_and(|(_, keywords)| keywords.contains(keyword))
    }

    pub(crate) fn refresh(&mut self, rules: &[KeywordColorRule], labels: &[ColorLabel]) {
        self.resolved = self
            .keywords
            .iter()
            .map(|(key, (_, excluded))| {
                let rules = rules
                    .iter()
                    .filter(|rule| !excluded.contains(&rule.keyword))
                    .cloned()
                    .collect::<Vec<_>>();
                (key.clone(), resolve_color_rules(&rules, labels))
            })
            .collect();
    }

    pub(crate) fn rules_for(
        &self,
        path: &Path,
        shared: &Arc<ResolvedColorRules>,
    ) -> Arc<ResolvedColorRules> {
        path_match_map_get(&self.resolved, path)
            .unwrap_or(shared)
            .clone()
    }
}
