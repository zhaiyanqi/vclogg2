use std::io::Write as _;

use gpui_kit::base::input::RopeExt as _;

use super::*;

pub(super) const MAX_EDIT_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Clone, Copy)]
enum EditExitTarget {
    Document(u64),
    NewFile(u64),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct DiskVersion {
    len: u64,
    modified: Option<SystemTime>,
}

impl DiskVersion {
    fn read(path: &Path) -> anyhow::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}

#[derive(Clone, Copy)]
pub(super) enum EditEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Legacy(&'static encoding_rs::Encoding),
}

impl EditEncoding {
    fn from_name(name: &str) -> anyhow::Result<Self> {
        match name {
            "UTF-8" => Ok(Self::Utf8),
            "UTF-8 BOM" => Ok(Self::Utf8Bom),
            "UTF-16LE BOM" => Ok(Self::Utf16Le),
            "UTF-16BE BOM" => Ok(Self::Utf16Be),
            _ => encoding_rs::Encoding::for_label(name.as_bytes())
                .map(Self::Legacy)
                .ok_or_else(|| {
                    anyhow::anyhow!(crate::tr!(
                        "此文件编码不支持安全编辑",
                        "This file encoding cannot be edited safely"
                    ))
                }),
        }
    }

    fn decode(self, bytes: &[u8]) -> anyhow::Result<String> {
        let invalid = || {
            anyhow::anyhow!(crate::tr!(
                "文件包含无法按当前编码解码的内容",
                "The file contains text invalid in its detected encoding"
            ))
        };
        match self {
            Self::Utf8 => String::from_utf8(bytes.to_vec()).map_err(|_| invalid()),
            Self::Utf8Bom => String::from_utf8(
                bytes
                    .strip_prefix(&[0xef, 0xbb, 0xbf])
                    .unwrap_or(bytes)
                    .to_vec(),
            )
            .map_err(|_| invalid()),
            Self::Utf16Le | Self::Utf16Be => {
                let bytes = bytes
                    .strip_prefix(if matches!(self, Self::Utf16Le) {
                        &[0xff, 0xfe]
                    } else {
                        &[0xfe, 0xff]
                    })
                    .unwrap_or(bytes);
                if bytes.len() % 2 != 0 {
                    return Err(invalid());
                }
                let encoding = if matches!(self, Self::Utf16Le) {
                    encoding_rs::UTF_16LE
                } else {
                    encoding_rs::UTF_16BE
                };
                let (text, had_errors) = encoding.decode_without_bom_handling(bytes);
                anyhow::ensure!(!had_errors, "{}", invalid());
                Ok(text.into_owned())
            }
            Self::Legacy(encoding) => {
                let (text, had_errors) = encoding.decode_without_bom_handling(bytes);
                anyhow::ensure!(!had_errors, "{}", invalid());
                Ok(text.into_owned())
            }
        }
    }

    fn encode(self, text: &str) -> anyhow::Result<Vec<u8>> {
        match self {
            Self::Utf8 => Ok(text.as_bytes().to_vec()),
            Self::Utf8Bom => Ok([&[0xef, 0xbb, 0xbf][..], text.as_bytes()].concat()),
            Self::Utf16Le | Self::Utf16Be => {
                let mut bytes = if matches!(self, Self::Utf16Le) {
                    vec![0xff, 0xfe]
                } else {
                    vec![0xfe, 0xff]
                };
                for unit in text.encode_utf16() {
                    let pair = if matches!(self, Self::Utf16Le) {
                        unit.to_le_bytes()
                    } else {
                        unit.to_be_bytes()
                    };
                    bytes.extend_from_slice(&pair);
                }
                Ok(bytes)
            }
            Self::Legacy(encoding) => {
                let (bytes, _, had_errors) = encoding.encode(text);
                anyhow::ensure!(
                    !had_errors,
                    crate::tr!(
                        "文本包含当前文件编码无法保存的字符",
                        "Some characters cannot be saved in this file encoding"
                    )
                );
                Ok(bytes.into_owned())
            }
        }
    }
}

fn editor_source_line_range(editor: &EditorState, source_row: usize) -> std::ops::Range<usize> {
    let text = editor.text();
    let start = text.line_start_offset(source_row);
    let mut end = text.line_end_offset(source_row);
    if text.slice_line(source_row).chars().last() == Some('\r') {
        end -= 1;
    }
    start..end
}

fn select_editor_source_line(
    editor: &mut EditorState,
    source_row: usize,
    window: &mut Window,
    cx: &mut Context<EditorState>,
) {
    let range = editor_source_line_range(editor, source_row);
    editor.set_cursor_position(Position::new(source_row as u32, 0), window, cx);
    editor.set_selected_range(range, cx);
}

fn position_editor_source_line(
    editor: &mut EditorState,
    source_row: usize,
    anchor_viewport_y: Option<Pixels>,
    fallback_line_height: Pixels,
    cx: &mut Context<EditorState>,
) {
    let Some(viewport_y) = anchor_viewport_y else {
        return;
    };
    let line_height = editor.line_height().unwrap_or(fallback_line_height);
    let offset_y = (viewport_y - line_height * source_row).min(px(0.));
    editor.set_scroll_offset(point(editor.scroll_offset().x, offset_y), cx);
}

fn align_editor_source_line(
    editor: &mut EditorState,
    source_row: usize,
    screen_y: Pixels,
    cx: &mut Context<EditorState>,
) -> bool {
    let range = editor_source_line_range(editor, source_row);
    let Some(bounds) = editor.range_to_bounds(&(range.start..range.start)) else {
        return false;
    };
    let offset = editor.scroll_offset();
    editor.set_scroll_offset(point(offset.x, offset.y + screen_y - bounds.top()), cx);
    true
}

#[derive(Clone, Copy)]
struct EditorEntryAnchor {
    viewport_y: Pixels,
    screen_y: Pixels,
}

#[derive(Clone, Copy)]
struct EditorEntryReveal {
    document_id: u64,
    source_row: usize,
    anchor: Option<EditorEntryAnchor>,
    remaining_frames: u8,
}

impl Workspace {
    pub(super) fn enter_document_edit_action(
        &mut self,
        _: &EnterEditMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) || window.has_active_sheet(cx) {
            return;
        }
        let Some(tab) = self.active_document() else {
            return;
        };
        if tab.document.metadata().file_size > MAX_EDIT_BYTES {
            window.notify_message(
                crate::tr!(
                    "仅支持编辑不超过 100 MB 的文件",
                    "Only files up to 100 MB can be edited"
                ),
                cx,
            );
            return;
        }
        self.enter_document_edit(tab.id, window, cx);
    }

