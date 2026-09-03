use std::{fs, time::SystemTime};

use super::*;

struct TemporaryFile(PathBuf);

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        _ = fs::remove_file(&self.0);
    }
}

#[test]
fn clipboard_collection_iterates_compressed_rows_in_document_order() {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("测试时间应晚于 Unix epoch")
        .as_nanos();
    let temporary = TemporaryFile(std::env::temp_dir().join(format!(
        "vclogg2-line-copy-{}-{nonce}.log",
        std::process::id()
    )));
    fs::write(&temporary.0, b"alpha\nbeta\ngamma").expect("应能创建复制测试日志");
    let document = Arc::new(LogDocument::open(&temporary.0).expect("应能打开复制测试日志"));

    let copied = collect_log_lines_for_clipboard(
        vec![(document, [0, 2].into_iter().collect())],
        true,
        &SearchCancellation::default(),
    )
    .expect("未取消的复制应完成");

    assert_eq!(copied.text, "1\talpha\n3\tgamma");
    assert_eq!(copied.count, 2);
    assert_eq!(copied.first_source_row, Some(0));
}

#[test]
fn cancelled_clipboard_collection_does_not_read_documents() {
    let cancellation = SearchCancellation::default();
    cancellation.cancel();
    let document = Arc::new(LogDocument::placeholder("cancelled-copy.log"));

    assert!(
        collect_log_lines_for_clipboard(
            vec![(document, [0].into_iter().collect())],
            false,
            &cancellation,
        )
        .is_cancelled()
    );
}

#[test]
fn changed_sources_abort_copy_and_color_keyword_tasks() {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("测试时间应晚于 Unix epoch")
        .as_nanos();
    let temporary = TemporaryFile(std::env::temp_dir().join(format!(
        "vclogg2-changed-line-task-{}-{nonce}.log",
        std::process::id()
    )));
    fs::write(&temporary.0, b"alpha\n").expect("应能创建原始行任务日志");
    let document = Arc::new(LogDocument::open(&temporary.0).expect("应能打开原始行任务日志"));
    fs::write(&temporary.0, b"omega\n").expect("应能原地改写行任务日志");

    assert!(matches!(
        collect_log_lines_for_clipboard(
            vec![(document.clone(), [0].into_iter().collect())],
            false,
            &SearchCancellation::default(),
        ),
        DocumentLineTask::SourceUnavailable
    ));
    assert!(matches!(
        prepare_color_keywords(
            ColorKeywordTarget {
                document_id: 3,
                document: document.clone(),
                selection: ColorKeywordSelection::Rows([0].into_iter().collect()),
            },
            true,
            &SearchCancellation::default(),
        ),
        DocumentLineTask::SourceUnavailable
    ));
    let matcher = SearchMatcher::quick_find("alpha", true, false, false)
        .expect("页内查找表达式应有效")
        .expect("非空页内查找应生成匹配器");
    assert!(matches!(
        Workspace::find_quick_match(
            QuickFindSource::Document {
                document,
                rows: None,
                row_count: 1,
            },
            QuickFindTarget::Log(3),
            matcher,
            QuickFindDirection::Forward,
            0,
            SearchCancellation::default(),
        ),
        DocumentLineTask::SourceUnavailable
    ));
}

#[test]
fn color_keyword_preparation_decodes_selected_rows_off_state() {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("测试时间应晚于 Unix epoch")
        .as_nanos();
    let temporary = TemporaryFile(std::env::temp_dir().join(format!(
        "vclogg2-color-keywords-{}-{nonce}.log",
        std::process::id()
    )));
    fs::write(&temporary.0, b" alpha \n\nbeta\nalpha").expect("应能创建颜色关键词测试日志");
    let document = Arc::new(LogDocument::open(&temporary.0).expect("应能打开颜色关键词测试日志"));

    let prepared = prepare_color_keywords(
        ColorKeywordTarget {
            document_id: 7,
            document: document.clone(),
            selection: ColorKeywordSelection::Rows([0, 1, 2, 3].into_iter().collect()),
        },
        true,
        &SearchCancellation::default(),
    )
    .expect("未取消的颜色关键词解析应完成");

    assert_eq!(prepared.document_id, 7);
    assert!(Arc::ptr_eq(&prepared.document, &document));
    assert_eq!(
        prepared.keywords,
        BTreeSet::from(["alpha".to_string(), "beta".to_string()])
    );
}

