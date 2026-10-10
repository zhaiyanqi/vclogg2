use super::super::{
    DocumentLoadState, LogRegion, PreparedDocument, SearchRange, SearchResult, StateStore, Task,
    Workspace,
};
use super::{RowTag, TagColor, TagStyle, source_digest};
use crate::log_table::VirtualLogListStateExt as _;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};
use std::{collections::BTreeMap, sync::Arc};
use vclogg_core::LogDocument;

#[gpui_kit::test]
fn n_opens_new_mark_dialog_when_the_selected_row_already_has_a_mark(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::actions::init(cx);
        Workspace::init_window_registry(cx);
        crate::notifications::init(cx);
        crate::app_icon::init(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("marked.log");
    std::fs::write(&path, "existing marked line\n").unwrap();
    let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
    let prepared = PreparedDocument {
        document: Arc::new(LogDocument::open(&path).unwrap()),
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
    let mut workspace = None;
    let handle = cx.open_window(size(px(1400.), px(900.)), |window, cx| {
        let view = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
        view.update(cx, |view, cx| {
            view.persistence._bootstrap_task = Task::ready(());
            view.persistence.state_tasks.clear();
            view._cloud_client_bootstrap_task = Task::ready(());
            view.persistence.store = Some(store);
            view.install_documents(
                vec![(path.clone(), Ok(prepared))],
                Some(&path),
                &BTreeMap::new(),
                None,
                true,
                window,
                cx,
            );
            view.documents[0].file.row_tags.insert(
                "existing".into(),
                RowTag {
                    source_row: 0,
                    label: "已有标记".into(),
                    color: TagColor::Neutral,
                    style: TagStyle::default(),
                    x: 10_000,
                    y: 0,
                    source_digest: source_digest("existing marked line"),
                },
            );
            view.documents[0].log_table.update(cx, |table, cx| {
                table.delegate().begin_pointer_selection(0, false, false, 1);
                table.set_active_log_row(0, cx);
                table.refresh(cx);
            });
            view.active_log_region = LogRegion::Body;
            view.log_viewer.focus_handle.focus(window, cx);
        });
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    let workspace = workspace.unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        workspace
            .read(cx)
            .log_viewer
            .focus_handle
            .clone()
            .focus(window, cx);
        window.render_frame(cx);
        assert!(workspace.read(cx).selected_tag_context(cx).is_some());
        window.press("n", cx);
        assert!(
            workspace
                .read(cx)
                .row_tags
                .dialog
                .as_ref()
                .and_then(|dialog| dialog.upgrade())
                .is_some()
        );
        assert_eq!(
            workspace.read(cx).documents[0].file.row_tags.row(0).count(),
            1
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("enter", cx);
        let view = workspace.read(cx);
        let tags = &view.documents[0].file.row_tags;
        assert_eq!(tags.row(0).count(), 2);
        let original = tags.get(0, "existing").unwrap();
        let appended = tags
            .row(0)
            .find(|(id, _)| id.as_str() != "existing")
            .unwrap()
            .1;
        assert!(appended.x > original.x);
    })
    .unwrap();
}
