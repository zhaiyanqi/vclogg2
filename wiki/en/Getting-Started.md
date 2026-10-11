# Getting started

[简体中文](../zh-CN/Getting-Started.md) · [Home / 首页](Home.md)

1. Open a log with **Ctrl+O / ⌘O**, the toolbar button, a recent-file entry or file drag-and-drop. Indexing runs in the background.
2. Read the main log area. Click a source line to select it; double-click selects a word. Use **Ctrl+G / ⌘G** to jump to a source line.
3. Focus search with **Ctrl+F / ⌘F**. Enter `error|timeout|retry` for literal alternatives, then press Enter. Turn on regex explicitly for regular expressions.
4. Select a result to reveal its source context. The result row number identifies a source line, not its position in the filtered list.
5. Press **M** in a log region to mark selected rows; **N** adds a text tag to one source line. Use **Save…** in results to export the committed set.
6. Enable **Follow tail** to stay at the newest line of the actively monitored file. Scrolling back can turn following off.

File tabs and search tabs have different responsibilities: one file can have several independent queries. The search-tab **+** menu creates current-file, global or directory searches. Settings open with **Ctrl+, / ⌘,**.

Normal reading does not change source files. **I** enters explicit editing; see [editing](Editing.md) before saving. For details continue with [search](Search-and-Filters.md), [interface](Interface.md), and [shortcuts](Keyboard-Shortcuts.md).
