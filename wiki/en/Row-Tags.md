# Text tags

[简体中文](../zh-CN/Row-Tags.md) · [Home / 首页](Home.md)

Select one source row and press **N**, or use its context menu → Text tags → Add. Group headers are not source rows. The target must belong to an opened, ready document. Keyboard creation starts at the text origin and defaults to vertical centering.

The add/edit dialog owns a draft with text, font, size, height, foreground/background colors, transparency, bold, rounded shape and vertical centering. Text must contain 1–128 characters without newlines. Confirming saves; cancel/Esc discards the draft. Generated “mark 1” names increment across windows and languages. New tags reuse the last confirmed style, while vertical centering starts enabled.

A fixed log-line preview sits below the scrolling form, with a searchable history list to its right. Preview dragging only changes the preview position; saving an existing tag retains its real source position, and a new tag retains the original placement target. Tag height is clamped to the base log line height, including when wrapping is enabled; rendering clamps without destroying the saved preference.

Transparency ranges from 0% opaque to 100% transparent and combines with color alpha. Vertical centering allows horizontal dragging while retaining the old vertical coordinate for later use when centering is disabled. Escape during dragging or window deactivation cancels that drag.

History reuses complete styles and deduplicates by exact case-sensitive text, newest-first. Deleting a history entry affects reuse only, not existing annotations or the current draft; successful deletion is immediate and is not undone by canceling the dialog. Every added tag is an independent instance.

The row context menu can clear all text tags belonging to that file, including invisible rows, while retaining ordinary row marks and reuse history. Tags persist separately from source text and are excluded from search, copy and exports. Source-line digest validation prevents attaching an old tag to changed content.
