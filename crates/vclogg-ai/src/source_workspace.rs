//! Bounded, read-only source inspection for explicitly configured roots.
use crate::{ToolCall, ToolResult};
use anyhow::{Context, Result, bail};
use serde_json::json;
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;

pub(crate) fn capture_roots(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter_map(|path| path.canonicalize().ok().filter(|path| path.is_dir()))
        .collect()
}

fn scoped_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path.components().any(|part| {
            !matches!(
                part,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        bail!("Use a relative path within the selected workspace directory");
    }
    let canonical = root
        .join(path)
        .canonicalize()
        .context("Source path unavailable")?;
    if !canonical.starts_with(root) {
        bail!("Source path leaves the selected workspace directory");
    }
    Ok(canonical)
}

pub(crate) async fn execute(roots: &[PathBuf], call: &ToolCall) -> ToolResult {
    match execute_inner(roots, call).await {
        Ok(value) => ToolResult::ok(value),
        Err(error) => ToolResult::error(error.to_string()),
    }
}

async fn execute_inner(roots: &[PathBuf], call: &ToolCall) -> Result<serde_json::Value> {
    let index = call.arguments["root"]
        .as_u64()
        .context("Missing workspace root")? as usize;
    let root = roots
        .get(index)
        .context("Workspace root unavailable; check configuration")?;
    // Recheck the directory at use time; a replaced symlink cannot silently widen scope.
    if root.canonicalize().ok().as_deref() != Some(root.as_path()) {
        bail!("Workspace root changed; configure it again");
    }
    let relative = call.arguments["path"].as_str().unwrap_or("");
    if call.name == "read_source" {
        if relative.is_empty() {
            bail!("Source path is required");
        }
        let path = scoped_path(root, relative)?;
        if !path.is_file() {
            bail!("Source path is not a file");
        }
        let metadata = tokio::fs::metadata(&path).await?;
        if metadata.len() > 4 * 1024 * 1024 {
            bail!("Source file is too large");
        }
        let content = tokio::fs::read_to_string(&path)
            .await
            .context("Source file is not UTF-8")?;
        let start = call.arguments["start_line"].as_u64().unwrap_or(1).max(1) as usize;
        let lines = content
            .lines()
            .skip(start - 1)
            .take(100)
            .collect::<Vec<_>>();
        let mut text = String::new();
        for (offset, line) in lines.iter().enumerate() {
            if text.len() + line.len() > 16 * 1024 {
                break;
            }
            text.push_str(&format!("{}: {}\n", start + offset, line));
        }
        return Ok(
            json!({"root":index,"path":relative,"start_line":start,"text":text,"truncated":lines.len()==100 || text.len()>=16*1024}),
        );
    }
    let query = call.arguments["query"].as_str().context("Missing query")?;
    if query.is_empty() {
        bail!("Search query is empty");
    }
    let target = if relative.is_empty() {
        root.clone()
    } else {
        scoped_path(root, relative)?
    };
    let mut command = tokio::process::Command::new("rg");
    command.args([
        "--line-number",
        "--no-heading",
        "--with-filename",
        "--color",
        "never",
        "--max-columns",
        "300",
        "--max-columns-preview",
        "--max-count",
        "20",
    ]);
    if !call.arguments["regex"].as_bool().unwrap_or(false) {
        command.arg("--fixed-strings");
    }
    command.current_dir(root);
    command
        .arg("--")
        .arg(query)
        .arg(if relative.is_empty() {
            Path::new(".")
        } else {
            target.strip_prefix(root)?
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().context("ripgrep (rg) is unavailable")?;
    let mut stdout = child
        .stdout
        .take()
        .context("ripgrep output unavailable")?
        .take(32 * 1024 + 1);
    let mut output = Vec::new();
    let read = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stdout.read_to_end(&mut output),
    )
    .await;
    let truncated = output.len() > 32 * 1024;
    if truncated || !matches!(&read, Ok(Ok(_))) {
        _ = child.kill().await;
    }
    let status = child.wait().await?;
    read.context("Source search timed out")??;
    if status.code().is_some_and(|code| code > 1) {
        bail!("ripgrep search failed; check the query");
    }
    output.truncate(32 * 1024);
    Ok(
        json!({"root":index,"directory":root,"matches":String::from_utf8_lossy(&output),"truncated":truncated}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn rejects_paths_outside_root() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        assert!(scoped_path(&root, "../secret").is_err());
        assert!(scoped_path(&root, "/etc/passwd").is_err());
    }

    #[tokio::test]
    async fn finds_and_reads_source_with_relative_paths() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("sample.rs"), "fn investigate_log() {}\n").unwrap();
        let roots = capture_roots(&[root.path().to_path_buf()]);
        let search = execute(
            &roots,
            &ToolCall {
                id: "search".into(),
                name: "rg_search".into(),
                arguments: json!({"root":0,"query":"investigate_log"}),
            },
        )
        .await;
        assert!(!search.is_error, "{:?}", search.value);
        assert!(
            search.value["matches"]
                .as_str()
                .unwrap()
                .contains("sample.rs:1:")
        );
        let read = execute(
            &roots,
            &ToolCall {
                id: "read".into(),
                name: "read_source".into(),
                arguments: json!({"root":0,"path":"sample.rs","start_line":1}),
            },
        )
        .await;
        assert!(!read.is_error, "{:?}", read.value);
        assert!(
            read.value["text"]
                .as_str()
                .unwrap()
                .contains("investigate_log")
        );
    }
}
