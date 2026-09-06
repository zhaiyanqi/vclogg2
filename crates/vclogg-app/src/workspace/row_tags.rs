use super::*;
use crate::{
    log_tag_layer::{LogTagLayer, PositionedTag, TagDragPreview, TagGeometryHandle},
    log_tags::{RowTag, TagColor, TagPreset, TagStyle, source_digest},
    row_tag_dialog::{RowTagDialog, RowTagPreview},
};
use std::sync::Weak;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TagTarget {
    document_id: u64,
    source_row: usize,
    region: WrappedRegion,
}

#[derive(Clone)]
pub(super) struct TagContext {
    target: TagTarget,
    document: Weak<LogDocument>,
    text: LogText,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

#[derive(Default)]
pub(super) struct TagInteractionState {
    context: Option<(TagContext, Point<Pixels>)>,
    pub(super) menu_target: Option<TagMenuTarget>,
    dialog: Option<WeakEntity<RowTagDialog>>,
    drag: Option<TagDrag>,
}

#[derive(Clone)]
pub(super) struct TagMenuTarget {
    target: TagTarget,
    id: String,
    document: Weak<LogDocument>,
    text: SharedString,
}

#[derive(Clone)]
struct TagEditRequest {
    target: TagTarget,
    document: Weak<LogDocument>,
    id: String,
    draft: RowTag,
    is_new: bool,
}

#[derive(Clone)]
struct TagDragPayload {
    target: TagTarget,
    document: Weak<LogDocument>,
    id: String,
    geometry: TagGeometryHandle,
    font_size: Pixels,
}

struct TagDrag {
    payload: TagDragPayload,
    grab_offset: Point<Pixels>,
    position: Point<Pixels>,
}

impl Workspace {
    pub(super) fn tag_row_context(
        &self,
        document_id: u64,
        source_row: usize,
        region: WrappedRegion,
        text: &LogText,
        source_unavailable: bool,
        bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    ) -> Option<TagContext> {
        if source_unavailable {
            return None;
        }
        let tab = if region == WrappedRegion::GlobalResults {
            self.documents
                .get(self.presentation_document_ix_for_global_result(document_id)?)?
        } else {
            self.documents.iter().find(|tab| tab.id == document_id)?
        };
        if tab.load_state != DocumentLoadState::Ready {
            return None;
        }
        Some(TagContext {
            target: TagTarget {
                document_id: tab.id,
                source_row,
                region,
            },
            document: Arc::downgrade(&tab.document),
            text: text.clone(),
            bounds,
        })
    }

    pub(super) fn prepare_tag_context(
        &mut self,
        context: Option<TagContext>,
        position: Point<Pixels>,
    ) {
        self.row_tags.context = context.map(|context| (context, position));
    }

    fn tag_target_is_current(&self, target: TagTarget, document: &Weak<LogDocument>) -> bool {
        let Some(document) = document.upgrade() else {
            return false;
        };
        self.documents.iter().any(|tab| {
            tab.id == target.document_id
                && Arc::ptr_eq(&tab.document, &document)
                && tab.document.contains_source_row(target.source_row)
        })
    }

