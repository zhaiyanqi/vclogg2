# Search and filters

[简体中文](../zh-CN/Search-and-Filters.md) · [Home / 首页](Home.md)

## Scope and search tabs

| Scope | Source |
| --- | --- |
| Current | Active document, results retain source-line identity |
| Global | Selected open files, grouped by file |
| Directory | A configured disk directory, extensions, recursion and hidden-directory options |

Directory results are execution-time snapshots, not continuous recursive monitoring. Use the scope configuration controls to choose participating paths.

The bottom search-tab strip holds current-file, global and directory searches in a single order. Its **+** menu creates a search and focuses an empty query. Current-file search titles default to a filename and number; global/directory titles include local creation time. Rename by double-click or context menu, close by button/middle-click/menu, and drag to reorder. Closing the last search tab of a file invokes that file's close flow; global/directory searches may be closed to zero.

Each search tab keeps its query draft, last completed query, results, result mode, selection, collapsed groups, viewport and per-file source-line limits. File annotations are shared. Case/regex preferences synchronize, but already committed results retain their execution options until searched again.

One search executes per window and additional submissions queue. Resubmitting the same tab supersedes its old work. Switching tabs does not cancel a search; stopping affects the current tab. Completed background searches do not steal focus. Results publish as a complete validated snapshot, not partial previews.

## Syntax

Literal mode splits alternatives on `|`:

```text
error|timeout|retry
```

Leading, trailing and internal spaces are significant. Other regex punctuation is literal unless regex is enabled. In regex mode, for example `ERROR.*request_id=[0-9]+`, matching is line-oriented and uses Rust regex semantics: backreferences and look-around are unsupported. Invalid expressions report an error without replacing the last valid result.

Enter or the search button submits. Settings → Search sets a per-file result limit: **0 means unlimited**; nonzero limits retain the first N source-order matches for each file. Export includes only the committed result, including any truncation.

| Result mode | Rows |
| --- | --- |
| Marks and matches | Union, deduplicated by source line |
| Matches only | Query matches |
| Marks only | Marked lines |

An empty query is not “match all”; it can display marks according to result mode in any scope.

## Source-line limits

A log/result row context menu can set a start or end source line, or clear limits. Both ends are inclusive; an unset end follows EOF, and a reversed range is empty. Limits belong to the current search tab and file. Out-of-range text dims immediately, while old results remain until a new search completes. A group header can clear limits but cannot supply a row boundary.

## Quick find and reusable filters

Click the intended log/result region, then press **Primary+Shift+F**. Quick find has separate case, whole-word and regex options; literal quick find treats `|` as a literal character. Enter/Shift+Enter moves next/previous; Esc closes.

Search history is persisted, deduplicated exactly and ordered newest-first. Multiline query input and autocomplete support filter names, pinyin and initials. Predefined filters add complete query segments; regex filters can enable regex mode. Filters use UUID identity; identical contents with different UUIDs may coexist. Export format is v5, with older JSON/array and Qt CONF/INI imports supported. See [cloud filters](Cloud-Filters.md) for revision merging.
