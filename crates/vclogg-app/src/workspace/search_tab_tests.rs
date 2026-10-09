use super::search_tabs::{SearchTabGroup, SearchTabId, SearchTabOwner, SearchTabs};
use crate::search_context::{PersistedSearchTab, PersistedSearchTabGroup, SearchTabQuery};
use gpui_kit::TestAppContext;

fn group(ids: &[u64], active: u64) -> SearchTabGroup {
    SearchTabGroup::restored(PersistedSearchTabGroup {
        active,
        next_id: 0,
        tabs: ids
            .iter()
            .map(|&id| PersistedSearchTab {
                id,
                draft: SearchTabQuery {
                    text: format!("query {id}"),
                    ..Default::default()
                },
                ..Default::default()
            })
            .collect(),
    })
}

#[test]
fn scope_selection_restores_file_selection_but_uses_first_global_or_directory_tab() {
    let mut tabs = group(&[1, 3, 2], 2);
    assert_eq!(
        tabs.scope_target(SearchTabOwner::File(10)),
        Some(SearchTabId(2))
    );
    for owner in [SearchTabOwner::AllOpen, SearchTabOwner::Directory] {
        assert_eq!(tabs.scope_target(owner), Some(SearchTabId(1)));
        tabs.tabs.swap(0, 1);
        assert_eq!(tabs.scope_target(owner), Some(SearchTabId(3)));
        tabs.tabs.swap(0, 1);
    }
    assert_eq!(tabs.active, SearchTabId(2));
}

#[test]
fn closing_the_last_shared_tab_stays_closed_after_restore() {
    for owner in [SearchTabOwner::AllOpen, SearchTabOwner::Directory] {
        let mut tabs = group(&[5], 5);
        tabs.close(owner, SearchTabId(5));
        let restored = SearchTabGroup::restored(tabs.persisted());
        assert!(restored.tabs.is_empty());
        assert_eq!(restored.scope_target(owner), None);
        assert_eq!(restored.next_id, 6);
    }
}

#[test]
fn closing_the_last_file_search_preserves_the_existing_session() {
    let owner = SearchTabOwner::File(10);
    let mut tabs = group(&[4, 5], 5);
    assert!(tabs.is_closable(owner, SearchTabId(4)));
    tabs.close(owner, SearchTabId(4));
    tabs.tabs[0].saved.completed = Some(tabs.tabs[0].saved.draft.clone());
    tabs.tabs[0].saved.name = Some("custom title".into());
    assert!(!tabs.is_closable(owner, SearchTabId(5)));
    tabs.close(owner, SearchTabId(5));
    assert_eq!(tabs.active, SearchTabId(5));
    assert_eq!(tabs.tabs.len(), 1);
    assert_eq!(tabs.tabs[0].saved.draft.text, "query 5");
    assert!(tabs.tabs[0].saved.completed.is_some());
    assert_eq!(tabs.tabs[0].saved.name.as_deref(), Some("custom title"));
    assert!(!tabs.tabs[0].facade_dirty);
    let restored = SearchTabGroup::restored(tabs.persisted());
    assert_eq!(restored.scope_target(owner), Some(SearchTabId(5)));
    assert_eq!(restored.next_id, 6);
}

#[test]
fn closing_an_inactive_search_keeps_selection_and_draft() {
    let mut tabs = group(&[1, 2, 3], 2);
    tabs.close(SearchTabOwner::AllOpen, SearchTabId(1));
    assert_eq!(tabs.active, SearchTabId(2));
    assert_eq!(tabs.tabs[0].saved.draft.text, "query 2");
    tabs.close(SearchTabOwner::AllOpen, SearchTabId(2));
    assert_eq!(tabs.active, SearchTabId(3));
}

