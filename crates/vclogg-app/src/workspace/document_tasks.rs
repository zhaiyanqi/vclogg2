use super::*;

#[derive(Clone, Copy)]
pub(super) struct SearchPreparationOptions {
    pub(super) case_sensitive: bool,
    pub(super) regex: bool,
    pub(super) max_results: Option<usize>,
}

fn search_path_snapshot(
    path: &Path,
    matcher: Option<&SearchMatcher>,
    max_results: Option<usize>,
    cancellation: &SearchCancellation,
) -> Result<Option<(Arc<LogDocument>, SearchResult)>> {
    if cancellation.is_cancelled() {
        return Ok(None);
    }
    let opened = if let Some(matcher) = matcher {
        if let Some(cache_dir) = crate::app_paths::index_cache_dir() {
            LogDocument::open_with_index_cache_and_search_cancellable(
                path,
                cache_dir,
                matcher,
                max_results,
                cancellation,
            )?
        } else {
            LogDocument::open_cancellable(path, cancellation)?.map(|document| {
                let run = search_with_compiled_matcher(
                    &document,
                    Some(matcher),
                    max_results,
                    cancellation,
                );
                (document, None, run)
            })
        }
    } else {
        LogDocument::open_cancellable(path, cancellation)?.map(|document| {
            let run = search_with_compiled_matcher(&document, None, max_results, cancellation);
            (document, None, run)
        })
    };
    let Some((document, pending_index_cache, run)) = opened else {
        return Ok(None);
    };
    let document = Arc::new(document);
    match run {
        SearchRun::Completed(search_result) => {
            let document = Arc::new(document.project_source_rows(&search_result.line_indices));
            document.release_source_handle();
            if !cancellation.is_cancelled()
                && let Some(cache_write) = pending_index_cache
            {
                _ = cache_write.persist();
            }
            Ok(Some((document, search_result)))
        }
        SearchRun::SourceChanged => Err(anyhow::anyhow!(
            "搜索期间文件内容已改变，请重新加载后重试：{}",
            path.display()
        )),
        SearchRun::Cancelled => Ok(None),
    }
}

pub(super) fn run_directory_search(
    options: DirectorySearchOptions,
    query: SearchQuery,
    open_document_paths: BTreeSet<PathMatchKey>,
    cancellation: SearchCancellation,
) -> Result<DirectorySearchRun> {
    let matcher = SearchMatcher::new(&query)?;
    let Some(enumeration) = enumerate_directory_search_paths(&options, &cancellation)? else {
        return Ok(DirectorySearchRun {
            cancelled: true,
            results: Vec::new(),
            matcher,
            file_count: 0,
            open_error_count: 0,
            unreadable_directory_count: 0,
        });
    };
    let file_count = enumeration.paths.len();
    let unreadable_directory_count = enumeration.unreadable_directory_count;
    let scan_paths =
        directory_search_scan_paths(enumeration.paths, matcher.is_some(), &open_document_paths);
    let outcomes = prepare_paths_bounded_while(
        scan_paths,
        || !cancellation.is_cancelled(),
        |path| search_path_snapshot(path, matcher.as_ref(), query.max_results, &cancellation),
    );
    if cancellation.is_cancelled() {
        return Ok(DirectorySearchRun {
            cancelled: true,
            results: Vec::new(),
            matcher,
            file_count,
            open_error_count: 0,
            unreadable_directory_count,
        });
    }
    let mut open_error_count = 0;
    let results = outcomes
        .into_iter()
        .filter_map(|(path, outcome)| match outcome {
            Ok(Some((document, search_result)))
                if !search_result.line_indices.is_empty()
                    || path_match_set_contains(&open_document_paths, &path) =>
            {
                let title: SharedString = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string())
                    .into();
                Some(DirectorySearchResult {
                    title,
                    path,
                    document,
                    search_result,
                })
            }
            Ok(Some(_)) | Ok(None) => None,
            Err(_) => {
                open_error_count += 1;
                None
            }
        })
        .collect();
    Ok(DirectorySearchRun {
        cancelled: false,
        results,
        matcher,
        file_count,
        open_error_count,
        unreadable_directory_count,
    })
}