#[test]
fn cancelled_color_keyword_preparation_does_not_read_documents() {
    let cancellation = SearchCancellation::default();
    cancellation.cancel();
    let target = ColorKeywordTarget {
        document_id: 1,
        document: Arc::new(LogDocument::placeholder("cancelled-color.log")),
        selection: ColorKeywordSelection::Rows([0].into_iter().collect()),
    };

    assert!(prepare_color_keywords(target, true, &cancellation).is_cancelled());
}

#[test]
fn clearing_color_rules_skips_selected_row_decoding() {
    let target = ColorKeywordTarget {
        document_id: 1,
        document: Arc::new(LogDocument::placeholder("clear-color.log")),
        selection: ColorKeywordSelection::Rows([usize::MAX].into_iter().collect()),
    };

    let prepared = prepare_color_keywords(target, false, &SearchCancellation::default())
        .expect("清除颜色规则无需读取所选行");

    assert!(prepared.keywords.is_empty());
}

#[test]
fn color_rule_updates_build_matchers_without_mutating_input_state() {
    let document = Arc::new(LogDocument::placeholder("color-rule-update.log"));
    let label = default_color_labels()[0].clone();
    let prepared = prepare_color_rule_update(
        ColorRuleUpdateInput {
            target: ColorKeywordTarget {
                document_id: 9,
                document,
                selection: ColorKeywordSelection::Text("needle".to_string()),
            },
            collect_keywords: true,
            action: ColorRuleAction::Apply {
                label_id: Some(label.id.clone()),
                clear_all: false,
            },
            rules: Vec::new(),
            labels: vec![label.clone()],
            last_color_label_id: None,
            propagation_targets: Vec::new(),
            session_target: None,
        },
        &SearchCancellation::default(),
    )
    .expect("颜色规则更新应完成");

    assert!(prepared.expected_rules.is_empty());
    assert_eq!(prepared.rules.len(), 1);
    assert_eq!(prepared.rules[0].keyword, "needle");
    assert_eq!(
        prepared.rules[0].label_id.as_deref(),
        Some(label.id.as_str())
    );
    assert!(prepared.resolved.is_some());
    assert!(matches!(prepared.outcome, ColorRuleOutcome::Applied));
}

#[test]
fn global_keyword_color_update_is_synchronized_to_files_and_session() {
    let primary_document = Arc::new(LogDocument::placeholder("primary-color.log"));
    let secondary_document = Arc::new(LogDocument::placeholder("secondary-color.log"));
    let label = default_color_labels()[0].clone();
    let unrelated = KeywordColorRule {
        label_id: None,
        keyword: "keep-me".to_string(),
        color: 0x123456,
        alpha: u8::MAX,
        case_sensitive: true,
        enabled: true,
    };
    let prepared = prepare_color_rule_update(
        ColorRuleUpdateInput {
            target: ColorKeywordTarget {
                document_id: 9,
                document: primary_document,
                selection: ColorKeywordSelection::Text("needle".to_string()),
            },
            collect_keywords: true,
            action: ColorRuleAction::Apply {
                label_id: Some(label.id.clone()),
                clear_all: false,
            },
            rules: Vec::new(),
            labels: vec![label.clone()],
            last_color_label_id: None,
            propagation_targets: vec![ColorRulePropagationTarget {
                document_id: 10,
                document: secondary_document,
                expected_rules: vec![unrelated.clone()],
            }],
            session_target: Some(ColorRuleSessionTarget {
                scope: SearchScope::Directory,
                expected_revision: 4,
                expected_rules: Vec::new(),
            }),
        },
        &SearchCancellation::default(),
    )
    .expect("全局关键词颜色规则应在后台原子准备完成");

    assert_eq!(prepared.propagated_files.len(), 1);
    assert!(prepared.propagated_files[0].rules.contains(&unrelated));
    let propagated_rule = prepared.propagated_files[0]
        .rules
        .iter()
        .find(|rule| rule.keyword == "needle")
        .expect("其他结果文件应获得相同关键词规则");
    assert_eq!(propagated_rule.label_id.as_deref(), Some(label.id.as_str()));
    let session = prepared
        .search_session
        .expect("目录会话应获得未打开结果可使用的覆盖规则");
    assert_eq!(session.scope, SearchScope::Directory);
    assert_eq!(session.expected_revision, 4);
    assert_eq!(session.rules, vec![propagated_rule.clone()]);
}

