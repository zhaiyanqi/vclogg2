# Marks and highlighting

[简体中文](../zh-CN/Marks-and-Highlighting.md) · [Home / 首页](Home.md)

| Feature | Meaning |
| --- | --- |
| Row mark (M) | Adds source rows to the marks set used by result modes |
| Text tag (N) | Adds a styled annotation to one source line; also marks that row |
| Color label (Primary+D) | Creates text-color rules from selected text or rows |
| Search / quick-find highlight | Shows query matches or the current local-find target |

M toggles the selected rows as a group: if all are marked, remove the marks; otherwise mark the unmarked rows. The mark column remains visible independently of line numbers. Decorative dots are not copied into source text.

Text tags can be edited and dragged without changing the log. Removing tags, clearing tag reuse history and unmarking rows are separate operations. A changed source-line digest hides an old annotation while retaining its record. See [text tags](Row-Tags.md).

Settings → Highlight configures color labels, log-coloring groups, match styles and selection styles. Primary+D prefers the actual text selection; otherwise it uses selected nonempty rows. File color rules persist with the session. Changing a label definition updates references across windows.

Coloring groups include editable presets for Android Logcat, Java/Spring, Python, Nginx errors and HTTP access logs. Selecting a group for editing is distinct from activating it. Regex coloring may use the named `highlight` capture to restrict the styled span. Search highlights, local find, labels and selection styles compose according to their configured precedence; they do not alter exported log text.