pub(super) fn run_persisted_all_open_search(
    paths: Vec<PathBuf>,
    query: SearchQuery,
    cancellation: SearchCancellation,
) -> Result<(bool, Vec<DirectorySearchResult>, Option<SearchMatcher>)> {
    let matcher = SearchMatcher::new(&query)?;
    let outcomes = prepare_paths_bounded_while(
        deduplicate_paths(paths),
        || !cancellation.is_cancelled(),
        |path| search_path_snapshot(path, matcher.as_ref(), query.max_results, &cancellation),
    );
    if cancellation.is_cancelled() {
        return Ok((true, Vec::new(), matcher));
    }
    let results = outcomes
        .into_iter()
        .filter_map(|(path, outcome)| {
            let (document, search_result) = outcome.ok().flatten()?;
            let title: SharedString = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
                .into();
            Some(DirectorySearchResult {
                title,
                path,
                document,
                search_result,
            })
        })
        .collect();
    Ok((false, results, matcher))
}

pub(super) fn directory_search_scan_paths(
    paths: Vec<PathBuf>,
    query_has_matcher: bool,
    open_document_paths: &BTreeSet<PathMatchKey>,
) -> Vec<PathBuf> {
    if query_has_matcher {
        return paths;
    }
    paths
        .into_iter()
        .filter(|path| path_match_set_contains(open_document_paths, path))
        .collect()
}

pub(super) fn collect_log_lines_for_clipboard(
    documents: Vec<(Arc<LogDocument>, CompressedRows)>,
    include_line_number: bool,
    cancellation: &SearchCancellation,
) -> DocumentLineTask<CopiedLogLines> {
    let mut text = String::new();
    let mut count = 0_usize;
    let mut first_source_row = None;
    for (document, rows) in documents {
        let mut reader = LineReader::default();
        for source_row in rows.iter() {
            if cancellation.is_cancelled() {
                return DocumentLineTask::Cancelled;
            }
            let Some(line) = reader.line(&document, source_row) else {
                return DocumentLineTask::SourceUnavailable;
            };
            if count > 0 {
                text.push('\n');
            }
            if include_line_number {
                text.push_str(&(source_row + 1).to_string());
                text.push('\t');
            }
            text.push_str(&line);
            first_source_row.get_or_insert(source_row);
            count = count.saturating_add(1);
        }
    }
    DocumentLineTask::Completed(CopiedLogLines {
        text,
        count,
        first_source_row,
    })
}

pub(super) fn prepare_color_keywords(
    target: ColorKeywordTarget,
    collect_keywords: bool,
    cancellation: &SearchCancellation,
) -> DocumentLineTask<PreparedColorKeywords> {
    if cancellation.is_cancelled() {
        return DocumentLineTask::Cancelled;
    }
    let keywords = if !collect_keywords {
        BTreeSet::new()
    } else {
        match target.selection {
            ColorKeywordSelection::Text(text) => std::iter::once(text).collect(),
            ColorKeywordSelection::Rows(rows) => {
                let mut keywords = BTreeSet::new();
                let mut reader = LineReader::default();
                for source_row in rows.iter() {
                    if cancellation.is_cancelled() {
                        return DocumentLineTask::Cancelled;
                    }
                    let Some(line) = reader.line(&target.document, source_row) else {
                        return DocumentLineTask::SourceUnavailable;
                    };
                    let line = line.trim();
                    if !line.is_empty() {
                        keywords.insert(line.to_string());
                    }
                }
                keywords
            }
        }
    };
    DocumentLineTask::Completed(PreparedColorKeywords {
        document_id: target.document_id,
        document: target.document,
        keywords,
    })
}