#[test]
fn restored_search_tabs_preserve_order_and_repair_duplicate_ids() {
    let tabs = group(&[0, 3, 1, 3], 999);
    assert_eq!(
        tabs.tabs.iter().map(|tab| tab.saved.id).collect::<Vec<_>>(),
        vec![3, 1]
    );
    assert_eq!(tabs.active, SearchTabId(3));
    assert_eq!(tabs.next_id, 4);
    assert!(group(&[0], 0).tabs.is_empty());
}

#[gpui_kit::test]
fn shared_search_tabs_remain_visible_when_switching_or_closing_files(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut tabs = SearchTabs::new(cx);
        tabs.groups
            .insert(SearchTabOwner::File(10), group(&[1, 2], 2));
        tabs.groups.insert(SearchTabOwner::File(20), group(&[1], 1));
        tabs.groups
            .insert(SearchTabOwner::AllOpen, group(&[1, 2], 2));
        tabs.groups
            .insert(SearchTabOwner::Directory, group(&[1], 1));
        let shared = vec![
            (SearchTabOwner::AllOpen, SearchTabId(1)),
            (SearchTabOwner::AllOpen, SearchTabId(2)),
            (SearchTabOwner::Directory, SearchTabId(1)),
        ];
        let visible = tabs.visible_keys();
        assert_eq!(
            visible[..3],
            [
                (SearchTabOwner::File(10), SearchTabId(1)),
                (SearchTabOwner::File(10), SearchTabId(2)),
                (SearchTabOwner::File(20), SearchTabId(1)),
            ]
        );
        assert_eq!(visible[3..], shared);
        tabs.groups.remove(&SearchTabOwner::File(20));
        let remaining = tabs.visible_keys();
        assert_eq!(remaining[..2], visible[..2]);
        assert_eq!(remaining[2..], shared);
        assert_eq!(tabs.groups[&SearchTabOwner::AllOpen].active, SearchTabId(2));
        assert_eq!(
            tabs.groups[&SearchTabOwner::File(10)].active,
            SearchTabId(2)
        );
    });
}

#[test]
fn closing_a_remembered_file_tab_defers_facade_capture_until_replacement_is_installed() {
    let mut tabs = group(&[1, 2], 1);
    tabs.close(SearchTabOwner::File(10), SearchTabId(1));
    assert_eq!(tabs.active, SearchTabId(2));
    assert_eq!(tabs.tabs[0].saved.draft.text, "query 2");
    assert!(tabs.tabs[0].facade_dirty);
}

