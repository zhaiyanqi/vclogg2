# Implementation status

[简体中文](../zh-CN/Implementation-Status.md) · [Home / 首页](Home.md)

This page describes source at `3c7086b`, not a newly executed acceptance report.

| Area | Current behavior / boundary |
| --- | --- |
| Large logs | Indexed virtual reading; memory and latency still depend on line count, encoding and line length |
| Search | Current file, selected open files and directory snapshots; independent search tabs; completion-only publication |
| Live refresh | Foreground window's active ready document only; head/tail sampling has a documented limitation |
| Editing | Explicit full-text edit mode, 100 MiB entry limit, disk size/time checks |
| Settings | One process-wide native settings window; preview, save and rollback |
| Persistence | SQLite schema v22; revision-aware session merging |
| Cloud | UUID/revision merge and cached browsing; desktop/server authentication compatibility must be checked |
| Updates | Help opens GitHub Releases; desktop does not automatically download/install updates |
| Removed features | AI analysis, MCP, memory and workspace sidebars |
| Documentation | Main-repository bilingual Markdown, mdBook, shared Pages artifact with the website |

See [UI acceptance](UI-Acceptance.md) and [filter acceptance](Filter-Branch-Acceptance.md) for manual review scenarios. Historical results are retained in the [archive](Archive.md); they are not current test results.