pub(super) fn prepare_color_rule_update(
    input: ColorRuleUpdateInput,
    cancellation: &SearchCancellation,
) -> DocumentLineTask<PreparedColorRuleUpdate> {
    let ColorRuleUpdateInput {
        target,
        collect_keywords,
        action,
        mut rules,
        labels,
        mut last_color_label_id,
        propagation_targets,
        session_target,
    } = input;
    let prepared = match prepare_color_keywords(target, collect_keywords, cancellation) {
        DocumentLineTask::Completed(prepared) => prepared,
        DocumentLineTask::Cancelled => return DocumentLineTask::Cancelled,
        DocumentLineTask::SourceUnavailable => return DocumentLineTask::SourceUnavailable,
    };
    if cancellation.is_cancelled() {
        return DocumentLineTask::Cancelled;
    }
    let expected_rules = rules.clone();
    let expected_labels = labels.clone();
    let keywords = prepared.keywords;
    let outcome = match action {
        ColorRuleAction::Cycle if keywords.is_empty() => ColorRuleOutcome::EmptyKeywords,
        ColorRuleAction::Cycle => {
            let remove = keywords.iter().all(|keyword| {
                rules.iter().any(|rule| {
                    rule.enabled && rule.case_sensitive && rule.keyword.as_str() == keyword.as_str()
                })
            });
            if remove {
                rules.retain(|rule| {
                    !(rule.enabled
                        && rule.case_sensitive
                        && keywords.contains(rule.keyword.as_str()))
                });
                ColorRuleOutcome::CycleRemoved {
                    count: keywords.len(),
                }
            } else if labels.is_empty() {
                ColorRuleOutcome::MissingLabels
            } else {
                let next_ix = last_color_label_id
                    .as_deref()
                    .and_then(|id| labels.iter().position(|label| label.id == id))
                    .map_or(0, |ix| (ix + 1) % labels.len());
                let label = labels[next_ix].clone();
                apply_color_label_to_keywords(&mut rules, &keywords, &label);
                last_color_label_id = Some(label.id.clone());
                ColorRuleOutcome::CycleApplied {
                    label,
                    count: keywords.len(),
                }
            }
        }
        ColorRuleAction::Apply {
            clear_all: true, ..
        } => {
            rules.clear();
            ColorRuleOutcome::Cleared
        }
        ColorRuleAction::Apply {
            clear_all: false, ..
        } if keywords.is_empty() => ColorRuleOutcome::EmptyKeywords,
        ColorRuleAction::Apply {
            label_id: Some(label_id),
            clear_all: false,
        } => {
            let Some(label) = labels.iter().find(|label| label.id == label_id) else {
                return DocumentLineTask::Completed(PreparedColorRuleUpdate {
                    document_id: prepared.document_id,
                    document: prepared.document,
                    expected_rules,
                    expected_labels,
                    rules,
                    resolved: None,
                    propagated_files: Vec::new(),
                    search_session: None,
                    last_color_label_id,
                    outcome: ColorRuleOutcome::MissingLabel,
                });
            };
            apply_color_label_to_keywords(&mut rules, &keywords, label);
            last_color_label_id = Some(label.id.clone());
            ColorRuleOutcome::Applied
        }
        ColorRuleAction::Apply {
            label_id: None,
            clear_all: false,
        } => {
            rules.retain(|rule| !(rule.case_sensitive && keywords.contains(rule.keyword.as_str())));
            ColorRuleOutcome::Removed
        }
    };
    let resolved = if matches!(
        &outcome,
        ColorRuleOutcome::EmptyKeywords
            | ColorRuleOutcome::MissingLabels
            | ColorRuleOutcome::MissingLabel
    ) {
        None
    } else {
        if cancellation.is_cancelled() {
            return DocumentLineTask::Cancelled;
        }
        let resolved = resolve_color_rules(&rules, &labels);
        if cancellation.is_cancelled() {
            return DocumentLineTask::Cancelled;
        }
        Some(resolved)
    };
    let mut propagated_files = Vec::new();
    let mut search_session = None;
    if resolved.is_some() {
        for target in propagation_targets {
            if cancellation.is_cancelled() {
                return DocumentLineTask::Cancelled;
            }
            let mut target_rules = target.expected_rules.clone();
            synchronize_keyword_color_rules(&mut target_rules, &rules, &keywords);
            let target_resolved = resolve_color_rules(&target_rules, &labels);
            propagated_files.push(PreparedColorRulePropagation {
                document_id: target.document_id,
                document: target.document,
                expected_rules: target.expected_rules,
                rules: target_rules,
                resolved: target_resolved,
            });
        }
        if let Some(target) = session_target {
            if cancellation.is_cancelled() {
                return DocumentLineTask::Cancelled;
            }
            let mut target_rules = target.expected_rules.clone();
            synchronize_keyword_color_rules(&mut target_rules, &rules, &keywords);
            let target_resolved = resolve_color_rules(&target_rules, &labels);
            search_session = Some(PreparedColorRuleSession {
                scope: target.scope,
                expected_revision: target.expected_revision,
                expected_rules: target.expected_rules,
                rules: target_rules,
                resolved: target_resolved,
            });
        }
    }
    DocumentLineTask::Completed(PreparedColorRuleUpdate {
        document_id: prepared.document_id,
        document: prepared.document,
        expected_rules,
        expected_labels,
        rules,
        resolved,
        propagated_files,
        search_session,
        last_color_label_id,
        outcome,
    })
}

