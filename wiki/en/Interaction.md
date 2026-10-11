# Interaction and focus

[简体中文](../zh-CN/Interaction.md) · [Home / 首页](Home.md)

Log text, current-file results and global results have separate focus, selection and viewport state. Commands act on the region actually focused, not merely the current search scope. Clicking a result reveals its source line while retaining the initiating region's command context.

Copy prefers the real text selection; otherwise it copies selected rows. Primary+Shift+C always includes source-line numbers. Primary+A selects the focused log region through compressed ranges; file-group headings do not count as log rows. M toggles marks across the selected source rows. N requires one source row. Cross-file color labeling is rejected rather than assigning a rule to the wrong document.

Double-click selects a word; whole-row and text selection remain distinct. Wrapping, font changes and result regrouping anchor to source identity rather than transient row indices. Follow tail can be disabled by deliberate navigation back into older content.

File-tab rename is a display-only change. Multiwindow copies use immutable state snapshots; a move closes its source only after successful installation with unchanged source state. Failed or changed-source transfers retain the source. Settings preview across windows and roll back uncommitted changes on cancel; immediate history cleanup is independently committed.

Input controls retain their typing and copy/paste semantics. See [shortcuts](Keyboard-Shortcuts.md), [search tabs](Search-and-Filters.md) and [editing](Editing.md).
