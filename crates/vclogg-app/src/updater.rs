use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

#[cfg(unix)]
use std::process::{Command, Stdio};

use reqwest::{
    blocking::{Client, Response},
    redirect::Policy,
};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use url::Url;

const UPDATE_SCHEMA: u32 = 1;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_BLOCKMAP_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_GITHUB_RELEASE_BYTES: u64 = 1024 * 1024;
const GITHUB_REPOSITORY: &str = "zhaiyanqi/vclogg2";
const GITHUB_API_VERSION: &str = "2026-03-10";
const GITHUB_JSON_MEDIA_TYPE: &str = "application/vnd.github+json";

#[cfg(target_os = "windows")]
const UPDATE_PLATFORM: &str = "windows";
#[cfg(target_os = "macos")]
const UPDATE_PLATFORM: &str = "macos";
#[cfg(target_os = "linux")]
const UPDATE_PLATFORM: &str = "linux";

#[cfg(target_arch = "x86_64")]
const UPDATE_ARCHITECTURE: &str = "x86_64";
#[cfg(target_arch = "aarch64")]
const UPDATE_ARCHITECTURE: &str = "aarch64";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateManifest {
    pub schema_version: u32,
    pub product: String,
    pub version: String,
    pub platform: String,
    pub architecture: String,
    pub artifact: String,
    pub sha256: String,
    pub size: u64,
    pub blockmap: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateBlockmap {
    schema_version: u32,
    algorithm: String,
    chunk_size: usize,
    file: String,
    chunks: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
enum UpdateAssetSource {
    StaticFeed,
    GitHubRelease,
}

#[derive(Clone, Debug)]
struct UpdateAsset {
    url: Url,
    source: UpdateAssetSource,
}

#[derive(Clone, Debug)]
pub struct AvailableUpdate {
    pub manifest: UpdateManifest,
    artifact: UpdateAsset,
    blockmap: UpdateAsset,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubReleaseAsset {
    name: String,
    state: String,
    size: u64,
    digest: Option<String>,
    browser_download_url: String,
}

#[derive(Clone, Debug)]
pub struct DownloadedUpdate {
    pub version: String,
    pub archive_path: PathBuf,
}

#[derive(Clone, Default)]
pub struct UpdateDownloadProgress {
    transferred: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
}

impl UpdateDownloadProgress {
    pub fn snapshot(&self) -> (u64, u64) {
        (
            self.transferred.load(Ordering::Acquire),
            self.total.load(Ordering::Acquire),
        )
    }

    fn set_total(&self, total: u64) {
        self.total.store(total, Ordering::Release);
    }

    fn set_transferred(&self, transferred: u64) {
        self.transferred.store(transferred, Ordering::Release);
    }
}

#[derive(Clone)]
pub struct UpdateClient {
    client: Client,
    github_asset_client: Client,
    root: PathBuf,
}

impl UpdateClient {
    pub fn open_default() -> anyhow::Result<Self> {
        let root = crate::app_paths::data_local_dir()
            .ok_or_else(|| {
                anyhow::anyhow!(crate::tr!(
                    "无法确定本机应用数据目录",
                    "Couldn’t determine the local application-data directory"
                ))
            })?
            .join("VCLogg2")
            .join("updates");
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30 * 60))
            .redirect(Policy::none())
            .user_agent(crate::build_info::USER_AGENT)
            .build()?;
        let github_asset_client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30 * 60))
            .redirect(github_redirect_policy())
            .user_agent(crate::build_info::USER_AGENT)
            .build()?;
        Ok(Self {
            client,
            github_asset_client,
            root,
        })
    }

    pub fn check(
        &self,
        server_url: &str,
        current_version: &str,
    ) -> anyhow::Result<Option<AvailableUpdate>> {
        let feed_url = build_feed_url(server_url)?;
        let manifest_url = feed_url.join("latest.json")?;
        let manifest_bytes = read_response_bytes(
            self.client.get(manifest_url).send()?.error_for_status()?,
            MAX_MANIFEST_BYTES,
        )?;
        let manifest: UpdateManifest = serde_json::from_slice(&manifest_bytes)?;
        validate_manifest(&manifest)?;
        let current = Version::parse(current_version).map_err(|error| {
            anyhow::anyhow!(crate::tr_args!(
                "当前版本格式无效：{error}",
                "Invalid current-version format: {error}"
            ))
        })?;
        let available = Version::parse(&manifest.version).map_err(|error| {
            anyhow::anyhow!(crate::tr_args!(
                "更新清单版本格式无效：{error}",
                "Invalid update-manifest version: {error}"
            ))
        })?;
        let artifact_url = feed_url.join(&manifest.artifact)?;
        let blockmap_url = feed_url.join(&manifest.blockmap)?;
        Ok((available > current).then_some(AvailableUpdate {
            manifest,
            artifact: UpdateAsset {
                url: artifact_url,
                source: UpdateAssetSource::StaticFeed,
            },
            blockmap: UpdateAsset {
                url: blockmap_url,
                source: UpdateAssetSource::StaticFeed,
            },
        }))
    }

    pub fn check_github(&self, current_version: &str) -> anyhow::Result<Option<AvailableUpdate>> {
        let release_url = Url::parse(&format!(
            "https://api.github.com/repos/{GITHUB_REPOSITORY}/releases/latest"
        ))?;
        let release: GitHubRelease = read_json_response(
            self.client
                .get(release_url)
                .header("Accept", GITHUB_JSON_MEDIA_TYPE)
                .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
                .send()?
                .error_for_status()?,
            MAX_GITHUB_RELEASE_BYTES,
        )?;
        let current = parse_version(
            current_version,
            crate::tr!("当前版本格式无效", "Invalid current-version format"),
        )?;
        let release_version_text = release.tag_name.strip_prefix('v').ok_or_else(|| {
            anyhow::anyhow!(crate::tr!(
                "GitHub Release 标签必须以 v 开头",
                "The GitHub Release tag must start with v"
            ))
        })?;
        let release_version = parse_version(
            release_version_text,
            crate::tr!(
                "GitHub Release 标签版本格式无效",
                "The GitHub Release tag has an invalid version"
            ),
        )?;
        if release_version <= current {
            return Ok(None);
        }

        let manifest_name = format!("latest-{UPDATE_PLATFORM}-{UPDATE_ARCHITECTURE}.json");
        let manifest_asset = find_github_asset(&release, &manifest_name)?;
        if manifest_asset.size == 0 || manifest_asset.size > MAX_MANIFEST_BYTES {
            anyhow::bail!(crate::tr!(
                "GitHub 更新清单大小超出安全范围",
                "The GitHub update manifest exceeds the safety limit"
            ));
        }
        let manifest_url = validate_github_asset_url(manifest_asset, &release.tag_name)?;
        let manifest_bytes = read_response_bytes(
            self.github_asset_client
                .get(manifest_url)
                .send()?
                .error_for_status()?,
            MAX_MANIFEST_BYTES,
        )?;
        let manifest: UpdateManifest = serde_json::from_slice(&manifest_bytes)?;
        validate_manifest(&manifest)?;
        let manifest_version = parse_version(
            &manifest.version,
            crate::tr!(
                "GitHub 更新清单版本格式无效",
                "The GitHub update manifest has an invalid version"
            ),
        )?;
        if manifest_version != release_version {
            anyhow::bail!(crate::tr!(
                "GitHub Release 标签与更新清单版本不一致",
                "The GitHub Release tag doesn’t match the update manifest version"
            ));
        }

        let artifact_asset = find_github_asset(&release, &manifest.artifact)?;
        if artifact_asset.size != manifest.size {
            anyhow::bail!(crate::tr!(
                "GitHub Release 中的更新包大小与清单不一致",
                "The GitHub Release asset size doesn’t match the update manifest"
            ));
        }
        if let Some(digest) = artifact_asset.digest.as_deref() {
            let expected_digest = format!("sha256:{}", manifest.sha256);
            if !digest.eq_ignore_ascii_case(&expected_digest) {
                anyhow::bail!(crate::tr!(
                    "GitHub Release 中的更新包摘要与清单不一致",
                    "The GitHub Release asset digest doesn’t match the update manifest"
                ));
            }
        }
        let blockmap_asset = find_github_asset(&release, &manifest.blockmap)?;
        if blockmap_asset.size == 0 || blockmap_asset.size > MAX_BLOCKMAP_BYTES {
            anyhow::bail!(crate::tr!(
                "GitHub Release 中的分块清单大小超出安全范围",
                "The GitHub Release blockmap exceeds the safety limit"
            ));
        }

        Ok(Some(AvailableUpdate {
            manifest,
            artifact: UpdateAsset {
                url: validate_github_asset_url(artifact_asset, &release.tag_name)?,
                source: UpdateAssetSource::GitHubRelease,
            },
            blockmap: UpdateAsset {
                url: validate_github_asset_url(blockmap_asset, &release.tag_name)?,
                source: UpdateAssetSource::GitHubRelease,
            },
        }))
    }

    pub fn check_latest(
        &self,
        current_version: &str,
        static_server_url: Option<&str>,
    ) -> anyhow::Result<Option<AvailableUpdate>> {
        let github_result = self.check_github(current_version);
        let Some(static_server_url) = static_server_url.filter(|url| !url.trim().is_empty()) else {
            return github_result;
        };
        let static_result = self.check(static_server_url, current_version);
        match (github_result, static_result) {
            (Ok(github), Ok(static_feed)) => select_newer_update(github, static_feed),
            (Ok(update), Err(error)) => {
                log::warn!("自建更新源检查失败，继续使用 GitHub Releases 结果：{error:#}");
                Ok(update)
            }
            (Err(error), Ok(update)) => {
                log::warn!("GitHub Releases 检查失败，继续使用自建更新源结果：{error:#}");
                Ok(update)
            }
            (Err(github_error), Err(static_error)) => anyhow::bail!(crate::tr_args!(
                "GitHub Releases 与自建更新源均检查失败：GitHub：{github_error:#}；自建源：{static_error:#}",
                "Both GitHub Releases and the static update feed failed: GitHub: {github_error:#}; static feed: {static_error:#}"
            )),
        }
    }

    pub fn download(
        &self,
        update: &AvailableUpdate,
        progress: &UpdateDownloadProgress,
    ) -> anyhow::Result<DownloadedUpdate> {
        let manifest = &update.manifest;
        validate_manifest(manifest)?;
        let blockmap: UpdateBlockmap =
            read_json_response(self.get_update_asset(&update.blockmap)?, MAX_BLOCKMAP_BYTES)?;
        validate_blockmap(&blockmap, manifest)?;

        let version_root = self.root.join(format!("v{}", manifest.version));
        fs::create_dir_all(&version_root)?;
        let final_path = version_root.join(&manifest.artifact);
        if final_path.is_file()
            && final_path.metadata()?.len() == manifest.size
            && hash_file(&final_path)?.eq_ignore_ascii_case(&manifest.sha256)
        {
            progress.set_total(manifest.size);
            progress.set_transferred(manifest.size);
            return Ok(DownloadedUpdate {
                version: manifest.version.clone(),
                archive_path: final_path,
            });
        }

        let temporary_path = version_root.join(format!(
            ".{}.part-{}",
            manifest.artifact,
            std::process::id()
        ));
        if temporary_path.is_file() {
            fs::remove_file(&temporary_path)?;
        }
        let mut response = self.get_update_asset(&update.artifact)?;
        if response
            .content_length()
            .is_some_and(|length| length != manifest.size)
        {
            anyhow::bail!(crate::tr!(
                "服务器返回的更新包大小与清单不一致",
                "The update size returned by the server doesn’t match the manifest"
            ));
        }
        progress.set_total(manifest.size);
        progress.set_transferred(0);
        let result = download_and_verify(
            &mut response,
            &temporary_path,
            manifest,
            &blockmap,
            progress,
        );
        if let Err(error) = result {
            _ = fs::remove_file(&temporary_path);
            return Err(error);
        }
        if final_path.exists() {
            fs::remove_file(&final_path)?;
        }
        fs::rename(&temporary_path, &final_path)?;
        Ok(DownloadedUpdate {
            version: manifest.version.clone(),
            archive_path: final_path,
        })
    }

    fn get_update_asset(&self, asset: &UpdateAsset) -> anyhow::Result<Response> {
        let client = match asset.source {
            UpdateAssetSource::StaticFeed => &self.client,
            UpdateAssetSource::GitHubRelease => &self.github_asset_client,
        };
        Ok(client.get(asset.url.clone()).send()?.error_for_status()?)
    }
}

