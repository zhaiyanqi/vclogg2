//! On-demand, local language server queries. Results never escape the selected root.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{ChildStdin, ChildStdout, Command},
};

async fn write_message(stdin: &mut ChildStdin, value: &Value) -> Result<()> {
    let body = serde_json::to_vec(value)?;
    stdin
        .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .await?;
    stdin.write_all(&body).await?;
    stdin.flush().await?;
    Ok(())
}

async fn read_message(stdout: &mut ChildStdout) -> Result<Value> {
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        let byte = stdout.read_u8().await?;
        header.push(byte);
        if header.len() > 8192 {
            bail!("Language server header too large");
        }
    }
    let header = std::str::from_utf8(&header)?;
    let length = header
        .split("\r\n")
        .find_map(|line| {
            line.strip_prefix("Content-Length: ")
                .and_then(|value| value.parse::<usize>().ok())
        })
        .context("Language server response has no length")?;
    if length > 4 * 1024 * 1024 {
        bail!("Language server response too large");
    }
    let mut body = vec![0; length];
    stdout.read_exact(&mut body).await?;
    Ok(serde_json::from_slice(&body)?)
}

async fn response(stdin: &mut ChildStdin, stdout: &mut ChildStdout, id: i32) -> Result<Value> {
    for _ in 0..100 {
        let message = read_message(stdout).await?;
        if message["id"] == id {
            return Ok(message);
        }
        if !message["id"].is_null() && message["method"].is_string() {
            write_message(
                stdin,
                &json!({"jsonrpc":"2.0","id":message["id"],"result":null}),
            )
            .await?;
        }
    }
    bail!("Language server did not answer")
}

fn language(path: &Path) -> Option<(&'static str, &'static [&'static str], &'static str)> {
    match path.extension()?.to_str()? {
        "rs" => Some(("rust-analyzer", &[], "rust")),
        "java" => Some(("jdtls", &[], "java")),
        "c" | "h" => Some(("clangd", &[], "c")),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Some(("clangd", &[], "cpp")),
        "py" | "pyi" => Some(("pyright-langserver", &["--stdio"], "python")),
        "js" | "mjs" | "cjs" => Some(("typescript-language-server", &["--stdio"], "javascript")),
        "jsx" => Some((
            "typescript-language-server",
            &["--stdio"],
            "javascriptreact",
        )),
        "ts" | "mts" | "cts" => Some(("typescript-language-server", &["--stdio"], "typescript")),
        "tsx" => Some((
            "typescript-language-server",
            &["--stdio"],
            "typescriptreact",
        )),
        "go" => Some(("gopls", &["serve"], "go")),
        "cs" => Some(("csharp-ls", &[], "csharp")),
        _ => None,
    }
}

fn location(root: &Path, item: &Value) -> Option<Value> {
    let uri = item["targetUri"]
        .as_str()
        .or_else(|| item["uri"].as_str())?;
    let range = if item.get("targetSelectionRange").is_some() {
        &item["targetSelectionRange"]
    } else {
        &item["range"]
    };
    let path = url::Url::parse(uri)
        .ok()?
        .to_file_path()
        .ok()?
        .canonicalize()
        .ok()?;
    if !path.starts_with(root) {
        return None;
    }
    let relative = path.strip_prefix(root).ok()?;
    Some(
        json!({"path":relative,"line":range["start"]["line"].as_u64()?+1,"column":range["start"]["character"].as_u64()?+1,"precision":"semantic"}),
    )
}

pub(crate) async fn query(
    root: &Path,
    path: &Path,
    source: &str,
    line: usize,
    column: usize,
    references: bool,
) -> Result<Vec<Value>> {
    let (server, arguments, language) = language(path).context("Unsupported language server")?;
    let root = PathBuf::from(root);
    let path = PathBuf::from(path);
    let source = source.to_owned();
    tokio::time::timeout(std::time::Duration::from_secs(20),async move {
        let mut command = Command::new(server);
        command.args(arguments);
        command.current_dir(&root).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true);
        let mut child = command.spawn().with_context(||format!("{server} unavailable"))?;
        let mut stdin = child.stdin.take().context("Language server stdin unavailable")?;
        let mut stdout = child.stdout.take().context("Language server stdout unavailable")?;
        let root_uri = url::Url::from_directory_path(&root).map_err(|_|anyhow::anyhow!("Invalid workspace root"))?.to_string();
        let file_uri = url::Url::from_file_path(&path).map_err(|_|anyhow::anyhow!("Invalid source path"))?.to_string();
        write_message(&mut stdin,&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":root_uri,"workspaceFolders":[{"uri":root_uri,"name":"workspace"}],"capabilities":{},"initializationOptions":{"cargo":{"buildScripts":{"enable":false}},"procMacro":{"enable":false},"checkOnSave":{"enable":false}}}})).await?;
        let initialized = response(&mut stdin,&mut stdout,1).await?;
        if initialized.get("error").is_some() { bail!("Language server initialization failed"); }
        write_message(&mut stdin,&json!({"jsonrpc":"2.0","method":"initialized","params":{}})).await?;
        write_message(&mut stdin,&json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":file_uri,"languageId":language,"version":1,"text":source}}})).await?;
        let method = if references { "textDocument/references" } else { "textDocument/definition" };
        let params = if references {
            json!({"textDocument":{"uri":file_uri},"position":{"line":line.saturating_sub(1),"character":column.saturating_sub(1)},"context":{"includeDeclaration":true}})
        } else {
            json!({"textDocument":{"uri":file_uri},"position":{"line":line.saturating_sub(1),"character":column.saturating_sub(1)}})
        };
        write_message(&mut stdin,&json!({"jsonrpc":"2.0","id":2,"method":method,"params":params})).await?;
        let reply = response(&mut stdin,&mut stdout,2).await?;
        if reply.get("error").is_some() { bail!("Language server query failed"); }
        let result = &reply["result"];
        let items = if let Some(array) = result.as_array() { array.clone() } else if result.is_object() { vec![result.clone()] } else { vec![] };
        let locations = items.iter().filter_map(|item|location(&root,item)).take(100).collect();
        _=child.kill().await;
        Ok(locations)
    }).await.context("Language server timed out")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_languages_to_semantic_servers() {
        for (path, server, language_id) in [
            ("main.rs", "rust-analyzer", "rust"),
            ("Main.java", "jdtls", "java"),
            ("main.cpp", "clangd", "cpp"),
            ("main.py", "pyright-langserver", "python"),
            ("main.jsx", "typescript-language-server", "javascriptreact"),
            ("main.tsx", "typescript-language-server", "typescriptreact"),
            ("main.go", "gopls", "go"),
            ("Main.cs", "csharp-ls", "csharp"),
        ] {
            let (actual_server, _, actual_language) = language(Path::new(path)).unwrap();
            assert_eq!((actual_server, actual_language), (server, language_id));
        }
    }

    #[tokio::test]
    async fn clangd_definition_stays_inside_root() {
        if std::process::Command::new("clangd")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("main.cpp");
        let source = "int ping() { return 1; }\nint main() { return ping(); }\n";
        std::fs::write(&path, source).unwrap();
        let locations = query(&root, &path, source, 2, 21, false).await.unwrap();
        assert!(
            locations
                .iter()
                .any(|item| item["path"] == "main.cpp" && item["line"] == 1),
            "{locations:?}"
        );
    }
}
