use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::{
    Method, StatusCode,
    blocking::Client,
    header::{ACCEPT, CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue, SET_COOKIE},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use url::Url;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const PROFILE_SCHEMA: u32 = 1;
const PROFILE_FILE: &str = "cloud-filter-profile.json";
const CLIENT_UUID_FILE: &str = "client-uuid";
const DIRECTORY_CACHE_SCHEMA: u32 = 1;
const DIRECTORY_CACHE_LIMIT: usize = 256;
const KEYRING_SERVICE: &str = "com.vclogg2.desktop.cloud";
const CLIENT_COOKIE: &str = "vclogg_client_session";
pub const FILTER_UUID_BRANCHES_CAPABILITY: &str = "filter-uuid-branches-v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudConnectionProfile {
    pub server_url: String,
    pub display_name: String,
    pub identity_id: String,
    pub connected: bool,
    pub insecure: bool,
    pub default_server_url: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl CloudConnectionProfile {
    pub fn supports_uuid_filter_branches(&self) -> bool {
        self.capabilities
            .iter()
            .any(|capability| capability == FILTER_UUID_BRANCHES_CAPABILITY)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudFilterItem {
    pub id: String,
    pub revision: u32,
    pub name: String,
    pub value: String,
    pub use_regex: bool,
    pub note: String,
    pub owner_id: String,
    pub owner_name: String,
    pub like_count: u64,
    pub download_count: u64,
    pub liked: bool,
    pub updated_at: i64,
    pub collaborative: bool,
    pub can_edit: bool,
    pub can_delete: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudFilterPage {
    pub items: Vec<CloudFilterItem>,
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloudDirectoryPage {
    pub page: CloudFilterPage,
    pub cached_at: i64,
    pub offline: bool,
    pub server_url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudFilterShareItem {
    pub client_filter_id: String,
    pub name: String,
    pub value: String,
    pub use_regex: bool,
    pub note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_from_filter_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collaborative: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudFilterShareResult {
    pub client_filter_id: String,
    pub filter_id: String,
    pub revision: u32,
    pub note: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudFilterUpdate {
    pub name: String,
    pub value: String,
    pub use_regex: bool,
    pub note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collaborative: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudFilterMutationResult {
    pub filter_id: String,
    pub revision: u32,
    pub note: String,
    pub collaborative: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudFilterRevisionSummary {
    pub revision: u32,
    pub collaborative: bool,
    pub editor_id: String,
    pub editor_name: String,
    pub editor_role: String,
    pub created_at: i64,
    pub current: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudFilterRevisionPage {
    pub items: Vec<CloudFilterRevisionSummary>,
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudFilterRevision {
    pub filter_id: String,
    pub revision: u32,
    pub name: String,
    pub value: String,
    pub use_regex: bool,
    pub note: String,
    pub collaborative: bool,
    pub owner_id: String,
    pub owner_name: String,
    pub editor_id: String,
    pub editor_name: String,
    pub editor_role: String,
    pub created_at: i64,
    pub current: bool,
}

#[derive(Clone)]
pub struct CloudClient {
    core: Arc<CloudCore>,
}

impl CloudClient {
    pub fn open_default() -> anyhow::Result<Self> {
        let data_root = crate::app_paths::application_data_dir()
            .ok_or_else(|| {
                anyhow::anyhow!(crate::tr!(
                    "无法确定本机应用数据目录",
                    "Couldn’t determine the local application-data directory"
                ))
            })?
            .join("cloud");
        let default_server_url = std::env::var("VCLOGG2_DEFAULT_CLOUD_API_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                option_env!("VCLOGG2_DEFAULT_CLOUD_API_URL")
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_default();
        Ok(Self {
            core: Arc::new(CloudCore::new(data_root, default_server_url)?),
        })
    }

    pub fn saved_connection(&self) -> anyhow::Result<Option<CloudConnectionProfile>> {
        self.core.saved_connection().map_err(Into::into)
    }

    pub fn default_server_url(&self) -> &str {
        &self.core.default_server_url
    }

    pub fn connect(
        &self,
        server_url: &str,
        display_name: &str,
    ) -> anyhow::Result<CloudConnectionProfile> {
        self.core
            .connect(server_url, display_name)
            .map_err(Into::into)
    }

    pub fn disconnect(&self) -> anyhow::Result<()> {
        self.core.disconnect().map_err(Into::into)
    }

    pub fn list_filters(
        &self,
        query: &str,
        sort: &str,
        page: u32,
        page_size: u32,
    ) -> anyhow::Result<CloudFilterPage> {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("q", query.trim())
            .append_pair("sort", sort)
            .append_pair("page", &page.max(1).to_string())
            .append_pair("pageSize", &page_size.clamp(1, 100).to_string())
            .finish();
        self.core
            .request_json(
                Method::GET,
                &format!("/api/v1/filters?{query}"),
                None,
                false,
            )
            .map_err(Into::into)
    }

    pub fn list_filters_resilient(
        &self,
        query: &str,
        sort: &str,
        page: u32,
        page_size: u32,
    ) -> anyhow::Result<CloudDirectoryPage> {
        let server_url = self.core.active_server_url()?;
        match self.list_filters(query, sort, page, page_size) {
            Ok(page_result) => {
                let cached_at = unix_millis();
                let _ = self.core.cache_directory_page(
                    &server_url,
                    query,
                    sort,
                    &page_result,
                    cached_at,
                );
                Ok(CloudDirectoryPage {
                    page: page_result,
                    cached_at,
                    offline: false,
                    server_url,
                })
            }
            Err(online_error) => self
                .cached_filters(&server_url, query, sort, page, page_size)
                .map_err(|cache_error| {
                    anyhow::anyhow!(crate::tr_args!(
                        "{online_error}；本机也没有可用的云端目录缓存（{cache_error}）",
                        "{online_error}; no usable cloud-directory cache is available locally ({cache_error})"
                    ))
                }),
        }
    }

    pub fn cached_filters(
        &self,
        server_url: &str,
        query: &str,
        sort: &str,
        page: u32,
        page_size: u32,
    ) -> anyhow::Result<CloudDirectoryPage> {
        let server_url = normalize_server_url(server_url)?;
        let (page, cached_at) =
            self.core
                .load_cached_directory(&server_url, query, sort, page, page_size)?;
        Ok(CloudDirectoryPage {
            page,
            cached_at,
            offline: true,
            server_url,
        })
    }

    pub fn get_filter(&self, filter_id: &str) -> anyhow::Result<CloudFilterItem> {
        validate_filter_id(filter_id)?;
        self.core
            .request_json(
                Method::GET,
                &format!("/api/v1/filters/{filter_id}"),
                None,
                false,
            )
            .map_err(Into::into)
    }

    pub fn list_revisions(
        &self,
        filter_id: &str,
        page: u32,
        page_size: u32,
    ) -> anyhow::Result<CloudFilterRevisionPage> {
        validate_filter_id(filter_id)?;
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("page", &page.max(1).to_string())
            .append_pair("pageSize", &page_size.clamp(1, 100).to_string())
            .finish();
        self.core
            .request_json(
                Method::GET,
                &format!("/api/v1/filters/{filter_id}/revisions?{query}"),
                None,
                false,
            )
            .map_err(Into::into)
    }

    pub fn get_revision(
        &self,
        filter_id: &str,
        revision: u32,
    ) -> anyhow::Result<CloudFilterRevision> {
        validate_filter_id(filter_id)?;
        if revision == 0 {
            anyhow::bail!(crate::tr!(
                "云端过滤器修订号无效",
                "Invalid cloud-filter revision"
            ));
        }
        self.core
            .request_json(
                Method::GET,
                &format!("/api/v1/filters/{filter_id}/revisions/{revision}"),
                None,
                false,
            )
            .map_err(Into::into)
    }

    pub fn share_filters(
        &self,
        items: &[CloudFilterShareItem],
    ) -> anyhow::Result<Vec<CloudFilterShareResult>> {
        #[derive(Deserialize)]
        struct ShareResponse {
            items: Vec<CloudFilterShareResult>,
        }
        let body = serde_json::to_string(&json!({ "items": items }))?;
        self.core
            .request_json::<ShareResponse>(Method::POST, "/api/v1/filters/batch", Some(body), true)
            .map(|result| result.items)
            .map_err(Into::into)
    }

    pub fn update_filter(
        &self,
        filter_id: &str,
        update: &CloudFilterUpdate,
    ) -> anyhow::Result<CloudFilterMutationResult> {
        validate_filter_id(filter_id)?;
        self.core
            .request_json(
                Method::PATCH,
                &format!("/api/v1/filters/{filter_id}"),
                Some(serde_json::to_string(update)?),
                true,
            )
            .map_err(Into::into)
    }

    pub fn delete_filter(&self, filter_id: &str) -> anyhow::Result<()> {
        validate_filter_id(filter_id)?;
        self.core
            .request_empty(
                Method::DELETE,
                &format!("/api/v1/filters/{filter_id}"),
                None,
                true,
            )
            .map_err(Into::into)
    }

    pub fn record_downloads(&self, items: &[(String, u32)]) -> anyhow::Result<u64> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Download<'a> {
            filter_id: &'a str,
            revision: u32,
        }
        #[derive(Deserialize)]
        struct DownloadResponse {
            counted: u64,
        }
        let items = items
            .iter()
            .map(|(filter_id, revision)| Download {
                filter_id,
                revision: *revision,
            })
            .collect::<Vec<_>>();
        let body = serde_json::to_string(&json!({ "items": items }))?;
        self.core
            .request_json::<DownloadResponse>(
                Method::POST,
                "/api/v1/downloads/batch",
                Some(body),
                true,
            )
            .map(|result| result.counted)
            .map_err(Into::into)
    }

    pub fn set_liked(&self, filter_id: &str, liked: bool) -> anyhow::Result<(bool, u64)> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct LikeResponse {
            liked: bool,
            like_count: u64,
        }
        validate_filter_id(filter_id)?;
        self.core
            .request_json::<LikeResponse>(
                if liked { Method::PUT } else { Method::DELETE },
                &format!("/api/v1/filters/{filter_id}/like"),
                None,
                true,
            )
            .map(|result| (result.liked, result.like_count))
            .map_err(Into::into)
    }
}

#[derive(Debug)]
pub struct CloudError {
    code: Option<String>,
    message: String,
    current_revision: Option<u32>,
}

impl CloudError {
    fn message(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
            current_revision: None,
        }
    }

    fn coded(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: Some(code.into()),
            message: message.into(),
            current_revision: None,
        }
    }

    fn response(status: StatusCode, body: &str) -> Self {
        let error = serde_json::from_str::<Value>(body).ok();
        let code = error
            .as_ref()
            .and_then(|value| value.pointer("/error/code")?.as_str().map(str::to_owned));
        let message = error
            .as_ref()
            .and_then(|value| value.pointer("/error/message")?.as_str().map(str::to_owned))
            .unwrap_or_else(|| {
                crate::tr_args!(
                    "服务器请求失败（{}）",
                    "Server request failed ({})",
                    status.as_u16()
                )
            });
        let current_revision = error.as_ref().and_then(|value| {
            value
                .pointer("/error/currentRevision")
                .or_else(|| value.get("currentRevision"))?
                .as_u64()
                .and_then(|revision| u32::try_from(revision).ok())
        });
        Self {
            code,
            message,
            current_revision,
        }
    }

    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    pub fn current_revision(&self) -> Option<u32> {
        self.current_revision
    }
}

pub fn cloud_error(error: &anyhow::Error) -> Option<&CloudError> {
    error.downcast_ref::<CloudError>()
}

impl std::fmt::Display for CloudError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(code) = &self.code {
            write!(formatter, "{code}: {}", self.message)
        } else {
            formatter.write_str(&self.message)
        }
    }
}

impl std::error::Error for CloudError {}

type CloudResult<T> = Result<T, CloudError>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedProfile {
    schema_version: u32,
    server_url: String,
    display_name: String,
    identity_id: String,
    #[serde(default)]
    capabilities: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SecretState {
    cookie: String,
    csrf_token: String,
    expires_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryCache {
    schema_version: u32,
    server_url: String,
    pages: Vec<DirectoryCachePage>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryCachePage {
    query: String,
    sort: String,
    fetched_at: i64,
    page: CloudFilterPage,
}

#[derive(Default)]
struct RuntimeState {
    loaded: bool,
    profile: Option<SavedProfile>,
    secret: Option<SecretState>,
}

struct CloudCore {
    data_root: PathBuf,
    default_server_url: String,
    client: Client,
    state: Mutex<RuntimeState>,
}

impl CloudCore {
    fn new(data_root: PathBuf, default_server_url: String) -> CloudResult<Self> {
        let default_server_url = if default_server_url.trim().is_empty() {
            String::new()
        } else {
            normalize_server_url(&default_server_url)?
        };
        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .redirect(Policy::none())
            .build()
            .map_err(|_| {
                CloudError::message(crate::tr!(
                    "无法初始化 Rust 云端网络客户端",
                    "Couldn’t initialize the Rust cloud network client"
                ))
            })?;
        let core = Self {
            data_root,
            default_server_url,
            client,
            state: Mutex::new(RuntimeState::default()),
        };
        core.ensure_client_uuid()?;
        Ok(core)
    }

    fn profile_path(&self) -> PathBuf {
        self.data_root.join(PROFILE_FILE)
    }

    fn client_uuid_path(&self) -> PathBuf {
        self.data_root.join(CLIENT_UUID_FILE)
    }

    fn directory_cache_path(&self, server_url: &str) -> PathBuf {
        self.data_root
            .join("directory-cache")
            .join(format!("{}.json", credential_account(server_url)))
    }

    fn lock_state(&self) -> CloudResult<MutexGuard<'_, RuntimeState>> {
        self.state.lock().map_err(|_| {
            CloudError::message(crate::tr!(
                "Rust 云端客户端状态不可用",
                "Rust cloud-client state is unavailable"
            ))
        })
    }

    fn ensure_loaded(&self, state: &mut RuntimeState) -> CloudResult<()> {
        if state.loaded {
            return Ok(());
        }
        state.loaded = true;
        let bytes = match fs::read(self.profile_path()) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => {
                return Err(CloudError::message(crate::tr!(
                    "无法读取云端连接资料",
                    "Couldn’t read the cloud connection profile"
                )));
            }
        };
        let profile = serde_json::from_slice::<SavedProfile>(&bytes).map_err(|_| {
            CloudError::message(crate::tr!(
                "云端连接资料已损坏",
                "The cloud connection profile is corrupted"
            ))
        })?;
        if profile.schema_version != PROFILE_SCHEMA
            || normalize_server_url(&profile.server_url)? != profile.server_url
        {
            return Err(CloudError::message(crate::tr!(
                "云端连接资料格式无效",
                "Invalid cloud connection profile"
            )));
        }
        state.secret = load_secret(&profile.server_url)?;
        state.profile = Some(profile);
        Ok(())
    }

    fn saved_connection(&self) -> CloudResult<Option<CloudConnectionProfile>> {
        let mut state = self.lock_state()?;
        self.ensure_loaded(&mut state)?;
        Ok(state
            .profile
            .as_ref()
            .map(|profile| CloudConnectionProfile {
                server_url: profile.server_url.clone(),
                display_name: profile.display_name.clone(),
                identity_id: if state.secret.is_some() {
                    profile.identity_id.clone()
                } else {
                    String::new()
                },
                connected: state.secret.is_some(),
                insecure: profile.server_url.starts_with("http://"),
                default_server_url: self.default_server_url.clone(),
                capabilities: profile.capabilities.clone(),
            }))
    }

    fn active_server_url(&self) -> CloudResult<String> {
        let mut state = self.lock_state()?;
        self.ensure_loaded(&mut state)?;
        state
            .profile
            .as_ref()
            .map(|profile| profile.server_url.clone())
            .ok_or_else(|| {
                CloudError::message(crate::tr!(
                    "尚未连接云端服务器",
                    "Not connected to the cloud server"
                ))
            })
    }

    fn read_directory_cache(&self, server_url: &str) -> CloudResult<DirectoryCache> {
        let path = self.directory_cache_path(server_url);
        let bytes = fs::read(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                CloudError::message(crate::tr!("未找到缓存", "Cache not found"))
            } else {
                CloudError::message(crate::tr!(
                    "无法读取云端目录缓存",
                    "Couldn’t read the cloud-directory cache"
                ))
            }
        })?;
        let cache = serde_json::from_slice::<DirectoryCache>(&bytes).map_err(|_| {
            CloudError::message(crate::tr!(
                "云端目录缓存已损坏",
                "The cloud-directory cache is corrupted"
            ))
        })?;
        if cache.schema_version != DIRECTORY_CACHE_SCHEMA || cache.server_url != server_url {
            return Err(CloudError::message(crate::tr!(
                "云端目录缓存格式无效",
                "Invalid cloud-directory cache"
            )));
        }
        Ok(cache)
    }

    fn cache_directory_page(
        &self,
        server_url: &str,
        query: &str,
        sort: &str,
        page: &CloudFilterPage,
        fetched_at: i64,
    ) -> CloudResult<()> {
        let mut cache = self
            .read_directory_cache(server_url)
            .unwrap_or_else(|_| DirectoryCache {
                schema_version: DIRECTORY_CACHE_SCHEMA,
                server_url: server_url.to_string(),
                pages: Vec::new(),
            });
        let query = query.trim().to_string();
        cache.pages.retain(|entry| {
            entry.query != query
                || entry.sort != sort
                || entry.page.page != page.page
                || entry.page.page_size != page.page_size
        });
        cache.pages.push(DirectoryCachePage {
            query,
            sort: sort.to_string(),
            fetched_at,
            page: page.clone(),
        });
        cache
            .pages
            .sort_by_key(|entry| std::cmp::Reverse(entry.fetched_at));
        cache.pages.truncate(DIRECTORY_CACHE_LIMIT);
        let bytes = serde_json::to_vec_pretty(&cache).map_err(|_| {
            CloudError::message(crate::tr!(
                "无法序列化云端目录缓存",
                "Couldn’t serialize the cloud-directory cache"
            ))
        })?;
        persist_atomic(
            &self.directory_cache_path(server_url),
            &bytes,
            crate::tr!("云端目录缓存", "cloud-directory cache"),
        )
    }

    fn load_cached_directory(
        &self,
        server_url: &str,
        query: &str,
        sort: &str,
        page: u32,
        page_size: u32,
    ) -> CloudResult<(CloudFilterPage, i64)> {
        let cache = self.read_directory_cache(server_url)?;
        let cached_at = cache
            .pages
            .iter()
            .map(|entry| entry.fetched_at)
            .max()
            .ok_or_else(|| {
                CloudError::message(crate::tr!(
                    "缓存中没有云端过滤器",
                    "The cache contains no cloud filters"
                ))
            })?;
        let mut items = std::collections::BTreeMap::<String, CloudFilterItem>::new();
        for item in cache.pages.into_iter().flat_map(|entry| entry.page.items) {
            match items.entry(item.id.clone()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(item);
                }
                std::collections::btree_map::Entry::Occupied(mut entry)
                    if (item.revision, item.updated_at)
                        > (entry.get().revision, entry.get().updated_at) =>
                {
                    entry.insert(item);
                }
                std::collections::btree_map::Entry::Occupied(_) => {}
            }
        }
        let query = query.trim().to_lowercase();
        let mut items = items
            .into_values()
            .filter(|item| {
                query.is_empty()
                    || [
                        item.name.as_str(),
                        item.value.as_str(),
                        item.note.as_str(),
                        item.owner_name.as_str(),
                        item.owner_id.as_str(),
                    ]
                    .into_iter()
                    .any(|value| value.to_lowercase().contains(&query))
            })
            .collect::<Vec<_>>();
        match sort {
            "downloads" => items.sort_by_key(|item| {
                std::cmp::Reverse((item.download_count, item.updated_at, item.id.clone()))
            }),
            "likes" => items.sort_by_key(|item| {
                std::cmp::Reverse((item.like_count, item.updated_at, item.id.clone()))
            }),
            _ => items.sort_by_key(|item| {
                std::cmp::Reverse((item.updated_at, item.revision, item.id.clone()))
            }),
        }
        let page = page.max(1);
        let page_size = page_size.clamp(1, 100);
        let total = items.len() as u64;
        let start = usize::try_from(u64::from(page - 1) * u64::from(page_size))
            .unwrap_or(usize::MAX)
            .min(items.len());
        let end = start.saturating_add(page_size as usize).min(items.len());
        Ok((
            CloudFilterPage {
                items: items[start..end].to_vec(),
                page,
                page_size,
                total,
            },
            cached_at,
        ))
    }

    fn connect(&self, server_url: &str, display_name: &str) -> CloudResult<CloudConnectionProfile> {
        let server_url = normalize_server_url(if server_url.trim().is_empty() {
            &self.default_server_url
        } else {
            server_url
        })?;
        let display_name = display_name.trim();
        if display_name.is_empty() || display_name.chars().count() > 64 {
            return Err(CloudError::message(crate::tr!(
                "工号或昵称必须包含 1–64 个字符",
                "Employee ID or nickname must contain 1–64 characters"
            )));
        }
        let client_uuid = self.ensure_client_uuid()?;
        let mut state = self.lock_state()?;
        self.ensure_loaded(&mut state)?;
        let existing = state.profile.clone().filter(|profile| {
            profile.server_url == server_url
                && profile.display_name == display_name
                && profile.identity_id == client_uuid
        });
        let existing_secret = if existing.is_some() {
            state
                .secret
                .clone()
                .or_else(|| load_secret(&server_url).ok().flatten())
        } else {
            None
        };
        let (profile, secret) = match (existing, existing_secret) {
            (Some(mut profile), Some(secret)) => {
                match self.send_session(&profile, &secret, Method::GET, "/api/v1/me", None, false) {
                    Ok(response) if response.status.is_success() => {
                        #[derive(Deserialize)]
                        #[serde(rename_all = "camelCase")]
                        struct IdentityResponse {
                            #[serde(default)]
                            capabilities: Vec<String>,
                        }
                        let identity = serde_json::from_str::<IdentityResponse>(&response.body)
                            .map_err(|_| {
                                CloudError::message(crate::tr!(
                                    "服务器返回了无效的身份响应",
                                    "The server returned an invalid identity response"
                                ))
                            })?;
                        profile.capabilities = identity.capabilities;
                        (profile, secret)
                    }
                    _ => self.authorize(&server_url, display_name)?,
                }
            }
            _ => self.authorize(&server_url, display_name)?,
        };
        save_secret(&profile.server_url, &secret)?;
        self.persist_profile(&profile)?;
        state.profile = Some(profile.clone());
        state.secret = Some(secret);
        Ok(CloudConnectionProfile {
            server_url: profile.server_url.clone(),
            display_name: profile.display_name.clone(),
            identity_id: profile.identity_id.clone(),
            connected: true,
            insecure: profile.server_url.starts_with("http://"),
            default_server_url: self.default_server_url.clone(),
            capabilities: profile.capabilities.clone(),
        })
    }

    fn disconnect(&self) -> CloudResult<()> {
        let mut state = self.lock_state()?;
        self.ensure_loaded(&mut state)?;
        state.secret = None;
        Ok(())
    }

    fn request_json<T: for<'de> Deserialize<'de>>(
        &self,
        method: Method,
        target: &str,
        body: Option<String>,
        csrf: bool,
    ) -> CloudResult<T> {
        let response = self.authenticated_request(method, target, body, csrf)?;
        serde_json::from_str(&response.body).map_err(|_| {
            CloudError::message(crate::tr!(
                "服务器返回了无效数据",
                "The server returned invalid data"
            ))
        })
    }

    fn request_empty(
        &self,
        method: Method,
        target: &str,
        body: Option<String>,
        csrf: bool,
    ) -> CloudResult<()> {
        self.authenticated_request(method, target, body, csrf)
            .map(|_| ())
    }

    fn authenticated_request(
        &self,
        method: Method,
        target: &str,
        body: Option<String>,
        csrf: bool,
    ) -> CloudResult<HttpResponse> {
        let mut state = self.lock_state()?;
        self.ensure_loaded(&mut state)?;
        let profile = state.profile.clone().ok_or_else(|| {
            CloudError::message(crate::tr!(
                "请先连接云端过滤器服务器",
                "Connect to the cloud-filter server first"
            ))
        })?;
        let secret = state.secret.clone().ok_or_else(|| {
            CloudError::message(crate::tr!(
                "请先连接云端过滤器服务器",
                "Connect to the cloud-filter server first"
            ))
        })?;
        let first = self.send_session(
            &profile,
            &secret,
            method.clone(),
            target,
            body.clone(),
            csrf,
        )?;
        if first.status != StatusCode::UNAUTHORIZED {
            return first.success();
        }
        let (profile, secret) = self.authorize(&profile.server_url, &profile.display_name)?;
        save_secret(&profile.server_url, &secret)?;
        self.persist_profile(&profile)?;
        state.profile = Some(profile.clone());
        state.secret = Some(secret.clone());
        self.send_session(&profile, &secret, method, target, body, csrf)?
            .success()
    }

    fn authorize(
        &self,
        server_url: &str,
        display_name: &str,
    ) -> CloudResult<(SavedProfile, SecretState)> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct AuthorizationResponse {
            id: String,
            display_name: String,
            csrf_token: String,
            #[serde(default)]
            capabilities: Vec<String>,
        }
        let body = serde_json::to_string(&json!({
            "displayName": display_name,
            "uuid": self.ensure_client_uuid()?,
        }))
        .map_err(|_| {
            CloudError::message(crate::tr!(
                "无法序列化客户端授权请求",
                "Couldn’t serialize the client authorization request"
            ))
        })?;
        let response = self.send_public(
            server_url,
            Method::POST,
            "/api/v1/identities/register",
            Some(body),
        )?;
        let response = response.success()?;
        let authorization =
            serde_json::from_str::<AuthorizationResponse>(&response.body).map_err(|_| {
                CloudError::message(crate::tr!(
                    "服务器返回了无效的授权响应",
                    "The server returned an invalid authorization response"
                ))
            })?;
        let (cookie, max_age) = parse_client_cookie(&response.headers)?;
        Ok((
            SavedProfile {
                schema_version: PROFILE_SCHEMA,
                server_url: server_url.to_string(),
                display_name: authorization.display_name,
                identity_id: authorization.id,
                capabilities: authorization.capabilities,
            },
            SecretState {
                cookie,
                csrf_token: authorization.csrf_token,
                expires_at: unix_millis().saturating_add(max_age.saturating_mul(1000)),
            },
        ))
    }

    fn send_public(
        &self,
        server_url: &str,
        method: Method,
        target: &str,
        body: Option<String>,
    ) -> CloudResult<HttpResponse> {
        let url = Url::parse(&format!("{server_url}{target}")).map_err(|_| {
            CloudError::message(crate::tr!("云端请求地址无效", "Invalid cloud request URL"))
        })?;
        let body = body.unwrap_or_default();
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        if !body.is_empty() {
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        }
        self.send(method, url, headers, body)
    }

    fn send_session(
        &self,
        profile: &SavedProfile,
        secret: &SecretState,
        method: Method,
        target: &str,
        body: Option<String>,
        csrf: bool,
    ) -> CloudResult<HttpResponse> {
        let url = Url::parse(&format!("{}{target}", profile.server_url)).map_err(|_| {
            CloudError::message(crate::tr!("云端请求地址无效", "Invalid cloud request URL"))
        })?;
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        insert_header(
            &mut headers,
            COOKIE.as_str(),
            &format!("{CLIENT_COOKIE}={}", secret.cookie),
        )?;
        if csrf {
            // Keep the existing server protocol header for deployed services.
            insert_header(&mut headers, "X-VCLogg-CSRF", &secret.csrf_token)?;
        }
        let body = body.unwrap_or_default();
        if !body.is_empty() {
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        }
        self.send(method, url, headers, body)
    }

    fn send(
        &self,
        method: Method,
        url: Url,
        headers: HeaderMap,
        body: String,
    ) -> CloudResult<HttpResponse> {
        let mut request = self.client.request(method, url).headers(headers);
        if !body.is_empty() {
            request = request.body(body);
        }
        let response = request.send().map_err(|_| {
            CloudError::message(crate::tr!(
                "连接云端过滤器服务器失败或超时",
                "The cloud-filter server connection failed or timed out"
            ))
        })?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.text().map_err(|_| {
            CloudError::message(crate::tr!(
                "无法读取云端服务器响应",
                "Couldn’t read the cloud server response"
            ))
        })?;
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }

    fn ensure_client_uuid(&self) -> CloudResult<String> {
        let path = self.client_uuid_path();
        if let Ok(value) = fs::read_to_string(&path)
            && let Ok(value) = uuid::Uuid::parse_str(value.trim())
        {
            return Ok(value.hyphenated().to_string());
        }
        let value = uuid::Uuid::new_v4().hyphenated().to_string();
        persist_atomic(
            &path,
            value.as_bytes(),
            crate::tr!("客户端 UUID", "client UUID"),
        )?;
        Ok(value)
    }

    fn persist_profile(&self, profile: &SavedProfile) -> CloudResult<()> {
        let bytes = serde_json::to_vec_pretty(profile).map_err(|_| {
            CloudError::message(crate::tr!(
                "无法序列化云端连接资料",
                "Couldn’t serialize the cloud connection profile"
            ))
        })?;
        persist_atomic(
            &self.profile_path(),
            &bytes,
            crate::tr!("云端连接资料", "cloud connection profile"),
        )
    }
}

struct HttpResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

impl HttpResponse {
    fn success(self) -> CloudResult<Self> {
        if self.status.is_success() {
            Ok(self)
        } else {
            Err(CloudError::response(self.status, &self.body))
        }
    }
}

fn normalize_server_url(value: &str) -> CloudResult<String> {
    let mut url = Url::parse(value.trim()).map_err(|_| {
        CloudError::message(crate::tr!("服务器地址格式无效", "Invalid server address"))
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(CloudError::message(crate::tr!(
            "服务器地址必须是无账号、查询或片段的 HTTP/HTTPS 地址",
            "The server address must be an HTTP/HTTPS URL without credentials, a query, or a fragment"
        )));
    }
    if url.host_str().is_none() {
        return Err(CloudError::message(crate::tr!(
            "服务器地址缺少主机名",
            "The server address has no host name"
        )));
    }
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(&path);
    Ok(url.to_string().trim_end_matches('/').to_string())
}

fn validate_filter_id(filter_id: &str) -> anyhow::Result<()> {
    if filter_id.is_empty()
        || !filter_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        anyhow::bail!(crate::tr!("云端过滤器 ID 无效", "Invalid cloud-filter ID"));
    }
    Ok(())
}

fn parse_client_cookie(headers: &HeaderMap) -> CloudResult<(String, i64)> {
    for value in headers.get_all(SET_COOKIE) {
        let raw = value.to_str().map_err(|_| {
            CloudError::message(crate::tr!(
                "服务器返回了无效的会话 Cookie",
                "The server returned an invalid session cookie"
            ))
        })?;
        let mut parts = raw.split(';').map(str::trim);
        let Some(pair) = parts.next() else { continue };
        let Some(cookie) = pair.strip_prefix(&format!("{CLIENT_COOKIE}=")) else {
            continue;
        };
        let max_age = parts
            .find_map(|part| part.strip_prefix("Max-Age="))
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                CloudError::message(crate::tr!(
                    "服务器会话 Cookie 缺少有效期",
                    "The server session cookie has no expiration"
                ))
            })?;
        if cookie.is_empty() {
            return Err(CloudError::message(crate::tr!(
                "服务器返回了空的会话 Cookie",
                "The server returned an empty session cookie"
            )));
        }
        return Ok((cookie.to_string(), max_age));
    }
    Err(CloudError::message(crate::tr!(
        "服务器未发放客户端会话 Cookie",
        "The server didn’t issue a client session cookie"
    )))
}

fn load_secret(server_url: &str) -> CloudResult<Option<SecretState>> {
    let entry =
        keyring::Entry::new(KEYRING_SERVICE, &credential_account(server_url)).map_err(|_| {
            CloudError::coded(
                "credential_store_unavailable",
                crate::tr!("无法读取云端会话", "Couldn’t read the cloud session"),
            )
        })?;
    match entry.get_password() {
        Ok(value) => serde_json::from_str(&value).map(Some).map_err(|_| {
            CloudError::message(crate::tr!(
                "系统凭据库中的云端会话已损坏",
                "The cloud session in the system credential store is corrupted"
            ))
        }),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(CloudError::coded(
            "credential_store_unavailable",
            crate::tr!("无法读取云端会话", "Couldn’t read the cloud session"),
        )),
    }
}

fn save_secret(server_url: &str, secret: &SecretState) -> CloudResult<()> {
    let entry =
        keyring::Entry::new(KEYRING_SERVICE, &credential_account(server_url)).map_err(|_| {
            CloudError::coded(
                "credential_store_unavailable",
                crate::tr!("无法保存云端会话", "Couldn’t save the cloud session"),
            )
        })?;
    let value = serde_json::to_string(secret).map_err(|_| {
        CloudError::message(crate::tr!(
            "无法序列化云端会话",
            "Couldn’t serialize the cloud session"
        ))
    })?;
    entry.set_password(&value).map_err(|_| {
        CloudError::coded(
            "credential_store_unavailable",
            crate::tr!("无法保存云端会话", "Couldn’t save the cloud session"),
        )
    })
}

fn credential_account(server_url: &str) -> String {
    hex_lower(&Sha256::digest(server_url.as_bytes()))
}

fn persist_atomic(path: &Path, bytes: &[u8], label: &str) -> CloudResult<()> {
    let parent = path.parent().ok_or_else(|| {
        CloudError::message(crate::tr_args!(
            "{label}目录无效",
            "Invalid {label} directory"
        ))
    })?;
    fs::create_dir_all(parent).map_err(|_| {
        CloudError::message(crate::tr_args!(
            "无法创建{label}目录",
            "Couldn’t create the {label} directory"
        ))
    })?;
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes).map_err(|_| {
        CloudError::message(crate::tr_args!(
            "无法保存{label}",
            "Couldn’t save the {label}"
        ))
    })?;
    if path.exists() {
        fs::remove_file(path).map_err(|_| {
            CloudError::message(crate::tr_args!(
                "无法更新{label}",
                "Couldn’t update the {label}"
            ))
        })?;
    }
    fs::rename(temporary, path).map_err(|_| {
        CloudError::message(crate::tr_args!(
            "无法更新{label}",
            "Couldn’t update the {label}"
        ))
    })
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn insert_header(headers: &mut HeaderMap, name: &str, value: &str) -> CloudResult<()> {
    let name = reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
        CloudError::message(crate::tr!(
            "Rust 客户端生成了无效请求头",
            "The Rust client generated an invalid request header"
        ))
    })?;
    let value = HeaderValue::from_str(value).map_err(|_| {
        CloudError::message(crate::tr!(
            "Rust 客户端生成了无效请求头",
            "The Rust client generated an invalid request header"
        ))
    })?;
    headers.insert(name, value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_conflict_is_structured() {
        let error = CloudError::response(
            StatusCode::CONFLICT,
            r#"{"error":{"code":"revision_conflict","message":"changed","currentRevision":7}}"#,
        );
        assert_eq!(error.code(), Some("revision_conflict"));
        assert_eq!(error.current_revision(), Some(7));
    }

    #[test]
    fn capability_controls_uuid_branch_mutations() {
        let mut profile = CloudConnectionProfile {
            server_url: "https://example.test".into(),
            display_name: "Alice".into(),
            identity_id: "identity".into(),
            connected: true,
            insecure: false,
            default_server_url: String::new(),
            capabilities: Vec::new(),
        };
        assert!(!profile.supports_uuid_filter_branches());
        profile
            .capabilities
            .push(FILTER_UUID_BRANCHES_CAPABILITY.to_string());
        assert!(profile.supports_uuid_filter_branches());
    }
}