#[cfg(windows)]
pub fn launch_installer(update: &DownloadedUpdate) -> anyhow::Result<()> {
    let current_executable = std::env::current_exe()?;
    let install_directory = current_executable
        .parent()
        .ok_or_else(|| {
            anyhow::anyhow!(crate::tr!(
                "无法确定当前安装目录",
                "Couldn’t determine the current installation directory"
            ))
        })?
        .to_path_buf();
    crate::windows_update_helper::launch(
        &current_executable,
        &update.archive_path,
        &install_directory,
    )
}

#[cfg(unix)]
pub fn launch_installer(update: &DownloadedUpdate) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let current_executable = std::env::current_exe()?;
    let install_directory = current_install_directory(&current_executable)?;
    let helper_path = update
        .archive_path
        .parent()
        .ok_or_else(|| {
            anyhow::anyhow!(crate::tr!(
                "无法确定更新暂存目录",
                "Couldn’t determine the update staging directory"
            ))
        })?
        .join("Apply-VCLogg2Update.sh");
    fs::write(
        &helper_path,
        include_str!("../../../scripts/Apply-VCLogg2Update.sh"),
    )?;
    fs::set_permissions(&helper_path, fs::Permissions::from_mode(0o700))?;

    let log_path = helper_path.with_extension("log");
    let log = File::create(log_path)?;
    let error_log = log.try_clone()?;
    Command::new("/bin/sh")
        .arg(&helper_path)
        .arg("--archive")
        .arg(&update.archive_path)
        .arg("--install-directory")
        .arg(install_directory)
        .arg("--wait-pid")
        .arg(std::process::id().to_string())
        .arg("--platform")
        .arg(UPDATE_PLATFORM)
        .arg("--launch")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(error_log))
        .spawn()?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn current_install_directory(current_executable: &Path) -> anyhow::Result<PathBuf> {
    if let Some(app_bundle) = current_executable
        .ancestors()
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
    {
        return Ok(app_bundle.to_path_buf());
    }
    current_executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            anyhow::anyhow!(crate::tr!(
                "无法确定当前安装目录",
                "Couldn’t determine the current installation directory"
            ))
        })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn current_install_directory(current_executable: &Path) -> anyhow::Result<PathBuf> {
    current_executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            anyhow::anyhow!(crate::tr!(
                "无法确定当前安装目录",
                "Couldn’t determine the current installation directory"
            ))
        })
}

