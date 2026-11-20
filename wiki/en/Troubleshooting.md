# Troubleshooting

[简体中文](../zh-CN/Troubleshooting.md) · [Guide home](Home.md)

## Search returns fewer lines than expected

Start with a known result. Open the [five-line sample](examples/checkout.txt), choose **Current**, press **Ctrl+F / ⌘F** to focus the main query field and turn regex off. Enter `id=42`, press **Enter** and select **Matches only**. With the whole file in scope and the per-file limit at 0, you should see **source lines 1 through 5**.

| What do you see? | What to check next |
| --- | --- |
| The previous query’s results | Check whether you submitted the new query or see a regex error. The last results remain available while a new search is queued or running. |
| Nothing, or just rows with a dot | Select **Matches only** and check for stray spaces. An empty query does not list the whole file. If quick find is open, close it with Esc and submit in the main query field. |
| Exactly lines 2, 3 and 4 | Right-click a source row, select **Clear search limits** and submit again. [Compare line boundaries](Search-and-Filters.md#search-only-the-lines-around-an-incident) |
| Only the first few hits from each file | Check the per-file limit under **Settings → Search**. Set it to **0** and submit again to see all matches. |
| The sample works but a file is missing from your investigation | **Global** uses the participating files. In **Directory**, check the path, file types and subdirectories. The two-service exercise should give you two hits per file. [Try the directory search](Search-and-Filters.md#find-a-request-in-a-folder-on-disk) |

In the main search, literal `retry|timeout` finds either word. In quick find the same input looks for the words joined by an actual vertical bar.

## Reading, editing and settings

| Symptom | What to do |
| --- | --- |
| The file grows but the view stays put | Bring its window and tab to the foreground, wait for indexing and turn on **View → Follow end**. After a rotation or rewrite in the middle of a file, use **F5** for a full reload. Network drives can take longer to report events. [Watch a sample line arrive](Live-Logs.md#watch-one-line-arrive) |
| Characters or line endings look wrong | Inspect the detected encoding in **Encoding** and choose **Detect and reload**. Binary mode shows 16 bytes per row. For a report, record the encoding, BOM and line endings. |
| An edit will not save | If another process changed the file, copy the edits you want to keep and reopen the file. For text the original encoding cannot represent, paste into a new UTF-8 file. [Editing and saving](Editing.md) |
| A shortcut does nothing | Click the intended log pane, then check custom bindings and conflicts in Settings. On macOS check function-key behavior. **M** marks rows, **N** needs one source row, and **W** toggles wrap. [Keyboard shortcuts](Keyboard-Shortcuts.md) |
| A settings change disappears | Correct the invalid value or key binding flagged in Settings. Valid changes save automatically. If writing fails, choose **Retry**. [Settings](Settings.md) |
| A recent file will not disappear from history | Check whether it is open, pinned, marked or annotated. These states protect the history record. [Local data](Data-and-Privacy.md) |

## Startup, network and updates

| Symptom | What to do |
| --- | --- |
| Windows reports a data-path error | Check `vclogg2-data-dir.txt` beside the executable. If present, it must name an absolute, writable directory. [Data locations](Data-and-Privacy.md) |
| A new build looks like the old one | Check the version and build time in **About** and the executable path. Starting another copy with the same data root can route the request to the existing process. Quit it before launching the intended build. |
| The operating system questions a package | Use the appropriate project release and inspect its origin and signing information. The current macOS package lacks Developer ID notarization. Windows signing depends on the release configuration. [Choose a package](Installation.md) |
| Building from source fails | Check the stable Rust toolchain, the platform prerequisites and Git dependency connectivity. Build at the repository root with the lockfile. [Build instructions](Building.md) |
| Cloud registration or sharing fails | Check the server address, connectivity and registration policy. Sharing needs a server with filter-branch support. Cached pages are limited to earlier visits. [Connection and sharing](Cloud-Filters.md#if-the-connection-or-sharing-fails) |
| Checking for updates opens the browser | A manual check contacts the manifest service. A failed check shows a valid cached result and its check time, or opens Releases when there is no usable cache. If neither site opens, check DNS, proxy and certificate settings. |

## Report a reproducible problem

Include the version, OS and architecture, file size and encoding, the steps you took, and the expected and actual source lines. For example, “I searched the five-line sample for `id=42` in Current scope, with limits cleared and a per-file limit of 0. I expected lines 1–5 and saw only line 1.” Replace those details with your observations. [Open a GitHub issue](https://github.com/zhaiyanqi/vclogg2/issues)

For timing problems, include elapsed time, line count, wrapping, search scope and match limit. Export application logs from **Settings → Advanced** when relevant. Review sample text, paths, logs and crash reports before sharing. Keep state databases, cloud session files, cookies and signing credentials private. [Performance recording](Architecture.md) · [Historical measurements](Archive.md)
