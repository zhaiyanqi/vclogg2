use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use anyhow::Result;

pub(crate) struct OpenRequest {
    pub(crate) paths: Vec<PathBuf>,
}

pub(crate) enum Startup {
    Primary(PrimaryInstance),
    Forwarded,
}

pub(crate) struct PrimaryInstance {
    #[cfg(windows)]
    server: windows::ServerPipe,
    #[cfg(unix)]
    server: unix::ServerSocket,
}

pub(crate) fn command_line_paths() -> Vec<PathBuf> {
    let working_directory = std::env::current_dir().ok();
    let mut seen = BTreeSet::new();
    let mut paths = Vec::new();

    for argument in std::env::args_os().skip(1) {
        if argument.is_empty() || argument.to_string_lossy().starts_with('-') {
            continue;
        }
        let path = PathBuf::from(argument);
        let path = if path.is_absolute() {
            path
        } else if let Some(working_directory) = &working_directory {
            working_directory.join(path)
        } else {
            path
        };
        if seen.insert(path_identity(&path)) {
            paths.push(path);
        }
    }

    paths
}

pub(crate) fn acquire_or_forward(paths: &[PathBuf]) -> Result<Startup> {
    #[cfg(windows)]
    {
        windows::acquire_or_forward(paths)
    }

    #[cfg(unix)]
    {
        unix::acquire_or_forward(paths)
    }
}

impl PrimaryInstance {
    pub(crate) fn start_listener(self) -> Result<Option<async_channel::Receiver<OpenRequest>>> {
        #[cfg(windows)]
        {
            windows::start_listener(self.server).map(Some)
        }

        #[cfg(unix)]
        {
            unix::start_listener(self.server).map(Some)
        }
    }
}

#[cfg(unix)]
mod unix {
    use std::{
        ffi::OsString,
        fmt::Write as _,
        fs,
        io::{Read as _, Write as _},
        os::unix::{
            ffi::{OsStrExt as _, OsStringExt as _},
            fs::{FileTypeExt as _, PermissionsExt as _},
            net::{UnixListener, UnixStream},
        },
        path::{Path, PathBuf},
        thread,
        time::{Duration, Instant},
    };

    use anyhow::{Context as _, Result, anyhow, bail};
    use sha2::{Digest as _, Sha256};

    use super::{OpenRequest, PrimaryInstance, Startup};

    const SOCKET_MAGIC: &[u8; 8] = b"VCLGIPC1";
    const MAX_PATHS: usize = 4_096;
    const MAX_REQUEST_BYTES: usize = 1024 * 1024;
    const STARTUP_FORWARD_TIMEOUT: Duration = Duration::from_secs(5);
    const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
    const RETRY_INTERVAL: Duration = Duration::from_millis(40);

    pub(super) struct ServerSocket {
        listener: UnixListener,
        path: PathBuf,
    }

    impl Drop for ServerSocket {
        fn drop(&mut self) {
            _ = fs::remove_file(&self.path);
        }
    }

    pub(super) fn acquire_or_forward(paths: &[PathBuf]) -> Result<Startup> {
        let socket_path = socket_path()?;
        let request = encode_request(paths)?;
        let started_at = Instant::now();

        loop {
            match UnixListener::bind(&socket_path) {
                Ok(listener) => {
                    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
                        .with_context(|| {
                            format!("无法限制单实例套接字权限：{}", socket_path.display())
                        })?;
                    return Ok(Startup::Primary(PrimaryInstance {
                        server: ServerSocket {
                            listener,
                            path: socket_path,
                        },
                    }));
                }
                Err(bind_error) => match UnixStream::connect(&socket_path) {
                    Ok(stream) => match forward_request(stream, &request) {
                        Ok(()) => return Ok(Startup::Forwarded),
                        Err(_) if started_at.elapsed() < STARTUP_FORWARD_TIMEOUT => {
                            thread::sleep(RETRY_INTERVAL);
                        }
                        Err(forward_error) => {
                            return Err(forward_error).with_context(|| {
                                format!(
                                    "已有 VCLogg2 实例，但无法转交启动请求（创建套接字错误 {bind_error}）"
                                )
                            });
                        }
                    },
                    Err(connect_error)
                        if matches!(
                            connect_error.kind(),
                            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                        ) =>
                    {
                        remove_stale_socket(&socket_path)?;
                        if started_at.elapsed() >= STARTUP_FORWARD_TIMEOUT {
                            return Err(connect_error).with_context(|| {
                                format!("无法创建或连接单实例套接字：{}", socket_path.display())
                            });
                        }
                        thread::sleep(RETRY_INTERVAL);
                    }
                    Err(_connect_error) if started_at.elapsed() < STARTUP_FORWARD_TIMEOUT => {
                        thread::sleep(RETRY_INTERVAL);
                    }
                    Err(connect_error) => {
                        return Err(connect_error).with_context(|| {
                            format!("无法连接已有 VCLogg2 实例：{}", socket_path.display())
                        });
                    }
                },
            }
        }
    }