#[gpui_kit::test]
fn search_tab_menu_clicks_and_keyboard_share_scope_selection(cx: &mut TestAppContext) {
    use super::{SearchScope, Workspace};
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, Task, px, size};

    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::actions::init(cx);
        Workspace::init_window_registry(cx);
        crate::notifications::init(cx);
        crate::app_icon::init(cx);
    });
    let mut workspace = None;
    let handle = cx.open_window(size(px(1400.), px(900.)), |window, cx| {
        let view = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
        view.update(cx, |view, _| {
            view.persistence._bootstrap_task = Task::ready(());
            view.persistence.state_tasks.clear();
            view._cloud_client_bootstrap_task = Task::ready(());
        });
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    let workspace = workspace.unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("add-search-tab", cx);
        assert!(
            window.find("popup-menu").bounds().bottom()
                <= window.find("add-search-tab").bounds().top(),
            "the add menu must open above its trigger"
        );
        window.within("popup-menu").click(0usize, cx);
        assert!(workspace.read(cx).active_search_tab_key().is_none());
        window.within("popup-menu").click(1usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            workspace.read(cx).global_search.scope,
            SearchScope::AllOpenFiles
        );
        window.click("add-search-tab", cx);
        window.within("popup-menu").click(2usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            workspace.read(cx).global_search.scope,
            SearchScope::Directory
        );
        assert!(window.find("search-tab-AllOpen-1").visible());
        window.click("search-tab-AllOpen-1", cx);
        assert_eq!(
            workspace.read(cx).global_search.scope,
            SearchScope::AllOpenFiles
        );
        window.press("right", cx);
        assert_eq!(
            workspace.read(cx).global_search.scope,
            SearchScope::Directory
        );
        window.click("add-search-tab", cx);
        window.within("popup-menu").click(1usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            workspace.read(cx).active_search_tab_key(),
            Some((SearchTabOwner::AllOpen, SearchTabId(2)))
        );
        assert_eq!(
            workspace.read(cx).visible_search_tab_keys(),
            vec![
                (SearchTabOwner::AllOpen, SearchTabId(1)),
                (SearchTabOwner::Directory, SearchTabId(1)),
                (SearchTabOwner::AllOpen, SearchTabId(2)),
            ]
        );
        window.click("search-scope", cx);
        window.within("popup-menu").click(1usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            workspace.read(cx).active_search_tab_key(),
            Some((SearchTabOwner::AllOpen, SearchTabId(1)))
        );
        window.click("close-search-tab-Directory-1", cx);
        assert_eq!(
            workspace.read(cx).active_search_tab_key(),
            Some((SearchTabOwner::AllOpen, SearchTabId(1)))
        );
        assert!(window.try_find("search-tab-Directory-1").is_none());
        window.click("search-scope", cx);
        window.within("popup-menu").click(2usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            workspace.read(cx).active_search_tab_key(),
            Some((SearchTabOwner::Directory, SearchTabId(2)))
        );
        window.click("toggle-search-panel", cx);
        assert!(!workspace.read(cx).search_panel_expanded());
        assert!(window.find("search-tab-AllOpen-1").visible());
        window.click("search-tab-AllOpen-1", cx);
        assert!(workspace.read(cx).search_panel_expanded());
    })
    .unwrap();
}

