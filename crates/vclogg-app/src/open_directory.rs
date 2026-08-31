use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context as _, bail};

struct OpenDirectoryInvocation {
    executable: String,
    arguments: Vec<String>,
    directory: PathBuf,
}

pub fn launch_custom(command_line: &str, file_path: &Path) -> anyhow::Result<bool> {
    let Some(invocation) = build_invocation(command_line, file_path)? else {
        return Ok(false);
    };
    let mut command = Command::new(&invocation.executable);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x0800_0000);
    }
    command
        .args(&invocation.arguments)
        .current_dir(&invocation.directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| {
            crate::tr_args!(
                "无法启动打开目录命令：{}",
                "Couldn’t start the open-folder command: {}",
                invocation.executable
            )
        })?;
    Ok(true)
}

fn build_invocation(
    command_line: &str,
    file_path: &Path,
) -> anyhow::Result<Option<OpenDirectoryInvocation>> {
    if command_line.trim().is_empty() {
        return Ok(None);
    }
    let file_path = absolute_path(file_path)?;
    let directory = file_path
        .parent()
        .ok_or_else(|| {
            anyhow::anyhow!(crate::tr!(
                "无法确定文件所在目录",
                "Couldn’t determine the containing folder"
            ))
        })?
        .to_path_buf();
    let mut tokens = split_command_line(command_line)?;
    if tokens.is_empty() {
        bail!(crate::tr!(
            "打开目录命令不能为空",
            "The open-folder command can’t be empty"
        ));
    }
    let executable = tokens.remove(0);
    let directory_text = directory.to_string_lossy();
    let file_text = file_path.to_string_lossy();
    let mut used_placeholder = false;
    let mut substitute = |value: String| {
        if value.contains("{directory}") || value.contains("{path}") {
            used_placeholder = true;
        }
        value
            .replace("{directory}", &directory_text)
            .replace("{path}", &file_text)
    };
    let executable = substitute(executable);
    let mut arguments = tokens.into_iter().map(&mut substitute).collect::<Vec<_>>();
    if !used_placeholder {
        arguments.push(directory_text.into_owned());
    }
    Ok(Some(OpenDirectoryInvocation {
        executable,
        arguments,
        directory,
    }))
}

fn split_command_line(command_line: &str) -> anyhow::Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut token_started = false;
    for character in command_line.trim().chars() {
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            } else {
                token.push(character);
            }
            token_started = true;
            continue;
        }
        match character {
            '\'' | '"' => {
                quote = Some(character);
                token_started = true;
            }
            character if character.is_whitespace() => {
                if token_started {
                    tokens.push(std::mem::take(&mut token));
                    token_started = false;
                }
            }
            _ => {
                token.push(character);
                token_started = true;
            }
        }
    }
    if quote.is_some() {
        bail!(crate::tr!(
            "打开目录命令包含未闭合的引号",
            "The open-folder command contains an unclosed quote"
        ));
    }
    if token_started {
        tokens.push(token);
    }
    Ok(tokens)
}

fn absolute_path(path: &Path) -> anyhow::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .context(crate::tr!(
                "无法读取当前目录",
                "Couldn’t read the current directory"
            ))?
            .join(path))
    }
}