#[cfg(test)]
mod performance_tests {
    use std::{hint::black_box, time::Instant};

    use super::UpdateClient;

    #[test]
    #[ignore = "手动性能基准：cargo test -p vclogg2 --release benchmark_update_client_initialization -- --ignored --nocapture"]
    fn benchmark_update_client_initialization() {
        const RUNS: usize = 20;
        let started = Instant::now();
        for _ in 0..RUNS {
            black_box(UpdateClient::open_default().expect("更新客户端应能初始化"));
        }
        let elapsed = started.elapsed();

        eprintln!(
            "初始化更新客户端 {RUNS} 次：{elapsed:?}，平均：{:?}",
            elapsed / RUNS as u32
        );
    }
}

fn parse_version(value: &str, description: &str) -> anyhow::Result<Version> {
    Version::parse(value).map_err(|error| anyhow::anyhow!("{description}: {error}"))
}

fn select_newer_update(
    first: Option<AvailableUpdate>,
    second: Option<AvailableUpdate>,
) -> anyhow::Result<Option<AvailableUpdate>> {
    let Some(first) = first else {
        return Ok(second);
    };
    let Some(second) = second else {
        return Ok(Some(first));
    };
    let first_version = parse_version(
        &first.manifest.version,
        crate::tr!(
            "更新清单版本格式无效",
            "The update manifest has an invalid version"
        ),
    )?;
    let second_version = parse_version(
        &second.manifest.version,
        crate::tr!(
            "更新清单版本格式无效",
            "The update manifest has an invalid version"
        ),
    )?;
    Ok(Some(if second_version > first_version {
        second
    } else {
        first
    }))
}

