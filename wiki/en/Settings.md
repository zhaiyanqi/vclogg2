# Adjust the workspace for reading

[简体中文](../zh-CN/Settings.md) · [Guide home](Home.md)

## Give dense logs more space

Open a log and leave a few results visible. Choose **Tools → Settings → Appearance** (or open Settings with **Ctrl+, / ⌘,**). Change **Log font size** to make the characters larger. Adjust the extra line spacing to separate adjacent records. Watch the same lines in the workspace as you adjust them.

![Reading example showing the same two records with tighter and wider line spacing](images/line-spacing.svg)

Valid changes apply across windows and save automatically. Close Settings with **Esc** or **Ctrl+W / ⌘W**. To undo an appearance change, set the value back. Font and line spacing affect both original text and results. Use **W** in the log when a long line runs off the right edge.

## Where to find a setting

| I want to… | Category |
| --- | --- |
| Change language, path display, close confirmation, wheel behavior or word boundaries | **General** |
| Set a team server address and test the connection | **Network**. Use **Save and test** to connect. [Cloud filters](Cloud-Filters.md) |
| Change theme, log font, size, line spacing, line numbers or search control size | **Appearance** |
| Color log levels or change selection, keyword and label colors | **Highlight**. [Highlighting guide](Marks-and-Highlighting.md) |
| Use case sensitivity or regex, or change the maximum number of results per file | **Search**. Set the limit to **0** for unlimited matches. |
| Find a recent file or manage temporary results and previous searches | **History** |
| Inspect the state database and file-index cache or free space | **Storage** |
| Record, clear or reset a key binding | **Shortcuts**. Conflicting combinations are flagged. [Shortcut reference](Keyboard-Shortcuts.md) |
| Export or clear application diagnostics, choose an icon, or enable the Windows Explorer file menu | **Advanced** |
| Check the version and build information | **About** |

## Starting values

A new configuration uses Chinese and the light theme. Log text defaults to Consolas at 13 px with 6 px of extra line spacing. The line-number column starts at 60 px. Fonts are chosen from those available on the system, with a fallback if a saved font is missing. Existing file sessions can retain their own wrapping, separators and line-number state.

Search starts with case and regex off, match highlighting on and log-level coloring off. The toolbar and search field share a default control height of 30 px. Their font sizes can be set independently, and the controls grow to fit larger text.

**View → Horizontal scrollbar** controls scrollbar visibility across windows. Horizontal wheel navigation still works with it hidden. The notification switch is also in **View**. Messages from the current run remain available in the notification panel.