pub(super) fn synchronize_keyword_color_rules(
    destination: &mut Vec<KeywordColorRule>,
    source: &[KeywordColorRule],
    keywords: &BTreeSet<String>,
) {
    for keyword in keywords {
        destination.retain(|rule| !(rule.case_sensitive && rule.keyword == *keyword));
        if let Some(rule) = source
            .iter()
            .find(|rule| rule.case_sensitive && rule.keyword == *keyword)
        {
            destination.push(rule.clone());
        }
    }
}

pub(super) fn apply_color_label_to_keywords(
    rules: &mut Vec<KeywordColorRule>,
    keywords: &BTreeSet<String>,
    label: &ColorLabel,
) {
    for keyword in keywords {
        if let Some(rule) = rules
            .iter_mut()
            .find(|rule| rule.case_sensitive && rule.keyword == *keyword)
        {
            rule.label_id = Some(label.id.clone());
            rule.color = label.background_color;
            rule.alpha = label.background_alpha;
            rule.enabled = true;
        } else {
            rules.push(KeywordColorRule {
                label_id: Some(label.id.clone()),
                keyword: keyword.clone(),
                color: label.background_color,
                alpha: label.background_alpha,
                case_sensitive: true,
                enabled: true,
            });
        }
    }
}

pub(super) fn prepare_color_rule_resolutions(
    inputs: Vec<ColorRuleResolutionInput>,
    labels: &[ColorLabel],
    cancellation: &SearchCancellation,
) -> Option<Vec<PreparedColorRuleResolution>> {
    let mut prepared = Vec::with_capacity(inputs.len());
    for input in inputs {
        if cancellation.is_cancelled() {
            return None;
        }
        let resolved = resolve_color_rules(&input.rules, labels);
        prepared.push(PreparedColorRuleResolution {
            document_id: input.document_id,
            document: input.document,
            rules: input.rules,
            resolved,
        });
    }
    (!cancellation.is_cancelled()).then_some(prepared)
}

pub(super) fn prepare_paths_bounded<T, F>(paths: Vec<PathBuf>, operation: F) -> Vec<(PathBuf, T)>
where
    T: Send,
    F: Fn(&std::path::Path) -> T + Sync,
{
    prepare_paths_bounded_while(paths, || true, operation)
}