    pub(super) fn append_add_tag_menu(
        menu: PopupMenu,
        workspace: Entity<Self>,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let this = workspace.read(cx);
        let context = this.row_tags.context.clone().filter(|(context, _)| {
            this.open_task.is_none()
                && this.persistence.store.is_some()
                && this.tag_target_is_current(context.target, &context.document)
        });
        let Some((context, position)) = context else {
            return menu
                .item(PopupMenuItem::new(crate::tr!("文字标记", "Text mark")).disabled(true));
        };
        let has_tags = this
            .documents
            .iter()
            .find(|tab| tab.id == context.target.document_id)
            .is_some_and(|tab| !tab.file.row_tags.is_empty());
        let recent = cx
            .global::<WorkspaceWindowRegistry>()
            .row_tag_presets
            .clone()
            .unwrap_or_default();
        menu.submenu(
            crate::tr!("文字标记", "Text mark"),
            window,
            cx,
            move |mut menu, window, _| {
                let create_context = context.clone();
                menu = menu.item(
                    PopupMenuItem::new(crate::tr!("新增标记…", "New mark…")).on_click(
                        window.listener_for(&workspace, move |this, _, window, cx| {
                            this.add_row_tag(create_context.clone(), position, None, window, cx);
                        }),
                    ),
                );
                let clear_context = context.clone();
                menu = menu.item(
                    PopupMenuItem::new(crate::tr!("清除所有标记", "Clear all marks"))
                        .disabled(!has_tags)
                        .on_click(window.listener_for(&workspace, move |this, _, window, cx| {
                            this.clear_document_row_tags(
                                clear_context.target,
                                &clear_context.document,
                                window,
                                cx,
                            );
                        })),
                );
                if !recent.is_empty() {
                    menu = menu.separator();
                }
                for preset in recent.iter().take(10) {
                    let context = context.clone();
                    let preset = preset.clone();
                    menu = menu.item(PopupMenuItem::new(preset.label.clone()).on_click(
                        window.listener_for(&workspace, move |this, _, window, cx| {
                            this.add_row_tag(
                                context.clone(),
                                position,
                                Some(preset.clone()),
                                window,
                                cx,
                            );
                        }),
                    ));
                }
                menu
            },
        )
    }

    fn add_row_tag(
        &mut self,
        context: TagContext,
        pointer: Point<Pixels>,
        preset: Option<TagPreset>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_task.is_some()
            || !self.tag_target_is_current(context.target, &context.document)
        {
            return;
        }
        let Some(bounds) = context.bounds.get() else {
            return;
        };
        let font_size = px(self.app_settings.log_font_size as f32);
        let position = pointer - bounds.origin;
        let draft = RowTag {
            source_row: context.target.source_row,
            label: String::new(),
            color: TagColor::Neutral,
            style: TagStyle::default(),
            x: position_units(position.x.max(px(0.)), font_size),
            y: position_units(position.y.max(px(0.)), font_size),
            source_digest: source_digest(context.text.source()),
        };
        let id = uuid::Uuid::new_v4().to_string();
        if let Some(preset) = preset {
            self.save_row_tag(
                TagEditRequest {
                    target: context.target,
                    document: context.document,
                    id,
                    draft,
                    is_new: true,
                },
                preset,
                window,
                cx,
            );
        } else {
            self.open_row_tag_dialog(
                TagEditRequest {
                    target: context.target,
                    document: context.document,
                    id,
                    draft,
                    is_new: true,
                },
                context.text.display().clone(),
                window,
                cx,
            );
        }
    }