fn find_github_asset<'a>(
    release: &'a GitHubRelease,
    name: &str,
) -> anyhow::Result<&'a GitHubReleaseAsset> {
    validate_file_name(name)?;
    let mut matches = release.assets.iter().filter(|asset| asset.name == name);
    let asset = matches.next().ok_or_else(|| {
        anyhow::anyhow!(crate::tr_args!(
            "GitHub Release 缺少更新文件：{name}",
            "The GitHub Release is missing update asset: {name}"
        ))
    })?;
    if matches.next().is_some() {
        anyhow::bail!(crate::tr_args!(
            "GitHub Release 包含重复更新文件：{name}",
            "The GitHub Release contains a duplicate update asset: {name}"
        ));
    }
    if asset.state != "uploaded" {
        anyhow::bail!(crate::tr_args!(
            "GitHub Release 更新文件尚未上传完成：{name}",
            "The GitHub Release asset hasn’t finished uploading: {name}"
        ));
    }
    Ok(asset)
}

fn validate_github_asset_url(asset: &GitHubReleaseAsset, tag_name: &str) -> anyhow::Result<Url> {
    let url = Url::parse(&asset.browser_download_url)?;
    let expected_path = format!(
        "/{GITHUB_REPOSITORY}/releases/download/{tag_name}/{}",
        asset.name
    );
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != expected_path
    {
        anyhow::bail!(crate::tr_args!(
            "GitHub Release 返回了无效的更新文件地址：{}",
            "GitHub returned an invalid update asset URL: {}",
            asset.name
        ));
    }
    Ok(url)
}

