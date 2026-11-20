# Follow a growing log

[简体中文](../zh-CN/Live-Logs.md) · [Guide home](Home.md)

Open the file your service is writing and keep its tab active in the foreground window. Once indexing finishes, VCLogg2 checks for changes. **View → Follow end** decides where you read after new lines arrive.

| What you want to do | Action | When the file grows |
| --- | --- | --- |
| Watch the newest line | Turn on **Follow end** | The log scrolls to the bottom after a refresh |
| Read an earlier error | Turn following off and return to the error | The active log and relevant search results refresh while the log keeps your reading position |
| Look at a different file | Switch file tabs | The newly active file is checked. Switching back checks the original file again. |

## Watch one line arrive

Save the [five-line sample](examples/checkout.txt) as `checkout.txt` in a working folder, open it in VCLogg2 and enable following. Open a terminal in that folder. Run the command for your shell to append line 6 to this **sample copy**.

**Windows PowerShell**

```powershell
Add-Content -LiteralPath .\checkout.txt -Value '09:14:07 INFO  id=42 retry finished'
```

**macOS or Linux shell**

```sh
printf '%s\n' '09:14:07 INFO  id=42 retry finished' >> checkout.txt
```

Return to the VCLogg2 window. It checks the active file and the sixth line should appear at the end. If you have already run `retry|timeout` on this file with no line or match limits, the matching source lines are now 2, 4 and 6.

## If the new line is missing

Check the path against the one used by the writer, then bring the window and file tab to the foreground. **F5** reloads the whole file and can help after rotation or changes to older text. A directory search uses a snapshot. Run it again to discover newly created files. [Directory search](Search-and-Filters.md)

<details>
<summary>Refresh timing, file monitoring and large-file behavior</summary>

Normal events coalesce over 400 ms. The active document is rechecked every 30 seconds to recover missed notifications; failure paths schedule retries, commonly after 5 seconds. These are scheduling intervals, not an end-to-end latency guarantee. Opening/search work can defer refresh. Leaving the tab or deactivating the window releases monitoring and cancels pending publication.

UTF-8/BOM growth can reuse the index and prior search prefix after checking identity and head/tail blocks. The old tail and appended data are rescanned, handling partial lines and CRLF boundaries. Truncation, replacement, other encodings, initial growth of an empty file or failed validation trigger a rebuild. Old content remains visible until the replacement is ready.

**Limitation:** rewriting an unsampled middle region while also appending may escape the fast head/tail check. **F5** always rebuilds fully. A fast snapshot is not proof of full-file equivalence; full searches retain their own source checks. Directory searches do not continuously discover new files.

Text selections are tracked by source-line identity and retained when their text still matches, including result regrouping. Changed text or removed results invalidate the corresponding selection. Collapsed groups retain selection metadata but hidden rows do not participate in visible text copying.

Source: [file_watch.rs](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-app/src/file_watch.rs), [document_lifecycle.rs](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-app/src/workspace/document_lifecycle.rs), [document.rs](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-core/src/document.rs).

</details>
