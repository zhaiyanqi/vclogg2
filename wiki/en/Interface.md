# Interface

[简体中文](../zh-CN/Interface.md) · [Home / 首页](Home.md)

Each native window owns its workspace. The title/menu area and file toolbar provide file opening, reload, encoding and follow controls. A horizontal file-tab strip selects documents; path breadcrumbs reveal the current file location. The empty workspace offers opening, recent/pinned files and previous-session entries.

The document area contains the virtual log view and a resizable search panel. Log rows have separate mark, line-number and text areas. Current-file results retain source-line numbers; global/directory results are grouped by file. Quick find belongs to the log or result region you actually focused.

The bottom status area contains the unified search-tab strip and selection/status information. File searches, global searches and directory searches share this strip. Its collapse control is independent of the search panel's visibility. Editing hides the search panel.

Settings are a separate, process-wide native window, not a workspace sidebar or modal sheet. The current workspace has no AI panel or left/right sidebars. See [UI layout](UI-Layout.md) for stable control IDs and [settings](Settings.md) for categories.

Tab renaming changes the displayed title, not the disk filename. File tabs can be reordered, copied or moved across windows. Copying captures a state snapshot; moving only closes the source when transfer succeeds and the source state is still unchanged.
