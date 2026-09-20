use std::io::Write as _;

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

impl Workspace {
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
            let table = tab.log_table.read(cx);
            let row_count = table.delegate().row_count();
            table
                .active_log_row()
                .or_else(|| {
                    (row_count > 0).then(|| {
                        tab.log_viewport
                            .first_visible(row_count, self.log_row_height())
                    })
                })
                .and_then(|row| table.delegate().source_row(row))
                .unwrap_or(0)
        };
        if let Some(edit) = self.documents[ix].edit.as_mut() {
            edit.active = true;
            edit.editor.update(cx, |editor, cx| {
                editor.set_cursor_position(Position::new(source_row as u32, 0), window, cx);
            });
            edit.editor.focus_handle(cx).focus(window, cx);
            cx.notify();
            return;
        }
        if self.documents[ix].edit_task.is_some() {
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
        self.documents[ix].edit_task = Some(cx.spawn_in(window, async move |this, cx| {
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
                this.documents[ix].edit_task = None;
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
                            editor.set_cursor_position(
                                Position::new(source_row as u32, 0),
                                window,
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
                            exit_after_save: false,
                            _subscription: subscription,
                        });
                        if this.active_tab_id == WorkspaceTabId::Document(document_id) {
                            editor.focus_handle(cx).focus(window, cx);
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
                edit.exit_after_save = true;
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
        } else if tab.edit_task.take().is_some() {
            cx.notify();
        }
    }

    fn confirm_edit_exit(
        &mut self,
        target: EditExitTarget,
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
                .title(if never_saved {
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
                                        this.discard_edit_and_exit(target, window, cx);
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
                                this.save_edit_and_exit(target, window, cx);
                            });
                        }
                    });
                    true
                })
        });
    }

    fn save_edit_and_exit(
        &mut self,
        target: EditExitTarget,
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
                    edit.exit_after_save = true;
                    self.save_document_edit(id, window, cx);
                }
            }
            EditExitTarget::NewFile(id) => {
                if let Some(draft) = self.new_file_drafts.get_mut(&id) {
                    draft.exit_after_save = true;
                    self.save_new_file_draft(id, window, cx);
                }
            }
        }
    }

    fn discard_edit_and_exit(
        &mut self,
        target: EditExitTarget,
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
                    edit.exit_after_save = false;
                    self.finish_document_edit(id, window, cx);
                }
            }
            EditExitTarget::NewFile(id) => {
                if let Some(draft) = self.new_file_drafts.get_mut(&id) {
                    draft.dirty = false;
                    draft.exit_after_save = false;
                    if draft.path.is_none() {
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
                        let exit_after_save =
                            std::mem::take(&mut edit.exit_after_save) && !edit.dirty;
                        window.notify_message(crate::tr!("文件已保存", "File saved"), cx);
                        if reload_after_save {
                            this.reload_document(document_id, ReloadStrategy::Full, window, cx);
                        } else if exit_after_save {
                            this.finish_document_edit(document_id, window, cx);
                        }
                    }
                    Err(error) => {
                        edit.exit_after_save = false;
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
        tab.edit_task = Some(task);
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
                exit_after_save: false,
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
                draft.exit_after_save = true;
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
                        let exit_after_save =
                            std::mem::take(&mut draft.exit_after_save) && !draft.dirty;
                        window.notify_message(crate::tr!("文件已保存", "File saved"), cx);
                        if exit_after_save {
                            this.finish_new_file_draft(id, window, cx);
                        } else {
                            this.open_saved_new_file_draft(id, window, cx);
                        }
                    }
                    Some(Err(error)) => {
                        draft.exit_after_save = false;
                        window.notify_message(
                            crate::tr_args!(
                                "无法保存文件：{error}",
                                "Could not save file: {error}"
                            ),
                            cx,
                        );
                    }
                    None => draft.exit_after_save = false,
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
