//! User-initiated update checks. Network and storage work run off the UI thread.
use std::{
    collections::BTreeMap,
    io::Read as _,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::state_store::StateStore;

pub(crate) const RELEASES_URL: &str = "https://github.com/zhaiyanqi/vclogg2/releases";
const MANIFEST_URL: &str = "https://zhaiyanqi.github.io/vclogg2/latest.json";
const CACHE_SECONDS: u64 = 24 * 60 * 60;
const MAX_BODY_BYTES: u64 = 8 * 1024 * 1024;
static IN_FLIGHT: AtomicBool = AtomicBool::new(false);
static CACHE: Mutex<Option<CheckCache>> = Mutex::new(None);

pub(crate) struct CheckGuard;

impl CheckGuard {
    pub(crate) fn acquire() -> Option<Self> {
        IN_FLIGHT
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self)
    }
}

impl Drop for CheckGuard {
    fn drop(&mut self) {
        IN_FLIGHT.store(false, Ordering::Release);
    }
}

#[derive(Clone, Deserialize, Serialize)]
struct LocalizedNotes {
    release_notes: String,
    important_notes: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct ReleaseRecord {
    version: String,
    published_at: String,
    download_url: String,
    #[serde(default)]
    release_notes: String,
    #[serde(default)]
    important_notes: String,
    #[serde(default)]
    localized_notes: BTreeMap<String, LocalizedNotes>,
}

impl ReleaseRecord {
    pub(crate) fn release_version(&self) -> &str {
        &self.version
    }
    pub(crate) fn release_notes(&self, language: crate::i18n::Language) -> String {
        let key = match language {
            crate::i18n::Language::Chinese => "zh-CN",
            crate::i18n::Language::English => "en",
        };
        self.localized_notes
            .get(key)
            .map(|notes| notes.release_notes.clone())
            .unwrap_or_else(|| legacy_language_notes(&self.release_notes, language))
    }