    pub(super) fn enter_document_edit(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.documents.iter().position(|tab| tab.id == document_id) else {
            return;
        };
        self.activate_workspace_tab(WorkspaceTabId::Document(document_id), window, cx);
        let tab = &self.documents[ix];
        let path = tab.document.path().to_path_buf();
        let encoding_name = tab.document.metadata().encoding_name.clone();
        let source_row = {
            let selected_table = match tab.view.selection_table {
                SelectionTable::Log => &tab.log_table,
                SelectionTable::Results => &tab.result_table,
            };
            let selected = selected_table.read(cx);
            let selected_row = selected
                .active_log_row()
                .and_then(|row| selected.delegate().source_row(row));
            let global_row = (self.active_log_region == LogRegion::GlobalResults)
                .then(|| {
                    let global = self.global_table.read(cx);
                    global
                        .active_log_row()
                        .and_then(|row| global.delegate().row_key(row))
                })
                .flatten()
                .and_then(|key| match key {
                    LogRowKey::Row {
                        document_id: selected_document_id,
                        source_row,
                    } if selected_document_id == document_id => Some(source_row),
                    _ => None,
                });
            let body = tab.log_table.read(cx);
            let row_count = body.delegate().row_count();
            global_row
                .or(selected_row)
                .or_else(|| {
                    body.active_log_row()
                        .and_then(|row| body.delegate().source_row(row))
                })
                .or_else(|| {
                    (row_count > 0)
                        .then(|| {
                            tab.log_viewport
                                .first_visible(row_count, self.log_row_height())
                        })
                        .and_then(|row| body.delegate().source_row(row))
                })
                .unwrap_or(0)
        };
        let anchor_position = if self.active_log_region == LogRegion::GlobalResults {
            let table = self.global_table.read(cx);
            let row_ix = table.delegate().row_ix_for_key(LogRowKey::Row {
                document_id,
                source_row,
            });
            row_ix.and_then(|row_ix| {
                self.global_viewport
                    .capture_viewport_position(
                        table.delegate().rows_len(),
                        Some(row_ix),
                        self.log_row_height(),
                    )
                    .filter(|position| position.row_ix == row_ix)
                    .map(|position| EditorEntryAnchor {
                        viewport_y: position.viewport_y,
                        screen_y: self.global_viewport.viewport().viewport_bounds().top()
                            + position.viewport_y,
                    })
            })
        } else {
            let (table, viewport) = match tab.view.selection_table {
                SelectionTable::Log => (&tab.log_table, &tab.log_viewport),
                SelectionTable::Results => (&tab.result_table, &tab.result_viewport),
            };
            let table = table.read(cx);
            let row_ix = table.delegate().row_ix_for_key(LogRowKey::Row {
                document_id,
                source_row,
            });
            row_ix.and_then(|row_ix| {
                viewport
                    .capture_viewport_position(
                        table.delegate().row_count(),
                        Some(row_ix),
                        self.log_row_height(),
                    )
                    .filter(|position| position.row_ix == row_ix)
                    .map(|position| EditorEntryAnchor {
                        viewport_y: position.viewport_y,
                        screen_y: viewport.viewport().viewport_bounds().top() + position.viewport_y,
                    })
            })
        };
        let fallback_line_height = self.log_row_height();
        if let Some(edit) = self.documents[ix].edit.as_mut() {
            edit.active = true;
            let editor = edit.editor.clone();
            edit.editor.update(cx, |editor, cx| {
                select_editor_source_line(editor, source_row, window, cx);
                position_editor_source_line(
                    editor,
                    source_row,
                    anchor_position.map(|anchor| anchor.viewport_y),
                    fallback_line_height,
                    cx,
                );
            });
            edit.editor.focus_handle(cx).focus(window, cx);
            self.schedule_editor_entry_reveal(
                editor,
                EditorEntryReveal {
                    document_id,
                    source_row,
                    anchor: anchor_position,
                    remaining_frames: 4,
                },
                window,
                cx,
            );
            cx.notify();
            return;
        }
        if self.documents[ix].edit_load_task.is_some() {
            return;
        }
        self.documents[ix].view.auto_follow = false;
        // Let the loading surface paint before the foreground editor builds its rope and
        // first layout. Cached reads can otherwise finish in the same frame as the click.
        let (painted, ready) = async_channel::bounded(1);
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |_, _| {
                _ = painted.try_send(());
            });
        });
        self.documents[ix].edit_load_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let metadata = std::fs::metadata(&path)?;
                    let disk_version = DiskVersion {
                        len: metadata.len(),
                        modified: metadata.modified().ok(),
                    };
                    anyhow::ensure!(
                        metadata.len() <= MAX_EDIT_BYTES,
                        crate::tr!(
                            "仅支持编辑不超过 100 MB 的文件",
                            "Only files up to 100 MB can be edited"
                        )
                    );
                    let bytes = std::fs::read(&path)?;
                    anyhow::ensure!(
                        bytes.len() as u64 <= MAX_EDIT_BYTES,
                        crate::tr!(
                            "仅支持编辑不超过 100 MB 的文件",
                            "Only files up to 100 MB can be edited"
                        )
                    );
                    let encoding = EditEncoding::from_name(&encoding_name)?;
                    let text = encoding.decode(&bytes)?;
                    Ok::<_, anyhow::Error>((text, encoding, disk_version))
                })
                .await;
            if ready.recv().await.is_err() {
                return;
            }
            _ = this.update_in(cx, |this, window, cx| {
                let Some(ix) = this.documents.iter().position(|tab| tab.id == document_id) else {
                    return;
                };
                this.documents[ix].edit_load_task = None;
                match result {
                    Ok((text, encoding, disk_version)) => {
                        // Resizing a wrapped editor recomputes wrapping for every line. Logs
                        // commonly have hundreds of thousands of lines, so keep width changes
                        // local to the viewport and allow horizontal scrolling while editing.
                        let editor = cx.new(|cx| {
                            EditorState::new(window, cx)
                                .soft_wrap(false)
                                .default_value(text)
                        });
                        editor.update(cx, |editor, cx| {
                            select_editor_source_line(editor, source_row, window, cx);
                            position_editor_source_line(
                                editor,
                                source_row,
                                anchor_position.map(|anchor| anchor.viewport_y),
                                fallback_line_height,
                                cx,
                            );
                        });
                        let subscription =
                            cx.subscribe(&editor, move |this, _, event: &InputEvent, cx| {
                                if matches!(event, InputEvent::Change)
                                    && let Some(edit) = this
                                        .documents
                                        .iter_mut()
                                        .find(|tab| tab.id == document_id)
                                        .and_then(|tab| tab.edit.as_mut())
                                {
                                    edit.dirty = true;
                                    cx.notify();
                                }
                            });
                        this.documents[ix].edit = Some(DocumentEditSession {
                            editor: editor.clone(),
                            encoding,
                            disk_version,
                            dirty: false,
                            active: true,
                            saving: false,
                            saved_since_enter: false,
                            pending_exit_row: None,
                            after_save: EditAfterSave::None,
                            save_task: None,
                            _subscription: subscription,
                        });
                        if this.active_tab_id == WorkspaceTabId::Document(document_id) {
                            editor.focus_handle(cx).focus(window, cx);
                            this.schedule_editor_entry_reveal(
                                editor,
                                EditorEntryReveal {
                                    document_id,
                                    source_row,
                                    anchor: anchor_position,
                                    remaining_frames: 4,
                                },
                                window,
                                cx,
                            );
                        }
                        cx.notify();
                    }
                    Err(error) => window.notify_message(
                        crate::tr_args!(
                            "无法进入编辑模式：{error}",
                            "Could not enter edit mode: {error}"
                        ),
                        cx,
                    ),
                }
            });
        }));
        cx.notify();
    }

    fn schedule_editor_entry_reveal(
        &self,
        editor: Entity<EditorState>,
        request: EditorEntryReveal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if request.remaining_frames == 0 {
            return;
        }
        cx.on_next_frame(window, move |this, window, cx| {
            let still_editing = this.active_tab_id == WorkspaceTabId::Document(request.document_id)
                && this
                    .documents
                    .iter()
                    .find(|tab| tab.id == request.document_id)
                    .and_then(|tab| tab.edit.as_ref())
                    .is_some_and(|edit| edit.active && edit.editor == editor);
            if !still_editing {
                return;
            }
            let expected_range = editor_source_line_range(editor.read(cx), request.source_row);
            if editor.read(cx).selected_range() != expected_range {
                return;
            }
            if let Some(anchor) = request.anchor {
                if let Some(line_height) = editor.read(cx).line_height() {
                    let aligned = editor.update(cx, |editor, cx| {
                        if align_editor_source_line(editor, request.source_row, anchor.screen_y, cx)
                        {
                            true
                        } else {
                            position_editor_source_line(
                                editor,
                                request.source_row,
                                Some(anchor.viewport_y),
                                line_height,
                                cx,
                            );
                            false
                        }
                    });
                    if !aligned {
                        this.schedule_editor_entry_reveal(
                            editor,
                            EditorEntryReveal {
                                remaining_frames: request.remaining_frames - 1,
                                ..request
                            },
                            window,
                            cx,
                        );
                    }
                } else {
                    this.schedule_editor_entry_reveal(
                        editor,
                        EditorEntryReveal {
                            remaining_frames: request.remaining_frames - 1,
                            ..request
                        },
                        window,
                        cx,
                    );
                }
                return;
            }
            let needs_reveal = editor
                .read(cx)
                .visible_row_range()
                .is_none_or(|range| !range.contains(&request.source_row));
            if !needs_reveal {
                return;
            }
            editor.update(cx, |editor, cx| {
                if editor.selected_range() == expected_range && editor.line_height().is_some() {
                    select_editor_source_line(editor, request.source_row, window, cx);
                }
            });
            if editor.read(cx).selected_range() == expected_range {
                this.schedule_editor_entry_reveal(
                    editor,
                    EditorEntryReveal {
                        remaining_frames: request.remaining_frames - 1,
                        ..request
                    },
                    window,
                    cx,
                );
            }
        });
    }

    pub(super) fn exit_document_edit(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(edit) = self
            .documents
            .iter_mut()
            .find(|tab| tab.id == document_id)
            .and_then(|tab| tab.edit.as_mut())
        {
            if edit.saving {
                edit.after_save = EditAfterSave::ExitMode;
                return;
            }
            if edit.dirty {
                self.confirm_edit_exit(EditExitTarget::Document(document_id), window, cx);
                return;
            }
        }
        self.finish_document_edit(document_id, window, cx);
    }

    fn finish_document_edit(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.documents.iter_mut().find(|tab| tab.id == document_id) else {
            return;
        };
        if let Some(edit) = tab.edit.as_mut() {
            let source_row = edit.editor.read(cx).cursor_position().line as usize;
            edit.active = false;
            let reload = edit.saved_since_enter;
            edit.saved_since_enter = false;
            edit.pending_exit_row = (reload || edit.saving).then_some(source_row);
            let release_clean_editor = !reload && !edit.saving && !edit.dirty;
            let visible_row = if self.select_and_center_log_source_row_atomically(
                document_id,
                source_row,
                window,
                cx,
            ) {
                Some(source_row)
            } else {
                let last_source_row = self
                    .documents
                    .iter()
                    .find(|tab| tab.id == document_id)
                    .and_then(|tab| {
                        tab.document
                            .source_row(tab.document.line_count().checked_sub(1)?)
                    });
                last_source_row.filter(|&row| {
                    self.select_and_center_log_source_row_atomically(document_id, row, window, cx)
                })
            };
            self.selected_source_row = visible_row;
            self.log_viewer.focus_handle.focus(window, cx);
            if reload {
                self.reload_document(document_id, ReloadStrategy::Full, window, cx);
            } else if release_clean_editor
                && let Some(tab) = self.documents.iter_mut().find(|tab| tab.id == document_id)
            {
                tab.edit = None;
            }
            cx.notify();
        } else if tab.edit_load_task.take().is_some() {
            cx.notify();
        }
    }

    fn confirm_edit_exit(
        &mut self,
        target: EditExitTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_edit_action(target, EditAfterSave::ExitMode, window, cx);
    }

    pub(super) fn confirm_edit_close(
        &mut self,
        tab_id: WorkspaceTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = match tab_id {
            WorkspaceTabId::Document(id) => EditExitTarget::Document(id),
            WorkspaceTabId::New(id) => EditExitTarget::NewFile(id),
        };
        self.confirm_edit_action(target, EditAfterSave::CloseTab, window, cx);
    }

    fn confirm_edit_action(
        &mut self,
        target: EditExitTarget,
        after_save: EditAfterSave,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            return;
        }
        let never_saved = matches!(target, EditExitTarget::NewFile(id) if self
            .new_file_drafts
            .get(&id)
            .is_some_and(|draft| draft.path.is_none()));
        let workspace = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let discard_workspace = workspace.clone();
            let save_workspace = workspace.clone();
            alert
                .title(if after_save == EditAfterSave::CloseTab {
                    crate::tr!("关闭标签前保存更改？", "Save changes before closing the tab?")
                } else if never_saved {
                    crate::tr!("保存新文件后退出编辑模式？", "Save the new file before exiting edit mode?")
                } else {
                    crate::tr!("保存更改后退出编辑模式？", "Save changes before exiting edit mode?")
                })
                .description(if never_saved {
                    crate::tr!("新文件尚未保存。选择保存时会先让你指定路径。", "This file has not been saved yet. Saving will ask you to choose a location.")
                } else {
                    crate::tr!("可以保存更改、放弃更改，或继续编辑。", "Save your changes, discard them, or continue editing.")
                })
                .footer(
                    DialogFooter::new()
                        .justify_center()
                        .child(crate::dialog_focus::dialog_cancel_action(
                            "edit-exit-cancel-action",
                            Button::new("edit-exit-cancel").label(crate::tr!("取消", "Cancel")),
                            cx,
                        ))
                        .child(
                            Button::new("edit-exit-discard")
                                .outline()
                                .label(crate::tr!("不保存", "Don't Save"))
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    discard_workspace.update(cx, |this, cx| {
                                        this.discard_edit_and_finish(target, after_save, window, cx);
                                    });
                                }),
                        )
                        .child(crate::dialog_focus::dialog_confirm_action(
                            "edit-exit-save-action",
                            Button::new("edit-exit-save")
                                .primary()
                                .label(crate::tr!("保存", "Save")),
                            cx,
                        )),
                )
                .on_ok(move |_, window, _| {
                    window.on_next_frame({
                        let workspace = save_workspace.clone();
                        move |window, cx| {
                            workspace.update(cx, |this, cx| {
                                this.save_edit_and_finish(target, after_save, window, cx);
                            });
                        }
                    });
                    true
                })
        });
    }

    fn save_edit_and_finish(
        &mut self,
        target: EditExitTarget,
        after_save: EditAfterSave,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            EditExitTarget::Document(id) => {
                if let Some(edit) = self
                    .documents
                    .iter_mut()
                    .find(|tab| tab.id == id)
                    .and_then(|tab| tab.edit.as_mut())
                {
                    edit.after_save = after_save;
                    self.save_document_edit(id, window, cx);
                }
            }
            EditExitTarget::NewFile(id) => {
                if let Some(draft) = self.new_file_drafts.get_mut(&id) {
                    draft.after_save = after_save;
                    self.save_new_file_draft(id, window, cx);
                }
            }
        }
    }

    fn discard_edit_and_finish(
        &mut self,
        target: EditExitTarget,
        action: EditAfterSave,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            EditExitTarget::Document(id) => {
                if let Some(edit) = self
                    .documents
                    .iter_mut()
                    .find(|tab| tab.id == id)
                    .and_then(|tab| tab.edit.as_mut())
                {
                    edit.dirty = false;
                    edit.after_save = EditAfterSave::None;
                    if action == EditAfterSave::CloseTab {
                        self.close_workspace_tabs(
                            BTreeSet::from([WorkspaceTabId::Document(id)]),
                            window,
                            cx,
                        );
                    } else {
                        self.finish_document_edit(id, window, cx);
                    }
                }
            }
            EditExitTarget::NewFile(id) => {
                if let Some(draft) = self.new_file_drafts.get_mut(&id) {
                    draft.dirty = false;
                    draft.after_save = EditAfterSave::None;
                    if action == EditAfterSave::CloseTab {
                        self.close_workspace_tabs(
                            BTreeSet::from([WorkspaceTabId::New(id)]),
                            window,
                            cx,
                        );
                    } else if draft.path.is_none() {
                        self.new_file_drafts.remove(&id);
                        self.log_viewer.focus_handle.focus(window, cx);
                        cx.notify();
                    } else {
                        self.finish_new_file_draft(id, window, cx);
                    }
                }
            }
        }
    }

    pub(super) fn save_document_edit(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.documents.iter_mut().find(|tab| tab.id == document_id) else {
            return;
        };
        let Some(edit) = tab.edit.as_mut() else {
            return;
        };
        if !edit.dirty || edit.saving {
            return;
        }
        let text = edit.editor.read(cx).value().to_string();
        let encoding = edit.encoding;
        let disk_version = edit.disk_version;
        let path = tab.document.path().to_path_buf();
        // A retained source handle can block replacement on Windows.
        // The editor owns the text now, and the document will be reloaded after editing.
        tab.document.release_source_handle();
        edit.saving = true;
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let bytes = encoding.encode(&text)?;
                    let new_version = atomic_write_edit(&path, &bytes, disk_version)?;
                    Ok::<_, anyhow::Error>((text, new_version))
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                let Some(edit) = this
                    .documents
                    .iter_mut()
                    .find(|tab| tab.id == document_id)
                    .and_then(|tab| tab.edit.as_mut())
                else {
                    return;
                };
                edit.saving = false;
                let reload_after_save = !edit.active;
                match result {
                    Ok((saved_text, new_version)) => {
                        edit.dirty = edit.editor.read(cx).value().as_ref() != saved_text;
                        edit.disk_version = new_version;
                        edit.saved_since_enter = true;
                        let after_save = std::mem::take(&mut edit.after_save);
                        let after_save = if edit.dirty {
                            EditAfterSave::None
                        } else {
                            after_save
                        };
                        window.notify_message(crate::tr!("文件已保存", "File saved"), cx);
                        if after_save == EditAfterSave::CloseTab {
                            this.close_workspace_tabs(
                                BTreeSet::from([WorkspaceTabId::Document(document_id)]),
                                window,
                                cx,
                            );
                        } else if reload_after_save {
                            this.reload_document(document_id, ReloadStrategy::Full, window, cx);
                        } else if after_save == EditAfterSave::ExitMode {
                            this.finish_document_edit(document_id, window, cx);
                        }
                    }
                    Err(error) => {
                        edit.after_save = EditAfterSave::None;
                        window.notify_message(
                            crate::tr_args!(
                                "无法保存文件：{error}",
                                "Could not save file: {error}"
                            ),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        });
        edit.save_task = Some(task);
        cx.notify();
    }

    pub(super) fn create_editable_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let WorkspaceTabId::New(id) = self.active_tab_id else {
            return;
        };
        if self.new_file_drafts.contains_key(&id) {
            self.resume_new_file_draft(id, window, cx);
            return;
        }
        let editor = cx.new(|cx| EditorState::new(window, cx).soft_wrap(false));
        let subscription = cx.subscribe(&editor, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(draft) = this.new_file_drafts.get_mut(&id)
            {
                draft.dirty = true;
                cx.notify();
            }
        });
        self.new_file_drafts.insert(
            id,
            NewFileDraft {
                editor: editor.clone(),
                path: None,
                disk_version: None,
                dirty: false,
                active: true,
                saving: false,
                after_save: EditAfterSave::None,
                save_task: None,
                _subscription: subscription,
            },
        );
        editor.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    pub(super) fn resume_new_file_draft(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(draft) = self.new_file_drafts.get_mut(&id) {
            draft.active = true;
            draft.editor.focus_handle(cx).focus(window, cx);
            cx.notify();
        }
    }

    pub(super) fn exit_new_file_draft(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(draft) = self.new_file_drafts.get_mut(&id) {
            if draft.saving {
                draft.after_save = EditAfterSave::ExitMode;
                return;
            }
            if draft.dirty || draft.path.is_none() {
                self.confirm_edit_exit(EditExitTarget::NewFile(id), window, cx);
                return;
            }
        }
        self.finish_new_file_draft(id, window, cx);
    }

    fn finish_new_file_draft(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(draft) = self.new_file_drafts.get_mut(&id) {
            draft.active = false;
            self.log_viewer.focus_handle.focus(window, cx);
            self.open_saved_new_file_draft(id, window, cx);
            cx.notify();
        }
    }

    fn open_saved_new_file_draft(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.new_file_drafts.get(&id).and_then(|draft| {
            (!draft.active && !draft.dirty && !draft.saving)
                .then(|| draft.path.clone())
                .flatten()
        }) else {
            return;
        };
        if self.open_task.is_some() || self.active_tab_id != WorkspaceTabId::New(id) {
            return;
        }
        self.new_file_drafts.remove(&id);
        self.begin_open_paths(vec![path], window, cx);
    }

    pub(super) fn save_new_file_draft(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = self.new_file_drafts.get_mut(&id) else {
            return;
        };
        if draft.saving || (draft.path.is_some() && !draft.dirty) {
            return;
        }
        let text = draft.editor.read(cx).value().to_string();
        let existing_path = draft.path.clone();
        let disk_version = draft.disk_version;
        let prompt = if existing_path.is_none() {
            let directory = dirs::document_dir()
                .or_else(dirs::home_dir)
                .unwrap_or_else(|| PathBuf::from("."));
            Some(cx.prompt_for_new_path(&directory, Some("untitled.log")))
        } else {
            None
        };
        draft.saving = true;
        let task = cx.spawn_in(window, async move |this, cx| {
            let path = if let Some(prompt) = prompt {
                match prompt.await {
                    Ok(Ok(Some(path))) => Some(Ok(path)),
                    Ok(Ok(None)) => None,
                    Ok(Err(error)) => Some(Err(anyhow::anyhow!(error))),
                    Err(error) => Some(Err(anyhow::anyhow!(error))),
                }
            } else {
                existing_path.map(Ok)
            };
            let result = match path {
                Some(Ok(path)) => Some(
                    cx.background_spawn(async move {
                        let version = if let Some(version) = disk_version {
                            atomic_write_edit(&path, text.as_bytes(), version)?
                        } else {
                            atomic_write_new(&path, text.as_bytes())?
                        };
                        Ok::<_, anyhow::Error>((path, text, version))
                    })
                    .await,
                ),
                Some(Err(error)) => Some(Err(error)),
                None => None,
            };
            _ = this.update_in(cx, |this, window, cx| {
                let Some(draft) = this.new_file_drafts.get_mut(&id) else {
                    return;
                };
                draft.saving = false;
                match result {
                    Some(Ok((path, saved_text, version))) => {
                        draft.dirty = draft.editor.read(cx).value().as_ref() != saved_text;
                        draft.path = Some(path);
                        draft.disk_version = Some(version);
                        let after_save = std::mem::take(&mut draft.after_save);
                        let after_save = if draft.dirty {
                            EditAfterSave::None
                        } else {
                            after_save
                        };
                        window.notify_message(crate::tr!("文件已保存", "File saved"), cx);
                        if after_save == EditAfterSave::CloseTab {
                            this.close_workspace_tabs(
                                BTreeSet::from([WorkspaceTabId::New(id)]),
                                window,
                                cx,
                            );
                        } else if after_save == EditAfterSave::ExitMode {
                            this.finish_new_file_draft(id, window, cx);
                        } else {
                            this.open_saved_new_file_draft(id, window, cx);
                        }
                    }
                    Some(Err(error)) => {
                        draft.after_save = EditAfterSave::None;
                        window.notify_message(
                            crate::tr_args!(
                                "无法保存文件：{error}",
                                "Could not save file: {error}"
                            ),
                            cx,
                        );
                    }
                    None => draft.after_save = EditAfterSave::None,
                }
                cx.notify();
            });
        });
        draft.save_task = Some(task);
        cx.notify();
    }
}

