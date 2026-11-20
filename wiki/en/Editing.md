# Edit a file, a clipboard snippet or a new draft

[简体中文](../zh-CN/Editing.md) · [Guide home](Home.md)

Check the path of the active tab first. Where **Ctrl+S / ⌘S** writes depends on how the editor was opened.

| What you have | How to start | Save destination |
| --- | --- | --- |
| An existing file | Press **I** in the log or choose **Edit → Edit file** | The source file at its current path |
| Text from a ticket for a quick investigation | **File → Open clipboard as temporary file…** | The log created in the application’s temporary directory |
| Text you want to keep | Choose **Create new file** in the empty workspace, then paste into the editor | A new path you choose on first save |

## Search a snippet copied from a ticket

1. Copy the text and press **Ctrl+V / ⌘V** outside an input in VCLogg2. If paste confirmation is enabled, choose **Create and edit**. The text opens in a temporary document’s editor.
2. Make and save any corrections. Choose **Edit → Exit edit mode** to return to reading. You can now search with **Ctrl+F / ⌘F**.
3. To keep a named copy, copy the edited text, choose **Create new file** in an empty workspace, paste and press **Ctrl+S / ⌘S**. Pick an unused filename such as `ticket-42.txt`. You can reopen it from that path later.

## When does the source file change?

Saving in the editor for an existing file replaces the file on disk. To record an observation, select a row and press **N** for a text tag. The original line is still available for copying and exporting. [Leave a note on a line](Row-Tags.md)

If edits are pending when you exit edit mode or close the tab, **Save** writes them first, **Don't Save** discards them and **Cancel** returns you to the editor. The first save of a new draft requires a new path. If the selected filename already exists, pick another name.

## If a save is refused or editing is slow

- **File changed on disk.** Saves check size and modification time. Copy the edits you want to keep, reopen the latest file and reconcile your changes.
- **Invalid encoding or characters that cannot be saved.** Existing files keep their encoding, including supported BOMs and legacy encodings. New characters must be representable in that encoding. To keep a separate UTF-8 copy, paste into a new file.
- **A large file takes time to enter edit mode.** Editing decodes the entire file, so memory needs depend on its size. You can search, annotate and export a subset in reading mode. [Export search results](Result-Export.md)