    pub(super) fn start_listener(
        server: ServerSocket,
    ) -> Result<async_channel::Receiver<OpenRequest>> {
        let (sender, receiver) = async_channel::unbounded();
        thread::Builder::new()
            .name("vclogg2-single-instance".to_string())
            .spawn(move || run_listener(server, sender))
            .context("无法启动 VCLogg2 单实例监听线程")?;
        Ok(receiver)
    }

    fn run_listener(server: ServerSocket, sender: async_channel::Sender<OpenRequest>) {
        loop {
            let (stream, _) = match server.listener.accept() {
                Ok(connection) => connection,
                Err(error) => {
                    log::error!("等待单实例客户端失败：{error}");
                    thread::sleep(RETRY_INTERVAL);
                    continue;
                }
            };
            match serve_connection(stream, &sender) {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => log::warn!("忽略无效的单实例启动请求：{error:#}"),
            }
        }
    }

    fn serve_connection(
        mut stream: UnixStream,
        sender: &async_channel::Sender<OpenRequest>,
    ) -> Result<bool> {
        stream.set_read_timeout(Some(CONNECTION_TIMEOUT))?;
        stream.set_write_timeout(Some(CONNECTION_TIMEOUT))?;
        let request = read_request(&mut stream);
        let keep_listening = match request {
            Ok(request) => {
                let accepted = sender.send_blocking(request).is_ok();
                stream.write_all(&[u8::from(accepted)])?;
                accepted
            }
            Err(error) => {
                _ = stream.write_all(&[0]);
                return Err(error);
            }
        };
        stream.flush()?;
        Ok(keep_listening)
    }

    fn forward_request(mut stream: UnixStream, request: &[u8]) -> Result<()> {
        stream.set_read_timeout(Some(CONNECTION_TIMEOUT))?;
        stream.set_write_timeout(Some(CONNECTION_TIMEOUT))?;
        stream.write_all(request)?;
        stream.shutdown(std::net::Shutdown::Write)?;
        let mut acknowledgement = [0_u8; 1];
        stream.read_exact(&mut acknowledgement)?;
        if acknowledgement[0] != 1 {
            bail!("已有 VCLogg2 实例未接受启动请求");
        }
        Ok(())
    }

    fn read_request(stream: &mut UnixStream) -> Result<OpenRequest> {
        let mut request = Vec::new();
        stream
            .take((MAX_REQUEST_BYTES + 1) as u64)
            .read_to_end(&mut request)?;
        if request.len() > MAX_REQUEST_BYTES {
            bail!("外部打开请求超过 {} KiB 限制", MAX_REQUEST_BYTES / 1024);
        }
        decode_request(&request)
    }

    fn encode_request(paths: &[PathBuf]) -> Result<Vec<u8>> {
        if paths.len() > MAX_PATHS {
            bail!("一次最多转交 {MAX_PATHS} 个文件路径");
        }
        let mut request = Vec::with_capacity(SOCKET_MAGIC.len() + 4);
        request.extend_from_slice(SOCKET_MAGIC);
        request.extend_from_slice(&(paths.len() as u32).to_le_bytes());
        for path in paths {
            let path = path.as_os_str().as_bytes();
            let path_len = u32::try_from(path.len()).context("文件路径过长")?;
            request.extend_from_slice(&path_len.to_le_bytes());
            request.extend_from_slice(path);
            if request.len() > MAX_REQUEST_BYTES {
                bail!("外部打开请求超过 {} KiB 限制", MAX_REQUEST_BYTES / 1024);
            }
        }
        Ok(request)
    }