    fn version(&self) -> Result<semver::Version> {
        let version = semver::Version::parse(&self.version)?;
        ensure!(
            version.pre.is_empty(),
            "Update channel requires a stable release"
        );
        ensure!(
            self.download_url == format!("{RELEASES_URL}/tag/v{}", self.version),
            "Unexpected release download URL"
        );
        chrono::DateTime::parse_from_rfc3339(&self.published_at)?;
        Ok(version)
    }
}

/// Compatibility for already-cached manifests generated before locale fields.
fn legacy_language_notes(body: &str, language: crate::i18n::Language) -> String {
    use crate::i18n::Language;
    let mut active = None;
    let mut fenced = false;
    let mut lines = Vec::new();
    for line in body.lines() {
        let stripped = line.trim();
        if stripped.starts_with("```") || stripped.starts_with("~~~") {
            fenced = !fenced;
        }
        if !fenced {
            if let Some(heading) = stripped.strip_prefix("## ") {
                active = match heading.trim().to_lowercase().as_str() {
                    "english" | "en" => Some(Language::English),
                    "简体中文" | "中文" | "zh-cn" => Some(Language::Chinese),
                    _ => None,
                };
                continue;
            }
            if stripped.starts_with("<a ")
                || stripped == "---"
                || stripped.starts_with("[Full Changelog / 完整变更记录]")
            {
                continue;
            }
        }
        if active == Some(language) {
            lines.push(line);
        }
    }
    lines.join("\n").trim().to_owned()
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Manifest {
    schema_version: u32,
    #[serde(flatten)]
    latest: ReleaseRecord,
    #[serde(default)]
    releases: Vec<ReleaseRecord>,
    #[serde(default)]
    history_complete: bool,
}

impl Manifest {
    pub(crate) fn release_version(&self) -> &str {
        self.latest.release_version()
    }
    pub(crate) fn download_url(&self) -> &str {
        &self.latest.download_url
    }

    pub(crate) fn upgrade_releases(&self) -> Vec<&ReleaseRecord> {
        let current = semver::Version::parse(crate::build_info::VERSION)
            .expect("build version was validated before displaying the manifest");
        let selected: Vec<_> = self
            .releases
            .iter()
            .filter(|release| {
                release
                    .version()
                    .is_ok_and(|version| version.cmp_precedence(&current).is_gt())
            })
            .collect();
        if selected.is_empty()
            && self
                .latest
                .version()
                .is_ok_and(|version| version.cmp_precedence(&current).is_gt())
        {
            vec![&self.latest]
        } else {
            selected
        }
    }

    fn version(&self) -> Result<semver::Version> {
        ensure!(
            self.schema_version == 1,
            "Unsupported update manifest schema"
        );
        let latest = self.latest.version()?;
        ensure!(
            self.releases.len() <= 30,
            "Too many releases in update manifest"
        );
        if let Some(first) = self.releases.first() {
            ensure!(
                first.version == self.latest.version
                    && first.download_url == self.latest.download_url,
                "Latest release disagrees with history"
            );
        }
        let mut previous = None;
        for release in &self.releases {
            let version = release.version()?;
            if let Some(previous) = previous {
                ensure!(
                    version.cmp_precedence(&previous).is_lt(),
                    "Release history must be unique and descending"
                );
            }
            previous = Some(version);
        }
        ensure!(
            !self.history_complete || !self.releases.is_empty(),
            "Missing complete history"
        );
        Ok(latest)
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct CheckCache {
    manifest: Option<Manifest>,
    checked_at: u64,
    failed_at: u64,
    failures: u32,
    retry_after_seconds: u64,
}

#[derive(Clone)]
pub(crate) enum CheckOutcome {
    Success { manifest: Manifest },
    UpToDate { version: String },
    Unavailable,
}

pub(crate) fn check(store: Option<&StateStore>) -> CheckOutcome {
    let mut memory = CACHE.lock().unwrap_or_else(|error| error.into_inner());
    let cache = memory.get_or_insert_with(|| {
        store
            .and_then(|store| match store.load_update_check_cache() {
                Ok(Some(json)) => serde_json::from_str(&json).ok(),
                Ok(None) => None,
                Err(error) => {
                    log::warn!("Could not read update cache: {error:#}");
                    None
                }
            })
            .unwrap_or_default()
    });
    let now = timestamp();
    // A backwards clock change invalidates deadlines instead of blocking checks indefinitely.
    if cache.failures > 0
        && now >= cache.failed_at
        && now - cache.failed_at < cache.retry_after_seconds.min(3600)
    {
        return CheckOutcome::Unavailable;
    }
    if cache.failures == 0
        && now >= cache.checked_at
        && now - cache.checked_at < CACHE_SECONDS
        && let Some(manifest) = &cache.manifest
        && let Ok(outcome) = check_manifest(manifest)
    {
        return outcome;
    }
    match fetch().and_then(|manifest| Ok((check_manifest(&manifest)?, manifest))) {
        Ok((outcome, manifest)) => {
            *cache = CheckCache {
                manifest: Some(manifest),
                checked_at: timestamp(),
                ..Default::default()
            };
            persist(store, cache);
            outcome
        }
        Err(error) => {
            log::warn!("Update check failed: {error:#}");
            cache.failures = cache.failures.saturating_add(1);
            cache.failed_at = timestamp();
            cache.retry_after_seconds =
                (60_u64 << cache.failures.saturating_sub(1).min(6)).min(3600);
            persist(store, cache);
            CheckOutcome::Unavailable
        }
    }
}

fn check_manifest(manifest: &Manifest) -> Result<CheckOutcome> {
    let latest = manifest.version()?;
    let current = semver::Version::parse(crate::build_info::VERSION)?;
    // Build metadata does not participate in SemVer precedence.
    if !latest.cmp_precedence(&current).is_gt() {
        return Ok(CheckOutcome::UpToDate {
            version: manifest.release_version().to_owned(),
        });
    }
    Ok(CheckOutcome::Success {
        manifest: manifest.clone(),
    })
}

fn fetch() -> Result<Manifest> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("VCLogg2/", env!("VCLOGG2_VERSION")))
        .build()?;
    // Request-level timeout also bounds the entire streamed response body.
    let response = client
        .get(MANIFEST_URL)
        .timeout(Duration::from_secs(10))
        .send()?
        .error_for_status()?;
    ensure!(
        response.status() == reqwest::StatusCode::OK,
        "Unexpected manifest response"
    );
    let mut body = Vec::new();
    response.take(MAX_BODY_BYTES + 1).read_to_end(&mut body)?;
    ensure!(
        body.len() as u64 <= MAX_BODY_BYTES,
        "Update manifest is too large"
    );
    serde_json::from_slice(&body).context("Invalid update manifest")
}

fn persist(store: Option<&StateStore>, cache: &CheckCache) {
    if let Some(store) = store {
        let result = serde_json::to_string(cache)
            .map_err(anyhow::Error::from)
            .and_then(|json| store.save_update_check_cache(&json));
        if let Err(error) = result {
            log::warn!("Could not save update cache: {error:#}");
        }
    }
}

fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
