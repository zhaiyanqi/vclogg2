use std::path::Path;

use anyhow::Context as _;
use anyhow::{Result, bail};

pub fn move_file_to_trash(path: &Path) -> Result<bool> {
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
            "Only regular files can be moved to the system trash: {}",
            path.display()
        ));
    }

    trash::delete(path).with_context(|| {
        crate::tr_args!(
            "无法把文件移入系统回收站：{}",
            "Couldn’t move the file to the system trash: {}",
            path.display()
        )
    })?;
    Ok(true)
}
