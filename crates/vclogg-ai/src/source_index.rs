//! Bounded source discovery and syntax candidates for configured workspace roots.
use crate::{ToolCall, source_workspace::scoped_path};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;
use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

const MAX_FILES: usize = 50_000;
const MAX_PATH_BYTES: u64 = 8 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const TYPESCRIPT_TAGS: &str = r#"
(function_declaration name: (identifier) @name) @definition.function
(function_signature name: (identifier) @name) @definition.function
(class_declaration name: (_) @name) @definition.class
(interface_declaration name: (type_identifier) @name) @definition.interface
(method_definition name: (property_identifier) @name) @definition.method
(method_signature name: (property_identifier) @name) @definition.method
"#;

fn grammar(path: &Path) -> Option<(Language, &'static str)> {
    match path.extension()?.to_str()? {
        "java" => Some((
            tree_sitter_java::LANGUAGE.into(),
            tree_sitter_java::TAGS_QUERY,
        )),
        "c" | "h" => Some((tree_sitter_c::LANGUAGE.into(), tree_sitter_c::TAGS_QUERY)),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Some((
            tree_sitter_cpp::LANGUAGE.into(),
            tree_sitter_cpp::TAGS_QUERY,
        )),
        "rs" => Some((
            tree_sitter_rust::LANGUAGE.into(),
            tree_sitter_rust::TAGS_QUERY,
        )),
        "py" | "pyi" => Some((
            tree_sitter_python::LANGUAGE.into(),
            tree_sitter_python::TAGS_QUERY,
        )),
        "js" | "jsx" | "mjs" | "cjs" => Some((
            tree_sitter_javascript::LANGUAGE.into(),
            tree_sitter_javascript::TAGS_QUERY,
        )),
        "ts" | "mts" | "cts" => Some((
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            TYPESCRIPT_TAGS,
        )),
        "tsx" => Some((tree_sitter_typescript::LANGUAGE_TSX.into(), TYPESCRIPT_TAGS)),
        "go" => Some((tree_sitter_go::LANGUAGE.into(), tree_sitter_go::TAGS_QUERY)),
        "cs" => Some((
            tree_sitter_c_sharp::LANGUAGE.into(),
            tree_sitter_c_sharp::TAGS_QUERY,
        )),
        _ => None,
    }
}

async fn files(root: &Path) -> Result<(Vec<PathBuf>, bool)> {
    let mut command = crate::ripgrep::command();
    command.args([
        "--files",
        "--hidden",
        "-g",
        "!.git",
        "-g",
        "!target",
        "-g",
        "!build",
        "-g",
        "!node_modules",
    ]);
    command
        .current_dir(root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().context("ripgrep (rg) is unavailable")?;
    let stdout = child.stdout.take().context("ripgrep output unavailable")?;
    let mut output = Vec::new();
    let read = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stdout.take(MAX_PATH_BYTES + 1).read_to_end(&mut output),
    )
    .await;
    if output.len() as u64 > MAX_PATH_BYTES || !matches!(&read, Ok(Ok(_))) {
        _ = child.kill().await;
    }
    let status = child.wait().await?;
    read.context("Source file discovery timed out")??;
    if status.code().is_some_and(|code| code > 1) {
        bail!("Source file discovery failed");
    }
    let output_truncated = output.len() as u64 > MAX_PATH_BYTES;
    output.truncate(MAX_PATH_BYTES as usize);
    let paths = String::from_utf8_lossy(&output)
        .lines()
        .take(MAX_FILES + 1)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let truncated = output_truncated || paths.len() > MAX_FILES;
    Ok((paths.into_iter().take(MAX_FILES).collect(), truncated))
}

