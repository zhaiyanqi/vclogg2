//! Workspace-root validation and structured source inspection that shell cannot replace.
use crate::{ToolCall, ToolResult};
use anyhow::{Context as _, Result, bail};
use serde_json::json;
use std::path::{Path, PathBuf};

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
    if !matches!(
        call.name.as_str(),
        "find_symbols"
            | "source_outline"
            | "locate_log_origin"
            | "find_definition"
            | "find_references"
    ) {
        bail!("Unknown structured source tool");
    }
    crate::source_index::execute(root, index, call).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_paths_outside_root() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        assert!(scoped_path(&root, "../secret").is_err());
        assert!(scoped_path(&root, "/etc/passwd").is_err());
    }

    #[tokio::test]
    async fn lists_captured_workspace_roots() {
        let root = tempfile::tempdir().unwrap();
        let roots = capture_roots(&[root.path().to_path_buf()]);
        let result = execute(
            &roots,
            &ToolCall {
                id: "roots".into(),
                name: "list_source_workspaces".into(),
                arguments: json!({}),
            },
        )
        .await;

        assert!(!result.is_error);
        assert_eq!(result.value["workspaces"][0]["root"], 0);
        assert_eq!(result.value["workspaces"][0]["path"], json!(roots[0]));
    }
}
