pub(super) const ANALYSIS: &str = r#"# Workflow: search-and-analysis

Use this guide for investigations and searches whose intent is ambiguous. Direct searches with a clear query can use tool definitions immediately. The search-logs and execute-search entries share this guide; read it once per run.

1. Choose the requested outcome. Background evidence gathering uses search_logs. A request to display/filter results uses show_search, followed by control_search status/results. Appending text without execution uses append_search. A request to explain existing results does not authorize changing the user's search.
2. Turn a symptom into a falsifiable question. For slow login, look for login/authentication events, elapsed durations and timeout/retry events; a generic ERROR hit alone does not establish the delay's cause. For unknown formats, read a few selected or head/tail rows first. Keep inferred keywords distinct from terms actually observed in the file. Literal query | means OR; spaces do not mean AND. If the user needs an exact pipe, escape it in regex mode.
3. Search by the strongest observed identifier or event. On zero hits, change the relevant spelling, case, identifier or scope and remember what was already tried. On many hits, summarize counts and choose source references across affected files and event boundaries. Positionally spaced samples are not distinct error categories; inspect minority files and rare failures deliberately.
4. Read evidence, then investigate the remaining gap. For a long truncated line, read_log_segment with search_id centers on the original match; start_character reads a continuation. Check both line truncation and match_truncated before interpreting a payload. For correlations, follow a request ID through start, error and completion/recovery and use timestamps carefully when timezone or clock order is unknown.
5. Separate a quantitative question from causal analysis. Exact retained match counts do not count unique incidents. Grouping by a request ID needs evidence of that identity; truncated search totals are lower bounds. All rows in a small relevant set may be read within budget. For large sets, use bounded samples and describe coverage.
6. Preserve verified key points before answering, unless the user requested read-only analysis or no visual changes. Open any unopened evidence file, then call set_marks with marked=true for only the decisive source rows: the triggering failure, causal transition, impact boundary, or confirmed recovery. Keep the set small (normally no more than eight rows across the investigation); do not mark every match, samples chosen only by position, unverified hypotheses, or generic severity rows.
7. Add useful keyword color after the row targets are verified. Call list_colors once, choose an existing semantically appropriate label, and call highlight_keyword separately for each relevant open file. Prefer one to three exact, discriminative terms actually observed in that file—an error code, exception, request/state name, or distinctive component. Do not invent a color ID or keyword, create labels, or broadly color INFO/ERROR/WARN unless that severity itself is the finding. If no suitable label or stable term exists, skip highlighting instead of guessing.
8. Finish when the question is answered by evidence or a specific missing observation blocks it. After two unproductive revisions, report the missing clue or ask for it. Do not exhaust the request budget polling a running UI search; return its running status when necessary. Report which rows were marked and which per-file keywords were highlighted; a failed or pending mutation must be described as partial completion, not success.
"#;

pub(super) const FILES: &str = r#"# Workflow: file-operations

Use this guide for ambiguous targets or combined file operations. Simple open/close/switch/reveal requests can use their tool definitions directly. Four file-operation entries share this guide; read it once per run.

Resolve identity before changing state. 'This file' follows current context; a named file uses list_logs paths and versions. Equal basenames in different directories are ambiguous: prefer the explicit path or ask which one. Use list_log_directory when directory grouping, rotations, sibling services or nearby files may identify related logs; use locate_files for a known filename/path fragment. Both stay within the captured directory and its configured filters. Configured project files discovered with source tools can be opened with their root and relative path, including when the project is outside the app's current log directory. No matches may mean a wrong directory or filter, not that the file does not exist on the device.

Choose the intended action: list_log_directory browses related files, locate_files finds paths, reveal_file shows a path in the sidebar, switch_file activates an existing tab while preserving its position, and open_file opens or activates a known target. Line navigation uses navigate. Do not jump to line 1 simply to switch files. If the target is outside the captured capabilities, explain the concrete action needed to make it available in the app.

For 'open then search/mark', open first and carry forward the returned document_id/version; an unopened search-result ID may be replaced by the actual tab ID. Pass the resulting ID explicitly to later file-specific searches. Do not substitute a different current tab if the requested target disappeared.

For closing, preserve the application's confirmation flow. confirmation_pending means waiting for the user's choice, not closed. Do not reissue close while confirmation is pending. A later status refresh can establish the outcome. For multiple requested operations, report which finished and which are pending/failed, and stop dependent actions when their prerequisite did not complete. File sources remain read-only; closing a tab does not delete its file.
"#;

pub(super) const MARKS: &str = r#"# Workflow: marks-and-highlighting

Use this guide for semantic targets, mixed mark types or recovery after partial edits. A clear action on an explicit row/keyword can use tool definitions directly. Three mark entries share this guide; read it once per run.

First identify the intended representation. set_marks sets/removes source-row bookmarks and is the default way to preserve verified decisive evidence from an investigation. text_mark adds/updates/removes user-visible annotation text; do not add prose annotations automatically when a line bookmark is sufficient. highlight_keyword applies an existing color-label rule to a keyword in one file and all its projections. Keyword colors do not create color labels, do not assign arbitrary whole-line colors, and do not set text annotation colors. If an unavailable capability was requested, explain that specific limitation instead of silently substituting a different action.

For explicit rows or keywords, avoid content reads that do not change the decision. For an investigation, establish the target rows first and mark only the causal or decision-relevant events, normally no more than eight rows total. Distinguish failed from recovered attempts; do not mark every generic error just because it matched a broad query. Open unopened targets before changing marks and use the returned new identity. Skip automatic visual changes when the user asks for read-only analysis.

Use explicit marked=true/false rather than toggling; set_marks is idempotent for an already-marked row. Inspect list_marks before an ambiguous text-annotation update or retry, use actual mark_id values, avoid duplicates, and preserve existing styling on updates. Before highlighting, call list_colors and use an actual returned ID. Choose one to three exact high-signal terms observed in each relevant file and apply highlight_keyword per file; never invent a term or use a broad severity token that would color unrelated events. For an explicit user color request, ask only if it is ambiguous or unavailable; for automatic investigation highlighting, skip the highlight when no semantic match exists rather than blocking the answer.

For batch operations, retain per-file targets and report partial completion accurately. After an error, inspect state before retrying a mutation; repeated 'add' actions may duplicate annotations. A tool failure or pending open operation is not evidence that marks were applied.
"#;

pub(super) const NAVIGATION: &str = r#"# Workflow: log-navigation

Use this guide when source lines, search result indices or historical references could be confused. Direct navigation to a known row can use the tool definition immediately.

Distinguish the target coordinate: a source reference is document_id/version plus a 1-based source line; a result_index is 1-based within a particular search_id and may refer to a different file. Never use a result index as a source line. 'Next' or 'previous' refers to the resolved search, while start/end refers to the resolved file. Clarify the intended search only when current context cannot resolve it.

Prefer navigate action=result for a search result so the application selects that result projection when available. For a specific source row, use action=line with its reference. A collapsed or unavailable projection may navigate to the source instead; report the actual returned region. To reveal the file's path use reveal_file, not a row jump.

Old-turn references require refreshed identity before new tool operations. A changed source can move a formerly relevant event to a different line: rerun a targeted search when the user means the event, rather than blindly reusing its old line number. Plain navigation needs no content read. If explanation is also requested, read only the relevant target and context after resolving the reference.
"#;