    fn open_row_tag_dialog(
        &mut self,
        request: TagEditRequest,
        text: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let TagEditRequest {
            target,
            document,
            id,
            draft,
            is_new,
        } = request;
        if self.open_task.is_some()
            || !self.tag_target_is_current(target, &document)
            || self
                .row_tags
                .dialog
                .as_ref()
                .and_then(WeakEntity::upgrade)
                .is_some()
        {
            return;
        }
        self.pause_tag_file_refresh();
        self.cancel_tag_drag(window, cx);
        self.row_drag_selection = None;
        TextSelection::clear(window, cx);
        self.tag_region_focus(target.region).focus(window, cx);
        let presets = cx
            .global::<WorkspaceWindowRegistry>()
            .row_tag_presets
            .clone()
            .unwrap_or_default();
        let font = px(self.app_settings.log_font_size as f32);
        let row_height = self.log_row_height();
        let tab = self
            .documents
            .iter()
            .find(|tab| tab.id == target.document_id)
            .unwrap();
        let preview = RowTagPreview::new(
            text,
            target.source_row,
            point(
                font * (draft.x as f32 / 1000.),
                font * (draft.y as f32 / 1000.),
            ),
            row_height,
            tab.log_table.read(cx).delegate(),
            cx,
        );
        let mut initial_preset = draft.preset();
        if is_new && let Some(last_used) = presets.first() {
            // Reuse the last confirmed appearance, keeping the new tag's text empty.
            initial_preset.color = last_used.color;
            initial_preset.style = last_used.style.clone();
        }
        let editor = cx.new(|cx| RowTagDialog::new(initial_preset, presets, preview, window, cx));
        self.row_tags.dialog = Some(editor.downgrade());
        let input = editor.read(cx).input();
        window.defer(cx, move |window, cx| {
            input.focus_handle(cx).focus(window, cx);
            input.update(cx, |input, cx| input.select_all(window, cx));
        });
        let workspace = cx.weak_entity();
        window.open_dialog(cx, move |dialog, window, cx| {
            let editor_submit = editor.clone();
            let editor_close = editor.clone();
            let workspace_submit = workspace.clone();
            let workspace_close = workspace.clone();
            let id = id.clone();
            let document = document.clone();
            let draft = draft.clone();
            dialog
                .title(if is_new {
                    crate::tr!("新增标记", "New mark")
                } else {
                    crate::tr!("编辑标记", "Edit mark")
                })
                .width(window.rem_size() * 46.)
                .close_button(false)
                .child(editor.clone())
                .footer(
                    DialogFooter::new()
                        .child(crate::dialog_focus::dialog_cancel_action(
                            "row-tag-cancel-action",
                            Button::new("row-tag-cancel").label(crate::tr!("取消", "Cancel")),
                            cx,
                        ))
                        .child(crate::dialog_focus::dialog_confirm_action(
                            "row-tag-save-action",
                            Button::new("row-tag-save").primary().label(if is_new {
                                crate::tr!("添加", "Add")
                            } else {
                                crate::tr!("保存", "Save")
                            }),
                            cx,
                        )),
                )
                .on_ok(move |_, window, cx| {
                    if RowTagDialog::is_composing(&editor_submit, window, cx) {
                        return false;
                    }
                    let preset = editor_submit.read(cx).value(cx);
                    if !preset.is_valid() {
                        editor_submit.update(cx, |editor, cx| {
                            editor.show_error(
                                crate::tr!(
                                    "请输入 1–128 个字符的标记文字",
                                    "Enter 1–128 characters for the mark"
                                )
                                .to_string(),
                                cx,
                            )
                        });
                        return false;
                    }
                    let saved = workspace_submit
                        .update(cx, |this, cx| {
                            this.save_row_tag(
                                TagEditRequest {
                                    target,
                                    document: document.clone(),
                                    id: id.clone(),
                                    draft: draft.clone(),
                                    is_new,
                                },
                                preset,
                                window,
                                cx,
                            )
                        })
                        .unwrap_or(false);
                    if !saved {
                        editor_submit.update(cx, |editor, cx| {
                            editor.show_error(
                                crate::tr!(
                                    "源行已变化，请取消后重新打开标记",
                                    "The source row changed. Cancel and reopen the mark"
                                )
                                .to_string(),
                                cx,
                            )
                        });
                    }
                    saved
                })
                .on_close(move |_, window, cx| {
                    editor_close.update(cx, |editor, cx| editor.cancel_preview_drag(window, cx));
                    _ = workspace_close.update(cx, |this, cx| {
                        this.row_tags.dialog = None;
                        cx.notify();
                    });
                })
        });
        cx.notify();
    }

    fn save_row_tag(
        &mut self,
        request: TagEditRequest,
        preset: TagPreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let TagEditRequest {
            target,
            document,
            id,
            draft,
            is_new,
        } = request;
        if !preset.is_valid() || !self.tag_target_is_current(target, &document) {
            return false;
        }
        let tab = self
            .documents
            .iter_mut()
            .find(|tab| tab.id == target.document_id)
            .unwrap();
        let Some(mut tag) = tab
            .file
            .row_tags
            .get(target.source_row, &id)
            .cloned()
            .or_else(|| is_new.then_some(draft))
        else {
            return false;
        };
        tag.label = preset.label.clone();
        tag.color = preset.color;
        tag.style = preset.style.clone();
        tab.file.row_tags.insert(id, tag);
        self.remember_row_tag_preset(preset, window, cx);
        self.row_tag_changed(
            target.document_id,
            is_new.then_some(target.source_row),
            window,
            cx,
        );
        true
    }

