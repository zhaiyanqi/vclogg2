use std::path::Path;

#[cfg(windows)]
use anyhow::Context as _;
use anyhow::{Result, bail};

#[cfg(windows)]
pub fn move_file_to_trash(path: &Path) -> Result<bool> {
    use std::{os::windows::ffi::OsStrExt as _, ptr};

    use windows_sys::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW,
        SHFileOperationW,
    };

    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| {
                crate::tr_args!(
                    "无法读取待删除文件：{}",
                    "Couldn’t read the file to delete: {}",
                    path.display()
                )
            });
        }
    };
    if !metadata.is_file() {
        bail!(crate::tr_args!(
            "只能把普通文件移入回收站：{}",
            "Only regular files can be moved to the Recycle Bin: {}",
            path.display()
        ));
    }

    let mut source = path.as_os_str().encode_wide().collect::<Vec<_>>();
    source.push(0);
    source.push(0);
    let mut operation = SHFILEOPSTRUCTW {
        hwnd: ptr::null_mut(),
        wFunc: FO_DELETE,
        pFrom: source.as_ptr(),
        pTo: ptr::null(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT) as u16,
        fAnyOperationsAborted: 0,
        hNameMappings: ptr::null_mut(),
        lpszProgressTitle: ptr::null(),
    };
    // SAFETY: `source` is a live, double-NUL-terminated UTF-16 path list for the
    // duration of the synchronous shell call, and all optional pointers are null.
    let result = unsafe { SHFileOperationW(&mut operation) };
    if result != 0 {
        bail!(crate::tr_args!(
            "系统回收站操作失败（代码 {result}）：{}",
            "The system Recycle Bin operation failed (code {result}): {}",
            path.display()
        ));
    }
    if operation.fAnyOperationsAborted != 0 {
        bail!(crate::tr_args!(
            "系统取消了回收站操作：{}",
            "The system canceled the Recycle Bin operation: {}",
            path.display()
        ));
    }
    Ok(true)
}

#[cfg(not(windows))]
pub fn move_file_to_trash(path: &Path) -> Result<bool> {
    bail!(crate::tr_args!(
        "当前平台尚不支持移入回收站：{}",
        "Moving files to the trash isn’t supported on this platform: {}",
        path.display()
    ))
}
