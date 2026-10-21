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

File-association and command-line requests first foreground the last active window and yield a presented frame before looking up file ownership or changing tabs. Routing is deferred until the frame callback releases its window, so a temporary GPUI window lease is not mistaken for a missing window. An already-open file then activates its owning window, which also presents before document handling. External opens scroll the tab bar to reveal the active file tab, whether newly created or already open, using the same reveal mechanism as dropped files. A cold start presents the empty workspace before installing initial file tabs. Reopening the application without paths activates its existing window. New files immediately receive loading tabs, including during startup state restoration or another file load. File preparation begins after the loading shell has been presented; the shared loader remains bounded and preserves session restoration. Closing a queued tab cancels its pending open, and background completion preserves the current tab unless an explicit search navigation is pending.

On Windows, the forwarding process obtains the server PID from the connected named pipe and requests restoration/foreground activation of a visible, unowned window belonging to that process before transmitting file paths. A visible last-active popup is preferred for activation. Restore uses `ShowWindowAsync`, so the sender does not wait for the target's resize callbacks. The primary retains the routing above as a fallback when no window exists yet or Windows denies foreground activation; file ownership is still resolved by the primary. The forwarding process skips data-directory write probes and crash-report maintenance.

Pipe connection retries and the client exchange share a five-second deadline; each connected server exchange has a five-second deadline. Overlapped operations are canceled at expiry and their kernel completion is drained before buffers are released. Idle listening has no timeout. After acknowledgement, the server waits for a receipt or client disconnection within the remaining deadline instead of calling unbounded `FlushFileBuffers`; older clients can still close after reading the acknowledgement. Only a busy or not-yet-created pipe is retried before connection. Once transmission starts, a failed acknowledgement is not automatically retried, since the primary may already have accepted the request.

## Data and state

`StateRepository` owns SQLite connections, SQL and migrations. **Current schema: v22**. App `StateStore` converts stable data records to presentation models; it does not own SQL. WAL and revision-aware session merging preserve unrelated updates from different windows. File paths use lossless native identity; display strings are not database identity.

File state (marks, text tags, color rules) is shared by searches of that file. Search tabs own draft/committed queries, scope, row limits, result mode, selection, collapsed groups and viewport. One search runs per window, with other requests queued; completing a background tab does not activate it. Stale tasks must be rejected before publication.

Line indices and compressed result sets avoid retaining all decoded text. Visible-line reads, search snapshots and exports retain stable source-line identity. Incremental refresh has an explicit head/tail sampling limitation; see [live logs](Live-Logs.md).

Ordinary browsing and annotations do not write source logs. Explicit [editing](Editing.md) is a separate write path with a 100 MiB entry limit and disk-version checks. Closing or renaming a tab changes workspace state, not the source filename.

The optional Go server is built independently. Its default signature requirement is not automatically compatible with the current desktop client's unsigned registration; see [cloud filters](Cloud-Filters.md).

Source: [workspace modules](https://github.com/zhaiyanqi/vclogg2/tree/main/crates/vclogg-app/src/workspace), [core](https://github.com/zhaiyanqi/vclogg2/tree/main/crates/vclogg-core/src), [data repository](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-data/src/state_repository.rs).

## Performance traces

`ui_performance/trace.rs` retains the application Recorder. `trace/startup.rs` begins CLI capture before platform initialization and attaches the foreground journal in the GPUI callback; the collector drains journal and task buffers every 50 ms. The independent writer in `trace/live.rs` waits 250 ms after draining. `trace/stream.rs` encodes batches of at most 2048 new completed events, replaces only the final JSON-array bracket, appends events and another bracket, and calls sync_data for each batch. Perfetto accepts truncated array tails, retaining complete events preceding a crash. Stop only drains the remainder, adds unfinished scopes and the final summary, then closes the file.

Capture owns Stream with its file handle, committed byte offset, event cursor, per-thread interval-edge indices and failed batch. Retries rewrite that same pending batch at its original offset without replaying committed history. Only new event Arcs and active scopes are cloned under short locks; sorting, serialization and I/O stay on background threads. Cache paths and timestamp/UUID names are unchanged; failed final writes retain state for retry. Active scopes produce instant observations about once per second, avoiding duplicate execution durations across periodic writes and final slices.

Application scopes register their starting thread and timestamp on entry. Completion moves them into the event buffer; freezing preserves active scopes and adds a GPUI boundary-task snapshot marked `incomplete`. Completed events and active scopes are bounded at 200,000 / 4,096, with losses in the summary. Skips, idle, power transitions and aggregated short polls use instant events; dirty-to-present and callback delivery latency use instant markers with elapsed durations rather than CPU-work slices. Application scopes cover startup, document preparation/indexing, search execution/publication, tab activation, visible-line loading and settings application. New scopes do not read the clock when capture is inactive. See [cold-start options](Building.md#cold-start-performance-capture).

Each Capture explicitly retains the UI Rust ThreadId from the foreground callback (manual capture) or main (cold start). Export labels each thread track `GPUI UI / Main` or `Background` using that identity and includes the source Rust thread in event arguments. UI tasks and application scopes are not classified as background work based on their category. Each actual Rust thread maps to one tid. Each batch sorts chronologically with outer intervals first. Late crossing intervals are split against previously emitted boundaries and continue on the same thread without rewriting history, retaining source event IDs and original timing instead of creating synthetic threads. Frame latency uses instant markers with elapsed_us. Segment expansion is bounded and omissions are reported.

Debug and Release control timings both delegate to on-demand trace scopes; timing elements are not installed while recording is inactive. Threshold-based console diagnostics, contributor aggregation and the background HangDetector monitor were removed to avoid logging overhead in rendering. The `ui-performance-profiler` feature remains only as a compatibility alias for existing commands.

### File-open INFO diagnostics

The dedicated `vclogg2::open` INFO channel starts at process entry and covers instance connection/retries, early sender activation, pipe I/O/timeouts/cancellation drain, UI delivery, window routing/restoration, activation events and frame callbacks, session loading, preview, full index preparation and installation. This diagnostic channel retains INFO even when the ordinary log level is Error or Off; other records still follow that setting.

A dedicated worker asynchronously writes `logs/open/open-<UTC timestamp>-<PID>.log` under the application data directory, rotating at approximately 8 MiB and retaining the latest 64 files on a best-effort basis. Forwarding processes also persist records and wait up to one second to drain on normal exit. Queue overflow, disk errors or abnormal termination can lose records; exports include the current process's dropped-record count. Activation and parsing threads do not wait for disk writes or cleanup. Settings' log export automatically appends recent diagnostic files, up to 16 MiB. Export promptly after reproducing; include relevant files from that directory if the export reports its size limit.

Correlate timestamps, PID, `stage=<PID>-<sequence>` and `elapsed_ms`. Windows receiver records link `client_pid` to the sender and `request` to UI delivery; subsequent batches correlate through paths, window IDs and time. `queued`/pipe ACK means only enqueued; `scope_exit` can also indicate failure or cancellation. `open_install_result ready=true` means the complete document was installed. `frame_callbacks_complete` marks subsequent frame callbacks, not confirmed desktop compositor presentation. Application logs cannot measure the interval between the OS double-click and process entry. Include the approximate double-click time, minimized state and whether multiple files were opened consecutively.