fn github_redirect_policy() -> Policy {
    Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 {
            return attempt.error("too many GitHub release asset redirects");
        }
        let url = attempt.url();
        let host_allowed = url.host_str().is_some_and(|host| {
            host == "github.com"
                || host == "githubusercontent.com"
                || host.ends_with(".githubusercontent.com")
        });
        if url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && host_allowed
        {
            attempt.follow()
        } else {
            attempt.error("GitHub release asset redirected to an untrusted URL")
        }
    })
}

fn build_feed_url(server_url: &str) -> anyhow::Result<Url> {
    let mut url = Url::parse(server_url.trim())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        anyhow::bail!(crate::tr!(
            "更新服务器地址必须是无账号信息的 HTTP 或 HTTPS 地址",
            "The update-server address must be an HTTP or HTTPS URL without credentials"
        ));
    }
    url.set_query(None);
    url.set_fragment(None);
    let base_path = url.path().trim_end_matches('/');
    url.set_path(&format!(
        "{base_path}/updates-vclogg2/{UPDATE_PLATFORM}-{UPDATE_ARCHITECTURE}/"
    ));
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::{UPDATE_ARCHITECTURE, UPDATE_PLATFORM, build_feed_url};

    #[test]
    fn update_feed_uses_the_vclogg2_product_path() {
        let platform_path = format!("{UPDATE_PLATFORM}-{UPDATE_ARCHITECTURE}");
        assert_eq!(
            build_feed_url("http://127.0.0.1:8787")
                .expect("root server URL should be valid")
                .as_str(),
            format!("http://127.0.0.1:8787/updates-vclogg2/{platform_path}/")
        );
        assert_eq!(
            build_feed_url("https://example.test/services/log-viewer/?ignored=1#fragment")
                .expect("nested server URL should be valid")
                .as_str(),
            format!("https://example.test/services/log-viewer/updates-vclogg2/{platform_path}/")
        );
    }

    #[test]
    fn update_feed_rejects_credentials_and_non_http_schemes() {
        let mut credentialed_url = url::Url::parse("https://example.test").unwrap();
        credentialed_url.set_username("user").unwrap();
        assert!(build_feed_url(credentialed_url.as_str()).is_err());
        assert!(build_feed_url("file:///tmp/log-viewer").is_err());
    }
}

fn validate_manifest(manifest: &UpdateManifest) -> anyhow::Result<()> {
    if manifest.schema_version != UPDATE_SCHEMA
        || manifest.product != "VCLogg2"
        || manifest.platform != UPDATE_PLATFORM
        || manifest.architecture != UPDATE_ARCHITECTURE
    {
        anyhow::bail!(crate::tr!(
            "更新清单与当前应用不兼容",
            "The update manifest isn’t compatible with this application"
        ));
    }
    validate_file_name(&manifest.artifact)?;
    validate_file_name(&manifest.blockmap)?;
    if manifest.size == 0 || manifest.size > MAX_ARTIFACT_BYTES {
        anyhow::bail!(crate::tr!(
            "更新包大小超出安全范围",
            "The update size exceeds the safety limit"
        ));
    }
    validate_sha256(&manifest.sha256)?;
    Ok(())
}

fn validate_blockmap(blockmap: &UpdateBlockmap, manifest: &UpdateManifest) -> anyhow::Result<()> {
    if blockmap.schema_version != UPDATE_SCHEMA
        || blockmap.algorithm != "sha256"
        || blockmap.file != manifest.artifact
        || blockmap.chunk_size == 0
        || blockmap.chunk_size > 16 * 1024 * 1024
    {
        anyhow::bail!(crate::tr!(
            "更新分块清单无效",
            "Invalid update chunk manifest"
        ));
    }
    let expected_chunks = manifest.size.div_ceil(u64::try_from(blockmap.chunk_size)?) as usize;
    if blockmap.chunks.len() != expected_chunks {
        anyhow::bail!(crate::tr!(
            "更新分块数量与文件大小不一致",
            "The update chunk count doesn’t match the file size"
        ));
    }
    for hash in &blockmap.chunks {
        validate_sha256(hash)?;
    }
    Ok(())
}