    fn remember_row_tag_preset(
        &mut self,
        preset: TagPreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        let (sender, receiver) = async_channel::bounded::<()>(1);
        let previous = cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
            let presets = registry.row_tag_presets.get_or_insert_with(Vec::new);
            presets.retain(|existing| existing.label != preset.label);
            presets.insert(0, preset.clone());
            registry.row_tag_preset_save_completion.replace(receiver)
        });
        self.persistence
            .state_tasks
            .push(cx.spawn_in(window, async move |_, cx| {
                if let Some(previous) = previous {
                    _ = previous.recv().await;
                }
                let result = cx
                    .background_spawn(async move { store.remember_row_tag_preset(&preset) })
                    .await;
                drop(sender);
                if let Err(error) = result {
                    _ = cx.update(|window, cx| {
                        window.push_notification(
                            crate::tr_args!(
                                "标记历史未能保存：{error}",
                                "Couldn’t save mark history: {error}"
                            ),
                            cx,
                        )
                    });
                }
            }));
    }

    fn tag_region_focus(&self, region: WrappedRegion) -> FocusHandle {
        if region == WrappedRegion::Log {
            self.log_viewer.focus_handle.clone()
        } else {
            self.search_results_viewer.focus_handle.clone()
        }
    }

    fn row_tag_changed(
        &mut self,
        document_id: u64,
        mark: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(source_row) = mark {
            let Some(tab) = self.documents.iter_mut().find(|tab| tab.id == document_id) else {
                return;
            };
            // Adding a tag always marks exactly its source row, never toggles a selection.
            tab.file.marked_rows.insert(source_row);
            tab.file.pending_restore_marked_rows.insert(source_row);
            let marked_rows = tab.file.marked_rows.clone();
            tab.log_table.update(cx, |table, cx| {
                table.delegate_mut().set_marked_rows(marked_rows);
                table.refresh(cx);
            });
            if tab.result_mode.includes_marks() {
                tab.results_visible = true;
            }
            if self.global_search.result_mode.includes_marks()
                && self.global_search.selected_documents.contains(&document_id)
            {
                self.global_search.results_visible = true;
            }
            self.refresh_document_result_rows_atomically(document_id, window, cx);
            self.refresh_global_result_rows(window, cx);
            self.refresh_active_document_surfaces_atomically(window, cx);
            self.schedule_workspace_search_state_save(window, cx);
        }
        self.schedule_checkpoint(document_id, window, cx);
        cx.notify();
    }

    fn clear_document_row_tags(
        &mut self,
        target: TagTarget,
        document: &Weak<LogDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_task.is_some()
            || self.persistence.store.is_none()
            || !self.tag_target_is_current(target, document)
        {
            return;
        }
        let tab = self
            .documents
            .iter_mut()
            .find(|tab| tab.id == target.document_id)
            .unwrap();
        if tab.file.row_tags.is_empty() {
            return;
        }
        tab.file.row_tags.clear();
        if self
            .row_tags
            .drag
            .as_ref()
            .is_some_and(|drag| drag.payload.target.document_id == target.document_id)
        {
            self.cancel_tag_drag(window, cx);
        }
        self.row_tag_changed(target.document_id, None, window, cx);
    }

    fn delete_row_tag(
        &mut self,
        target: TagTarget,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self
            .documents
            .iter_mut()
            .find(|tab| tab.id == target.document_id)
        else {
            return;
        };
        if tab.file.row_tags.remove(target.source_row, id).is_some() {
            self.row_tag_changed(target.document_id, None, window, cx);
        }
    }

    pub(super) fn render_row_tag_menu(
        menu: PopupMenu,
        workspace: Entity<Self>,
        menu_target: TagMenuTarget,
        window: &mut Window,
        _: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let TagMenuTarget {
            target,
            id,
            document,
            text,
        } = menu_target;
        let edit_id = id.clone();
        menu.item(
            PopupMenuItem::new(crate::tr!("编辑标记…", "Edit mark…")).on_click(
                window.listener_for(&workspace, move |this, _, window, cx| {
                    let tag = this
                        .documents
                        .iter()
                        .find(|tab| tab.id == target.document_id)
                        .and_then(|tab| tab.file.row_tags.get(target.source_row, &edit_id))
                        .cloned();
                    if let Some(tag) = tag {
                        this.open_row_tag_dialog(
                            TagEditRequest {
                                target,
                                document: document.clone(),
                                id: edit_id.clone(),
                                draft: tag,
                                is_new: false,
                            },
                            text.clone(),
                            window,
                            cx,
                        );
                    }
                }),
            ),
        )
        .separator()
        .item(
            PopupMenuItem::new(crate::tr!("删除标记", "Delete mark")).on_click(
                window.listener_for(&workspace, move |this, _, window, cx| {
                    this.delete_row_tag(target, &id, window, cx);
                }),
            ),
        )
    }

    pub(super) fn render_row_tags(
        &self,
        context: &TagContext,
        font_size: u16,
        base_height: Pixels,
        cx: &mut Context<Self>,
    ) -> Option<LogTagLayer> {
        let target = context.target;
        let tab = self
            .documents
            .iter()
            .find(|tab| tab.id == target.document_id)?;
        let tags = tab
            .file
            .row_tags
            .row(target.source_row)
            .map(|(id, tag)| (id.clone(), tag.clone()))
            .collect::<Vec<_>>();
        if tags.is_empty() {
            return None;
        }
        let digest = source_digest(context.text.source());
        let font = px(font_size as f32);
        let elements = tags
            .into_iter()
            .filter(|(_, tag)| tag.source_digest == digest)
            .map(|(id, tag)| {
                let geometry = self
                    .row_tags
                    .drag
                    .as_ref()
                    .filter(|drag| drag.payload.target == target && drag.payload.id == id)
                    .map_or_else(
                        || Rc::new(Cell::new(None)),
                        |drag| drag.payload.geometry.clone(),
                    );
                let position = self
                    .row_tags
                    .drag
                    .as_ref()
                    .filter(|drag| drag.payload.target == target && drag.payload.id == id)
                    .map_or_else(
                        || point(font * (tag.x as f32 / 1000.), font * (tag.y as f32 / 1000.)),
                        |drag| drag.position,
                    );
                let element = {
                    let edit_id = id.clone();
                    let edit_document = context.document.clone();
                    let edit_text = context.text.display().clone();
                    let edit_tag = tag.clone();
                    let menu_id = id.clone();
                    let menu_document = context.document.clone();
                    let menu_text = context.text.display().clone();
                    let payload = TagDragPayload {
                        target,
                        document: context.document.clone(),
                        id: id.clone(),
                        geometry: geometry.clone(),
                        font_size: font,
                    };
                    let drag_workspace = cx.weak_entity();
                    div()
                        .id(format!(
                            "row-tag-{}-{}-{}",
                            target.document_id, target.region as u8, id
                        ))
                        .max_w(font * 24.)
                        .overflow_hidden()
                        .opacity(tag.style.opacity())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                this.row_drag_selection = None;
                                GlobalState::suppress_text_selection(cx);
                                TextSelection::clear(window, cx);
                                if event.click_count == 2 {
                                    this.open_row_tag_dialog(
                                        TagEditRequest {
                                            target,
                                            document: edit_document.clone(),
                                            id: edit_id.clone(),
                                            draft: edit_tag.clone(),
                                            is_new: false,
                                        },
                                        edit_text.clone(),
                                        window,
                                        cx,
                                    );
                                }
                                cx.stop_propagation();
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, _, _, cx| {
                                this.row_tags.context = None;
                                this.row_tags.menu_target = Some(TagMenuTarget {
                                    target,
                                    id: menu_id.clone(),
                                    document: menu_document.clone(),
                                    text: menu_text.clone(),
                                });
                                cx.stop_propagation();
                            }),
                        )
                        .on_drag(
                            payload,
                            move |payload: &TagDragPayload, offset, window, cx| {
                                _ = drag_workspace.update(cx, |this, cx| {
                                    if let Some(geometry) = payload.geometry.get()
                                        && this.open_task.is_none()
                                        && this.tag_target_is_current(
                                            payload.target,
                                            &payload.document,
                                        )
                                    {
                                        this.pause_tag_file_refresh();
                                        this.row_tags.drag = Some(TagDrag {
                                            payload: payload.clone(),
                                            grab_offset: offset,
                                            position: geometry.tag.origin - geometry.content.origin,
                                        });
                                        this.tag_region_focus(payload.target.region)
                                            .focus(window, cx);
                                        cx.notify();
                                    }
                                });
                                cx.new(|_| TagDragPreview)
                            },
                        )
                        .child(
                            tag.preset()
                                .render_tag(font, base_height, cx)
                                .child(div().min_w_0().truncate().child(tag.label)),
                        )
                        .into_any_element()
                };
                PositionedTag {
                    position,
                    geometry,
                    element: Some(element),
                }
            })
            .collect();
        Some(LogTagLayer::new(
            format!(
                "row-tags-{}-{}-{}",
                target.document_id, target.region as u8, target.source_row
            ),
            elements,
        ))
    }

    fn update_tag_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = &mut self.row_tags.drag else {
            return;
        };
        let Some(geometry) = drag.payload.geometry.get() else {
            return;
        };
        drag.position = geometry.drag_position(position, drag.grab_offset);
        cx.notify();
    }

    pub(super) fn row_tag_drag_active(&self) -> bool {
        self.row_tags.drag.is_some()
    }

    pub(super) fn row_tag_interaction_active(&self) -> bool {
        self.row_tags.drag.is_some()
            || self
                .row_tags
                .dialog
                .as_ref()
                .and_then(WeakEntity::upgrade)
                .is_some()
    }

    fn pause_tag_file_refresh(&mut self) {
        if self.file_refresh_task.take().is_some()
            && let Some(watcher) = self.file_watch.as_ref()
            && let Some(document_id) = self.file_watch_document_id
        {
            watcher.retry([document_id]);
        }
    }

    fn finish_tag_drag(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_tag_drag(position, cx);
        let Some(drag) = self.row_tags.drag.take() else {
            return;
        };
        if !self.tag_target_is_current(drag.payload.target, &drag.payload.document) {
            return;
        }
        let tab = self
            .documents
            .iter_mut()
            .find(|tab| tab.id == drag.payload.target.document_id)
            .unwrap();
        if let Some(mut tag) = tab
            .file
            .row_tags
            .get(drag.payload.target.source_row, &drag.payload.id)
            .cloned()
        {
            tag.x = position_units(drag.position.x, drag.payload.font_size);
            tag.y = position_units(drag.position.y, drag.payload.font_size);
            tab.file.row_tags.insert(drag.payload.id, tag);
            self.row_tag_changed(drag.payload.target.document_id, None, window, cx);
        }
    }

    pub(super) fn cancel_tag_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.row_tags.drag.take().is_some() {
            cx.stop_active_drag(window);
            cx.notify();
        }
    }

    pub(super) fn cancel_tag_gesture(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape" && self.row_tags.drag.is_some() {
            self.cancel_tag_drag(window, cx);
            cx.stop_propagation();
        }
    }

    pub(super) fn render_tag_gesture_observer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace = cx.weak_entity();
        canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                let preparing = workspace.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, _, cx| {
                    if phase.capture() && event.button == MouseButton::Right {
                        _ = preparing.update(cx, |this, _| {
                            this.row_tags.context = None;
                            this.row_tags.menu_target = None;
                        });
                    }
                });
                let moving = workspace.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase.capture() {
                        _ = moving.update(cx, |this, cx| {
                            if this.row_tags.drag.is_some() {
                                this.update_tag_drag(event.position, cx);
                                cx.stop_propagation();
                            }
                        });
                    }
                });
                let finishing = workspace.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase.capture() && event.button == MouseButton::Left {
                        _ = finishing.update(cx, |this, cx| {
                            this.finish_tag_drag(event.position, window, cx)
                        });
                    }
                });
            },
        )
        .absolute()
        .size_full()
    }
}

fn position_units(position: Pixels, font_size: Pixels) -> u32 {
    ((position / font_size).max(0.) * 1000.).round() as u32
}
