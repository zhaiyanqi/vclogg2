# Export and time merge

[简体中文](../zh-CN/Result-Export.md) · [Home / 首页](Home.md)

Export captures the **entire committed result set at the moment of the command**, not only visible rows. A later search does not change that captured set. If a result limit truncated the search, only that truncated set is exported.

| Command | Output |
| --- | --- |
| Save… | Writes to a chosen file |
| Open in new tab | Creates an application-owned temporary result document |
| Merge by time | Stably merges cross-file results into a new tab |

Single-file output follows result order. Ordinary global output groups nonempty results by file and includes full-path group headings. Time merge has no file headings and recognizes line-start `MM-DD HH:mm:ss` followed by 1–6 fractional digits; `T` may replace the date/time space. Example: `09-08T14:25:03.120045`.

Untimestamped continuation lines inherit the preceding timestamp in that file; initial continuations may look ahead for its first timestamp. Equal timestamps use file order and source-line order. This is not a general timezone/JSON/cross-year normalization engine.

Canceling the save dialog creates nothing. Saving writes and syncs a unique temporary file in the destination directory before replacing the target; failures retain the existing destination. Display colors, marks and tags are not added to log text.

Temporary results use unique `vclogg2-search-*` directories and do not enter recent-file/session startup recovery. Explicitly save content needed permanently. History cleanup can trash eligible application-owned temporary results, protecting files still used by other windows.