#[test]
fn cycling_an_existing_color_rule_prepares_removal() {
    let document = Arc::new(LogDocument::placeholder("cycle-color-rule.log"));
    let label = default_color_labels()[0].clone();
    let rule = KeywordColorRule {
        label_id: Some(label.id.clone()),
        keyword: "needle".to_string(),
        color: label.color,
        alpha: label.alpha,
        case_sensitive: true,
        enabled: true,
    };
    let prepared = prepare_color_rule_update(
        ColorRuleUpdateInput {
            target: ColorKeywordTarget {
                document_id: 9,
                document,
                selection: ColorKeywordSelection::Text("needle".to_string()),
            },
            collect_keywords: true,
            action: ColorRuleAction::Cycle,
            rules: vec![rule.clone()],
            labels: vec![label],
            last_color_label_id: None,
            propagation_targets: Vec::new(),
            session_target: None,
        },
        &SearchCancellation::default(),
    )
    .expect("颜色规则循环更新应完成");

    assert_eq!(prepared.expected_rules, vec![rule]);
    assert!(prepared.rules.is_empty());
    assert!(matches!(
        prepared.outcome,
        ColorRuleOutcome::CycleRemoved { count: 1 }
    ));
    assert!(prepared.resolved.is_some());
}

#[test]
fn cancelled_color_rule_update_does_not_build_matchers() {
    let cancellation = SearchCancellation::default();
    cancellation.cancel();
    let target = ColorKeywordTarget {
        document_id: 9,
        document: Arc::new(LogDocument::placeholder("cancelled-rule-update.log")),
        selection: ColorKeywordSelection::Text("needle".to_string()),
    };

    assert!(
        prepare_color_rule_update(
            ColorRuleUpdateInput {
                target,
                collect_keywords: true,
                action: ColorRuleAction::Cycle,
                rules: Vec::new(),
                labels: default_color_labels(),
                last_color_label_id: None,
                propagation_targets: Vec::new(),
                session_target: None,
            },
            &cancellation,
        )
        .is_cancelled()
    );
}

#[test]
fn prepared_document_color_rules_reject_stale_label_snapshots() {
    let original_label = default_color_labels()[0].clone();
    let mut changed_label = original_label.clone();
    changed_label.color = 0x123456;
    let keyword_rules = vec![KeywordColorRule {
        label_id: Some(original_label.id.clone()),
        keyword: "needle".to_string(),
        color: original_label.color,
        alpha: original_label.alpha,
        case_sensitive: true,
        enabled: true,
    }];
    let original_labels = vec![original_label];
    let prepared = resolve_color_rules(&keyword_rules, &original_labels);

    let reused = installable_color_rules(
        Some(&original_labels),
        prepared.clone(),
        &keyword_rules,
        &original_labels,
    );
    assert!(Arc::ptr_eq(&reused, &prepared));

    let rebuilt = installable_color_rules(
        Some(&original_labels),
        prepared,
        &keyword_rules,
        std::slice::from_ref(&changed_label),
    );
    assert_eq!(
        rebuilt.matching_ranges("needle")[0].1,
        color_with_alpha(changed_label.color, changed_label.alpha)
    );

    let placeholder = installable_color_rules(
        None,
        rebuilt,
        &keyword_rules,
        std::slice::from_ref(&changed_label),
    );
    assert!(placeholder.matching_ranges("needle").is_empty());
}

#[test]
fn color_label_resolution_batch_preserves_document_inputs() {
    let document = Arc::new(LogDocument::placeholder("batch-color-rules.log"));
    let label = default_color_labels()[0].clone();
    let rule = KeywordColorRule {
        label_id: Some(label.id.clone()),
        keyword: "needle".to_string(),
        color: 0,
        alpha: 0,
        case_sensitive: true,
        enabled: true,
    };

    let prepared = prepare_color_rule_resolutions(
        vec![ColorRuleResolutionInput {
            document_id: 17,
            document: document.clone(),
            rules: vec![rule.clone()],
        }],
        std::slice::from_ref(&label),
        &SearchCancellation::default(),
    )
    .expect("颜色标签批量解析应完成");

    assert_eq!(prepared.len(), 1);
    assert_eq!(prepared[0].document_id, 17);
    assert!(Arc::ptr_eq(&prepared[0].document, &document));
    assert_eq!(prepared[0].rules, vec![rule]);
    assert_eq!(
        prepared[0].resolved.matching_ranges("needle")[0].1,
        color_with_alpha(label.color, label.alpha)
    );
}