#[gpui_kit::test]
fn search_tab_list_opens_upward_and_switches_and_closes_tabs(cx: &mut TestAppContext) {
    use super::Workspace;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, Task, px, size};

    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::actions::init(cx);
        Workspace::init_window_registry(cx);
        crate::notifications::init(cx);
        crate::app_icon::init(cx);
    });
    let mut workspace = None;
    let handle = cx.open_window(size(px(1400.), px(900.)), |window, cx| {
        let view = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
        view.update(cx, |view, _| {
            view.persistence._bootstrap_task = Task::ready(());
            view.persistence.state_tasks.clear();
            view._cloud_client_bootstrap_task = Task::ready(());
        });
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    let workspace = workspace.unwrap();
    for scope in [1usize, 2, 1] {
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("add-search-tab", cx);
            window.within("popup-menu").click(scope, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("search-tab-list", cx);
        assert!(
            window.find("search-tab-list").bounds().left()
                >= window.find("add-search-tab").bounds().right()
        );
        assert!(
            window.find("popup-menu").bounds().bottom()
                <= window.find("search-tab-list").bounds().top()
        );
        let first = window.find("search-tab-list-close-AllOpen-1").bounds();
        let second = window.find("search-tab-list-close-Directory-1").bounds();
        let third = window.find("search-tab-list-close-AllOpen-2").bounds();
        assert!(first.top() < second.top() && second.top() < third.top());
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        let active = Some((SearchTabOwner::AllOpen, SearchTabId(1)));
        assert_eq!(workspace.read(cx).active_search_tab_key(), active);
        window.click("search-tab-list", cx);
        window.click("search-tab-list-close-AllOpen-2", cx);
        assert_eq!(workspace.read(cx).active_search_tab_key(), active);
        assert!(window.find("popup-menu").visible());
        assert!(window.try_find("search-tab-list-close-AllOpen-2").is_none());
        assert!(
            workspace
                .read(cx)
                .search_tabs
                .closing
                .iter()
                .any(|tab| { tab.key == (SearchTabOwner::AllOpen, SearchTabId(2)) })
        );
        assert!(window.try_find("closing-search-tab-AllOpen-2").is_some());
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none());
        window.click("search-tab-list", cx);
        window.click("search-tab-list-close-AllOpen-1", cx);
        assert_eq!(
            workspace.read(cx).active_search_tab_key(),
            Some((SearchTabOwner::Directory, SearchTabId(1)))
        );
        assert!(window.find("popup-menu").visible());
        window.click("search-tab-list-close-Directory-1", cx);
        assert!(workspace.read(cx).visible_search_tab_keys().is_empty());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none());
    })
    .unwrap();
    cx.background_executor
        .advance_clock(std::time::Duration::from_millis(200));
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(workspace.read(cx).search_tabs.closing.is_empty());
        assert!(window.try_find("closing-search-tab-AllOpen-2").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn file_switches_restore_local_queries_without_leaving_shared_searches(cx: &mut TestAppContext) {
    use super::{DocumentLoadState, PreparedDocument, SearchScope, Workspace, WorkspaceTabId};
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, Task, px, size};
    use std::{collections::BTreeMap, sync::Arc};
    use vclogg_core::{LogDocument, SearchRange, SearchResult};

    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::actions::init(cx);
        Workspace::init_window_registry(cx);
        crate::notifications::init(cx);
        crate::app_icon::init(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let paths = [
        directory.path().join("a.log"),
        directory.path().join("b.log"),
    ];
    let prepared = paths
        .iter()
        .map(|path| {
            std::fs::write(path, "error\ninfo\n").unwrap();
            (
                path.clone(),
                Ok(PreparedDocument {
                    document: Arc::new(LogDocument::open(path).unwrap()),
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
                }),
            )
        })
        .collect();
    let mut workspace = None;
    let handle = cx.open_window(size(px(1400.), px(900.)), |window, cx| {
        let view = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
        view.update(cx, |view, cx| {
            view.persistence._bootstrap_task = Task::ready(());
            view.persistence.state_tasks.clear();
            view._cloud_client_bootstrap_task = Task::ready(());
            view.install_documents(
                prepared,
                Some(&paths[0]),
                &BTreeMap::new(),
                None,
                true,
                window,
                cx,
            );
        });
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    let workspace = workspace.unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        workspace.update(cx, |view, cx| {
            let a = view.documents[0].id;
            let b = view.documents[1].id;
            // Two local sessions belong to A; returning to A restores the selected one.
            view.search_tabs
                .groups
                .insert(SearchTabOwner::File(a), group(&[1, 2], 1));
            view.activate_search_tab(SearchTabOwner::File(a), SearchTabId(2), window, cx);
            view.query
                .update(cx, |query, cx| query.set_value("local A", window, cx));
            view.commit_workspace_tab_activation(WorkspaceTabId::Document(b), false, window, cx);
            assert_eq!(
                view.active_search_tab_key(),
                Some((SearchTabOwner::File(b), SearchTabId(1)))
            );
            view.query
                .update(cx, |query, cx| query.set_value("local B", window, cx));
            view.commit_workspace_tab_activation(WorkspaceTabId::Document(a), false, window, cx);
            assert_eq!(
                view.active_search_tab_key(),
                Some((SearchTabOwner::File(a), SearchTabId(2)))
            );
            assert_eq!(view.query.read(cx).value(), "local A");
            for scope in [SearchScope::AllOpenFiles, SearchScope::Directory] {
                view.set_search_scope(scope, window, cx);
                let selected = view.active_search_tab_key();
                view.query
                    .update(cx, |query, cx| query.set_value("shared query", window, cx));
                view.commit_workspace_tab_activation(
                    WorkspaceTabId::Document(b),
                    false,
                    window,
                    cx,
                );
                assert_eq!(view.active_search_tab_key(), selected);
                assert_eq!(view.query.read(cx).value(), "shared query");
                view.set_search_scope(SearchScope::CurrentFile, window, cx);
                assert_eq!(
                    view.active_search_tab_key(),
                    Some((SearchTabOwner::File(b), SearchTabId(1)))
                );
                assert_eq!(view.query.read(cx).value(), "local B");
                view.commit_workspace_tab_activation(
                    WorkspaceTabId::Document(a),
                    false,
                    window,
                    cx,
                );
                assert_eq!(view.query.read(cx).value(), "local A");
            }
        });
        window.render_frame(cx);
        let a = workspace.read(cx).documents[0].id;
        let b = workspace.read(cx).documents[1].id;
        let a_tab = format!("search-tab-File({a})-2");
        let b_tab = format!("search-tab-File({b})-1");
        assert_eq!(window.find(a_tab.clone()).label(), Some("a.log_2"));
        assert_eq!(window.find(b_tab.clone()).label(), Some("b.log_1"));
        window.click(b_tab, cx);
        assert_eq!(
            workspace.read(cx).active_tab_id,
            WorkspaceTabId::Document(b)
        );
        assert_eq!(workspace.read(cx).query.read(cx).value(), "local B");
        assert!(window.find(a_tab.clone()).visible());
        window.click(a_tab, cx);
        assert_eq!(
            workspace.read(cx).active_tab_id,
            WorkspaceTabId::Document(a)
        );
        assert_eq!(workspace.read(cx).query.read(cx).value(), "local A");
        // Closing an unrelated file's last search neither changes the active file nor its query.
        window.click(format!("close-search-tab-File({b})-1"), cx);
        assert_eq!(
            workspace.read(cx).active_tab_id,
            WorkspaceTabId::Document(a)
        );
        assert_eq!(workspace.read(cx).query.read(cx).value(), "local A");
        let b_remaining = format!("search-tab-File({b})-1");
        assert_eq!(window.find(b_remaining.clone()).label(), Some("b.log_1"));
        assert!(window.try_find(format!("search-tab-File({b})-2")).is_none());
        window.click("search-tab-list", cx);
        window.click(format!("search-tab-list-close-File({b})-1"), cx);
        assert!(
            workspace
                .read(cx)
                .search_tabs
                .state(SearchTabOwner::File(b), SearchTabId(1))
                .is_some()
        );
        window.press("escape", cx);
        window.click(b_remaining, cx);
        assert_eq!(
            workspace.read(cx).active_tab_id,
            WorkspaceTabId::Document(b)
        );
        for key in ["delete", "backspace"] {
            window.press(key, cx);
            assert_eq!(workspace.read(cx).query.read(cx).value(), "local B");
            assert_eq!(
                workspace.read(cx).active_search_tab_key(),
                Some((SearchTabOwner::File(b), SearchTabId(1)))
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn newly_opened_file_searches_append_after_existing_scopes(cx: &mut TestAppContext) {
    use super::Workspace;
    use gpui_kit::component::Root;
    use gpui_kit::{AppContext as _, Task, px, size};

    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::actions::init(cx);
        Workspace::init_window_registry(cx);
        crate::notifications::init(cx);
        crate::app_icon::init(cx);
    });
    cx.open_window(size(px(1400.), px(900.)), |window, cx| {
        let view = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
        view.update(cx, |view, cx| {
            view.persistence._bootstrap_task = Task::ready(());
            view.persistence.state_tasks.clear();
            view._cloud_client_bootstrap_task = Task::ready(());
            // Include legacy sessions without saved positions and a file whose ID
            // sorts before the previous file, as can happen after moving windows.
            view.search_tabs
                .groups
                .insert(SearchTabOwner::Directory, group(&[1], 1));
            let mut expected = vec![(SearchTabOwner::Directory, SearchTabId(1))];
            for owner in [
                SearchTabOwner::File(20),
                SearchTabOwner::AllOpen,
                SearchTabOwner::File(10),
            ] {
                view.ensure_search_tab_group(owner, cx);
                expected.push((owner, SearchTabId(1)));
                assert_eq!(view.visible_search_tab_keys(), expected);
            }
            assert!(view.search_tabs.reorder(expected[3], 0));
            expected.rotate_right(1);
            view.ensure_search_tab_group(SearchTabOwner::File(30), cx);
            expected.push((SearchTabOwner::File(30), SearchTabId(1)));
            assert_eq!(view.visible_search_tab_keys(), expected);
            view.search_tabs
                .groups
                .get_mut(&SearchTabOwner::AllOpen)
                .unwrap()
                .close(SearchTabOwner::AllOpen, SearchTabId(1));
            expected.retain(|(owner, _)| *owner != SearchTabOwner::AllOpen);
            view.ensure_search_tab_group(SearchTabOwner::AllOpen, cx);
            expected.push((SearchTabOwner::AllOpen, SearchTabId(2)));
            assert_eq!(view.visible_search_tab_keys(), expected);
        });
        Root::new(view, window, cx)
    });
}

#[gpui_kit::test]
fn search_tab_reordering_crosses_scopes_and_round_trips(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut tabs = SearchTabs::new(cx);
        tabs.groups.insert(SearchTabOwner::File(10), group(&[1], 1));
        tabs.groups
            .insert(SearchTabOwner::AllOpen, group(&[1, 2], 2));
        tabs.groups
            .insert(SearchTabOwner::Directory, group(&[1], 1));
        let directory = (SearchTabOwner::Directory, SearchTabId(1));
        assert!(tabs.reorder(directory, 0));
        assert!(tabs.reorder((SearchTabOwner::AllOpen, SearchTabId(2)), 1));
        let expected = tabs.visible_keys();
        assert_eq!(expected[0], directory);
        assert_eq!(expected[1], (SearchTabOwner::AllOpen, SearchTabId(2)));
        assert_eq!(
            tabs.groups[&SearchTabOwner::AllOpen].scope_target(SearchTabOwner::AllOpen),
            Some(SearchTabId(2))
        );
        let saved = tabs
            .groups
            .iter()
            .map(|(owner, group)| {
                let json = serde_json::to_string(&group.persisted()).unwrap();
                (
                    *owner,
                    SearchTabGroup::restored(serde_json::from_str(&json).unwrap()),
                )
            })
            .collect();
        tabs.groups = saved;
        assert_eq!(tabs.visible_keys(), expected);
        assert_eq!(tabs.groups[&SearchTabOwner::AllOpen].active, SearchTabId(2));
    });
}

#[gpui_kit::test]
fn search_tab_strip_toggle_stays_fixed_while_tabs_scroll(cx: &mut TestAppContext) {
    use super::{SearchScope, Workspace};
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, ScrollDelta, Task, point, px, size};

    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::actions::init(cx);
        Workspace::init_window_registry(cx);
        crate::notifications::init(cx);
        crate::app_icon::init(cx);
    });
    let mut workspace = None;
    let handle = cx.open_window(size(px(1000.), px(700.)), |window, cx| {
        let view = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
        view.update(cx, |view, cx| {
            view.persistence._bootstrap_task = Task::ready(());
            view.persistence.state_tasks.clear();
            view._cloud_client_bootstrap_task = Task::ready(());
            let mut tabs = group(&(1..=20).collect::<Vec<_>>(), 1);
            for tab in &mut tabs.tabs {
                tab.saved.name = Some(format!("Long search tab title {}", tab.saved.id));
            }
            view.search_tabs
                .groups
                .insert(SearchTabOwner::AllOpen, tabs);
            view.global_search.scope = SearchScope::AllOpenFiles;
            view.sync_search_tab(window, cx);
        });
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    let workspace = workspace.unwrap();
    let executor = cx.background_executor.clone();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let button = window.find("toggle-search-tab-strip").bounds();
        let container = window.find("search-tabs").bounds();
        let track = window.find("search-tab-viewport").bounds();
        assert_eq!(button.right(), container.right());
        assert!(track.right() <= button.left());
        assert!(workspace.read(cx).search_tabs.scroll.max_offset().x > px(0.));
        let before = workspace.read(cx).search_tabs.scroll.offset();
        window.scroll(
            "search-tab-viewport",
            ScrollDelta::Pixels(point(px(-300.), px(0.))),
            cx,
        );
        let scrolled = workspace.read(cx).search_tabs.scroll.offset();
        assert!(scrolled.x < before.x);
        assert_eq!(window.find("toggle-search-tab-strip").bounds(), button);
        let active = workspace.read(cx).active_search_tab_key();
        let keys = workspace.read(cx).visible_search_tab_keys();
        cx.set_reduce_motion(true);
        window.click("toggle-search-tab-strip", cx);
        assert!(workspace.read(cx).search_tabs.strip_collapsed);
        assert_eq!(
            window.find("search-tab-viewport").bounds().size.width,
            px(0.)
        );
        assert!(window.find("toggle-search-tab-strip").bounds().left() < button.left());
        let add = window.find("add-search-tab").bounds();
        let list = window.find("search-tab-list").bounds();
        let toggle = window.find("toggle-search-tab-strip").bounds();
        assert_eq!(toggle.left() - list.right(), list.left() - add.right());
        assert_eq!(workspace.read(cx).active_search_tab_key(), active);
        assert_eq!(workspace.read(cx).visible_search_tab_keys(), keys);
        window.click("toggle-search-tab-strip", cx);
        assert!(!workspace.read(cx).search_tabs.strip_collapsed);
        assert!(window.find("search-tab-viewport").visible());
        assert_eq!(window.find("toggle-search-tab-strip").bounds(), button);
        assert_eq!(workspace.read(cx).search_tabs.scroll.offset(), scrolled);
        workspace.update(cx, |view, cx| {
            view.search_tabs
                .groups
                .get_mut(&SearchTabOwner::AllOpen)
                .unwrap()
                .tabs
                .truncate(1);
            cx.notify();
        });
        window.render_frame(cx);
        let last = window.find("search-tab-AllOpen-1").bounds();
        let toggle = window.find("toggle-search-tab-strip").bounds();
        assert!(toggle.left() >= last.right());
        assert_eq!(toggle.left() - last.right(), list.left() - add.right());
        let close = window.find("close-search-tab-AllOpen-1").bounds();
        assert!(last.right() - close.right() <= px(1.));
        assert!(toggle.right() < container.right());
        cx.set_reduce_motion(false);
        window.click("toggle-search-tab-strip", cx);
        let initial_width = window.find("search-tab-viewport").bounds().size.width;
        for _ in 0..5 {
            executor.advance_clock(std::time::Duration::from_millis(16));
            window.simulate_next_frame(cx);
            window.render_frame(cx);
        }
        let intermediate_width = window.find("search-tab-viewport").bounds().size.width;
        assert!(intermediate_width > px(0.) && intermediate_width < initial_width);
        // Retarget the moving strip without racing a pointer hit against the next frame.
        workspace.update(cx, |view, cx| {
            view.search_tabs.strip_collapsed = false;
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("search-tab-viewport").bounds().size.width < initial_width);
        cx.set_reduce_motion(true);
        window.render_frame(cx);
        assert!(!workspace.read(cx).search_tabs.strip_collapsed);
        assert_eq!(window.find("toggle-search-tab-strip").bounds(), toggle);
    })
    .unwrap();
}

