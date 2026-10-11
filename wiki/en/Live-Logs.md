# Live logs

[简体中文](../zh-CN/Live-Logs.md) · [Home / 首页](Home.md)

Automatic monitoring applies only to the foreground window's active **Ready** document. Inactive tabs and unfocused windows keep their previous snapshots. Activating a window/tab checks immediately and watches the file's parent directory through `notify`.

**Follow tail** controls scrolling, not change detection. With it off, the active file and relevant results still refresh while preserving reading position. With it on, a successfully installed snapshot reveals the last line; navigating back or selecting old context can turn following off. Empty files can enable it before the first data arrives.

Normal events coalesce over 400 ms. The active document is rechecked every 30 seconds to recover missed notifications; failure paths schedule retries, commonly after 5 seconds. These are scheduling intervals, not an end-to-end latency guarantee. Opening/search work can defer refresh. Leaving the tab or deactivating the window releases monitoring and cancels pending publication.

UTF-8/BOM growth can reuse the index and prior search prefix after checking identity and head/tail blocks. The old tail and appended data are rescanned, handling partial lines and CRLF boundaries. Truncation, replacement, other encodings, initial growth of an empty file or failed validation trigger a rebuild. Old content remains visible until the replacement is ready.

**Limitation:** rewriting an unsampled middle region while also appending may escape the fast head/tail check. **F5** always rebuilds fully. A fast snapshot is not proof of full-file equivalence; full searches retain their own source checks. Directory searches do not continuously discover new files.

Text selections are tracked by source-line identity and retained when their text still matches, including result regrouping. Changed text or removed results invalidate the corresponding selection. Collapsed groups retain selection metadata but hidden rows do not participate in visible text copying.

Source: [file_watch.rs](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-app/src/file_watch.rs), [document_lifecycle.rs](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-app/src/workspace/document_lifecycle.rs), [document.rs](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-core/src/document.rs).