pub(super) fn prepare_paths_bounded_while<T, F, C>(
    paths: Vec<PathBuf>,
    should_continue: C,
    operation: F,
) -> Vec<(PathBuf, T)>
where
    T: Send,
    F: Fn(&std::path::Path) -> T + Sync,
    C: Fn() -> bool + Sync,
{
    if paths.len() <= 1 {
        return paths
            .into_iter()
            .take_while(|_| should_continue())
            .map(|path| {
                let result = operation(&path);
                (path, result)
            })
            .collect();
    }
    let worker_count = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(MAX_DOCUMENT_PREPARE_WORKERS)
        .min(paths.len());
    let mut worker_paths = (0..worker_count).map(|_| Vec::new()).collect::<Vec<_>>();
    for (path_ix, path) in paths.into_iter().enumerate() {
        worker_paths[path_ix % worker_count].push((path_ix, path));
    }
    std::thread::scope(|scope| {
        let operation = &operation;
        let should_continue = &should_continue;
        let handles = worker_paths
            .into_iter()
            .map(|worker_paths| {
                scope.spawn(move || {
                    let mut prepared = Vec::new();
                    for (path_ix, path) in worker_paths {
                        if !should_continue() {
                            break;
                        }
                        let result = operation(&path);
                        prepared.push((path_ix, path, result));
                    }
                    prepared
                })
            })
            .collect::<Vec<_>>();
        let mut prepared = Vec::new();
        for handle in handles {
            match handle.join() {
                Ok(worker_prepared) => prepared.extend(worker_prepared),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        prepared.sort_by_key(|(path_ix, _, _)| *path_ix);
        prepared
            .into_iter()
            .map(|(_, path, result)| (path, result))
            .collect()
    })
}

pub(super) fn prepare_document(
    path: &std::path::Path,
    cached_complete_document: Option<Arc<LogDocument>>,
    store: Option<&StateStore>,
    session_override: Option<FileSessionState>,
    search_options: SearchPreparationOptions,
    color_labels: &[ColorLabel],
) -> Result<PreparedDocument> {
    let (document, pending_index_cache) = match cached_complete_document {
        Some(document) => (document, None),
        None => {
            let (document, pending_index_cache) =
                if let Some(cache_dir) = crate::app_paths::index_cache_dir() {
                    LogDocument::open_with_index_cache(path, cache_dir)?
                } else {
                    (LogDocument::open(path)?, None)
                };
            (Arc::new(document), pending_index_cache)
        }
    };
    let mut warning = None;
    let session = if session_override.is_some() {
        session_override
    } else {
        match store.map(|store| store.load_session(path)).transpose() {
            Ok(session) => session.flatten(),
            Err(error) => {
                warning = Some(crate::tr_args!(
                    "{} 的会话未能读取，将使用默认视图：{error}",
                    "The session for {} couldn’t be read; the default view will be used: {error}",
                    path.display(),
                ));
                None
            }
        }
    };
    let query = SearchQuery {
        text: session
            .as_ref()
            .map(|state| state.query_text.clone())
            .unwrap_or_default(),
        case_sensitive: search_options.case_sensitive,
        regex: search_options.regex,
        max_results: search_options.max_results,
    };
    let search = (|| {
        let matcher = SearchMatcher::new(&query)?;
        let result = if query.text.is_empty() {
            SearchResult::default()
        } else {
            search_document_with_matcher(&document, &query, matcher.as_ref())?
        };
        Ok::<_, anyhow::Error>((result, matcher))
    })();
    let (search_result, search_matcher) = match search {
        Ok(search) => search,
        Err(error) => {
            warning = Some(crate::tr_args!(
                "{} 的已保存查询未能恢复：{error}",
                "The saved query for {} couldn’t be restored: {error}",
                path.display()
            ));
            (SearchResult::default(), None)
        }
    };
    let resolved_color_rules = session
        .as_ref()
        .map(|session| resolve_color_rules(&session.keyword_color_rules, color_labels))
        .unwrap_or_default();

    Ok(PreparedDocument {
        document,
        cached_complete_document: None,
        session,
        color_labels_snapshot: Some(color_labels.to_vec()),
        resolved_color_rules,
        search_result,
        search_matcher,
        search_case_sensitive: search_options.case_sensitive,
        search_regex: search_options.regex,
        warning,
        load_state: DocumentLoadState::Ready,
        pending_index_cache,
        upgrade_frame: None,
    })
}

pub(super) fn load_document_upgrade_frames(
    jobs: Vec<DocumentUpgradeLoadJob>,
) -> Vec<PreparedDocumentUpgradeFrame> {
    jobs.into_iter()
        .map(|job| {
            let mut reader = LinePreviewReader::default();
            let log_lines = job.log_request.load(|source_row, max_bytes| {
                reader.line_preview(&job.document, *source_row, max_bytes)
            });
            let result_lines = job.result_request.load(|source_row, max_bytes| {
                reader.line_preview(&job.document, *source_row, max_bytes)
            });
            PreparedDocumentUpgradeFrame {
                path: job.path,
                previous_document: job.previous_document,
                document: job.document,
                result_rows: job.result_rows,
                log_lines,
                result_lines,
                log_anchor: job.log_anchor,
                result_anchor: job.result_anchor,
                log_measured_heights: job.log_measured_heights,
                result_measured_heights: job.result_measured_heights,
                row_height: job.row_height,
                log_word_wrap: job.log_word_wrap,
                result_word_wrap: job.result_word_wrap,
                log_jump: job.log_jump,
            }
        })
        .collect()
}

pub(super) fn installable_color_rules(
    prepared_labels: Option<&[ColorLabel]>,
    prepared_rules: Arc<ResolvedColorRules>,
    keyword_rules: &[KeywordColorRule],
    current_labels: &[ColorLabel],
) -> Arc<ResolvedColorRules> {
    match prepared_labels {
        None => Arc::default(),
        Some(labels) if labels == current_labels => prepared_rules,
        Some(_) => resolve_color_rules(keyword_rules, current_labels),
    }
}

pub(super) fn prepare_document_shell(
    path: &std::path::Path,
    session: Option<FileSessionState>,
    case_sensitive: bool,
    regex: bool,
) -> PreparedDocument {
    PreparedDocument {
        document: Arc::new(LogDocument::placeholder(path)),
        cached_complete_document: None,
        session,
        color_labels_snapshot: None,
        resolved_color_rules: Arc::default(),
        search_result: SearchResult::default(),
        search_matcher: None,
        search_case_sensitive: case_sensitive,
        search_regex: regex,
        warning: None,
        load_state: DocumentLoadState::Opening,
        pending_index_cache: None,
        upgrade_frame: None,
    }
}

pub(super) fn search_reloaded_document(
    document: &LogDocument,
    previous_document: &LogDocument,
    kind: DocumentRefreshKind,
    previous_result: &SearchResult,
    query: &SearchQuery,
    matcher: Option<&SearchMatcher>,
    cancellation: &SearchCancellation,
) -> Result<SearchResult> {
    let run = match kind {
        DocumentRefreshKind::Appended => search_appended_with_compiled_matcher(
            document,
            previous_document.line_count(),
            previous_result,
            matcher,
            query.max_results,
            cancellation,
        ),
        DocumentRefreshKind::Rebuilt => {
            search_with_compiled_matcher(document, matcher, query.max_results, cancellation)
        }
    };
    match run {
        SearchRun::Completed(result) => Ok(result),
        SearchRun::SourceChanged => {
            anyhow::bail!("搜索期间文件内容已改变：{}", document.path().display())
        }
        SearchRun::Cancelled => anyhow::bail!("搜索已取消"),
    }
}

pub(super) fn search_document_with_matcher(
    document: &LogDocument,
    query: &SearchQuery,
    matcher: Option<&SearchMatcher>,
) -> Result<SearchResult> {
    search_reloaded_document(
        document,
        document,
        DocumentRefreshKind::Rebuilt,
        &SearchResult::default(),
        query,
        matcher,
        &SearchCancellation::default(),
    )
}

pub(super) fn prepare_document_preview(
    path: &std::path::Path,
    store: Option<&StateStore>,
    session_override: Option<FileSessionState>,
    search_options: SearchPreparationOptions,
    color_labels: &[ColorLabel],
) -> Result<PreparedDocument> {
    let mut warning = None;
    let session = if session_override.is_some() {
        session_override
    } else {
        match store.map(|store| store.load_session(path)).transpose() {
            Ok(session) => session.flatten(),
            Err(error) => {
                warning = Some(crate::tr_args!(
                    "{} 的会话未能读取，将使用默认视图：{error}",
                    "The session for {} couldn’t be read; the default view will be used: {error}",
                    path.display(),
                ));
                None
            }
        }
    };
    let preferred_row = session.as_ref().and_then(|session| {
        session
            .resume
            .viewer
            .viewport
            .as_ref()
            .map(|viewport| viewport.anchor_source_row)
            .or(session.selected_row)
    });
    let cached_preview = match (preferred_row, crate::app_paths::index_cache_dir()) {
        (Some(preferred_row), Some(cache_dir)) if preferred_row > 0 => {
            LogDocument::open_cached_preview_with_complete_document(
                path,
                cache_dir,
                preferred_row,
                PREVIEW_LINE_LIMIT,
                PREVIEW_BYTE_LIMIT,
            )?
        }
        _ => None,
    };
    let (document, cached_complete_document) = match cached_preview {
        Some((preview, complete)) => (preview, Some(Arc::new(complete))),
        None => (
            LogDocument::open_preview(path, PREVIEW_BYTE_LIMIT, PREVIEW_LINE_LIMIT)?.0,
            None,
        ),
    };
    let document = Arc::new(document);
    let query = SearchQuery {
        text: session
            .as_ref()
            .map(|state| state.query_text.clone())
            .unwrap_or_default(),
        case_sensitive: search_options.case_sensitive,
        regex: search_options.regex,
        max_results: search_options.max_results,
    };
    let search_matcher = match SearchMatcher::new(&query) {
        Ok(matcher) => matcher,
        Err(error) => {
            warning = Some(crate::tr_args!(
                "{} 的已保存查询无效：{error}",
                "The saved query for {} is invalid: {error}",
                path.display()
            ));
            None
        }
    };
    let resolved_color_rules = session
        .as_ref()
        .map(|session| resolve_color_rules(&session.keyword_color_rules, color_labels))
        .unwrap_or_default();
    Ok(PreparedDocument {
        document,
        cached_complete_document,
        session,
        color_labels_snapshot: Some(color_labels.to_vec()),
        resolved_color_rules,
        search_result: SearchResult::default(),
        search_matcher,
        search_case_sensitive: search_options.case_sensitive,
        search_regex: search_options.regex,
        warning,
        load_state: DocumentLoadState::Preview,
        pending_index_cache: None,
        upgrade_frame: None,
    })
}

pub(super) fn compute_result_rows(
    mode: ResultMode,
    search_result: Option<&SearchResult>,
    marked_rows: &CompressedRows,
) -> CompressedRows {
    let matched_rows = search_result
        .map(|result| result.line_indices.clone())
        .unwrap_or_default();
    match mode {
        ResultMode::MatchesOnly => matched_rows,
        ResultMode::MarksOnly => marked_rows.clone(),
        ResultMode::MatchesAndMarks => {
            let mut rows = matched_rows;
            rows.insert_rows(marked_rows);
            rows
        }
    }
}

pub(super) fn group_result_rows_by_document(
    rows: &BTreeMap<u64, CompressedRows>,
    mut resolve_document_id: impl FnMut(u64, &CompressedRows) -> Option<u64>,
) -> Option<BTreeMap<u64, CompressedRows>> {
    let mut by_document = BTreeMap::<u64, CompressedRows>::new();
    for (&result_document_id, rows) in rows {
        if rows.is_empty() {
            continue;
        }
        let document_id = resolve_document_id(result_document_id, rows)?;
        by_document
            .entry(document_id)
            .or_default()
            .insert_rows(rows);
    }
    Some(by_document)
}

pub(super) fn result_snapshot_matches_document(
    result_path: &Path,
    result_document: &LogDocument,
    open_document: &LogDocument,
) -> bool {
    paths_match(result_path, open_document.path())
        && result_document.same_source_snapshot(open_document)
}
