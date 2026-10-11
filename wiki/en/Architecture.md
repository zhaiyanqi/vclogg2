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

## Data and state

`StateRepository` owns SQLite connections, SQL and migrations. **Current schema: v22**. App `StateStore` converts stable data records to presentation models; it does not own SQL. WAL and revision-aware session merging preserve unrelated updates from different windows. File paths use lossless native identity; display strings are not database identity.

File state (marks, text tags, color rules) is shared by searches of that file. Search tabs own draft/committed queries, scope, row limits, result mode, selection, collapsed groups and viewport. One search runs per window, with other requests queued; completing a background tab does not activate it. Stale tasks must be rejected before publication.

Line indices and compressed result sets avoid retaining all decoded text. Visible-line reads, search snapshots and exports retain stable source-line identity. Incremental refresh has an explicit head/tail sampling limitation; see [live logs](Live-Logs.md).

Ordinary browsing and annotations do not write source logs. Explicit [editing](Editing.md) is a separate write path with a 100 MiB entry limit and disk-version checks. Closing or renaming a tab changes workspace state, not the source filename.

The optional Go server is built independently. Its default signature requirement is not automatically compatible with the current desktop client's unsigned registration; see [cloud filters](Cloud-Filters.md).

Source: [workspace modules](https://github.com/zhaiyanqi/vclogg2/tree/main/crates/vclogg-app/src/workspace), [core](https://github.com/zhaiyanqi/vclogg2/tree/main/crates/vclogg-core/src), [data repository](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-data/src/state_repository.rs).