    fn decode_request(request: &[u8]) -> Result<OpenRequest> {
        if request.len() < SOCKET_MAGIC.len() + 4 || &request[..SOCKET_MAGIC.len()] != SOCKET_MAGIC
        {
            bail!("单实例启动请求协议不匹配");
        }
        let mut offset = SOCKET_MAGIC.len();
        let path_count = take_u32(request, &mut offset)? as usize;
        if path_count > MAX_PATHS {
            bail!("单实例启动请求包含过多文件路径");
        }
        let mut paths = Vec::with_capacity(path_count);
        for _ in 0..path_count {
            let byte_count = take_u32(request, &mut offset)? as usize;
            let end = offset
                .checked_add(byte_count)
                .filter(|end| *end <= request.len())
                .context("单实例启动请求中的文件路径不完整")?;
            let path = PathBuf::from(OsString::from_vec(request[offset..end].to_vec()));
            offset = end;
            if path.as_os_str().is_empty() {
                bail!("单实例启动请求包含空文件路径");
            }
            paths.push(path);
        }
        if offset != request.len() {
            bail!("单实例启动请求包含多余数据");
        }
        Ok(OpenRequest { paths })
    }

    fn take_u32(input: &[u8], offset: &mut usize) -> Result<u32> {
        let end = offset
            .checked_add(4)
            .filter(|end| *end <= input.len())
            .context("单实例启动请求不完整")?;
        let bytes = input[*offset..end]
            .try_into()
            .map_err(|_| anyhow!("单实例启动请求不完整"))?;
        *offset = end;
        Ok(u32::from_le_bytes(bytes))
    }

    fn socket_path() -> Result<PathBuf> {
        let identity = crate::app_paths::data_local_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("vclogg2-default-user"));
        let mut hasher = Sha256::new();
        hasher.update(identity.as_os_str().as_bytes());
        let digest = hasher.finalize();
        let mut user_key = String::with_capacity(16);
        for byte in digest.iter().take(8) {
            _ = write!(user_key, "{byte:02x}");
        }
        let directory = std::env::temp_dir().join(format!("vclogg2-{user_key}"));
        fs::create_dir_all(&directory)
            .with_context(|| format!("无法创建单实例套接字目录：{}", directory.display()))?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("无法限制单实例套接字目录权限：{}", directory.display()))?;
        Ok(directory.join("instance.sock"))
    }

    fn remove_stale_socket(path: &Path) -> Result<()> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(path)
                .with_context(|| format!("无法清理失效的单实例套接字：{}", path.display())),
            Ok(_) => bail!("单实例套接字路径被非套接字占用：{}", path.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("无法检查单实例套接字：{}", path.display()))
            }
        }
    }
}

fn path_identity(path: &Path) -> String {
    let identity = path.as_os_str().to_string_lossy();
    if cfg!(windows) {
        identity.to_lowercase()
    } else {
        identity.into_owned()
    }
}

#[cfg(windows)]
mod windows {
    use std::{
        ffi::OsString,
        fmt::Write as _,
        os::windows::ffi::{OsStrExt as _, OsStringExt as _},
        path::PathBuf,
        ptr::{null, null_mut},
        thread,
        time::{Duration, Instant},
    };