fn atomic_write_new(path: &Path, bytes: &[u8]) -> anyhow::Result<DiskVersion> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("File has no parent directory"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(path)?;
    DiskVersion::read(path)
}

fn atomic_write_edit(
    path: &Path,
    bytes: &[u8],
    expected: DiskVersion,
) -> anyhow::Result<DiskVersion> {
    anyhow::ensure!(
        DiskVersion::read(path)? == expected,
        crate::tr!(
            "文件已在磁盘上改变，请重新打开后再编辑",
            "The file changed on disk. Reopen it before editing"
        )
    );
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("File has no parent directory"))?;
    let permissions = std::fs::metadata(path)?.permissions();
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.as_file().set_permissions(permissions)?;
    anyhow::ensure!(
        DiskVersion::read(path)? == expected,
        crate::tr!(
            "文件已在磁盘上改变，请重新打开后再编辑",
            "The file changed on disk. Reopen it before editing"
        )
    );
    temporary.persist(path)?;
    DiskVersion::read(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn new_file_editor_saves_with_platform_shortcut(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::actions::init(cx);
            Workspace::init_window_registry(cx);
            crate::notifications::init(cx);
            crate::app_icon::init(cx);
        });
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("draft.log");
        std::fs::write(&path, "before").unwrap();
        let mut workspace = None;
        let window = cx.add_window(|window, cx| {
            let entity = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
            workspace = Some(entity.clone());
            Root::new(entity, window, cx)
        });
        let workspace = workspace.unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.create_editable_file(window, cx);
                let draft = workspace.new_file_drafts.get_mut(&1).unwrap();
                draft.path = Some(path.clone());
                draft.disk_version = Some(DiskVersion::read(&path).unwrap());
            });
            window.draw(cx).clear(cx);
        })
        .unwrap();

        cx.simulate_keystrokes(window.into(), "x");
        cx.simulate_keystrokes(window.into(), "secondary-s");
        cx.run_until_parked();

        assert_eq!(std::fs::read_to_string(path).unwrap(), "x");
    }

    #[gpui_kit::test]
    fn document_editor_saves_with_platform_shortcut(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::actions::init(cx);
            Workspace::init_window_registry(cx);
            crate::notifications::init(cx);
            crate::app_icon::init(cx);
        });
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.log");
        std::fs::write(&path, "before\n").unwrap();
        let document = Arc::new(LogDocument::open(&path).unwrap());
        let mut workspace = None;
        let window = cx.add_window(|window, cx| {
            let entity = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
            workspace = Some(entity.clone());
            Root::new(entity, window, cx)
        });
        let workspace = workspace.unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                let prepared = PreparedDocument {
                    document: document.clone(),
                    cached_complete_document: None,
                    session: None,
                    color_labels_snapshot: None,
                    resolved_color_rules: Arc::default(),
                    search_result: SearchResult::default(),
                    search_range: SearchRange::default(),
                    search_matcher: None,
                    search_case_sensitive: false,
                    search_regex: false,
                    warning: None,
                    load_state: DocumentLoadState::Ready,
                    pending_index_cache: None,
                    upgrade_frame: None,
                };
                workspace.install_documents(
                    vec![(path.clone(), Ok(prepared))],
                    Some(&path),
                    &BTreeMap::new(),
                    None,
                    true,
                    window,
                    cx,
                );
                let document_id = workspace.documents[0].id;
                let editor = cx.new(|cx| {
                    EditorState::new(window, cx)
                        .soft_wrap(false)
                        .default_value("before\n")
                });
                let subscription = cx.subscribe(&editor, move |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change)
                        && let Some(edit) = this
                            .documents
                            .iter_mut()
                            .find(|tab| tab.id == document_id)
                            .and_then(|tab| tab.edit.as_mut())
                    {
                        edit.dirty = true;
                        cx.notify();
                    }
                });
                workspace.documents[0].edit = Some(DocumentEditSession {
                    editor: editor.clone(),
                    encoding: EditEncoding::Utf8,
                    disk_version: DiskVersion::read(&path).unwrap(),
                    dirty: false,
                    active: true,
                    saving: false,
                    saved_since_enter: false,
                    pending_exit_row: None,
                    after_save: EditAfterSave::None,
                    save_task: None,
                    _subscription: subscription,
                });
                editor.focus_handle(cx).focus(window, cx);
            });
            window.draw(cx).clear(cx);
        })
        .unwrap();

        cx.simulate_keystrokes(window.into(), "x");
        cx.simulate_keystrokes(window.into(), "secondary-s");
        cx.run_until_parked();

        assert_eq!(std::fs::read_to_string(path).unwrap(), "xbefore\n");
    }

    struct EditorEntryHarness {
        editor: Entity<EditorState>,
    }

    impl Render for EditorEntryHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size(px(400.))
                .child(Editor::new(&self.editor).h(px(400.)))
        }
    }

    #[gpui_kit::test]
    fn editor_reveals_a_distant_source_row_after_first_layout(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let text = (0..500)
                .map(|row| format!("line {row}"))
                .collect::<Vec<_>>()
                .join("\n");
            let editor = cx.new(|cx| {
                EditorState::new(window, cx)
                    .soft_wrap(false)
                    .default_value(text)
            });
            editor.update(cx, |editor, cx| {
                select_editor_source_line(editor, 400, window, cx);
            });
            EditorEntryHarness { editor }
        });
        let editor = view.read_with(cx, |view, _| view.editor.clone());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            editor.update(cx, |editor, cx| {
                assert_eq!(editor.cursor_position().line, 400);
                assert_eq!(editor.selected_value().as_ref(), "line 400");
                select_editor_source_line(editor, 400, window, cx);
            });
            window.draw(cx).clear(cx);
        });
        let visible = editor.read_with(cx, |editor, _| editor.visible_row_range().unwrap());
        assert!(
            visible.contains(&400),
            "visible rows after entry: {visible:?}"
        );
        assert_eq!(
            editor.read_with(cx, |editor, _| editor.selected_value()),
            "line 400"
        );
    }

    #[gpui_kit::test]
    fn edit_entry_aligns_anchor_line_to_its_previous_screen_height(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let text = (0..500)
                .map(|row| format!("line {row}"))
                .collect::<Vec<_>>()
                .join("\n");
            let editor = cx.new(|cx| {
                EditorState::new(window, cx)
                    .soft_wrap(false)
                    .default_value(text)
            });
            editor.update(cx, |editor, cx| {
                select_editor_source_line(editor, 400, window, cx);
                position_editor_source_line(editor, 400, Some(px(80.)), px(20.), cx);
            });
            EditorEntryHarness { editor }
        });
        let editor = view.read_with(cx, |view, _| view.editor.clone());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            editor.update(cx, |editor, cx| {
                position_editor_source_line(editor, 400, Some(px(80.)), px(20.), cx);
            });
            window.draw(cx).clear(cx);
            let target_y = editor
                .read(cx)
                .range_to_bounds(&editor_source_line_range(editor.read(cx), 400));
            let target_y = target_y.expect("anchor row should be visible").top() + px(30.);
            editor.update(cx, |editor, cx| {
                assert!(align_editor_source_line(editor, 400, target_y, cx));
            });
            window.draw(cx).clear(cx);
            let bounds = editor
                .read(cx)
                .range_to_bounds(&editor_source_line_range(editor.read(cx), 400))
                .expect("anchor row should remain visible");
            assert!((f32::from(bounds.top() - target_y)).abs() < 2.);
            assert_eq!(editor.read(cx).selected_value().as_ref(), "line 400");
        });
    }

    #[gpui_kit::test]
    fn edit_entry_selects_only_the_anchor_line_text(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let editor = cx.new(|cx| {
                EditorState::new(window, cx)
                    .soft_wrap(false)
                    .default_value("first\r\n中文 log\r\n\r\nlast")
            });
            editor.update(cx, |editor, cx| {
                select_editor_source_line(editor, 1, window, cx);
            });
            EditorEntryHarness { editor }
        });
        let editor = view.read_with(cx, |view, _| view.editor.clone());
        cx.update(|window, cx| {
            assert_eq!(editor.read(cx).selected_value().as_ref(), "中文 log");
            editor.update(cx, |editor, cx| {
                select_editor_source_line(editor, 2, window, cx);
                assert!(editor.selected_range().is_empty());
                select_editor_source_line(editor, 3, window, cx);
                assert_eq!(editor.selected_value().as_ref(), "last");
            });
        });
    }

    #[test]
    fn saving_replaces_the_complete_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("example.log");
        std::fs::write(&path, "old\ncontent\n").unwrap();
        atomic_write_edit(
            &path,
            "new\n日志\n".as_bytes(),
            DiskVersion::read(&path).unwrap(),
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n日志\n");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn saving_replaces_an_open_document_after_releasing_its_source_handle() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("example.log");
        std::fs::write(&path, "original text\n").unwrap();
        let document = LogDocument::open(&path).unwrap();
        let version = DiskVersion::read(&path).unwrap();

        document.release_source_handle();
        atomic_write_edit(&path, b"edited text\n", version).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "edited text\n");
        assert!(document.source_changed().unwrap());
    }

    #[test]
    fn saving_rejects_a_changed_source() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("active.log");
        std::fs::write(&path, "first\n").unwrap();
        let version = DiskVersion::read(&path).unwrap();
        std::fs::write(&path, "first\nappended\n").unwrap();
        assert!(atomic_write_edit(&path, b"editor\n", version).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "first\nappended\n");
    }

    #[test]
    fn first_save_creates_a_file_without_overwriting_an_existing_one() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("new.log");
        atomic_write_new(&path, "first\n".as_bytes()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\n");
        assert!(atomic_write_new(&path, b"second\n").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\n");
    }

    #[test]
    fn editor_round_trip_preserves_bom_and_legacy_encoding() {
        for (encoding, text) in [
            (EditEncoding::Utf8Bom, "日志\n"),
            (EditEncoding::Utf16Le, "日志\n"),
            (EditEncoding::Utf16Be, "日志\n"),
            (EditEncoding::Legacy(encoding_rs::WINDOWS_1252), "café\n"),
        ] {
            let bytes = encoding.encode(text).unwrap();
            assert_eq!(encoding.decode(&bytes).unwrap(), text);
        }
        assert!(
            EditEncoding::Legacy(encoding_rs::WINDOWS_1252)
                .encode("日志")
                .is_err()
        );
    }
}