fn validate_file_name(value: &str) -> anyhow::Result<()> {
    let path = Path::new(value);
    if value.is_empty()
        || path.file_name().is_none()
        || path.file_name().is_some_and(|name| name != value)
        || path.components().count() != 1
    {
        anyhow::bail!(crate::tr!(
            "更新清单包含无效文件名",
            "The update manifest contains an invalid file name"
        ));
    }
    Ok(())
}

fn validate_sha256(value: &str) -> anyhow::Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!(crate::tr!(
            "更新清单包含无效 SHA-256",
            "The update manifest contains an invalid SHA-256 value"
        ));
    }
    Ok(())
}

fn read_json_response<T: for<'de> Deserialize<'de>>(
    response: Response,
    limit: u64,
) -> anyhow::Result<T> {
    Ok(serde_json::from_slice(&read_response_bytes(
        response, limit,
    )?)?)
}

fn read_response_bytes(response: Response, limit: u64) -> anyhow::Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        anyhow::bail!(crate::tr!(
            "服务器响应超过安全上限",
            "The server response exceeds the safety limit"
        ));
    }
    let mut body = Vec::new();
    response.take(limit + 1).read_to_end(&mut body)?;
    if body.len() as u64 > limit {
        anyhow::bail!(crate::tr!(
            "服务器响应超过安全上限",
            "The server response exceeds the safety limit"
        ));
    }
    Ok(body)
}

fn download_and_verify(
    response: &mut Response,
    path: &Path,
    manifest: &UpdateManifest,
    blockmap: &UpdateBlockmap,
    progress: &UpdateDownloadProgress,
) -> anyhow::Result<()> {
    let mut output = OpenOptions::new().create_new(true).write(true).open(path)?;
    let mut whole_hasher = Sha256::new();
    let mut chunk_hasher = Sha256::new();
    let mut chunk_bytes = 0_usize;
    let mut chunk_index = 0_usize;
    let mut transferred = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = response.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        transferred = transferred.saturating_add(read as u64);
        if transferred > manifest.size {
            anyhow::bail!(crate::tr!(
                "下载内容超过更新清单声明大小",
                "Downloaded content exceeds the size declared by the update manifest"
            ));
        }
        output.write_all(&buffer[..read])?;
        whole_hasher.update(&buffer[..read]);
        let mut cursor = 0_usize;
        while cursor < read {
            let remaining = blockmap.chunk_size - chunk_bytes;
            let take = remaining.min(read - cursor);
            chunk_hasher.update(&buffer[cursor..cursor + take]);
            cursor += take;
            chunk_bytes += take;
            if chunk_bytes == blockmap.chunk_size {
                verify_chunk(&mut chunk_hasher, blockmap, chunk_index)?;
                chunk_index += 1;
                chunk_bytes = 0;
            }
        }
        progress.set_transferred(transferred);
    }
    if chunk_bytes > 0 {
        verify_chunk(&mut chunk_hasher, blockmap, chunk_index)?;
        chunk_index += 1;
    }
    if transferred != manifest.size || chunk_index != blockmap.chunks.len() {
        anyhow::bail!(crate::tr!("下载内容不完整", "The download is incomplete"));
    }
    let whole_hash = format!("{:x}", whole_hasher.finalize());
    if !whole_hash.eq_ignore_ascii_case(&manifest.sha256) {
        anyhow::bail!(crate::tr!(
            "更新包 SHA-256 校验失败",
            "Update package SHA-256 verification failed"
        ));
    }
    output.flush()?;
    output.sync_all()?;
    Ok(())
}

fn verify_chunk(
    hasher: &mut Sha256,
    blockmap: &UpdateBlockmap,
    index: usize,
) -> anyhow::Result<()> {
    let expected = blockmap.chunks.get(index).ok_or_else(|| {
        anyhow::anyhow!(crate::tr!(
            "更新包包含清单之外的分块",
            "The update package contains a chunk not listed in the manifest"
        ))
    })?;
    let actual = format!("{:x}", hasher.finalize_reset());
    if !actual.eq_ignore_ascii_case(expected) {
        anyhow::bail!(crate::tr_args!(
            "更新包第 {} 个分块校验失败",
            "Update package chunk {} failed verification",
            index + 1
        ));
    }
    Ok(())
}

fn hash_file(path: &Path) -> anyhow::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
