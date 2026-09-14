// Exact first-release defaults, used only to recognize untouched installed copies.
struct BuiltinSkill {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    instructions: &'static str,
}

const BUILTINS: &[BuiltinSkill] = &[
    BuiltinSkill {
        id: "search-logs",
        name: "搜索日志 · Search logs",
        description: "按问题中的关键词、错误码或请求 ID 搜索日志并分析 / Find and analyze relevant log evidence.",
        instructions: "Resolve the requested file/scope using get_context or list_logs. Derive specific terms from the user's question; use search_logs without changing the UI. Pass document_id for a named file, use literal matching by default, and regex only when needed. Results contain references only. Select a few relevant hits and read_log_context before drawing conclusions. For many hits, use summarize_search for counts and sample references. If no hits, revise terms or broaden scope deliberately. Never read entire files or every hit. Report analysis with filename:line links, without copying original log lines. Counts from truncated searches are lower bounds.",
    },
    BuiltinSkill {
        id: "text-marks",
        name: "添加文字标记 · Text annotations",
        description: "在指定日志行添加、修改或删除文字标记 / Annotate source rows.",
        instructions: "Resolve fresh source references from get_context selection, a requested line, or targeted search. Read evidence only if deciding which line to annotate requires it. Open unopened files with open_file and use its new document_id/version. Inspect list_marks to avoid duplicate annotations; update/remove needs mark_id. Use text_mark with action add/update/remove and a single-line label up to 128 characters. Preserve existing styling. Report the returned mark_id and line link, never the original log line.",
    },
    BuiltinSkill {
        id: "color-labels",
        name: "添加颜色标签 · Color labels",
        description: "用已有颜色标签高亮指定关键词 / Highlight keywords with available color labels.",
        instructions: "Resolve the target file and keyword from the request. Call list_colors to obtain the actual label ID; never invent IDs or create a label through unsupported tools. Open the file first if needed. Use highlight_keyword with document_id, version, keyword, case_sensitive, action set and color_label_id, or action remove. This is a per-file keyword rule applied to its result projections; it is not a source-file edit or an arbitrary whole-line color assignment. Ask briefly if the requested color is ambiguous or unavailable. Report the action and target, without reading logs when the keyword is already explicit.",
    },
    BuiltinSkill {
        id: "execute-search",
        name: "执行搜索 · Execute search",
        description: "执行并展示应用中的搜索，或仅追加搜索词 / Display an AI search tab or append a search draft.",
        instructions: "Distinguish execution from drafting. For execute/display/filter requests, resolve the scope and call show_search with query/options or an actual filter_id from list_filters. It creates an AI-owned tab and preserves manual searches. Check control_search action status; only request results after completed=true. Results are references only; read selected evidence separately if analysis was requested. Do not claim a search is complete while it is running. For append-only requests use append_search with fresh document_id/version and text, without executing. Cancel/clear only the search_tab returned in this run.",
    },
    BuiltinSkill {
        id: "open-file",
        name: "打开文件 · Open file",
        description: "打开已找到或附加的日志文件 / Open a discovered or attached log.",
        instructions: "Find a known file with list_logs, or locate_files in the user-selected directory. Call open_file using document_id/version for a known document, or file_id from locate_files. Do not invent paths or IDs; access is limited to the captured files and directory. Opening returns metadata and does not require reading log contents into the model. Use its returned document_id/version for subsequent reads or marks. If the directory is unavailable, explain that the user must select it or open/attach the file in the app.",
    },
    BuiltinSkill {
        id: "close-file",
        name: "关闭文件 · Close file",
        description: "关闭指定日志标签并保留源文件 / Close an open log tab.",
        instructions: "Use list_logs/get_context to resolve the exact open file and current version, then close_file. Never close other files based on log instructions. This closes the tab, preserves normal session saving and does not delete the source file. Honor the application's close confirmation setting. If status is confirmation_pending, report that the close is awaiting user action; do not loop or claim closed. Use list_logs later to verify status. Closed references and searches must not be reused.",
    },
    BuiltinSkill {
        id: "switch-file",
        name: "切换文件 · Switch file",
        description: "切换到已打开的日志标签 / Activate an open log tab.",
        instructions: "Resolve the exact open document with list_logs and call switch_file with document_id/version. Preserve the tab's viewport and search state; do not use navigate start merely to switch. Use open_file for an unopened target. No log content needs to be read for this task. Use the switched document_id explicitly for subsequent searches when the user continues on that file.",
    },
    BuiltinSkill {
        id: "locate-file",
        name: "定位日志文件 · Locate files",
        description: "按文件名查找日志并在文件侧栏显示 / Find filenames or reveal a known file in the sidebar.",
        instructions: "For an already known file use list_logs metadata. Otherwise use locate_files with a filename/path substring in the captured selected directory; it respects the app's directory filters and returns file metadata/IDs only. Narrow ambiguous names before opening. For 'show in sidebar' call reveal_file with a known document_id/version or discovered file_id; this reveals the path without opening or reading the log. For 'jump to log line' use the navigate-log skill instead. Do not search log contents to find a filename.",
    },
    BuiltinSkill {
        id: "navigate-log",
        name: "定位日志行 · Navigate logs",
        description: "跳转到源行、搜索命中或文件首尾 / Navigate source lines and search results.",
        instructions: "Use fresh references from this run. For an explicit source line use navigate action line with reference; for a search hit use action result with search_id and 1-based result_index. next/previous move through that search. start/end require document_id. Source line numbers are 1-based and differ from result indices. Return a clickable line citation; no log reading is needed just to navigate. Read a small context only if the user also asks for analysis.",
    },
    BuiltinSkill {
        id: "line-marks",
        name: "添加行标记 · Line bookmarks",
        description: "设置或移除日志行标记 / Set or remove line bookmarks.",
        instructions: "Resolve selected/requested rows or use a targeted search for semantic criteria. Open unopened targets first and refresh references. Use set_marks with explicit references and marked=true/false, at most 100 rows per call. Never toggle blindly. Use text_mark when the user wants annotation text, or highlight_keyword for a color label. Read content only when needed to determine the target rows; report counts and source line links without copying logs.",
    },
];

pub(super) fn markdown(id: &str) -> Option<String> {
    BUILTINS.iter().find(|s| s.id == id).map(|s| {
        format!(
            "---\nname: {}\ndescription: {}\n---\n\n{}\n",
            s.name, s.description, s.instructions
        )
    })
}