#[test]
fn cancelled_color_label_resolution_batch_stops_before_work() {
    let cancellation = SearchCancellation::default();
    cancellation.cancel();

    assert!(
        prepare_color_rule_resolutions(
            vec![ColorRuleResolutionInput {
                document_id: 17,
                document: Arc::new(LogDocument::placeholder("cancelled-label-batch.log")),
                rules: Vec::new(),
            }],
            &default_color_labels(),
            &cancellation,
        )
        .is_none()
    );
}

#[test]
fn result_presentation_state_requires_both_path_and_snapshot_identity() {
    let stored = Path::new("logs/a.log");
    let result = LogDocument::placeholder(stored);
    assert!(result_snapshot_matches_document(stored, &result, &result));

    let other_path = LogDocument::placeholder("logs/b.log");
    assert!(!result_snapshot_matches_document(
        stored,
        &result,
        &other_path
    ));
}

#[test]
fn pending_search_jump_rejects_a_reopened_snapshot_at_the_same_path() {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("测试时间应晚于 Unix epoch")
        .as_nanos();
    let temporary = TemporaryFile(std::env::temp_dir().join(format!(
        "vclogg2-directory-jump-{}-{nonce}.log",
        std::process::id()
    )));
    fs::write(&temporary.0, b"before\n").expect("应能创建测试日志");
    let expected_document = Arc::new(LogDocument::open(&temporary.0).expect("应能打开原始快照"));
    let pending = PendingSearchResultJump {
        path: temporary.0.clone(),
        source_row: 7,
        expected_document: expected_document.clone(),
    };

    assert!(pending.matches(&expected_document));
    fs::write(&temporary.0, b"after!\n").expect("应能替换测试日志内容");
    let reopened = LogDocument::open(&temporary.0).expect("应能打开替换后的快照");
    assert!(!pending.matches(&reopened));
}

#[test]
fn unopened_match_rows_create_pending_jumps_for_global_and_directory_results() {
    let result = GlobalSearchDocumentResult {
        title: "unopened.log".into(),
        path: PathBuf::from("logs/unopened.log"),
        document: Arc::new(LogDocument::placeholder("logs/unopened.log")),
        search_result: SearchResult::default(),
        failure: None,
    };

    for scope in [SearchScope::AllOpenFiles, SearchScope::Directory] {
        let pending = pending_search_result_jump(Some(scope), Some(&result), 17)
            .expect("工作区搜索结果应在打开文件后继续定位");

        assert_eq!(pending.path, PathBuf::from("logs/unopened.log"));
        assert_eq!(pending.source_row, 17);
        assert!(Arc::ptr_eq(&pending.expected_document, &result.document));
    }

    assert!(
        pending_search_result_jump(Some(SearchScope::CurrentFile), Some(&result), 17).is_none()
    );
    assert!(pending_search_result_jump(None, Some(&result), 17).is_none());
}

#[test]
fn search_result_jump_waits_for_a_selection_frame_until_the_target_is_ready() {
    assert!(should_defer_search_result_jump(None));
    assert!(should_defer_search_result_jump(Some(
        DocumentLoadState::Opening
    )));
    assert!(should_defer_search_result_jump(Some(
        DocumentLoadState::Preview
    )));
    assert!(!should_defer_search_result_jump(Some(
        DocumentLoadState::Ready
    )));
}

#[test]
fn unopened_global_result_prepares_its_target_in_the_first_ready_frame() {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("测试时间应晚于 Unix epoch")
        .as_nanos();
    let temporary = TemporaryFile(std::env::temp_dir().join(format!(
        "vclogg2-global-result-jump-{}-{nonce}.log",
        std::process::id()
    )));
    fs::write(&temporary.0, b"before\nneedle\nafter\n").expect("应能创建跳转测试日志");
    let full = Arc::new(LogDocument::open(&temporary.0).expect("应能打开完整测试日志"));
    let sparse = Arc::new(full.project_source_rows(&[1].into_iter().collect()));
    let pending = PendingSearchResultJump {
        path: temporary.0.clone(),
        source_row: 1,
        expected_document: sparse,
    };

    let prepared = resolved_prepared_search_jump(None, Some(&pending), &temporary.0, &full)
        .expect("即使未生成可见行预加载帧，全局结果目标也应进入文件首帧准备");

    assert_eq!(prepared.source_row, 1);
    assert_eq!(prepared.row_ix, 1);
}