    use anyhow::{Context as _, Result, anyhow, bail};
    use sha2::{Digest as _, Sha256};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, PIPE_ACCESS_DUPLEX, ReadFile,
            WriteFile,
        },
        System::Pipes::{
            CallNamedPipeW, ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe,
            PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_WAIT,
        },
    };

    use super::{OpenRequest, PrimaryInstance, Startup};

    const PIPE_MAGIC: &[u8; 8] = b"VCLGIPC1";
    const MAX_PATHS: usize = 4_096;
    const MAX_REQUEST_BYTES: usize = 1024 * 1024;
    const CLIENT_CALL_TIMEOUT_MS: u32 = 500;
    const STARTUP_FORWARD_TIMEOUT: Duration = Duration::from_secs(5);
    const RETRY_INTERVAL: Duration = Duration::from_millis(40);

    pub(super) struct ServerPipe(OwnedHandle);

    struct OwnedHandle(HANDLE);

    // Windows kernel handles may be closed or used from any process thread. This
    // wrapper transfers sole ownership to the blocking named-pipe listener.
    unsafe impl Send for OwnedHandle {}

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    pub(super) fn acquire_or_forward(paths: &[PathBuf]) -> Result<Startup> {
        let name = pipe_name();
        let request = encode_request(paths)?;
        let started_at = Instant::now();

        loop {
            let create_error = match create_server_pipe(&name) {
                Ok(server) => {
                    return Ok(Startup::Primary(PrimaryInstance { server }));
                }
                Err(error) => error,
            };

            match forward_request(&name, &request) {
                Ok(()) => return Ok(Startup::Forwarded),
                Err(_) if started_at.elapsed() < STARTUP_FORWARD_TIMEOUT => {
                    thread::sleep(RETRY_INTERVAL);
                }
                Err(forward_error) => {
                    return Err(forward_error).with_context(|| {
                        format!(
                            "已有 VCLogg2 实例，但无法转交启动请求（创建管道错误 {create_error}）"
                        )
                    });
                }
            }
        }
    }

    pub(super) fn start_listener(
        server: ServerPipe,
    ) -> Result<async_channel::Receiver<OpenRequest>> {
        let (sender, receiver) = async_channel::unbounded();
        thread::Builder::new()
            .name("vclogg2-single-instance".to_string())
            .spawn(move || run_listener(server, sender))
            .context("无法启动 VCLogg2 单实例监听线程")?;
        Ok(receiver)
    }

    fn run_listener(server: ServerPipe, sender: async_channel::Sender<OpenRequest>) {
        loop {
            match serve_connection(&server, &sender) {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => {
                    log::error!("单实例启动请求处理失败：{error:#}");
                    unsafe { DisconnectNamedPipe(server.0.0) };
                    thread::sleep(RETRY_INTERVAL);
                }
            }
        }
    }

    fn serve_connection(
        server: &ServerPipe,
        sender: &async_channel::Sender<OpenRequest>,
    ) -> Result<bool> {
        let connected = unsafe { ConnectNamedPipe(server.0.0, null_mut()) } != 0;
        if !connected {
            let error = unsafe { GetLastError() };
            if error != windows_sys::Win32::Foundation::ERROR_PIPE_CONNECTED {
                bail!("等待单实例客户端失败：Windows 错误 {error}");
            }
        }

        let request = read_request(server.0.0);
        let keep_listening = match request {
            Ok(request) => {
                let accepted = sender.send_blocking(request).is_ok();
                write_acknowledgement(server.0.0, accepted)?;
                accepted
            }
            Err(error) => {
                _ = write_acknowledgement(server.0.0, false);
                log::warn!("忽略无效的单实例启动请求：{error:#}");
                true
            }
        };

        unsafe {
            FlushFileBuffers(server.0.0);
            DisconnectNamedPipe(server.0.0);
        }
        Ok(keep_listening)
    }

    fn create_server_pipe(name: &[u16]) -> std::result::Result<ServerPipe, u32> {
        let pipe = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                MAX_REQUEST_BYTES as u32,
                MAX_REQUEST_BYTES as u32,
                CLIENT_CALL_TIMEOUT_MS,
                null(),
            )
        };
        if pipe == INVALID_HANDLE_VALUE {
            return Err(unsafe { GetLastError() });
        }
        Ok(ServerPipe(OwnedHandle(pipe)))
    }

    fn forward_request(name: &[u16], request: &[u8]) -> Result<()> {
        let mut acknowledgement = [0u8; 1];
        let mut bytes_read = 0u32;
        let forwarded = unsafe {
            CallNamedPipeW(
                name.as_ptr(),
                request.as_ptr().cast(),
                request.len() as u32,
                acknowledgement.as_mut_ptr().cast(),
                acknowledgement.len() as u32,
                &mut bytes_read,
                CLIENT_CALL_TIMEOUT_MS,
            )
        };
        if forwarded == 0 {
            let error = unsafe { GetLastError() };
            bail!("无法连接已有 VCLogg2 实例：Windows 错误 {error}");
        }
        if bytes_read != 1 || acknowledgement[0] != 1 {
            bail!("已有 VCLogg2 实例未接受启动请求");
        }
        Ok(())
    }

    fn read_request(pipe: HANDLE) -> Result<OpenRequest> {
        let mut buffer = vec![0u8; MAX_REQUEST_BYTES];
        let mut bytes_read = 0u32;
        let read = unsafe {
            ReadFile(
                pipe,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                &mut bytes_read,
                null_mut(),
            )
        };
        if read == 0 {
            let error = unsafe { GetLastError() };
            bail!("读取单实例启动请求失败：Windows 错误 {error}");
        }
        buffer.truncate(bytes_read as usize);
        decode_request(&buffer)
    }

    fn write_acknowledgement(pipe: HANDLE, accepted: bool) -> Result<()> {
        let acknowledgement = [u8::from(accepted)];
        let mut bytes_written = 0u32;
        let written = unsafe {
            WriteFile(
                pipe,
                acknowledgement.as_ptr(),
                acknowledgement.len() as u32,
                &mut bytes_written,
                null_mut(),
            )
        };
        if written == 0 || bytes_written != acknowledgement.len() as u32 {
            let error = unsafe { GetLastError() };
            bail!("回复单实例启动请求失败：Windows 错误 {error}");
        }
        Ok(())
    }

    fn encode_request(paths: &[PathBuf]) -> Result<Vec<u8>> {
        if paths.len() > MAX_PATHS {
            bail!("一次最多转交 {MAX_PATHS} 个文件路径");
        }
        let wide_paths = paths
            .iter()
            .map(|path| path.as_os_str().encode_wide().collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let mut request = Vec::with_capacity(PIPE_MAGIC.len() + 4);
        request.extend_from_slice(PIPE_MAGIC);
        request.extend_from_slice(&(wide_paths.len() as u32).to_le_bytes());
        for path in wide_paths {
            let path_len = u32::try_from(path.len()).context("文件路径过长")?;
            request.extend_from_slice(&path_len.to_le_bytes());
            for unit in path {
                request.extend_from_slice(&unit.to_le_bytes());
            }
            if request.len() > MAX_REQUEST_BYTES {
                bail!("外部打开请求超过 {} KiB 限制", MAX_REQUEST_BYTES / 1024);
            }
        }
        Ok(request)
    }

    fn decode_request(request: &[u8]) -> Result<OpenRequest> {
        if request.len() < PIPE_MAGIC.len() + 4 || &request[..PIPE_MAGIC.len()] != PIPE_MAGIC {
            bail!("单实例启动请求协议不匹配");
        }
        let mut offset = PIPE_MAGIC.len();
        let path_count = take_u32(request, &mut offset)? as usize;
        if path_count > MAX_PATHS {
            bail!("单实例启动请求包含过多文件路径");
        }

        let mut paths = Vec::with_capacity(path_count);
        for _ in 0..path_count {
            let unit_count = take_u32(request, &mut offset)? as usize;
            let byte_count = unit_count
                .checked_mul(2)
                .context("单实例启动请求中的文件路径长度溢出")?;
            let end = offset
                .checked_add(byte_count)
                .filter(|end| *end <= request.len())
                .context("单实例启动请求中的文件路径不完整")?;
            let units = request[offset..end]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|bytes| u16::from_le_bytes(*bytes))
                .collect::<Vec<_>>();
            offset = end;
            let path = PathBuf::from(OsString::from_wide(&units));
            if path.as_os_str().is_empty() {
                bail!("单实例启动请求包含空文件路径");
            }
            paths.push(path);
        }
        if offset != request.len() {
            bail!("单实例启动请求包含多余数据");
        }
        Ok(OpenRequest { paths })
    }

    fn take_u32(input: &[u8], offset: &mut usize) -> Result<u32> {
        let end = offset
            .checked_add(4)
            .filter(|end| *end <= input.len())
            .context("单实例启动请求不完整")?;
        let bytes = input[*offset..end]
            .try_into()
            .map_err(|_| anyhow!("单实例启动请求不完整"))?;
        *offset = end;
        Ok(u32::from_le_bytes(bytes))
    }

    fn pipe_name() -> Vec<u16> {
        // The user profile is hashed only to keep the named pipe user-specific;
        // this path is never opened or written by the single-instance protocol.
        let identity = dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("vclogg2-default-user"));
        let mut hasher = Sha256::new();
        for unit in identity.as_os_str().encode_wide() {
            hasher.update(unit.to_le_bytes());
        }
        let digest = hasher.finalize();
        let mut user_key = String::with_capacity(16);
        for byte in digest.iter().take(8) {
            _ = write!(user_key, "{byte:02x}");
        }
        format!(r"\\.\pipe\VCLogg2.SingleInstance.v1.{user_key}")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    }
}