async fn symbol_files(root: &Path, query: &str) -> Result<(Vec<PathBuf>, bool)> {
    let mut command = crate::ripgrep::command();
    command.args([
        "--files-with-matches",
        "--fixed-strings",
        "--ignore-case",
        "--hidden",
        "-g",
        "!.git",
        "-g",
        "!target",
        "-g",
        "!build",
        "-g",
        "!node_modules",
        "-g",
        "*.{java,c,h,cc,cpp,cxx,hpp,hh,hxx,rs,py,pyi,js,jsx,mjs,cjs,ts,tsx,mts,cts,go,cs}",
        "--",
    ]);
    command
        .arg(query)
        .arg(".")
        .current_dir(root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().context("ripgrep (rg) is unavailable")?;
    let stdout = child.stdout.take().context("ripgrep output unavailable")?;
    let mut output = Vec::new();
    let read = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stdout.take(1024 * 1024 + 1).read_to_end(&mut output),
    )
    .await;
    if output.len() > 1024 * 1024 || !matches!(&read, Ok(Ok(_))) {
        _ = child.kill().await;
    }
    let status = child.wait().await?;
    read.context("Symbol file search timed out")??;
    if status.code().is_some_and(|code| code > 1) {
        bail!("Symbol file search failed");
    }
    let paths = String::from_utf8_lossy(&output)
        .lines()
        .take(501)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let truncated = output.len() > 1024 * 1024 || paths.len() > 500;
    Ok((paths.into_iter().take(500).collect(), truncated))
}

fn definitions(root: &Path, relative: &Path) -> Result<Vec<Value>> {
    let Some((language, tags)) = grammar(relative) else {
        return Ok(Vec::new());
    };
    let path = scoped_path(root, relative.to_str().context("Invalid source path")?)?;
    if !path.is_file() || path.metadata()?.len() > MAX_FILE_BYTES {
        return Ok(Vec::new());
    }
    let source = std::fs::read(&path)?;
    let mut parser = Parser::new();
    parser.set_language(&language)?;
    let tree = parser
        .parse(&source, None)
        .context("Source could not be parsed")?;
    let query = Query::new(&language, tags)?;
    let mut cursor = QueryCursor::new();
    cursor.set_match_limit(10_000);
    let names = query.capture_names();
    let mut results = Vec::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_slice());
    while let Some(m) = matches.next() {
        let kind = m
            .captures
            .iter()
            .find_map(|c| names[c.index as usize].strip_prefix("definition."));
        let name = m
            .captures
            .iter()
            .find(|c| names[c.index as usize] == "name");
        if let (Some(kind), Some(name)) = (kind, name) {
            let node = name.node;
            let value = String::from_utf8_lossy(&source[node.byte_range()]).to_string();
            results.push(json!({"path":relative,"line":node.start_position().row+1,"column":node.start_position().column+1,"name":value,"kind":kind,"precision":"syntax_candidate"}));
            if results.len() >= 500 {
                break;
            }
        }
    }
    results.sort_by(|a, b| {
        (a["line"].as_u64(), a["name"].as_str()).cmp(&(b["line"].as_u64(), b["name"].as_str()))
    });
    results.dedup_by(|a, b| a["line"] == b["line"] && a["name"] == b["name"]);
    Ok(results)
}

fn token_at(source: &str, line: usize, column: usize) -> Option<String> {
    let line = source.lines().nth(line.checked_sub(1)?)?;
    let bytes = line.as_bytes();
    let mut pos = column.checked_sub(1)?.min(bytes.len());
    if pos == bytes.len() || !ident(bytes[pos]) {
        pos = pos.checked_sub(1)?;
    }
    if !ident(*bytes.get(pos)?) {
        return None;
    }
    let mut start = pos;
    let mut end = pos + 1;
    while start > 0 && ident(bytes[start - 1]) {
        start -= 1;
    }
    while end < bytes.len() && ident(bytes[end]) {
        end += 1;
    }
    Some(line[start..end].to_string())
}
fn ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