#[gpui_kit::test]
fn search_tabs_reorder_during_drag_and_keep_labels_inside_the_strip(cx: &mut TestAppContext) {
    use super::{SearchScope, Workspace};
    use gpui_kit::component::{Root, Theme};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, InputEvent as _, MouseButton, MouseDownEvent, MouseMoveEvent,
        MouseUpEvent, Task, point, px, size,
    };

    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::actions::init(cx);
        Workspace::init_window_registry(cx);
        crate::notifications::init(cx);
        crate::app_icon::init(cx);
    });
    let mut workspace = None;
    let handle = cx.open_window(size(px(1400.), px(900.)), |window, cx| {
        let view = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
        view.update(cx, |view, cx| {
            view.persistence._bootstrap_task = Task::ready(());
            view.persistence.state_tasks.clear();
            view._cloud_client_bootstrap_task = Task::ready(());
            let mut global = group(&[1, 2], 1);
            global.tabs[0].saved.name = Some("中文上下边缘 gjpq_1".into());
            view.search_tabs
                .groups
                .insert(SearchTabOwner::AllOpen, global);
            view.search_tabs
                .groups
                .insert(SearchTabOwner::Directory, group(&[1], 1));
            view.global_search.scope = SearchScope::AllOpenFiles;
            view.sync_search_tab(window, cx);
        });
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    let workspace = workspace.unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let source_bounds = window.find("search-tab-Directory-1").bounds();
        let from = source_bounds.center();
        let to = window.find("search-tab-AllOpen-1").bounds().center();
        window.dispatch_event(
            MouseDownEvent {
                position: from,
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        for fraction in [0.15, 0.5, 1.] {
            window.dispatch_event(
                MouseMoveEvent {
                    position: point(
                        from.x + (to.x - from.x) * fraction,
                        if fraction == 0.15 {
                            from.y
                        } else {
                            from.y - px(120.)
                        },
                    ),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
        }
        // Assert before releasing the mouse: this must be live sorting, not drop-only sorting.
        let directory = (SearchTabOwner::Directory, SearchTabId(1));
        assert_eq!(workspace.read(cx).visible_search_tab_keys()[0], directory);
        assert_eq!(workspace.read(cx).search_tabs.hidden_drag, Some(directory));
        assert_eq!(
            window.find("search-tab-drag-preview").bounds().size,
            source_bounds.size
        );
        assert!(
            workspace
                .read(cx)
                .search_tabs
                .motion_offsets
                .values()
                .any(|offset| *offset != px(0.))
        );
        let layout = workspace.read(cx).search_tabs.layout.borrow();
        assert!(
            layout
                .painted
                .iter()
                .any(|(key, bounds)| bounds.left() != layout.slots[key].left()),
            "reordered tabs should be travelling between old and new slots"
        );
        drop(layout);
        window.dispatch_event(
            MouseUpEvent {
                position: point(to.x, from.y - px(120.)),
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
        assert!(workspace.read(cx).tab_drag.flight.is_some());
        window.render_frame(cx);
        assert!(
            window.find("search-tab-drag-preview").bounds().top()
                < window.find("search-tab-Directory-1").bounds().top()
        );
        assert!(!workspace.read(cx).search_tabs.dragging);
        assert_eq!(workspace.read(cx).search_tabs.hidden_drag, Some(directory));
        cx.set_reduce_motion(true);
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).visible_search_tab_keys()[0], directory);
        for font_size in [13., 16., 22.] {
            Theme::update(cx, |theme| theme.font_size = px(font_size));
            window.render_frame(cx);
            let tab = window.find("search-tab-AllOpen-1").bounds();
            let label = window.find("search-tab-label-AllOpen-1").bounds();
            assert!(label.top() >= tab.top());
            assert!(label.bottom() <= tab.bottom());
            assert!(
                label.size.height >= px(font_size),
                "CJK and descenders need a full line box"
            );
        }
        window.click("search-tab-AllOpen-1", cx);
        window.press("alt-left", cx);
        assert_eq!(
            workspace.read(cx).visible_search_tab_keys()[0],
            (SearchTabOwner::AllOpen, SearchTabId(1))
        );
    })
    .unwrap();
    cx.run_until_parked();
    for _ in 0..10 {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
    }
    cx.update(|cx| assert_eq!(workspace.read(cx).search_tabs.hidden_drag, None));
}
