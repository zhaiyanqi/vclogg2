//! Bounded, read-only source inspection for explicitly configured roots.
use crate::{ToolCall, ToolResult};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;

const RG_OUTPUT_BYTES: usize = 256 * 1024;
const SOURCE_READ_BYTES: usize = 32 * 1024;
const RG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub(crate) fn capture_roots(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter_map(|path| path.canonicalize().ok().filter(|path| path.is_dir()))
        .collect()
}

pub(crate) fn scoped_path(root: &Path, relative: &str) -> Result<PathBuf> {
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
    if call.name == "list_source_workspaces" {
        return ToolResult::ok(json!({
            "workspaces": roots
                .iter()
                .enumerate()
                .map(|(id, path)| json!({"root": id, "path": path}))
                .collect::<Vec<_>>()
        }));
    }
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
    if matches!(
        call.name.as_str(),
        "find_source_files"
            | "find_symbols"
            | "source_outline"
            | "locate_log_origin"
            | "find_definition"
            | "find_references"
    ) {
        return crate::source_index::execute(root, index, call).await;
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
        let limit = call.arguments["limit"].as_u64().unwrap_or(100) as usize;
        let total_lines = content.lines().count();
        let mut text = String::new();
        let mut returned = 0;
        let mut line_truncated = false;
        for (offset, line) in content.lines().skip(start - 1).take(limit).enumerate() {
            let rendered = format!("{}: {}\n", start + offset, line);
            if text.len() + rendered.len() > SOURCE_READ_BYTES {
                let prefix = format!("{}: ", start + offset);
                let remaining = SOURCE_READ_BYTES.saturating_sub(text.len() + prefix.len() + 4);
                if remaining > 0 {
                    let end = line.floor_char_boundary(remaining.min(line.len()));
                    text.push_str(&prefix);
                    text.push_str(&line[..end]);
                    text.push_str("…\n");
                    returned += 1;
                    line_truncated = true;
                }
                break;
            }
            text.push_str(&rendered);
            returned += 1;
        }
        let next_line = start.saturating_add(returned);
        return Ok(json!({
            "root": index,
            "path": relative,
            "start_line": start,
            "returned_lines": returned,
            "total_lines": total_lines,
            "next_line": (next_line <= total_lines).then_some(next_line),
            "text": text,
            "truncated": next_line <= total_lines || line_truncated,
            "line_truncated": line_truncated
        }));
    }
    if call.name == "rg_list_files" {
        return rg_list_files(root, index, call).await;
    }
    let query = call.arguments["query"].as_str().context("Missing query")?;
    if query.is_empty() {
        bail!("Search query is empty");
    }
    let target = target_argument(root, relative)?;
    if call.name == "rg_count" {
        return rg_count(root, index, call, query, &target).await;
    }
    if call.name != "rg_search" {
        bail!("Unknown source tool");
    }
    let mut command = crate::ripgrep::command();
    command.args([
        "--json",
        "--line-number",
        "--color",
        "never",
        "--max-columns",
        "500",
        "--max-columns-preview",
    ]);
    add_search_options(&mut command, call);
    let before = call.arguments["context_before"].as_u64().unwrap_or(0);
    let after = call.arguments["context_after"].as_u64().unwrap_or(0);
    if before > 0 {
        command.arg("--before-context").arg(before.to_string());
    }
    if after > 0 {
        command.arg("--after-context").arg(after.to_string());
    }
    let max_results = call.arguments["max_results"].as_u64().unwrap_or(50) as usize;
    command.arg("--").arg(query).arg(&target);
    let (output, output_truncated) = run_rg(root, command, RG_OUTPUT_BYTES).await?;
    let mut rows = Vec::new();
    let mut stopped_early = false;
    for line in String::from_utf8_lossy(&output).lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let kind = event["type"].as_str().unwrap_or_default();
        if !matches!(kind, "match" | "context") {
            continue;
        }
        let data = &event["data"];
        let Some(path) = data["path"]["text"].as_str() else {
            continue;
        };
        if scoped_path(root, path).is_err() {
            continue;
        }
        let mut text = data["lines"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        while text.ends_with(['\n', '\r']) {
            text.pop();
        }
        let submatches = data["submatches"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        json!({
                            "start": item["start"],
                            "end": item["end"],
                            "text": item["match"]["text"]
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let column = submatches
            .first()
            .and_then(|item| item["start"].as_u64())
            .map(|column| column + 1);
        rows.push(json!({
            "kind": kind,
            "path": path.trim_start_matches("./"),
            "line": data["line_number"],
            "column": column,
            "text": text,
            "submatches": submatches
        }));
        if rows.len() >= max_results || serde_json::to_vec(&rows)?.len() > 56 * 1024 {
            stopped_early = true;
            break;
        }
    }
    let truncated = output_truncated || stopped_early;
    Ok(json!({"root":index,"directory":root,"rows":rows,"truncated":truncated}))
}

fn target_argument(root: &Path, relative: &str) -> Result<PathBuf> {
    if relative.is_empty() {
        return Ok(PathBuf::from("."));
    }
    let target = scoped_path(root, relative)?;
    Ok(target.strip_prefix(root)?.to_path_buf())
}

fn add_search_options(command: &mut tokio::process::Command, call: &ToolCall) {
    if !call.arguments["regex"].as_bool().unwrap_or(false) {
        command.arg("--fixed-strings");
    }
    if call.arguments["ignore_case"].as_bool().unwrap_or(false) {
        command.arg("--ignore-case");
    }
    if call.arguments["word"].as_bool().unwrap_or(false) {
        command.arg("--word-regexp");
    }
    if call.arguments["include_hidden"].as_bool().unwrap_or(false) {
        command.arg("--hidden");
    }
    if let Some(globs) = call.arguments["globs"].as_array() {
        for glob in globs.iter().filter_map(Value::as_str) {
            command.arg("--glob").arg(glob);
        }
    }
}

async fn rg_list_files(root: &Path, index: usize, call: &ToolCall) -> Result<Value> {
    let relative = call.arguments["path"].as_str().unwrap_or("");
    let target = target_argument(root, relative)?;
    let offset = call.arguments["offset"].as_u64().unwrap_or(0) as usize;
    let limit = call.arguments["limit"].as_u64().unwrap_or(100) as usize;
    let mut command = crate::ripgrep::command();
    command.args(["--files", "--color", "never"]);
    if call.arguments["include_hidden"].as_bool().unwrap_or(false) {
        command.arg("--hidden");
    }
    if let Some(globs) = call.arguments["globs"].as_array() {
        for glob in globs.iter().filter_map(Value::as_str) {
            command.arg("--glob").arg(glob);
        }
    }
    command.arg("--").arg(&target);
    let (output, output_truncated) = run_rg(root, command, RG_OUTPUT_BYTES).await?;
    let decoded = String::from_utf8_lossy(&output);
    let mut all = decoded
        .lines()
        .map(|path| path.trim_start_matches("./"))
        .filter(|path| scoped_path(root, path).is_ok())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    all.sort();
    all.dedup();
    let paths = all
        .iter()
        .skip(offset)
        .take(limit)
        .cloned()
        .collect::<Vec<_>>();
    let next = offset.saturating_add(paths.len());
    Ok(json!({
        "root": index,
        "path": relative,
        "paths": paths,
        "next_offset": (next < all.len()).then_some(next),
        "returned": paths.len(),
        "discovered": all.len(),
        "truncated": output_truncated
    }))
}

async fn rg_count(
    root: &Path,
    index: usize,
    call: &ToolCall,
    query: &str,
    target: &Path,
) -> Result<Value> {
    let max_files = call.arguments["max_files"].as_u64().unwrap_or(100) as usize;
    let mut command = crate::ripgrep::command();
    command.args(["--count-matches", "--with-filename", "--color", "never"]);
    add_search_options(&mut command, call);
    command.arg("--").arg(query).arg(target);
    let (output, output_truncated) = run_rg(root, command, RG_OUTPUT_BYTES).await?;
    let mut counts = String::from_utf8_lossy(&output)
        .lines()
        .filter_map(|line| {
            let (path, count) = line.rsplit_once(':')?;
            let count = count.parse::<u64>().ok()?;
            let path = path.trim_start_matches("./");
            scoped_path(root, path).ok()?;
            Some((path.to_owned(), count))
        })
        .collect::<Vec<_>>();
    counts.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let discovered = counts.len();
    counts.truncate(max_files);
    let counts = counts
        .into_iter()
        .map(|(path, count)| json!({"path":path,"count":count}))
        .collect::<Vec<_>>();
    Ok(json!({
        "root": index,
        "counts": counts,
        "files_with_matches": discovered,
        "truncated": output_truncated || discovered > max_files
    }))
}

async fn run_rg(
    root: &Path,
    mut command: tokio::process::Command,
    limit: usize,
) -> Result<(Vec<u8>, bool)> {
    command
        .current_dir(root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().context("ripgrep (rg) is unavailable")?;
    let mut stdout = child
        .stdout
        .take()
        .context("ripgrep output unavailable")?
        .take(limit as u64 + 1);
    let mut output = Vec::new();
    match tokio::time::timeout(RG_TIMEOUT, stdout.read_to_end(&mut output)).await {
        Ok(result) => {
            result?;
        }
        Err(_) => {
            _ = child.kill().await;
            _ = child.wait().await;
            bail!("ripgrep operation timed out");
        }
    }
    let truncated = output.len() > limit;
    if truncated {
        _ = child.kill().await;
    }
    let status = child.wait().await?;
    if !truncated && status.code().is_some_and(|code| code > 1) {
        bail!("ripgrep operation failed; check the query, path and globs");
    }
    output.truncate(limit);
    Ok((output, truncated))
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
    async fn lists_searches_counts_and_reads_source_with_relative_paths() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("sample.rs"),
            "fn investigate_log() {}\nfn INVESTIGATE_LOG() {}\n",
        )
        .unwrap();
        std::fs::write(root.path().join("notes.txt"), "investigate_log\n").unwrap();
        let roots = capture_roots(&[root.path().to_path_buf()]);
        let workspaces = execute(
            &roots,
            &ToolCall {
                id: "roots".into(),
                name: "list_source_workspaces".into(),
                arguments: json!({}),
            },
        )
        .await;
        assert_eq!(workspaces.value["workspaces"][0]["root"], 0);
        let files = execute(
            &roots,
            &ToolCall {
                id: "files".into(),
                name: "rg_list_files".into(),
                arguments: json!({"root":0,"globs":["*.rs"]}),
            },
        )
        .await;
        assert_eq!(files.value["paths"], json!(["sample.rs"]));
        let search = execute(
            &roots,
            &ToolCall {
                id: "search".into(),
                name: "rg_search".into(),
                arguments: json!({"root":0,"query":"investigate_log","ignore_case":true}),
            },
        )
        .await;
        assert!(!search.is_error, "{:?}", search.value);
        assert_eq!(search.value["rows"][0]["path"], "sample.rs");
        assert_eq!(search.value["rows"][0]["line"], 1);
        assert_eq!(search.value["rows"][0]["column"], 4);
        let counts = execute(
            &roots,
            &ToolCall {
                id: "count".into(),
                name: "rg_count".into(),
                arguments: json!({"root":0,"query":"investigate_log","ignore_case":true}),
            },
        )
        .await;
        assert_eq!(counts.value["counts"][0]["path"], "sample.rs");
        assert_eq!(counts.value["counts"][0]["count"], 2);
        let read = execute(
            &roots,
            &ToolCall {
                id: "read".into(),
                name: "read_source".into(),
                arguments: json!({"root":0,"path":"sample.rs","start_line":1,"limit":1}),
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
        assert_eq!(read.value["returned_lines"], 1);
        assert_eq!(read.value["next_line"], 2);
    }
}
