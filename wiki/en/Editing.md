# File editing and clipboard documents

[简体中文](../zh-CN/Editing.md) · [Home / 首页](Home.md)

Normal viewing, searches, marks and text tags do not alter source text. **Entering edit mode and saving does replace the file.**

Press **I** outside an input to edit the active document. There is no fixed file-size limit for entering edit mode; practical capacity depends on available memory. The editor decodes the full file, unlike virtual log browsing. Unsupported or invalid encodings are rejected. UTF-8/BOM, UTF-16 BOM and supported legacy encodings are preserved; text that cannot be encoded back is rejected.

Use **Ctrl+S / ⌘S** in the editor to save. Existing-file saves compare size and modification time before writing and again before replacement, write and sync a temporary file in the same directory, preserve permissions and replace the destination. This is not a filesystem lock or a content-hash concurrency guarantee: avoid editing a file another process is actively writing. A detected change requires reopening before editing.

New-file drafts use a save dialog. Their first save refuses to overwrite an existing file. Unsaved changes use the editor's save/discard/cancel flow when leaving; do not assume closing a draft saves it.

**Ctrl+V / ⌘V** outside an input opens clipboard text as an application-owned temporary document in edit mode, subject to the paste confirmation setting. Input controls retain ordinary paste. Keep a deliberate saved copy if you need the content permanently. The search panel is hidden during editing.

Source: [document_editing.rs](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-app/src/workspace/document_editing.rs), [clipboard_paste.rs](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-app/src/workspace/clipboard_paste.rs), [actions.rs](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-app/src/actions.rs).
