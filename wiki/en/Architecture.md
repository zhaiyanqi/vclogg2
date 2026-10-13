# Architecture

[简体中文](../zh-CN/Architecture.md) · [Home / 首页](Home.md)

The Cargo workspace separates application presentation, file/search algorithms and durable storage. `crates/vclogg-app` builds package/binary **vclogg2**; use `-p vclogg2` in Cargo commands.

```text
crates/vclogg-app (vclogg2)
  ├── vclogg-core: files, indexing, decoding, search, compressed rows
  └── vclogg-data: SQLite, records, path encoding, cache lifecycle
server/: independent Go module + Vue administration UI
compat/xim-ctext/: legacy XIM decoder bridge
vendor/block/: existing macOS dependency compatibility patch
wiki/: bilingual mdBook sources
 doc/: existing website and assets
```

App composes core and data. Core does not depend on data/app; data does not depend on app. Neither core nor data owns GPUI entities. `scripts/check-architecture.sh` statically guards these boundaries.

## Capability map

| Area | Source and responsibility |
| --- | --- |
| Window composition | `workspace.rs`, `workspace/render_shell.rs`, `window_registry.rs`: native windows, focus, external requests, shutdown snapshots |
| Document lifecycle | `document_opening.rs`, `document_tasks.rs`, `document_lifecycle.rs`: bounded preparation, install, reload, recovery |
| File tabs | `tab_lifecycle.rs`, `tab_drag.rs`, `cross_window_tab_drag.rs`, `tab_motion.rs`: activation, transfer, order, transitions |
| Search tabs | `search_tabs.rs`, `search_tab_ui.rs`, `search_tab_order.rs`, `search_tab_activation.rs`, `search_tab_tasks.rs`: per-tab state, queue and publication |
| Search execution | `search_orchestration.rs`, `search_limits.rs`, `quick_find.rs`: scope, source ranges, committed queries and local find |
| Presentation | `log_presentation.rs`, `viewport_orchestration.rs`, `log_viewport.rs`, `virtual_log_list.rs`, `virtual_log_lines.rs`: visible rows, source identity, selection, wrapping |
| Settings | `settings_dialog.rs`, `workspace/settings_window.rs`, `preferences.rs`, `highlight_preferences.rs`: singleton settings window, draft preview and save |
| Explicit writes | `document_editing.rs`, `clipboard_paste.rs`, `result_export_flow.rs`: editing, temporary clipboard documents and captured-result export |
| File engine | core `document.rs`, `document/index_cache.rs`, `document/text_window.rs`: immutable snapshots, positional reads, decoding and line indices |
| Search engine | core `search.rs`, `search/cache.rs`, `result_set.rs`, `cancellation.rs`: matching, cache, compressed source rows and cancellation |
| Persistence | data `state_repository.rs`, `state.rs`, `path_codec.rs`, `index_cache.rs`: transactions, stable DTOs, lossless paths and cache maintenance |

Paths in the first seven rows are under `crates/vclogg-app/src/`; unqualified capability filenames are under `workspace/`.

File-association requests activate the window that already owns the file, or use the last active window for a new file. New files immediately receive loading tabs, including during startup state restoration or another file load. File preparation begins after the loading shell has been presented; the shared loader remains bounded and preserves session restoration. Closing a queued tab cancels its pending open, and background completion preserves the current tab unless an explicit search navigation is pending.

## Data and state

`StateRepository` owns SQLite connections, SQL and migrations. **Current schema: v22**. App `StateStore` converts stable data records to presentation models; it does not own SQL. WAL and revision-aware session merging preserve unrelated updates from different windows. File paths use lossless native identity; display strings are not database identity.

File state (marks, text tags, color rules) is shared by searches of that file. Search tabs own draft/committed queries, scope, row limits, result mode, selection, collapsed groups and viewport. One search runs per window, with other requests queued; completing a background tab does not activate it. Stale tasks must be rejected before publication.

Line indices and compressed result sets avoid retaining all decoded text. Visible-line reads, search snapshots and exports retain stable source-line identity. Incremental refresh has an explicit head/tail sampling limitation; see [live logs](Live-Logs.md).

Ordinary browsing and annotations do not write source logs. Explicit [editing](Editing.md) is a separate write path with a 100 MiB entry limit and disk-version checks. Closing or renaming a tab changes workspace state, not the source filename.

The optional Go server is built independently. Its default signature requirement is not automatically compatible with the current desktop client's unsigned registration; see [cloud filters](Cloud-Filters.md).

Source: [workspace modules](https://github.com/zhaiyanqi/vclogg2/tree/main/crates/vclogg-app/src/workspace), [core](https://github.com/zhaiyanqi/vclogg2/tree/main/crates/vclogg-core/src), [data repository](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-data/src/state_repository.rs).

## Performance traces

`ui_performance/trace.rs` retains the application Recorder. `trace/startup.rs` starts CLI capture before platform initialization and attaches the foreground journal in the GPUI callback; the collector drains journal and task buffers every 50 ms. The independent writer in `trace/live.rs` waits one second after each save, generating complete Chrome Trace Event JSON snapshots on a shared Instant timeline and atomically replacing the session file through a temporary file in the same directory. Stop triggers a final snapshot. app_paths owns cache resolution; timestamp-and-UUID names isolate sessions. Manual capture no longer prompts for a destination. Writes retry automatically; failed final saves retain data for retry. Snapshot locking only clones Arc events and active scopes; sorting, serialization and file I/O occur on the writer thread.

Application scopes register their starting thread and timestamp on entry. Completion moves them into the event buffer; freezing preserves active scopes and adds a GPUI boundary-task snapshot marked `incomplete`. Completed events and active scopes are bounded at 200,000 / 4,096, with losses in the summary. Skips, idle, power transitions and aggregated short polls use instant events; dirty-to-present and callback delivery latency use instant markers with elapsed durations rather than CPU-work slices. Application scopes cover startup, document preparation/indexing, search execution/publication, tab activation, visible-line loading and settings application. New scopes do not read the clock when capture is inactive. See [cold-start options](Building.md#cold-start-performance-capture).

Each Capture explicitly retains the UI Rust ThreadId from the foreground callback (manual capture) or main (cold start). Export labels each thread track `GPUI UI / Main` or `Background` using that identity and includes the source Rust thread in event arguments. UI tasks and application scopes are not classified as background work based on their category. Each actual Rust thread maps to one tid. Chronological, outer-first export splits crossing intervals at parent boundaries and continues them on the same thread, retaining source event IDs and original timing instead of creating synthetic threads. Frame latency uses instant markers with elapsed_us. Segment expansion is bounded and omissions are reported.

Debug and Release control timings both delegate to on-demand trace scopes; timing elements are not installed while recording is inactive. Threshold-based console diagnostics, contributor aggregation and the background HangDetector monitor were removed to avoid logging overhead in rendering. The `ui-performance-profiler` feature remains only as a compatibility alias for existing commands.