#[test]
fn selected_search_result_row_overrides_the_files_persisted_row_while_opening() {
    let selected_result_jump = PreparedLogJump {
        source_row: 17,
        row_ix: 17,
    };

    assert_eq!(
        opening_restore_source_row(Some(selected_result_jump), Some(4)),
        Some(17)
    );
    assert_eq!(opening_restore_source_row(None, Some(4)), Some(4));
}

#[test]
fn persisted_all_open_search_restores_an_unopened_source() {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("测试时间应晚于 Unix epoch")
        .as_nanos();
    let temporary = TemporaryFile(std::env::temp_dir().join(format!(
        "vclogg2-all-open-restore-{}-{nonce}.log",
        std::process::id()
    )));
    fs::write(&temporary.0, b"before\nneedle\nafter\n").expect("应能创建恢复测试日志");

    let (cancelled, results, matcher) = run_persisted_all_open_search(
        vec![temporary.0.clone()],
        SearchQuery {
            text: "needle".into(),
            case_sensitive: true,
            regex: false,
            max_results: Some(100),
        },
        SearchCancellation::default(),
    )
    .expect("未打开的搜索源应能恢复");

    assert!(!cancelled);
    assert!(matcher.is_some());
    assert_eq!(results.len(), 1);
    assert!(paths_match(&results[0].path, &temporary.0));
    assert_eq!(
        results[0]
            .search_result
            .line_indices
            .iter()
            .collect::<Vec<_>>(),
        [1]
    );
}

#[test]
fn persisted_search_restore_prioritizes_active_scope_then_restores_the_other_scope() {
    assert_eq!(
        next_persisted_search_restore_scope(SearchScope::Directory, true, true),
        Some(SearchScope::Directory)
    );
    assert_eq!(
        next_persisted_search_restore_scope(SearchScope::Directory, true, false),
        Some(SearchScope::AllOpenFiles)
    );
    assert_eq!(
        next_persisted_search_restore_scope(SearchScope::CurrentFile, true, true),
        Some(SearchScope::AllOpenFiles)
    );
    assert_eq!(
        next_persisted_search_restore_scope(SearchScope::AllOpenFiles, false, true),
        Some(SearchScope::Directory)
    );
}

#[test]
fn directory_group_activation_waits_for_a_complete_target_frame() {
    let pending = Path::new("logs/a.log");

    assert!(should_defer_directory_group_activation(
        Some(pending),
        pending,
        DocumentLoadState::Opening,
    ));
    assert!(should_defer_directory_group_activation(
        Some(pending),
        pending,
        DocumentLoadState::Preview,
    ));
    assert!(!should_defer_directory_group_activation(
        Some(pending),
        pending,
        DocumentLoadState::Ready,
    ));
    assert!(!should_defer_directory_group_activation(
        Some(pending),
        Path::new("logs/b.log"),
        DocumentLoadState::Opening,
    ));
}

#[test]
fn result_row_target_resolution_is_atomic_and_groups_by_open_document() {
    let selected = BTreeMap::from([
        (101, [2, 7].into_iter().collect()),
        (102, [4].into_iter().collect()),
    ]);
    let grouped = group_result_rows_by_document(&selected, |id, _| {
        Some(match id {
            101 => 1,
            102 => 2,
            _ => return None,
        })
    })
    .map(|groups| {
        groups
            .into_iter()
            .map(|(document_id, rows)| (document_id, rows.iter().collect::<Vec<_>>()))
            .collect::<BTreeMap<_, _>>()
    });
    assert_eq!(
        grouped,
        Some(BTreeMap::from([(1, vec![2, 7]), (2, vec![4])]))
    );

    let unresolved = group_result_rows_by_document(&selected, |id, _| (id == 101).then_some(1));
    assert!(unresolved.is_none());
}
