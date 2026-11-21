# Implementation status

[简体中文](../zh-CN/Implementation-Status.md) · [Home / 首页](Home.md)

This is a source reference for the workflows in the [reader’s guide](Home.md).

| Area | Current behavior / boundary |
| --- | --- |
| Large logs | Indexed virtual reading; memory and latency still depend on line count, encoding and line length |
| Search | Current file, selected open files and directory snapshots; independent search tabs; completion-only publication |
| Live refresh | Foreground window's active ready document only; head/tail sampling has a documented limitation |
| Editing | Explicit full-text edit mode, no fixed file-size limit, disk size/time checks |
| Settings | One settings window per process. Valid changes apply across windows and save automatically. |
| Persistence | SQLite schema v24; revision-aware session merging |
| Cloud | UUID/revision merge and cached browsing; desktop/server authentication compatibility must be checked |
| Updates | Manual checks request the remote manifest first; failures show a valid cache with a connection warning and last success time, never an up-to-date claim; no valid cache opens Releases; no automatic download/install |
| Removed features | AI analysis, MCP and memory. The current left sidebar shows files or vertical tabs. |
| Documentation | Main-repository bilingual Markdown, mdBook, shared Pages artifact with the website |

See [UI acceptance](UI-Acceptance.md) and [filter acceptance](Filter-Branch-Acceptance.md) for manual review scenarios. Historical results are retained in the [archive](Archive.md); they are not current test results.
