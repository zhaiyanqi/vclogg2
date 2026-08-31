use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

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

#[derive(Clone, Debug)]
pub struct AvailableUpdate {
    pub manifest: UpdateManifest,
    feed_url: Url,
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
            .user_agent(concat!("VCLogg2/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { client, root })
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
        Ok((available > current).then_some(AvailableUpdate { manifest, feed_url }))
    }

    pub fn download(
        &self,
        update: &AvailableUpdate,
        progress: &UpdateDownloadProgress,
    ) -> anyhow::Result<DownloadedUpdate> {
        let manifest = &update.manifest;
        validate_manifest(manifest)?;
        let blockmap_url = update.feed_url.join(&manifest.blockmap)?;
        let blockmap: UpdateBlockmap = read_json_response(
            self.client.get(blockmap_url).send()?.error_for_status()?,
            MAX_BLOCKMAP_BYTES,
        )?;
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
        let artifact_url = update.feed_url.join(&manifest.artifact)?;
        let mut response = self.client.get(artifact_url).send()?.error_for_status()?;
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
}

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
    let helper_path = update
        .archive_path
        .parent()
        .ok_or_else(|| {
            anyhow::anyhow!(crate::tr!(
                "无法确定更新暂存目录",
                "Couldn’t determine the update staging directory"
            ))
        })?
        .join("Apply-VCLogg2Update.ps1");
    fs::write(
        &helper_path,
        include_str!("../../../scripts/Apply-VCLogg2Update.ps1"),
    )?;
    let powershell = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    if !powershell.is_file() {
        anyhow::bail!(crate::tr!(
            "找不到系统 PowerShell，无法启动更新安装助手",
            "System PowerShell wasn’t found, so the update installer couldn’t start"
        ));
    }
    let log_path = helper_path.with_extension("log");
    let log = File::create(log_path)?;
    let error_log = log.try_clone()?;
    let mut command = Command::new(powershell);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x0800_0000);
    }
    command
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&helper_path)
        .arg("-ArchivePath")
        .arg(&update.archive_path)
        .arg("-InstallDirectory")
        .arg(install_directory)
        .arg("-WaitForProcessId")
        .arg(std::process::id().to_string())
        .arg("-Launch")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(error_log))
        .spawn()?;
    Ok(())
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
    url.set_path(&format!("{base_path}/updates-vclogg2/win-x64/"));
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::build_feed_url;

    #[test]
    fn update_feed_uses_the_vclogg2_product_path() {
        assert_eq!(
            build_feed_url("http://127.0.0.1:8787")
                .expect("root server URL should be valid")
                .as_str(),
            "http://127.0.0.1:8787/updates-vclogg2/win-x64/"
        );
        assert_eq!(
            build_feed_url("https://example.test/services/log-viewer/?ignored=1#fragment")
                .expect("nested server URL should be valid")
                .as_str(),
            "https://example.test/services/log-viewer/updates-vclogg2/win-x64/"
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
        || manifest.platform != "windows"
        || manifest.architecture != "x86_64"
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