pub(crate) async fn execute(root: &Path, index: usize, call: &ToolCall) -> Result<Value> {
    match call.name.as_str() {
        "find_source_files" => {
            let query = call.arguments["query"]
                .as_str()
                .context("Missing query")?
                .to_lowercase();
            let (paths, truncated) = files(root).await?;
            let matches = paths
                .iter()
                .filter(|p| {
                    grammar(p).is_some() && p.to_string_lossy().to_lowercase().contains(&query)
                })
                .take(100)
                .collect::<Vec<_>>();
            Ok(json!({"root":index,"paths":matches,"truncated":truncated || matches.len()==100}))
        }
        "source_outline" => {
            let relative = call.arguments["path"].as_str().context("Missing path")?;
            if grammar(Path::new(relative)).is_none() {
                bail!("Unsupported source language");
            }
            let symbols = definitions(root, Path::new(relative))?;
            Ok(
                json!({"root":index,"path":relative,"symbols":symbols,"truncated":symbols.len()==500}),
            )
        }
        "find_symbols" => {
            let query = call.arguments["query"]
                .as_str()
                .context("Missing query")?
                .to_lowercase();
            if query.len() < 2 {
                bail!("Use at least two characters for a symbol search");
            }
            let (paths, files_truncated) = symbol_files(root, &query).await?;
            let mut symbols = Vec::new();
            let mut scanned = 0;
            for path in paths.iter().filter(|p| grammar(p).is_some()).take(500) {
                scanned += 1;
                if let Ok(items) = definitions(root, path) {
                    symbols.extend(items.into_iter().filter(|s| {
                        s["name"]
                            .as_str()
                            .is_some_and(|n| n.to_lowercase().contains(&query))
                    }));
                }
                if symbols.len() >= 100 {
                    break;
                }
            }
            let truncated = files_truncated || scanned == 500 || symbols.len() >= 100;
            symbols.truncate(100);
            Ok(
                json!({"root":index,"symbols":symbols,"truncated":truncated,"precision":"syntax_candidate"}),
            )
        }
        "locate_log_origin" => locate(root, index, call).await,
        "find_definition" | "find_references" => {
            let relative = call.arguments["path"].as_str().context("Missing path")?;
            let path = scoped_path(root, relative)?;
            if grammar(&path).is_none() {
                bail!("Unsupported source language");
            }
            if path.metadata()?.len() > 4 * 1024 * 1024 {
                bail!("Source file is too large for a language server query");
            }
            let source = std::fs::read_to_string(&path).context("Source is not UTF-8")?;
            let line = call.arguments["line"].as_u64().context("Missing line")? as usize;
            let column = call.arguments["column"]
                .as_u64()
                .context("Missing column")? as usize;
            let lsp = crate::source_lsp::query(
                root,
                &path,
                &source,
                line,
                column,
                call.name == "find_references",
            )
            .await;
            if let Ok(locations) = &lsp {
                return Ok(
                    json!({"root":index,"locations":locations,"precision":"semantic","provider":"lsp"}),
                );
            }
            let name = token_at(&source, line, column).context("No symbol at location")?;
            let fallback = if call.name == "find_definition" {
                let (paths, _) = symbol_files(root, &name).await?;
                let mut results = Vec::new();
                for path in paths.iter().filter(|p| grammar(p).is_some()).take(500) {
                    if let Ok(items) = definitions(root, path) {
                        results.extend(items.into_iter().filter(|s| s["name"] == name));
                    }
                    if results.len() >= 100 {
                        break;
                    }
                }
                results
            } else {
                text_candidates(root, &name, 100).await?
            };
            Ok(
                json!({"root":index,"name":name,"locations":fallback,"precision":"syntax_candidate","reason":lsp.err().map(|e|e.to_string()).unwrap_or_default(),"truncated":fallback.len()>=100}),
            )
        }
        _ => bail!("Unknown source tool"),
    }
}

async fn text_candidates(root: &Path, query: &str, limit: usize) -> Result<Vec<Value>> {
    let mut command = crate::ripgrep::command();
    command.args([
        "--line-number",
        "--with-filename",
        "--no-heading",
        "--fixed-strings",
        "--max-columns",
        "300",
        "--max-columns-preview",
        "--max-count",
        "5",
        "--",
    ]);
    command
        .arg(query)
        .arg(".")
        .current_dir(root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let mut output = Vec::new();
    let stdout = child.stdout.take().context("Search output unavailable")?;
    let read = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stdout.take(64 * 1024 + 1).read_to_end(&mut output),
    )
    .await;
    if output.len() > 64 * 1024 || !matches!(&read, Ok(Ok(_))) {
        _ = child.kill().await;
    }
    _ = child.wait().await?;
    read.context("Source search timed out")??;
    let mut results = Vec::new();
    for line in String::from_utf8_lossy(&output).lines() {
        let mut parts = line.splitn(3, ':');
        if let (Some(path), Some(number), Some(text)) = (parts.next(), parts.next(), parts.next()) {
            let path = path.trim_start_matches("./");
            if let Ok(number) = number.parse::<usize>()
                && grammar(Path::new(path)).is_some()
                && scoped_path(root, path).is_ok()
            {
                results.push(
                    json!({"path":path,"line":number,"text":text,"precision":"text_candidate"}),
                );
            }
        }
        if results.len() >= limit {
            break;
        }
    }
    Ok(results)
}

