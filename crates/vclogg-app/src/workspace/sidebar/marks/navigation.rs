use super::*;
use crate::workspace::search_tabs;

/// A single continuation for expansion of a result group. It cannot survive a
/// new click, tab/source change, result replacement, or another group operation.
pub(in crate::workspace) struct PendingMarkResultJump {
    document_id: u64,
    source: std::sync::Weak<LogDocument>,
    key: LogRowKey,
    table_id: gpui_kit::EntityId,
    content_revision: u64,
    layout_revision: u64,
    toggle_revision: u64,
    scope: SearchScope,
    search_tab: Option<(search_tabs::SearchTabOwner, search_tabs::SearchTabId)>,
    activation_revision: u64,
}

impl Workspace {
    pub(in crate::workspace) fn navigate_to_mark(
        &mut self,
        document_id: u64,
        source: Arc<LogDocument>,
        source_row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Cancel only an expansion initiated by a previous mark navigation.
        if let Some(pending) = self.pending_mark_result_jump.take()
            && pending.toggle_revision == self.global_group_toggle_revision
        {
            self.global_group_toggle_task.take();
            self.global_group_toggle_revision = self.global_group_toggle_revision.saturating_add(1);
        }
        let Some(tab) = self.active_document() else {
            return;
        };
        if tab.id != document_id
            || !Arc::ptr_eq(&tab.document, &source)
            || !source.has_complete_line_index()
            || source.local_row(source_row).is_none()
            || (!tab.file.marked_rows.contains(source_row)
                && tab.file.row_tags.row(source_row).next().is_none())
        {
            return;
        }
        if !self.select_and_center_log_source_row_atomically(document_id, source_row, window, cx) {
            return;
        }
        if let Some(tab) = self.documents.iter_mut().find(|tab| tab.id == document_id) {
            tab.view.auto_follow = false;
            tab.view.selection_table = SelectionTable::Log;
        }
        self.selected_source_row = Some(source_row);
        self.remember_user_log_region(LogRegion::Body);
        self.log_viewer.focus_handle.focus(window, cx);

        match self.global_search.scope {
            SearchScope::CurrentFile => {
                let Some(tab) = self.active_document().filter(|tab| tab.results_visible) else {
                    cx.notify();
                    return;
                };
                let table = tab.result_table.clone();
                let key = LogRowKey::Row {
                    document_id,
                    source_row,
                };
                let row_ix = table.read(cx).delegate().row_ix_for_key(key);
                if let Some(row_ix) = row_ix {
                    if !table
                        .read(cx)
                        .delegate()
                        .visible_document()
                        .same_source_snapshot(&source)
                    {
                        cx.notify();
                        return;
                    }
                    tab.result_viewport.take_pending_scrollbar_offset();
                    let viewport = table.read(cx).viewport().clone();
                    self.invalidate_log_scroll_frame((document_id, WrappedRegion::Results));
                    table.update(cx, |table, cx| {
                        table.delegate().settle_table_selection(row_ix);
                        // Do not emit a result-row event: the body already owns focus.
                        table.delegate().set_active_log_row(Some(row_ix));
                        cx.notify();
                    });
                    VirtualLogListScrollHandle::new(&viewport)
                        .scroll_to_item(row_ix, ScrollStrategy::Center);
                }
            }
            SearchScope::AllOpenFiles | SearchScope::Directory
                if self.global_search.results_visible =>
            {
                let table = self.global_table.read(cx);
                let delegate = table.delegate();
                let Some(key) = delegate.source_row_key_for_document(&source, source_row) else {
                    cx.notify();
                    return;
                };
                if let Some(row_ix) = delegate.row_ix_for_key(key) {
                    self.select_mark_global_result(row_ix, cx);
                } else {
                    let LogRowKey::Row {
                        document_id: group_id,
                        ..
                    } = key
                    else {
                        return;
                    };
                    self.pending_mark_result_jump = Some(PendingMarkResultJump {
                        document_id,
                        source: Arc::downgrade(&source),
                        key,
                        table_id: self.global_table.entity_id(),
                        content_revision: delegate.content_revision(),
                        layout_revision: delegate.layout_revision().saturating_add(1),
                        toggle_revision: self.global_group_toggle_revision.saturating_add(1),
                        scope: self.global_search.scope,
                        search_tab: self.active_search_tab_key(),
                        activation_revision: self.tab_activation_revision,
                    });
                    let anchor = Some(RowViewportAnchor {
                        key,
                        viewport_y: px(0.),
                        fallback_ix: 0,
                    });
                    self.prepare_global_group_toggle(
                        group_id,
                        anchor,
                        BTreeMap::new(),
                        self.log_row_height(),
                        window,
                        cx,
                    );
                }
            }
            _ => {}
        }
        cx.notify();
    }

    pub(in crate::workspace) fn mark_group_expansion_is_current(&mut self, cx: &App) -> bool {
        let Some(pending) = &self.pending_mark_result_jump else {
            return true;
        };
        if pending.toggle_revision != self.global_group_toggle_revision {
            self.pending_mark_result_jump = None;
            return true;
        }
        let LogRowKey::Row { source_row, .. } = pending.key else {
            self.pending_mark_result_jump = None;
            return false;
        };
        let current = self.active_document().is_some_and(|tab| {
            tab.id == pending.document_id
                && (tab.file.marked_rows.contains(source_row)
                    || tab.file.row_tags.row(source_row).next().is_some())
                && pending
                    .source
                    .upgrade()
                    .is_some_and(|source| Arc::ptr_eq(&tab.document, &source))
        }) && self.global_search.scope == pending.scope
            && self.selected_source_row == Some(source_row)
            && self.active_search_tab_key() == pending.search_tab
            && self.tab_activation_revision == pending.activation_revision
            && self.global_search.results_visible
            && self.global_table.entity_id() == pending.table_id
            && self.global_table.read(cx).delegate().content_revision() == pending.content_revision
            && self
                .global_table
                .read(cx)
                .delegate()
                .layout_revision()
                .saturating_add(1)
                == pending.layout_revision;
        if !current {
            self.pending_mark_result_jump = None;
        }
        current
    }

    pub(in crate::workspace) fn complete_mark_result_jump(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_mark_result_jump.take() else {
            return;
        };
        let Some(tab) = self.active_document() else {
            return;
        };
        if tab.id != pending.document_id
            || !pending
                .source
                .upgrade()
                .is_some_and(|source| Arc::ptr_eq(&tab.document, &source))
            || self.global_search.scope != pending.scope
            || self.active_search_tab_key() != pending.search_tab
            || self.tab_activation_revision != pending.activation_revision
            || !self.global_search.results_visible
            || self.global_table.entity_id() != pending.table_id
            || self.global_group_toggle_revision != pending.toggle_revision
        {
            return;
        }
        let delegate = self.global_table.read(cx).delegate();
        if delegate.content_revision() != pending.content_revision
            || delegate.layout_revision() != pending.layout_revision
        {
            return;
        }
        if let Some(row_ix) = delegate.row_ix_for_key(pending.key) {
            self.select_mark_global_result(row_ix, cx);
        }
    }

    fn select_mark_global_result(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        self.invalidate_log_scroll_frame((0, WrappedRegion::GlobalResults));
        self.global_viewport.take_pending_scrollbar_offset();
        self.global_table.update(cx, |table, cx| {
            table.delegate().settle_table_selection(row_ix);
            table.delegate().set_active_log_row(Some(row_ix));
            cx.notify();
        });
        self.global_viewport.center_row(row_ix);
    }
}
