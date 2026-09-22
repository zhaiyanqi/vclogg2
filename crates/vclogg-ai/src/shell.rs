use crate::{Cancellation, ToolCall, ToolResult};
use anyhow::{Context as _, Result, bail};
use serde_json::json;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::io::AsyncReadExt as _;

const OUTPUT_BYTES: usize = 32 * 1024;

pub(crate) enum CommandPolicy {
    Automatic,
    Confirm(String),
    Deny(String),
}

pub(crate) fn classify(command: &str) -> CommandPolicy {
    let normalized = command.trim().to_ascii_lowercase();
    if normalized.is_empty() || command.contains('\0') {
        return CommandPolicy::Deny("Command is empty or invalid".into());
    }
    let destructive = [
        "rm -rf /",
        "rm -fr /",
        "git reset --hard",
        "git clean -fd",
        "mkfs",
        "format-volume",
        "diskpart",
        "del /s",
        "erase /s",
        "rmdir /s",
        "rd /s",
        "remove-item -recurse",
        "diskutil erase",
        "shutdown",
        "reboot",
        ":(){:|:&};:",
    ];
    let formats_volume = normalized.starts_with("format ")
        || normalized.contains("/c format ")
        || normalized.contains("& format ");
    if formats_volume
        || destructive
            .iter()
            .any(|pattern| normalized.contains(pattern))
    {
        return CommandPolicy::Deny("The command is explicitly destructive".into());
    }
    let sensitive_syntax = [
        ">", "`", "$(", "${", "../", "~/", "&&", "||", ";", "&", "\n",
    ];
    if sensitive_syntax
        .iter()
        .any(|pattern| command.contains(pattern))
    {
        return CommandPolicy::Confirm(
            "The command uses shell syntax that can access data or cause side effects".into(),
        );
    }
    let segments = command.split('|').map(str::trim).collect::<Vec<_>>();
    if segments.iter().any(|segment| segment.is_empty()) {
        return CommandPolicy::Confirm("The shell pipeline is ambiguous".into());
    }
    for segment in segments {
        let mut words = segment.split_whitespace();
        let program = words.next().unwrap_or_default();
        let arguments = words.collect::<Vec<_>>();
        let safe = match program {
            "pwd" | "ls" | "head" | "tail" | "wc" | "cat" | "grep" | "tree" | "cd" => true,
            "dir" | "type" | "more" | "findstr" => cfg!(windows),
            "rg" => !arguments
                .iter()
                .any(|arg| matches!(*arg, "--pre" | "--pre-glob")),
            "fd" => !arguments
                .iter()
                .any(|arg| matches!(*arg, "-x" | "-X" | "--exec" | "--exec-batch")),
            "git" => arguments.first().is_some_and(|arg| {
                matches!(
                    *arg,
                    "status" | "diff" | "log" | "show" | "grep" | "ls-files" | "rev-parse"
                )
            }),
            _ => false,
        };
        let risky_argument = arguments.iter().any(|arg| {
            arg.starts_with("--output")
                || matches!(*arg, "--ext-diff" | "--textconv")
                || [
                    ".env",
                    ".ssh",
                    "id_rsa",
                    "id_ed25519",
                    "credentials",
                    "keychain",
                ]
                .iter()
                .any(|name| arg.to_ascii_lowercase().contains(name))
        });
        if !safe {
            return CommandPolicy::Confirm(format!(
                "`{program}` is not in the automatic read-only command set"
            ));
        }
        if risky_argument {
            return CommandPolicy::Confirm(
                "The command may execute another program, write output, or read sensitive data"
                    .into(),
            );
        }
        if arguments.iter().any(|arg| {
            arg.starts_with('/')
                || arg.starts_with('~')
                || arg.starts_with("\\\\")
                || arg.contains(":\\")
                || arg.contains("$HOME")
                || arg.contains("%USERPROFILE%")
                || (cfg!(windows) && arg.contains('%'))
        }) {
            return CommandPolicy::Confirm(
                "The command may access a path outside the selected workspace".into(),
            );
        }
    }
    CommandPolicy::Automatic
}

pub(crate) async fn execute(
    root: &Path,
    call: &ToolCall,
    cancellation: &Cancellation,
) -> ToolResult {
    match execute_inner(root, call, cancellation).await {
        Ok(value) => ToolResult::ok(value),
        Err(error) => ToolResult::error(error.to_string()),
    }
}

