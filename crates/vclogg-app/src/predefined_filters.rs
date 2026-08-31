use std::{collections::BTreeMap, fmt, str::FromStr};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::cloud_filters::CloudFilterItem;

/// A filter's immutable logical identity. Names and filter contents never participate in identity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FilterBranchId(Uuid);

impl FilterBranchId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(value: &str) -> Option<Self> {
        Uuid::parse_str(value.trim()).ok().map(Self)
    }
}

impl Default for FilterBranchId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for FilterBranchId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(formatter)
    }
}

impl FromStr for FilterBranchId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        Uuid::parse_str(value.trim()).map(Self)
    }
}

impl Serialize for FilterBranchId {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for FilterBranchId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterSnapshot {
    pub name: String,
    pub value: String,
    pub use_regex: bool,
    pub note: String,
    pub collaborative: bool,
}

impl FilterSnapshot {
    fn normalized(mut self) -> Self {
        self.name = self.name.trim().to_string();
        self.value = self.value.trim().to_string();
        self.note = self.note.trim().to_string();
        self
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RemoteFilterRelation {
    Tracking,
    DerivedFrom,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFilterReference {
    pub server_url: String,
    pub filter_id: FilterBranchId,
    pub revision: u32,
    pub owner_id: String,
    pub owner_name: String,
    pub relation: RemoteFilterRelation,
    pub baseline: FilterSnapshot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PredefinedFilter {
    /// Serialized as `uuid` so v5 has one identity field and no legacy `id` alias.
    #[serde(rename = "uuid")]
    pub id: FilterBranchId,
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub use_regex: bool,
    #[serde(default)]
    pub note: String,
    #[serde(default = "default_collaborative")]
    pub collaborative: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remote_references: Vec<RemoteFilterReference>,
}

fn default_collaborative() -> bool {
    true
}

impl PredefinedFilter {
    pub fn new(_existing: &[Self]) -> Self {
        Self {
            id: FilterBranchId::new(),
            name: String::new(),
            value: String::new(),
            use_regex: false,
            note: String::new(),
            collaborative: true,
            remote_references: Vec::new(),
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: FilterSnapshot) {
        self.name = snapshot.name;
        self.value = snapshot.value;
        self.use_regex = snapshot.use_regex;
        self.note = snapshot.note;
        self.collaborative = snapshot.collaborative;
    }

    pub fn tracking_reference(&self, server_url: &str) -> Option<&RemoteFilterReference> {
        let server_url = normalized_server_url(server_url);
        self.remote_references.iter().find(|reference| {
            reference.relation == RemoteFilterRelation::Tracking
                && reference.filter_id == self.id
                && normalized_server_url(&reference.server_url) == server_url
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyRemoteReference {
    #[serde(default)]
    server_url: String,
    #[serde(default)]
    filter_id: String,
    #[serde(default)]
    revision: u32,
    #[serde(default)]
    owner_id: String,
    #[serde(default)]
    owner_name: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    snapshot: Option<FilterSnapshot>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportedFilter {
    #[serde(default, rename = "id")]
    _legacy_id: String,
    #[serde(default)]
    uuid: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    use_regex: bool,
    #[serde(default)]
    note: String,
    #[serde(default = "default_collaborative")]
    collaborative: bool,
    #[serde(default)]
    remote_references: Vec<RemoteFilterReference>,
    #[serde(default)]
    source: Option<LegacyRemoteReference>,
    #[serde(default)]
    published: Option<LegacyRemoteReference>,
}

pub fn normalize_predefined_filters(filters: Vec<PredefinedFilter>) -> Vec<PredefinedFilter> {
    let mut normalized = Vec::<PredefinedFilter>::new();
    for mut filter in filters {
        filter.apply_snapshot(filter_snapshot(&filter).normalized());
        normalize_remote_references(&mut filter);
        if let Some(existing) = normalized.iter_mut().find(|item| item.id == filter.id)
            && filter_snapshot(existing) == filter_snapshot(&filter)
        {
            merge_remote_references(&mut existing.remote_references, filter.remote_references);
        } else {
            normalized.push(filter);
        }
    }
    normalized
}

fn normalize_imported(filters: Vec<ImportedFilter>) -> Vec<PredefinedFilter> {
    filters
        .into_iter()
        .filter_map(|filter| {
            let snapshot = FilterSnapshot {
                name: filter.name,
                value: filter.value,
                use_regex: filter.use_regex,
                note: filter.note,
                collaborative: filter.collaborative,
            }
            .normalized();
            if snapshot.name.is_empty() || snapshot.value.is_empty() {
                return None;
            }
            let id = FilterBranchId::parse(&filter.uuid).unwrap_or_default();
            let mut remote_references = filter.remote_references;
            if let Some(reference) = filter.source {
                append_legacy_reference(
                    &mut remote_references,
                    id,
                    reference,
                    RemoteFilterRelation::DerivedFrom,
                    &snapshot,
                );
            }
            if let Some(reference) = filter.published {
                append_legacy_reference(
                    &mut remote_references,
                    id,
                    reference,
                    RemoteFilterRelation::Tracking,
                    &snapshot,
                );
            }
            let mut result = PredefinedFilter {
                id,
                name: snapshot.name,
                value: snapshot.value,
                use_regex: snapshot.use_regex,
                note: snapshot.note,
                collaborative: snapshot.collaborative,
                remote_references,
            };
            normalize_remote_references(&mut result);
            Some(result)
        })
        .collect()
}

fn append_legacy_reference(
    references: &mut Vec<RemoteFilterReference>,
    local_id: FilterBranchId,
    reference: LegacyRemoteReference,
    fallback_relation: RemoteFilterRelation,
    local_snapshot: &FilterSnapshot,
) {
    let Some(filter_id) = FilterBranchId::parse(&reference.filter_id) else {
        return;
    };
    let relation = if filter_id == local_id {
        RemoteFilterRelation::Tracking
    } else {
        fallback_relation
    };
    let mut baseline = reference.snapshot.unwrap_or_else(|| local_snapshot.clone());
    if baseline.note.is_empty() {
        baseline.note = reference.note;
    }
    references.push(RemoteFilterReference {
        server_url: reference.server_url,
        filter_id,
        revision: reference.revision,
        owner_id: reference.owner_id,
        owner_name: reference.owner_name,
        relation,
        baseline: baseline.normalized(),
    });
}

fn normalize_remote_references(filter: &mut PredefinedFilter) {
    let mut references = Vec::new();
    for mut reference in std::mem::take(&mut filter.remote_references) {
        reference.server_url = normalized_server_url(&reference.server_url);
        reference.baseline = reference.baseline.normalized();
        if reference.relation == RemoteFilterRelation::Tracking && reference.filter_id != filter.id
        {
            reference.relation = RemoteFilterRelation::DerivedFrom;
        }
        merge_remote_reference(&mut references, reference);
    }
    filter.remote_references = references;
}

fn merge_remote_references(
    destination: &mut Vec<RemoteFilterReference>,
    references: Vec<RemoteFilterReference>,
) {
    for reference in references {
        merge_remote_reference(destination, reference);
    }
}

fn merge_remote_reference(
    references: &mut Vec<RemoteFilterReference>,
    reference: RemoteFilterReference,
) {
    if let Some(existing) = references.iter_mut().find(|existing| {
        normalized_server_url(&existing.server_url) == normalized_server_url(&reference.server_url)
            && existing.filter_id == reference.filter_id
            && existing.relation == reference.relation
    }) {
        if reference.revision > existing.revision
            || (reference.revision == existing.revision && reference.baseline == existing.baseline)
        {
            *existing = reference;
        }
    } else {
        references.push(reference);
    }
}

fn normalized_server_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

pub fn query_includes_filter(query: &str, filter_value: &str) -> bool {
    let value = filter_value.trim();
    if value.is_empty() {
        return false;
    }
    query.match_indices(value).any(|(index, matched)| {
        let before = query[..index].trim_end();
        let after = query[index + matched.len()..].trim_start();
        (before.is_empty() || before.ends_with('|')) && (after.is_empty() || after.starts_with('|'))
    })
}

pub fn toggle_filter_in_query(query: &str, filter: &PredefinedFilter, checked: bool) -> String {
    if checked {
        add_filter_to_query(query, &filter.value)
    } else {
        remove_filter_from_query(query, &filter.value)
    }
}

fn add_filter_to_query(query: &str, filter_value: &str) -> String {
    let current = query.trim();
    let value = filter_value.trim();
    if value.is_empty() || query_includes_filter(current, value) {
        return current.to_string();
    }
    if current.is_empty() {
        value.to_string()
    } else {
        format!("{current}|{value}")
    }
}

fn remove_filter_from_query(query: &str, filter_value: &str) -> String {
    let value = filter_value.trim();
    if value.is_empty() {
        return query.trim().to_string();
    }
    let Some((index, matched)) = query.match_indices(value).find(|(index, matched)| {
        let before = query[..*index].trim_end();
        let after = query[*index + matched.len()..].trim_start();
        (before.is_empty() || before.ends_with('|')) && (after.is_empty() || after.starts_with('|'))
    }) else {
        return query.trim().to_string();
    };

    let mut start = index;
    let mut end = index + matched.len();
    let after = &query[end..];
    let after_spaces = after.len() - after.trim_start().len();
    if after[after_spaces..].starts_with('|') {
        end += after_spaces + 1;
        let remainder = &query[end..];
        end += remainder.len() - remainder.trim_start().len();
    } else {
        let before = &query[..start];
        let trimmed = before.trim_end();
        if trimmed.ends_with('|') {
            start = trimmed.len() - 1;
            start -= query[..start].len() - query[..start].trim_end().len();
        }
    }
    format!("{}{}", &query[..start], &query[end..])
        .trim()
        .to_string()
}

pub fn parse_filter_import(text: &str) -> Result<Vec<PredefinedFilter>> {
    let text = text.trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        bail!(crate::tr!("文件内容为空", "The file is empty"));
    }
    let (imported, source_count) = if text
        .lines()
        .any(|line| line.trim() == "[PredefinedFiltersCollection]")
    {
        let imported = parse_qsettings_filters(text)?;
        let source_count = imported.len();
        (imported, source_count)
    } else {
        let parsed: Value = serde_json::from_str(text).map_err(|_| {
            anyhow!(crate::tr!(
                "不支持的文件格式，请选择 VCLogg2 JSON 或 Qt 预定义过滤器 CONF/INI 文件",
                "Unsupported file format. Select a VCLogg2 JSON or Qt predefined-filter CONF/INI file"
            ))
        })?;
        let source = parsed.get("filters").cloned().unwrap_or(parsed);
        let values = source.as_array().ok_or_else(|| {
            anyhow!(crate::tr!(
                "文件中没有过滤器列表",
                "The file doesn’t contain a filter list"
            ))
        })?;
        (
            values
                .iter()
                .filter_map(|value| serde_json::from_value::<ImportedFilter>(value.clone()).ok())
                .collect(),
            values.len(),
        )
    };
    let normalized = normalize_imported(imported);
    if source_count > 0 && normalized.is_empty() {
        bail!(crate::tr!("没有可用的过滤器", "No usable filters found"));
    }
    Ok(normalized)
}

pub fn parse_stored_filter(text: &str) -> Result<PredefinedFilter> {
    let imported = serde_json::from_str::<ImportedFilter>(text).map_err(|_| {
        anyhow!(crate::tr!(
            "预定义过滤器记录格式无效",
            "Invalid predefined-filter record"
        ))
    })?;
    normalize_imported(vec![imported])
        .into_iter()
        .next()
        .ok_or_else(|| {
            anyhow!(crate::tr!(
                "预定义过滤器记录缺少名称或匹配值",
                "The predefined-filter record is missing a name or match value"
            ))
        })
}

fn parse_qsettings_filters(text: &str) -> Result<Vec<ImportedFilter>> {
    let mut in_collection = false;
    let mut found_collection = false;
    let mut filters = BTreeMap::<usize, ImportedFilter>::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_collection = trimmed == "[PredefinedFiltersCollection]";
            found_collection |= in_collection;
            continue;
        }
        if !in_collection {
            continue;
        }
        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let Some(rest) = key.trim().strip_prefix("filters\\") else {
            continue;
        };
        let Some((index, field)) = rest.split_once('\\') else {
            continue;
        };
        let Ok(index) = index.parse::<usize>() else {
            continue;
        };
        if index == 0 || !matches!(field, "name" | "filter" | "regex") {
            continue;
        }
        let entry = filters.entry(index).or_insert_with(|| ImportedFilter {
            _legacy_id: format!("filter-{index}"),
            uuid: String::new(),
            name: String::new(),
            value: String::new(),
            use_regex: false,
            note: String::new(),
            collaborative: true,
            remote_references: Vec::new(),
            source: None,
            published: None,
        });
        let value = decode_qsettings_value(raw_value);
        match field {
            "name" => entry.name = value,
            "filter" => entry.value = value,
            "regex" => {
                entry.use_regex = matches!(value.to_ascii_lowercase().as_str(), "true" | "1")
            }
            _ => {}
        }
    }
    if !found_collection {
        bail!(crate::tr!(
            "不支持的文件格式，请选择 VCLogg2 JSON 或 Qt 预定义过滤器 CONF/INI 文件",
            "Unsupported file format. Select a VCLogg2 JSON or Qt predefined-filter CONF/INI file"
        ));
    }
    Ok(filters.into_values().collect())
}

fn decode_qsettings_value(raw: &str) -> String {
    let trimmed = raw.trim();
    let value = trimmed
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(trimmed);
    let mut decoded = String::new();
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        let Some(escaped) = chars.next() else {
            decoded.push('\\');
            break;
        };
        match escaped {
            '0' => decoded.push('\0'),
            'a' => decoded.push('\x07'),
            'b' => decoded.push('\x08'),
            'f' => decoded.push('\x0c'),
            'n' => decoded.push('\n'),
            'r' => decoded.push('\r'),
            't' => decoded.push('\t'),
            'v' => decoded.push('\x0b'),
            '"' => decoded.push('"'),
            '\\' => decoded.push('\\'),
            'x' => {
                let hex = chars.by_ref().take(4).collect::<String>();
                if hex.len() == 4
                    && let Ok(codepoint) = u32::from_str_radix(&hex, 16)
                    && let Some(character) = char::from_u32(codepoint)
                {
                    decoded.push(character);
                } else {
                    decoded.push_str("\\x");
                    decoded.push_str(&hex);
                }
            }
            unknown => {
                decoded.push('\\');
                decoded.push(unknown);
            }
        }
    }
    decoded
}

pub fn export_filter_json(filters: &[PredefinedFilter]) -> Result<String> {
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "version": 5,
        "filters": filters,
    }))?)
}

pub fn filter_snapshot(filter: &PredefinedFilter) -> FilterSnapshot {
    FilterSnapshot {
        name: filter.name.trim().to_string(),
        value: filter.value.trim().to_string(),
        use_regex: filter.use_regex,
        note: filter.note.trim().to_string(),
        collaborative: filter.collaborative,
    }
}

pub fn item_snapshot(item: &CloudFilterItem) -> FilterSnapshot {
    FilterSnapshot {
        name: item.name.trim().to_string(),
        value: item.value.trim().to_string(),
        use_regex: item.use_regex,
        note: item.note.trim().to_string(),
        collaborative: item.collaborative,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilterField {
    Name,
    Value,
    UseRegex,
    Note,
    Collaborative,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilterMergeConflict {
    pub id: FilterBranchId,
    pub base: Option<FilterSnapshot>,
    pub local: FilterSnapshot,
    pub incoming: FilterSnapshot,
    pub fields: Vec<FilterField>,
    pub incoming_remote_references: Vec<RemoteFilterReference>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilterMergeResult {
    pub snapshot: FilterSnapshot,
    pub conflicting_fields: Vec<FilterField>,
}

pub fn merge_filter_snapshots(
    base: Option<&FilterSnapshot>,
    local: &FilterSnapshot,
    remote: &FilterSnapshot,
) -> FilterMergeResult {
    let mut conflicts = Vec::new();
    let name = merge_field(
        FilterField::Name,
        base.map(|base| &base.name),
        &local.name,
        &remote.name,
        &mut conflicts,
    );
    let value = merge_field(
        FilterField::Value,
        base.map(|base| &base.value),
        &local.value,
        &remote.value,
        &mut conflicts,
    );
    let use_regex = merge_field(
        FilterField::UseRegex,
        base.map(|base| &base.use_regex),
        &local.use_regex,
        &remote.use_regex,
        &mut conflicts,
    );
    let note = merge_field(
        FilterField::Note,
        base.map(|base| &base.note),
        &local.note,
        &remote.note,
        &mut conflicts,
    );
    let collaborative = merge_field(
        FilterField::Collaborative,
        base.map(|base| &base.collaborative),
        &local.collaborative,
        &remote.collaborative,
        &mut conflicts,
    );
    FilterMergeResult {
        snapshot: FilterSnapshot {
            name,
            value,
            use_regex,
            note,
            collaborative,
        },
        conflicting_fields: conflicts,
    }
}

fn merge_field<T: Clone + Eq>(
    field: FilterField,
    base: Option<&T>,
    local: &T,
    remote: &T,
    conflicts: &mut Vec<FilterField>,
) -> T {
    if local == remote {
        return local.clone();
    }
    if base.is_some_and(|base| local == base) {
        return remote.clone();
    }
    if base.is_some_and(|base| remote == base) {
        return local.clone();
    }
    conflicts.push(field);
    local.clone()
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FilterCollectionMerge {
    pub filters: Vec<PredefinedFilter>,
    pub conflicts: Vec<FilterMergeConflict>,
}

pub fn merge_filter_collections(
    local: &[PredefinedFilter],
    incoming: Vec<PredefinedFilter>,
) -> FilterCollectionMerge {
    let mut filters = local.to_vec();
    let mut conflicts = Vec::new();
    for imported in incoming {
        let Some(index) = filters.iter().position(|filter| filter.id == imported.id) else {
            filters.push(imported);
            continue;
        };
        let local_snapshot = filter_snapshot(&filters[index]);
        let incoming_snapshot = filter_snapshot(&imported);
        if local_snapshot == incoming_snapshot {
            merge_remote_references(
                &mut filters[index].remote_references,
                imported.remote_references,
            );
            continue;
        }
        let base = common_baseline(&filters[index], &imported);
        let merged = merge_filter_snapshots(base.as_ref(), &local_snapshot, &incoming_snapshot);
        if merged.conflicting_fields.is_empty() {
            filters[index].apply_snapshot(merged.snapshot);
            merge_remote_references(
                &mut filters[index].remote_references,
                imported.remote_references,
            );
        } else {
            conflicts.push(FilterMergeConflict {
                id: imported.id,
                base,
                local: local_snapshot,
                incoming: incoming_snapshot,
                fields: merged.conflicting_fields,
                incoming_remote_references: imported.remote_references,
            });
        }
    }
    FilterCollectionMerge { filters, conflicts }
}

pub fn resolve_filter_conflict(
    local: &PredefinedFilter,
    conflict: &FilterMergeConflict,
    resolved: FilterSnapshot,
) -> PredefinedFilter {
    debug_assert_eq!(local.id, conflict.id);
    let mut updated = local.clone();
    updated.apply_snapshot(resolved.normalized());
    merge_remote_references(
        &mut updated.remote_references,
        conflict.incoming_remote_references.clone(),
    );
    normalize_remote_references(&mut updated);
    updated
}

fn common_baseline(
    local: &PredefinedFilter,
    incoming: &PredefinedFilter,
) -> Option<FilterSnapshot> {
    local
        .remote_references
        .iter()
        .filter(|reference| reference.relation == RemoteFilterRelation::Tracking)
        .filter_map(|local_reference| {
            incoming
                .remote_references
                .iter()
                .filter(|incoming_reference| {
                    incoming_reference.relation == RemoteFilterRelation::Tracking
                        && incoming_reference.filter_id == local_reference.filter_id
                        && normalized_server_url(&incoming_reference.server_url)
                            == normalized_server_url(&local_reference.server_url)
                })
                .filter_map(|incoming_reference| {
                    if local_reference.revision == incoming_reference.revision {
                        (local_reference.baseline == incoming_reference.baseline)
                            .then(|| (local_reference.revision, local_reference.baseline.clone()))
                    } else if local_reference.revision < incoming_reference.revision {
                        Some((local_reference.revision, local_reference.baseline.clone()))
                    } else {
                        Some((
                            incoming_reference.revision,
                            incoming_reference.baseline.clone(),
                        ))
                    }
                })
                .max_by_key(|(revision, _)| *revision)
        })
        .max_by_key(|(revision, _)| *revision)
        .map(|(_, snapshot)| snapshot)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloudFilterLocalStatus {
    NotDownloaded,
    Synced,
    LocalModified,
    RemoteUpdated,
    AutoMerge,
    Conflict,
    RemoteDeleted,
    ProtocolUnsupported,
}

pub fn remote_deleted_status(
    filter: &PredefinedFilter,
    server_url: &str,
    cloud_id: &str,
) -> Option<CloudFilterLocalStatus> {
    let cloud_id = FilterBranchId::parse(cloud_id)?;
    filter
        .tracking_reference(server_url)
        .filter(|reference| reference.filter_id == cloud_id)
        .map(|_| CloudFilterLocalStatus::RemoteDeleted)
}

pub fn find_local_filter_by_cloud_id<'a>(
    filters: &'a [PredefinedFilter],
    cloud_id: &str,
) -> Option<&'a PredefinedFilter> {
    let cloud_id = FilterBranchId::parse(cloud_id)?;
    filters.iter().find(|filter| filter.id == cloud_id)
}

pub fn remote_revision_anomaly(
    local: &PredefinedFilter,
    server_url: &str,
    item: &CloudFilterItem,
) -> bool {
    let Some(reference) = local.tracking_reference(server_url) else {
        return false;
    };
    let remote = item_snapshot(item);
    item.revision < reference.revision
        || (item.revision == reference.revision && remote != reference.baseline)
}

pub fn cloud_filter_local_status(
    filters: &[PredefinedFilter],
    server_url: &str,
    item: &CloudFilterItem,
) -> CloudFilterLocalStatus {
    if FilterBranchId::parse(&item.id).is_none() {
        return CloudFilterLocalStatus::ProtocolUnsupported;
    }
    let Some(local) = find_local_filter_by_cloud_id(filters, &item.id) else {
        return CloudFilterLocalStatus::NotDownloaded;
    };
    let remote = item_snapshot(item);
    let Some(reference) = local.tracking_reference(server_url) else {
        return if filter_snapshot(local) == remote {
            CloudFilterLocalStatus::RemoteUpdated
        } else {
            CloudFilterLocalStatus::Conflict
        };
    };
    if remote_revision_anomaly(local, server_url, item) {
        return CloudFilterLocalStatus::Conflict;
    }
    let local_snapshot = filter_snapshot(local);
    let local_changed = local_snapshot != reference.baseline;
    if item.revision == reference.revision {
        return if local_changed {
            CloudFilterLocalStatus::LocalModified
        } else {
            CloudFilterLocalStatus::Synced
        };
    }
    if !local_changed {
        return CloudFilterLocalStatus::RemoteUpdated;
    }
    let merge = merge_filter_snapshots(Some(&reference.baseline), &local_snapshot, &remote);
    if merge.conflicting_fields.is_empty() {
        CloudFilterLocalStatus::AutoMerge
    } else {
        CloudFilterLocalStatus::Conflict
    }
}

fn reference_from_item(server_url: &str, item: &CloudFilterItem) -> Result<RemoteFilterReference> {
    let filter_id = FilterBranchId::parse(&item.id).ok_or_else(|| {
        anyhow!(crate::tr!(
            "服务器返回了无效的过滤器 UUID",
            "The server returned an invalid filter UUID"
        ))
    })?;
    Ok(RemoteFilterReference {
        server_url: normalized_server_url(server_url),
        filter_id,
        revision: item.revision,
        owner_id: item.owner_id.clone(),
        owner_name: item.owner_name.clone(),
        relation: RemoteFilterRelation::Tracking,
        baseline: item_snapshot(item),
    })
}

fn upsert_tracking_reference(filter: &mut PredefinedFilter, reference: RemoteFilterReference) {
    filter.remote_references.retain(|existing| {
        !(existing.relation == RemoteFilterRelation::Tracking
            && normalized_server_url(&existing.server_url)
                == normalized_server_url(&reference.server_url))
    });
    filter.remote_references.push(reference);
}

pub fn create_local_filter_from_cloud(
    _existing: &[PredefinedFilter],
    server_url: &str,
    item: &CloudFilterItem,
) -> Result<PredefinedFilter> {
    let reference = reference_from_item(server_url, item)?;
    let snapshot = item_snapshot(item);
    Ok(PredefinedFilter {
        id: reference.filter_id,
        name: snapshot.name,
        value: snapshot.value,
        use_regex: snapshot.use_regex,
        note: snapshot.note,
        collaborative: snapshot.collaborative,
        remote_references: vec![reference],
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloudMergeKind {
    FastForward,
    Merged,
    BaselineAttached,
}

#[allow(clippy::result_large_err)]
pub fn merge_cloud_filter(
    local: &PredefinedFilter,
    server_url: &str,
    item: &CloudFilterItem,
) -> std::result::Result<(PredefinedFilter, CloudMergeKind), FilterMergeConflict> {
    let remote = item_snapshot(item);
    let reference = reference_from_item(server_url, item).map_err(|_| FilterMergeConflict {
        id: local.id,
        base: None,
        local: filter_snapshot(local),
        incoming: remote.clone(),
        fields: all_filter_fields(),
        incoming_remote_references: Vec::new(),
    })?;
    if reference.filter_id != local.id {
        return Err(FilterMergeConflict {
            id: local.id,
            base: None,
            local: filter_snapshot(local),
            incoming: remote,
            fields: all_filter_fields(),
            incoming_remote_references: vec![reference],
        });
    }
    let local_snapshot = filter_snapshot(local);
    let Some(baseline_reference) = local.tracking_reference(server_url) else {
        if local_snapshot != remote {
            let fields = merge_filter_snapshots(None, &local_snapshot, &remote).conflicting_fields;
            return Err(FilterMergeConflict {
                id: local.id,
                base: None,
                local: local_snapshot,
                incoming: remote,
                fields,
                incoming_remote_references: vec![reference],
            });
        }
        let mut updated = local.clone();
        upsert_tracking_reference(&mut updated, reference);
        return Ok((updated, CloudMergeKind::BaselineAttached));
    };
    if remote_revision_anomaly(local, server_url, item) {
        return Err(FilterMergeConflict {
            id: local.id,
            base: Some(baseline_reference.baseline.clone()),
            local: local_snapshot,
            incoming: remote,
            fields: all_filter_fields(),
            incoming_remote_references: vec![reference],
        });
    }
    if item.revision == baseline_reference.revision {
        return Ok((local.clone(), CloudMergeKind::BaselineAttached));
    }
    let merge =
        merge_filter_snapshots(Some(&baseline_reference.baseline), &local_snapshot, &remote);
    if !merge.conflicting_fields.is_empty() {
        return Err(FilterMergeConflict {
            id: local.id,
            base: Some(baseline_reference.baseline.clone()),
            local: local_snapshot,
            incoming: remote,
            fields: merge.conflicting_fields,
            incoming_remote_references: vec![reference],
        });
    }
    let mut updated = local.clone();
    let kind = if local_snapshot == baseline_reference.baseline {
        CloudMergeKind::FastForward
    } else {
        CloudMergeKind::Merged
    };
    updated.apply_snapshot(merge.snapshot);
    upsert_tracking_reference(&mut updated, reference);
    Ok((updated, kind))
}

fn all_filter_fields() -> Vec<FilterField> {
    vec![
        FilterField::Name,
        FilterField::Value,
        FilterField::UseRegex,
        FilterField::Note,
        FilterField::Collaborative,
    ]
}

pub fn attach_published_reference(
    filter: &PredefinedFilter,
    server_url: &str,
    filter_id: String,
    revision: u32,
    owner_id: &str,
    owner_name: &str,
) -> Result<PredefinedFilter> {
    let remote_id = FilterBranchId::parse(&filter_id).ok_or_else(|| {
        anyhow!(crate::tr!(
            "服务器返回了无效的过滤器 UUID",
            "The server returned an invalid filter UUID"
        ))
    })?;
    if remote_id != filter.id {
        bail!(crate::tr!(
            "服务器返回的过滤器 UUID 与本地分支不一致",
            "The filter UUID returned by the server doesn’t match the local branch"
        ));
    }
    let mut updated = filter.clone();
    let baseline = filter_snapshot(&updated);
    upsert_tracking_reference(
        &mut updated,
        RemoteFilterReference {
            server_url: normalized_server_url(server_url),
            filter_id: remote_id,
            revision,
            owner_id: owner_id.to_string(),
            owner_name: owner_name.to_string(),
            relation: RemoteFilterRelation::Tracking,
            baseline,
        },
    );
    Ok(updated)
}

pub fn keep_local_filter_at_cloud_revision(
    local: &PredefinedFilter,
    server_url: &str,
    item: &CloudFilterItem,
) -> Result<PredefinedFilter> {
    let mut updated = local.clone();
    upsert_tracking_reference(&mut updated, reference_from_item(server_url, item)?);
    Ok(updated)
}

pub fn fork_local_filter(local: &PredefinedFilter) -> PredefinedFilter {
    let mut fork = local.clone();
    fork.id = FilterBranchId::new();
    for reference in &mut fork.remote_references {
        if reference.relation == RemoteFilterRelation::Tracking {
            reference.relation = RemoteFilterRelation::DerivedFrom;
        }
    }
    normalize_remote_references(&mut fork);
    fork
}

pub fn detach_cloud_reference(
    local: &PredefinedFilter,
    server_url: &str,
    filter_id: &str,
) -> PredefinedFilter {
    let server_url = normalized_server_url(server_url);
    let remote_id = FilterBranchId::parse(filter_id);
    let mut updated = local.clone();
    updated.remote_references.retain(|reference| {
        normalized_server_url(&reference.server_url) != server_url
            || remote_id.is_none_or(|id| reference.filter_id != id)
    });
    updated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: FilterBranchId, name: &str, value: &str) -> PredefinedFilter {
        PredefinedFilter {
            id,
            name: name.to_string(),
            value: value.to_string(),
            use_regex: false,
            note: String::new(),
            collaborative: true,
            remote_references: Vec::new(),
        }
    }

    fn cloud_item(id: FilterBranchId, revision: u32, name: &str, value: &str) -> CloudFilterItem {
        CloudFilterItem {
            id: id.to_string(),
            revision,
            name: name.to_string(),
            value: value.to_string(),
            use_regex: false,
            note: String::new(),
            owner_id: "owner".into(),
            owner_name: "Owner".into(),
            like_count: 0,
            download_count: 0,
            liked: false,
            updated_at: 0,
            collaborative: true,
            can_edit: true,
            can_delete: true,
        }
    }

    #[test]
    fn uuid_is_canonicalized_and_invalid_legacy_uuid_is_generated_once() {
        let imported = parse_filter_import(
            r#"{"version":4,"filters":[{"id":"row-1","uuid":"550E8400-E29B-41D4-A716-446655440000","name":"A","value":"x"},{"id":"row-2","uuid":"invalid","name":"B","value":"y"}]}"#,
        )
        .unwrap();
        assert_eq!(
            imported[0].id.to_string(),
            "550e8400-e29b-41d4-a716-446655440000"
        );
        assert_ne!(imported[1].id, imported[0].id);
        let exported = export_filter_json(&imported).unwrap();
        assert!(exported.contains("\"version\": 5"));
        assert!(!exported.contains("\"id\""));
    }

    #[test]
    fn equal_content_with_different_uuid_coexists() {
        let local = sample(FilterBranchId::new(), "same", "value");
        let incoming = sample(FilterBranchId::new(), "same", "value");
        let result = merge_filter_collections(&[local], vec![incoming]);
        assert_eq!(result.filters.len(), 2);
        assert!(result.conflicts.is_empty());
    }

    #[test]
    fn same_uuid_and_same_content_folds() {
        let id = FilterBranchId::new();
        let local = sample(id, "same", "value");
        let incoming = sample(id, "same", "value");
        let result = merge_filter_collections(&[local], vec![incoming]);
        assert_eq!(result.filters.len(), 1);
        assert!(result.conflicts.is_empty());
    }

    #[test]
    fn three_way_merge_combines_non_overlapping_fields() {
        let base = FilterSnapshot {
            name: "base".into(),
            value: "x".into(),
            use_regex: false,
            note: String::new(),
            collaborative: true,
        };
        let mut local = base.clone();
        local.note = "local note".into();
        let mut remote = base.clone();
        remote.value = "remote".into();
        let result = merge_filter_snapshots(Some(&base), &local, &remote);
        assert!(result.conflicting_fields.is_empty());
        assert_eq!(result.snapshot.note, "local note");
        assert_eq!(result.snapshot.value, "remote");
    }

    #[test]
    fn three_way_merge_handles_boolean_fields_independently() {
        let base = FilterSnapshot {
            name: "base".into(),
            value: "x".into(),
            use_regex: false,
            note: String::new(),
            collaborative: true,
        };
        let mut local = base.clone();
        local.use_regex = true;
        let mut remote = base.clone();
        remote.collaborative = false;

        let result = merge_filter_snapshots(Some(&base), &local, &remote);

        assert!(result.conflicting_fields.is_empty());
        assert!(result.snapshot.use_regex);
        assert!(!result.snapshot.collaborative);
    }

    #[test]
    fn three_way_merge_reports_only_fields_changed_on_both_sides() {
        let base = FilterSnapshot {
            name: "base".into(),
            value: "x".into(),
            use_regex: false,
            note: String::new(),
            collaborative: true,
        };
        let mut local = base.clone();
        local.name = "local".into();
        let mut remote = base.clone();
        remote.name = "remote".into();
        let result = merge_filter_snapshots(Some(&base), &local, &remote);
        assert_eq!(result.conflicting_fields, vec![FilterField::Name]);
    }

    #[test]
    fn different_content_without_baseline_conflicts() {
        let id = FilterBranchId::new();
        let result =
            merge_filter_collections(&[sample(id, "local", "x")], vec![sample(id, "remote", "x")]);
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].fields, vec![FilterField::Name]);
    }

    #[test]
    fn same_remote_revision_with_different_baselines_is_not_a_common_base() {
        let id = FilterBranchId::new();
        let mut local = sample(id, "local", "x");
        let mut incoming = sample(id, "remote", "x");
        let local_baseline = filter_snapshot(&local);
        let incoming_baseline = filter_snapshot(&incoming);
        local.remote_references.push(RemoteFilterReference {
            server_url: "https://example.test".into(),
            filter_id: id,
            revision: 3,
            owner_id: "owner".into(),
            owner_name: "Owner".into(),
            relation: RemoteFilterRelation::Tracking,
            baseline: local_baseline,
        });
        incoming.remote_references.push(RemoteFilterReference {
            server_url: "https://example.test".into(),
            filter_id: id,
            revision: 3,
            owner_id: "owner".into(),
            owner_name: "Owner".into(),
            relation: RemoteFilterRelation::Tracking,
            baseline: incoming_baseline,
        });

        let result = merge_filter_collections(&[local], vec![incoming]);

        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].base, None);
    }

    #[test]
    fn fork_gets_new_uuid_and_keeps_only_lineage() {
        let id = FilterBranchId::new();
        let mut local = sample(id, "local", "x");
        local.remote_references.push(RemoteFilterReference {
            server_url: "https://example.test/".into(),
            filter_id: id,
            revision: 2,
            owner_id: "owner".into(),
            owner_name: "Owner".into(),
            relation: RemoteFilterRelation::Tracking,
            baseline: filter_snapshot(&local),
        });
        let fork = fork_local_filter(&local);
        assert_ne!(fork.id, local.id);
        assert_eq!(
            fork.remote_references[0].relation,
            RemoteFilterRelation::DerivedFrom
        );
        assert!(fork.tracking_reference("https://example.test").is_none());
    }

    #[test]
    fn checkout_preserves_remote_uuid_and_fast_forward_updates_baseline() {
        let id = FilterBranchId::new();
        let first = cloud_item(id, 1, "A", "one");
        let local = create_local_filter_from_cloud(&[], "https://example.test", &first).unwrap();
        assert_eq!(local.id, id);
        assert_eq!(
            cloud_filter_local_status(std::slice::from_ref(&local), "https://example.test", &first),
            CloudFilterLocalStatus::Synced
        );

        let second = cloud_item(id, 2, "A", "two");
        let (updated, kind) = merge_cloud_filter(&local, "https://example.test", &second).unwrap();
        assert_eq!(kind, CloudMergeKind::FastForward);
        assert_eq!(updated.id, id);
        assert_eq!(updated.value, "two");
        assert_eq!(
            updated
                .tracking_reference("https://example.test")
                .unwrap()
                .revision,
            2
        );
    }

    #[test]
    fn remote_revision_never_moves_baseline_backward() {
        let id = FilterBranchId::new();
        let latest = cloud_item(id, 3, "A", "three");
        let local = create_local_filter_from_cloud(&[], "https://example.test", &latest).unwrap();
        let stale = cloud_item(id, 2, "A", "two");
        assert_eq!(
            cloud_filter_local_status(std::slice::from_ref(&local), "https://example.test", &stale),
            CloudFilterLocalStatus::Conflict
        );
        assert!(merge_cloud_filter(&local, "https://example.test", &stale).is_err());
        assert_eq!(
            local
                .tracking_reference("https://example.test")
                .unwrap()
                .revision,
            3
        );
    }

    #[test]
    fn same_revision_with_different_content_is_an_anomaly() {
        let id = FilterBranchId::new();
        let baseline = cloud_item(id, 3, "A", "three");
        let local = create_local_filter_from_cloud(&[], "https://example.test", &baseline).unwrap();
        let inconsistent = cloud_item(id, 3, "A", "different");

        assert!(remote_revision_anomaly(
            &local,
            "https://example.test",
            &inconsistent
        ));
        assert_eq!(
            cloud_filter_local_status(
                std::slice::from_ref(&local),
                "https://example.test",
                &inconsistent
            ),
            CloudFilterLocalStatus::Conflict
        );
        assert!(merge_cloud_filter(&local, "https://example.test", &inconsistent).is_err());
        assert_eq!(local.value, "three");
    }

    #[test]
    fn automatic_remote_merge_keeps_local_change_pending_submission() {
        let id = FilterBranchId::new();
        let base = cloud_item(id, 1, "A", "one");
        let mut local = create_local_filter_from_cloud(&[], "https://example.test", &base).unwrap();
        local.note = "local note".into();
        let remote = cloud_item(id, 2, "A", "two");
        let (merged, kind) = merge_cloud_filter(&local, "https://example.test", &remote).unwrap();
        assert_eq!(kind, CloudMergeKind::Merged);
        assert_eq!(merged.note, "local note");
        assert_eq!(merged.value, "two");
        assert_eq!(
            cloud_filter_local_status(&[merged], "https://example.test", &remote),
            CloudFilterLocalStatus::LocalModified
        );
    }

    #[test]
    fn remote_delete_detaches_only_matching_reference() {
        let id = FilterBranchId::new();
        let item = cloud_item(id, 1, "A", "one");
        let local = create_local_filter_from_cloud(&[], "https://example.test", &item).unwrap();
        let detached = detach_cloud_reference(&local, "https://example.test/", &item.id);
        assert_eq!(detached.id, id);
        assert!(detached.remote_references.is_empty());
        assert_eq!(detached.name, "A");
    }
}