async fn locate(root: &Path, index: usize, call: &ToolCall) -> Result<Value> {
    let clue = call.arguments["clue"].as_str().context("Missing clue")?;
    if clue.is_empty() {
        bail!("Empty log clue");
    }
    let clue = &clue[..clue.floor_char_boundary(clue.len().min(2048))];
    let mut candidates = Vec::new();
    for word in clue.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '[' | ']' | ','))
    {
        let word = word.trim_end_matches(':');
        if let Some((file, number)) = word.rsplit_once(':')
            && let Ok(line) = number.parse::<usize>()
        {
            let file = file.trim_start_matches("at ");
            let file = Path::new(file);
            if grammar(file).is_some() {
                let (paths, _) = files(root).await?;
                for path in paths.iter().filter(|p| p.ends_with(file)).take(10) {
                    if scoped_path(root, &path.to_string_lossy()).is_ok() {
                        candidates.push(
                            json!({"path":path,"line":line,"precision":"log_location_candidate"}),
                        );
                    }
                }
            }
        }
    }
    let text = clue.split([':', '(']).next().unwrap_or(clue).trim();
    if candidates.is_empty() && text.len() >= 4 {
        candidates = text_candidates(root, text, 20).await?;
    }
    candidates.truncate(20);
    Ok(
        json!({"root":index,"locations":candidates,"precision":"candidate","note":"Verify candidates against source and surrounding logs"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn finds_files_symbols_and_stack_frames() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("Worker.java"),
            "class Worker { void processOrder() {} }",
        )
        .unwrap();
        let call = |name: &str, arguments| ToolCall {
            id: "test".into(),
            name: name.into(),
            arguments,
        };
        let found = execute(
            &root,
            0,
            &call("find_source_files", json!({"query":"worker"})),
        )
        .await
        .unwrap();
        assert_eq!(found["paths"][0], "Worker.java");
        let symbols = execute(
            &root,
            0,
            &call("find_symbols", json!({"query":"processOrder"})),
        )
        .await
        .unwrap();
        assert_eq!(symbols["symbols"][0]["name"], "processOrder");
        let origin = execute(
            &root,
            0,
            &call(
                "locate_log_origin",
                json!({"clue":"at Worker.processOrder(Worker.java:1)"}),
            ),
        )
        .await
        .unwrap();
        assert_eq!(origin["locations"][0]["path"], "Worker.java");
        assert_eq!(origin["locations"][0]["line"], 1);
    }
    #[test]
    fn extracts_supported_symbols() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        for (file, content, name) in [
            ("a.java", "class Alpha { void runTask() {} }", "runTask"),
            ("a.cpp", "int run_task() { return 1; }", "run_task"),
            ("a.rs", "fn run_task() {}", "run_task"),
            ("a.py", "def run_task():\n    pass\n", "run_task"),
            ("a.js", "function runTask() {}", "runTask"),
            ("a.ts", "function runTask(): void {}", "runTask"),
            ("a.tsx", "function RunTask() { return <div />; }", "RunTask"),
            ("a.go", "package main\nfunc runTask() {}", "runTask"),
            ("a.cs", "class Alpha { void RunTask() {} }", "RunTask"),
        ] {
            std::fs::write(root.join(file), content).unwrap();
            let items = definitions(&root, Path::new(file)).unwrap();
            assert!(
                items.iter().any(|item| item["name"] == name),
                "{file}: {items:?}"
            );
        }
    }
}
