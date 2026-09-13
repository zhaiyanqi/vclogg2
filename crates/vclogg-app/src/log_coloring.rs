//! Global, user-editable coloring groups. Presets are seeded only during migration.
use serde::{Deserialize, Serialize};

use crate::color_labels::{LogLevelColorRule, LogRuleMatch, default_log_level_rules};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct LogColoringSettings {
    pub groups: Vec<LogColoringGroup>,
    pub active_group_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct LogColoringGroup {
    pub id: String,
    pub name: String,
    pub preset: Option<String>,
    #[serde(default)]
    pub example: String,
    pub rules: Vec<LogLevelColorRule>,
}

impl Default for LogColoringSettings {
    fn default() -> Self {
        Self::from_legacy(default_log_level_rules())
    }
}

impl LogColoringSettings {
    pub fn from_legacy(rules: Vec<LogLevelColorRule>) -> Self {
        let mut groups = vec![LogColoringGroup {
            id: "default".into(),
            name: crate::tr!("默认分组", "Default group").into(),
            preset: None,
            example: String::new(),
            rules,
        }];
        groups.extend(presets());
        Self {
            groups,
            active_group_id: "default".into(),
        }
    }

    pub fn from_initialized_json(value: &str) -> Self {
        let mut settings: Self = serde_json::from_str(value).unwrap_or_else(|error| {
            log::warn!("Could not decode log coloring groups: {error}");
            let mut fallback = Self::from_legacy(default_log_level_rules());
            fallback.groups.truncate(1);
            fallback
        });
        if settings.groups.is_empty() {
            settings
                .groups
                .push(Self::from_legacy(Vec::new()).groups.remove(0));
        }
        if !settings
            .groups
            .iter()
            .any(|group| group.id == settings.active_group_id)
        {
            settings.active_group_id = settings.groups[0].id.clone();
        }
        settings
    }

    pub fn active_rules(&self) -> &[LogLevelColorRule] {
        self.groups
            .iter()
            .find(|group| group.id == self.active_group_id)
            .map(|group| group.rules.as_slice())
            .unwrap_or_default()
    }

    /// Apply only edited groups and activation, preserving other windows' changes.
    pub fn merge(&self, base: &Self, current: &Self) -> Self {
        let mut merged = current.clone();
        merged.groups.retain(|group| {
            !base.groups.iter().any(|old| old.id == group.id)
                || self.groups.iter().any(|edited| edited.id == group.id)
        });
        for group in &self.groups {
            if base.groups.iter().find(|old| old.id == group.id) == Some(group) {
                continue;
            }
            if let Some(target) = merged.groups.iter_mut().find(|old| old.id == group.id) {
                *target = group.clone();
            } else {
                merged.groups.push(group.clone());
            }
        }
        if self.active_group_id != base.active_group_id {
            merged.active_group_id = self.active_group_id.clone();
        }
        if merged.groups.is_empty() {
            if let Some(group) = self.groups.first() {
                merged.groups.push(group.clone());
            } else {
                merged
                    .groups
                    .push(Self::from_legacy(Vec::new()).groups.remove(0));
            }
        }
        if !merged
            .groups
            .iter()
            .any(|group| group.id == merged.active_group_id)
        {
            merged.active_group_id = merged.groups[0].id.clone();
        }
        merged
    }
}

pub(crate) fn presets() -> Vec<LogColoringGroup> {
    vec![
        android_preset(),
        preset(
            "java",
            "Java / Spring",
            &["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "FATAL"],
            "2026-09-14 12:30:00.123 {level} [main] com.example.App : Example message",
            str::to_owned,
        ),
        preset(
            "python",
            "Python",
            &["DEBUG", "INFO", "WARNING", "ERROR", "CRITICAL"],
            "2026-09-14 12:30:00,123 {level} app: Example message",
            str::to_owned,
        ),
        preset(
            "nginx",
            crate::tr!("Nginx 错误日志", "Nginx error logs"),
            &[
                "debug", "info", "notice", "warn", "error", "crit", "alert", "emerg",
            ],
            "2026/09/14 12:30:00 [{level}] 1234#1234: Example message",
            |level| {
                format!(
                    r"^\d{{4}}/\d{{2}}/\d{{2}}\s+\d{{2}}:\d{{2}}:\d{{2}}\s+\[(?P<highlight>{level})\]\s+\d+#\d+:"
                )
            },
        ),
        preset(
            "http",
            crate::tr!("HTTP 访问日志", "HTTP access logs"),
            &["1xx", "2xx", "3xx", "4xx", "5xx"],
            "127.0.0.1 - - [14/Sep/2026:12:30:00 +0800] \"GET /example HTTP/1.1\" {level} 512",
            |level| {
                format!(
                    r#"^\S+\s+\S+\s+\S+\s+\[[^\]\r\n]+\]\s+"(?:[^"\\\r\n]|\\.)*"\s+(?P<highlight>{}\d{{2}})\s+(?:\d+|-)(?:\s|$)"#,
                    &level[..1]
                )
            },
        ),
    ]
}

fn android_preset() -> LogColoringGroup {
    let mut group = preset(
        "android",
        "Android Logcat",
        &["V", "D", "I", "W", "E", "F"],
        "09-14 12:30:00.123 1234 5678 {level} ActivityManager: Example message",
        |level| {
            format!(
                r"^(?:\d{{4}}-)?\d{{2}}-\d{{2}}\s+\d{{2}}:\d{{2}}:\d{{2}}\.\d+\s+\d+\s+\d+\s+(?P<highlight>{level})\s+[^:\r\n]+:"
            )
        },
    );
    let brief = group.rules.iter().enumerate().map(|(ix, rule)| {
        let mut rule = rule.clone();
        let level = ["V", "D", "I", "W", "E", "F"][ix];
        rule.id = format!("android-brief-{ix}");
        rule.keyword = format!(r"^(?:(?:\d{{4}}-)?\d{{2}}-\d{{2}}\s+\d{{2}}:\d{{2}}:\d{{2}}\.\d+\s+)?(?P<highlight>{level})/[^\r\n(]+\(\s*\d+\):");
        rule
    }).collect::<Vec<_>>();
    group.rules.extend(brief);
    group
}

fn preset(
    id: &str,
    name: &str,
    levels: &[&str],
    example: &str,
    pattern: impl Fn(&str) -> String,
) -> LogColoringGroup {
    let rules = levels
        .iter()
        .enumerate()
        .map(|(ix, level)| {
            // These colors are editable log data, not application chrome.
            let (text_color, background_color) = match *level {
                "V" | "TRACE" | "1xx" => (0x475569, 0xf1f5f9),
                "D" | "DEBUG" | "debug" => (0x1e40af, 0xdbeafe),
                "I" | "INFO" | "info" | "2xx" => (0x166534, 0xdcfce7),
                "W" | "WARN" | "WARNING" | "warn" | "notice" | "3xx" => (0x854d0e, 0xfef9c3),
                "E" | "ERROR" | "error" | "4xx" => (0x991b1b, 0xfee2e2),
                _ => (0x6b21a8, 0xf3e8ff),
            };
            LogLevelColorRule {
                id: format!("{id}-{ix}"),
                keyword: pattern(level),
                match_kind: if matches!(id, "java" | "python") {
                    LogRuleMatch::Keyword
                } else {
                    LogRuleMatch::Regex
                },
                keyword_only: false,
                text_color,
                background_color,
                text_alpha: u8::MAX,
                background_alpha: u8::MAX,
            }
        })
        .collect();
    LogColoringGroup {
        id: format!("preset-{id}"),
        name: name.into(),
        preset: Some(id.into()),
        example: example.into(),
        rules,
    }
}

impl LogColoringGroup {
    pub fn example_for(&self, rule: &LogLevelColorRule) -> String {
        let level = match self.preset.as_deref() {
            Some("http") => rule
                .keyword
                .split("(?P<highlight>")
                .nth(1)
                .and_then(|tail| tail.chars().next())
                .map(|value| format!("{value}00")),
            Some("android" | "nginx") => rule
                .keyword
                .split("(?P<highlight>")
                .nth(1)
                .and_then(|tail| tail.split(')').next())
                .map(str::to_owned),
            _ => None,
        }
        .unwrap_or_else(|| rule.keyword.clone());
        if self.preset.as_deref() == Some("android") && rule.keyword.contains(")/") {
            return format!("{level}/ActivityManager( 1234): Example message");
        }
        if self.example.is_empty() {
            format!("{level} {}", crate::tr!("日志", "log"))
        } else {
            self.example.replace("{level}", &level)
        }
    }
}