async fn execute_inner(
    root: &Path,
    call: &ToolCall,
    cancellation: &Cancellation,
) -> Result<serde_json::Value> {
    if root.canonicalize().ok().as_deref() != Some(root) {
        bail!("Workspace root changed; configure it again");
    }
    let command = call.arguments["command"]
        .as_str()
        .context("Missing shell command")?;
    let timeout = Duration::from_secs(
        call.arguments["timeout_seconds"]
            .as_u64()
            .unwrap_or(30)
            .clamp(1, 120),
    );
    #[cfg(windows)]
    let mut child = {
        let mut process = tokio::process::Command::new("cmd.exe");
        process.args(["/D", "/S", "/C", command]);
        use std::os::windows::process::CommandExt as _;
        // CREATE_NO_WINDOW prevents the GUI application from flashing a console window.
        process.creation_flags(0x0800_0000);
        process
    };
    #[cfg(not(windows))]
    let mut child = {
        // A piped, non-interactive shell allocates no Terminal or terminal-emulator window.
        let mut process = tokio::process::Command::new("/bin/sh");
        process.args(["-c", command]);
        process
    };
    let ripgrep = crate::ripgrep::executable();
    if let Some(directory) = ripgrep.parent().filter(|_| ripgrep.is_absolute()) {
        let paths = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .unwrap_or_default();
        let mut paths = std::iter::once(directory.to_path_buf())
            .chain(paths)
            .collect::<Vec<_>>();
        paths.dedup();
        if let Ok(path) = std::env::join_paths(paths) {
            child.env("PATH", path);
        }
    }
    child
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = child.spawn().context("Could not start the system shell")?;
    let stdout = child.stdout.take().context("Shell stdout unavailable")?;
    let stderr = child.stderr.take().context("Shell stderr unavailable")?;
    let read = async {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut stdout = stdout.take((OUTPUT_BYTES + 1) as u64);
        let mut stderr = stderr.take((OUTPUT_BYTES + 1) as u64);
        let (out_result, err_result) =
            tokio::join!(stdout.read_to_end(&mut out), stderr.read_to_end(&mut err),);
        out_result?;
        err_result?;
        Ok::<_, std::io::Error>((out, err))
    };
    let (mut stdout, mut stderr) = tokio::select! {
        _ = cancellation.cancelled() => {
            _ = child.kill().await;
            _ = child.wait().await;
            bail!("Shell command stopped");
        }
        result = tokio::time::timeout(timeout, read) => match result {
            Ok(result) => result?,
            Err(_) => {
                _ = child.kill().await;
                _ = child.wait().await;
                bail!("Shell command timed out");
            }
        }
    };
    let stdout_truncated = stdout.len() > OUTPUT_BYTES;
    let stderr_truncated = stderr.len() > OUTPUT_BYTES;
    stdout.truncate(OUTPUT_BYTES);
    stderr.truncate(OUTPUT_BYTES);
    if stdout_truncated || stderr_truncated {
        _ = child.kill().await;
    }
    let status = child.wait().await?;
    Ok(json!({
        "command": command,
        "cwd": root,
        "exit_code": status.code(),
        "success": status.success(),
        "stdout": String::from_utf8_lossy(&stdout),
        "stderr": String::from_utf8_lossy(&stderr),
        "stdout_truncated": stdout_truncated,
        "stderr_truncated": stderr_truncated,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_allows_bounded_reads_and_requires_confirmation_for_side_effects() {
        assert!(matches!(
            classify("rg TODO src | head"),
            CommandPolicy::Automatic
        ));
        assert!(matches!(
            classify("git diff -- src/lib.rs"),
            CommandPolicy::Automatic
        ));
        assert!(matches!(classify("cargo test"), CommandPolicy::Confirm(_)));
        assert!(matches!(
            classify("rg --pre helper TODO"),
            CommandPolicy::Confirm(_)
        ));
        assert!(matches!(classify("fd -x rm {}"), CommandPolicy::Confirm(_)));
        assert!(matches!(
            classify("git diff --output=patch"),
            CommandPolicy::Confirm(_)
        ));
        assert!(matches!(
            classify("cat ~/.ssh/id_rsa"),
            CommandPolicy::Confirm(_)
        ));
        assert!(matches!(classify("rm -rf /"), CommandPolicy::Deny(_)));
        assert!(matches!(classify("format C:"), CommandPolicy::Deny(_)));
        assert!(matches!(
            classify("rmdir /s /q cache"),
            CommandPolicy::Deny(_)
        ));
    }

    #[tokio::test]
    async fn executes_in_the_selected_workspace_with_bounded_output() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let call = ToolCall {
            id: "shell".into(),
            name: "shell".into(),
            arguments: json!({"root":0,"command":"pwd","timeout_seconds":5}),
        };
        let result = execute(&root, &call, &Cancellation::default()).await;
        assert!(!result.is_error, "{:?}", result.value);
        assert_eq!(result.value["success"], true);
        assert_eq!(result.value["cwd"], json!(root));
    }
}
