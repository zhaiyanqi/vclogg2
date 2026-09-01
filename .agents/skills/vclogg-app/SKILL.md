---
name: vclogg-app
description: Navigate and modify the VCLogg2 GPUI application shell, workspace orchestration, virtual log views, presentation state, dialogs, and interactions. Use for crates/vclogg-app UI behavior; do not place file scanning or persistence implementation here.
---

# VCLogg2 App

App owns GPUI entities, rendering, commands and user presentation state. Read [doc/architecture.md](../../../doc/architecture.md), then use the `gpui` and `gpui-component` skills for framework-specific changes.

## Route the change

- `workspace.rs`: application/window orchestration and current extraction source; place new code in a named capability module instead of extending this file.
- `workspace/document_tasks.rs`: bounded path preparation, document preview/index/search preparation, clipboard/color background snapshots and result grouping.
- `workspace/log_presentation.rs`: visible log/result rendering, highlight composition, text and row selection, drag gestures, context actions, wrapped tables and viewport anchors.
- `workspace/quick_find.rs`: page-local find lifecycle, matcher/highlight presentation, cancellable scans, source-version rejection and viewport navigation.
- `workspace/render_shell.rs`: title bar, file toolbar, tabs, search controls, quick-find controls and empty-workspace/recent-file shell rendering.
- `workspace/result_export_flow.rs`: capture immutable local/global result projections and orchestrate save, timestamp-merge and open-in-new-tab export flows.
- `workspace/search_orchestration.rs`: committed local/global/directory search scope, history and persisted presentation, cancellable task dispatch, result installation and clearing.
- `workspace/tab_lifecycle.rs`: prewarm and activate tabs, close/reorder them, transfer them across windows, persist their order, rename or trash files, and build tab context menus.
- `workspace/viewport_orchestration.rs`: line height and wrapping mode, atomic scroll-frame preparation, cancellable visible-line scheduling, viewport ownership and keyboard navigation.
- `workspace/window_registry.rs`: window registration/focus order, cross-window tab drag state, external open routing and process-quit persistence snapshots.
- `workspace/*_tests.rs`: capability-focused workspace regression suites kept outside the production orchestration file.
- `workspace_state.rs`: retained controllers, task registries and presentation/search coordination state.
- `log_table.rs`, `global_search_table.rs`, `virtual_log_lines.rs`, `sparse_virtual_list.rs`: source/projection/presentation virtualization and visible decoded-row lifetime.
- `selectable_log_text.rs`, `color_labels.rs`, `ui_theme.rs`: selection, highlighting and visual composition.
- `state_store.rs`: app-model adapter for settings, marks, color rules, search contexts and tab-resume records; all SQLite operations live in data.
- `*_dialog.rs`, `actions.rs`: feature dialogs, commands, focus and keyboard entry points.

User highlighting, selection, selected text, font size, line height, wrapping, marks, viewport and interaction state stay in app. File reads, index construction, search execution and result-set algorithms call core. SQLite, cached data and recovery storage call data.

## Preserve these invariants

- Render paths only compose installed visible snapshots; never synchronously read a log file from `render` or a row renderer.
- Keep source data, row projection and presentation state separate. Hidden or unreachable decoded rows are released while lightweight selection, mark and viewport state survives.
- Visible-row work is cancellable and results install only when document identity, projection and revision still match.
- Repeated elements use stable domain identity. Async tasks mutate entities only on the GPUI foreground context and notify once after coherent updates.

## Verify

Run the focused module regression first. Virtualization changes should cover the visible-line, projection and group-collapse tests. Finish with `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, `git diff --check`, and `bash scripts/check-architecture.sh`.
