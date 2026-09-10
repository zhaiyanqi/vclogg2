//! One physical search worker per window. Requests capture immutable inputs at submission.
use super::search_tabs::{SearchTabId, SearchTabOwner};
use super::*;
use crate::search_context::SearchTabQuery;
use rayon::prelude::{IntoParallelRefIterator as _, ParallelIterator as _};

type SearchDocument = (u64, SharedString, PathBuf, Arc<LogDocument>);

enum SearchTabInput {
    File(Arc<LogDocument>),
    Open(Vec<SearchDocument>),
    RestoreOpen(Vec<PathBuf>),
    Directory(DirectorySearchOptions, BTreeSet<PathMatchKey>),
}

pub(super) struct SearchTabJob {
    pub(super) owner: SearchTabOwner,
    pub(super) id: SearchTabId,
    revision: u64,
    query: SearchQuery,
    ranges: search_limits::FileSearchRanges,
    input: SearchTabInput,
}

enum SearchTabOutput {
    File(Arc<LogDocument>, SearchResult, Option<SearchMatcher>),
    Global(
        Vec<DirectorySearchResult>,
        Option<SearchMatcher>,
        Option<String>,
    ),
    Cancelled,
}

impl SearchTabJob {
    fn run(&self, cancellation: &SearchCancellation) -> Result<SearchTabOutput> {
        match &self.input {
            SearchTabInput::File(document) => {
                let matcher = SearchMatcher::new(&self.query)?;
                match search_result_cache().search_in_range(
                    document,
                    &self.query,
                    matcher.as_ref(),
                    cancellation,
                    self.ranges.get(document.path()),
                ) {
                    SearchRun::Completed(result) => {
                        Ok(SearchTabOutput::File(document.clone(), result, matcher))
                    }
                    SearchRun::Cancelled => Ok(SearchTabOutput::Cancelled),
                    SearchRun::SourceChanged => anyhow::bail!(
                        "文件内容已改变，请重新加载后重试 / The file changed; reload and retry"
                    ),
                }
            }
            SearchTabInput::Open(targets) => {
                let matcher = SearchMatcher::new(&self.query)?;
                let results = targets
                    .par_iter()
                    .map(|(_, title, path, document)| {
                        match search_result_cache().search_in_range(
                            document,
                            &self.query,
                            matcher.as_ref(),
                            cancellation,
                            self.ranges.get(path),
                        ) {
                            SearchRun::Completed(result) => {
                                let projected =
                                    Arc::new(document.project_source_rows(&result.line_indices));
                                projected.release_source_handle();
                                Ok(Some(DirectorySearchResult {
                                    title: title.clone(),
                                    path: path.clone(),
                                    document: projected,
                                    search_result: result,
                                }))
                            }
                            SearchRun::Cancelled => Ok(None),
                            SearchRun::SourceChanged => {
                                anyhow::bail!("文件内容已改变：{} / File changed", path.display())
                            }
                        }
                    })
                    .collect::<Result<Vec<_>>>()?;
                if results.iter().any(Option::is_none) {
                    return Ok(SearchTabOutput::Cancelled);
                }
                Ok(SearchTabOutput::Global(
                    results.into_iter().flatten().collect(),
                    matcher,
                    None,
                ))
            }
            SearchTabInput::RestoreOpen(paths) => {
                let (cancelled, results, matcher) = run_persisted_all_open_search_in_ranges(
                    paths.clone(),
                    self.query.clone(),
                    cancellation.clone(),
                    self.ranges.clone(),
                )?;
                Ok(if cancelled {
                    SearchTabOutput::Cancelled
                } else {
                    SearchTabOutput::Global(results, matcher, None)
                })
            }
            SearchTabInput::Directory(options, open_paths) => {
                let run = run_directory_search(
                    options.clone(),
                    self.query.clone(),
                    open_paths.clone(),
                    cancellation.clone(),
                    self.ranges.clone(),
                )?;
                if run.cancelled {
                    return Ok(SearchTabOutput::Cancelled);
                }
                let notice = if run.file_count == 0 {
                    Some(
                        crate::tr!(
                            "目录中没有符合文件类型的文件",
                            "No matching file types were found in the directory"
                        )
                        .to_string(),
                    )
                } else if run.open_error_count > 0 || run.unreadable_directory_count > 0 {
                    Some(crate::tr_args!(
                        "{} 个文件和 {} 个子目录无法读取",
                        "{} files and {} subdirectories couldn’t be read",
                        run.open_error_count,
                        run.unreadable_directory_count
                    ))
                } else {
                    None
                };
                Ok(SearchTabOutput::Global(run.results, run.matcher, notice))
            }
        }
    }
}

impl Workspace {
    pub(super) fn search_tab_is_searching(&self) -> bool {
        let Some((owner, id)) = self.active_search_tab_key() else {
            return false;
        };
        self.search_tabs
            .state(owner, id)
            .is_some_and(|state| state.saved.submitted.is_some())
    }

    pub(super) fn submit_search_tab(
        &mut self,
        restore: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_search_tab(window, cx);
        self.capture_active_search_tab(cx);
        let Some((owner, id)) = self.active_search_tab_key() else {
            return;
        };
        self.enqueue_search_tab(owner, id, restore, window, cx);
    }

    pub(super) fn enqueue_search_tab(
        &mut self,
        owner: SearchTabOwner,
        id: SearchTabId,
        restore: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.search_tabs.state(owner, id).cloned() else {
            return;
        };
        let query = if restore {
            state
                .saved
                .submitted
                .as_ref()
                .or(state.saved.completed.as_ref())
                .unwrap_or(&state.saved.draft)
                .query()
        } else {
            state.saved.draft.query()
        };
        let input =
            match owner {
                SearchTabOwner::File(document_id) => {
                    let Some(tab) = self.documents.iter().find(|tab| tab.id == document_id) else {
                        return;
                    };
                    if tab.load_state != DocumentLoadState::Ready {
                        return;
                    }
                    SearchTabInput::File(tab.document.clone())
                }
                SearchTabOwner::AllOpen
                    if restore
                        && state.saved.submitted.is_none()
                        && !state.saved.context.source_paths.is_empty() =>
                {
                    SearchTabInput::RestoreOpen(
                        state
                            .saved
                            .context
                            .source_paths
                            .iter()
                            .map(|path| decode_persisted_path(path))
                            .collect(),
                    )
                }
                SearchTabOwner::AllOpen => {
                    let targets =
                        self.documents
                            .iter()
                            .filter(|tab| {
                                state.saved.selected_paths.iter().any(|path| {
                                    Self::persisted_path_matches(tab.document.path(), path)
                                })
                            })
                            .collect::<Vec<_>>();
                    if targets.is_empty() {
                        window.notify_message(
                            crate::tr!(
                                "尚未选择参与全局搜索的文件",
                                "No files are selected for global search"
                            ),
                            cx,
                        );
                        return;
                    }
                    if targets
                        .iter()
                        .any(|tab| tab.load_state != DocumentLoadState::Ready)
                    {
                        window.notify_message(
                            crate::tr!(
                                "所选文件的完整索引建立后即可搜索",
                                "Wait for the selected files to finish indexing"
                            ),
                            cx,
                        );
                        return;
                    }
                    SearchTabInput::Open(
                        targets
                            .into_iter()
                            .map(|tab| {
                                (
                                    tab.id,
                                    tab.file.title.clone(),
                                    tab.document.path().to_path_buf(),
                                    tab.document.clone(),
                                )
                            })
                            .collect(),
                    )
                }
                SearchTabOwner::Directory => {
                    let options = Self::restored_directory_options(state.saved.directory.clone());
                    if options.directory.is_none() {
                        if !restore {
                            self.open_directory_search_dialog(window, cx);
                        }
                        return;
                    }
                    SearchTabInput::Directory(
                        options,
                        self.documents
                            .iter()
                            .map(|tab| path_match_key(tab.document.path()))
                            .collect(),
                    )
                }
            };
        self.search_tabs.cancel(owner, id);
        let slot = self.search_tabs.state_mut(owner, id).unwrap();
        slot.needs_restore = false;
        slot.saved.submitted = Some(SearchTabQuery::from_query(&query));
        let revision = slot.revision;
        self.search_tabs.queue.push_back(SearchTabJob {
            owner,
            id,
            revision,
            query,
            ranges: state.ranges,
            input,
        });
        self.persist_search_tabs(window, cx);
        self.pump_search_tab_queue(window, cx);
        cx.notify();
    }

    pub(super) fn restore_active_search_tab_if_needed(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_task.is_some() || self.file_refresh_task.is_some() {
            return;
        }
        let Some((owner, id)) = self.active_search_tab_key() else {
            return;
        };
        if let SearchTabOwner::File(document_id) = owner
            && self
                .documents
                .iter()
                .find(|tab| tab.id == document_id)
                .is_none_or(|tab| tab.load_state != DocumentLoadState::Ready)
        {
            return;
        }
        if self
            .search_tabs
            .state(owner, id)
            .is_some_and(|state| state.needs_restore)
        {
            // Consume the retry flag before submitting; failures require an explicit retry.
            self.search_tabs.state_mut(owner, id).unwrap().needs_restore = false;
            self.enqueue_search_tab(owner, id, true, window, cx);
        }
    }

    pub(super) fn pump_search_tab_queue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_tabs.running.is_some() {
            return;
        }
        let Some(job) = self.search_tabs.queue.pop_front() else {
            return;
        };
        let owner = job.owner;
        let id = job.id;
        let revision = job.revision;
        let cancellation = SearchCancellation::default();
        self.search_tabs.running = Some((owner, id, revision, cancellation.clone()));
        self.search_tabs.task = Some(cx.spawn_in(window, async move |this, cx| {
            let (job, result) = cx
                .background_spawn(async move {
                    let result = job.run(&cancellation);
                    (job, result)
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.search_tabs.running = None;
                this.search_tabs.task = None;
                let valid = this
                    .search_tabs
                    .state(owner, id)
                    .is_some_and(|state| state.revision == revision);
                if valid {
                    this.capture_active_search_tab(cx);
                    this.complete_search_tab(job, result, window, cx);
                }
                this.pump_search_tab_queue(window, cx);
                cx.notify();
            });
        }));
    }

    fn complete_search_tab(
        &mut self,
        job: SearchTabJob,
        output: Result<SearchTabOutput>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mut state) = self.search_tabs.state(job.owner, job.id).cloned() else {
            return;
        };
        state.saved.submitted = None;
        let visible = self.active_search_tab_key() == Some((job.owner, job.id));
        let mut completed = false;
        match output {
            Ok(SearchTabOutput::File(document, result, matcher)) => {
                let current = match job.owner {
                    SearchTabOwner::File(id) => self.documents.iter().find(|tab| tab.id == id),
                    _ => None,
                };
                if current.is_some_and(|tab| Arc::ptr_eq(&tab.document, &document)) {
                    state
                        .ranges
                        .completed(document.path(), job.ranges.get(document.path()), false);
                    state.local_result = Some((document, result, matcher));
                    state.saved.context.results_visible = true;
                    state.saved.local.results_visible = true;
                    completed = true;
                } else {
                    state.needs_restore = true;
                }
            }
            Ok(SearchTabOutput::Global(results, matcher, notice)) => {
                let results = results
                    .into_iter()
                    .map(|result| {
                        let document_id = self
                            .documents
                            .iter()
                            .find(|tab| {
                                result_snapshot_matches_document(
                                    &result.path,
                                    &result.document,
                                    &tab.document,
                                )
                            })
                            .map(|tab| tab.id)
                            .unwrap_or_else(|| {
                                self.global_search.directory_document_id(&result.path)
                            });
                        if job.owner == SearchTabOwner::AllOpen {
                            state.ranges.completed(
                                &result.path,
                                job.ranges.get(&result.path),
                                true,
                            );
                        }
                        (
                            document_id,
                            GlobalSearchDocumentResult {
                                title: result.title,
                                path: result.path,
                                document: result.document,
                                search_result: result.search_result,
                                failure: None,
                            },
                        )
                    })
                    .collect::<GlobalSearchResults>();
                state.saved.context.results_visible = true;
                state.saved.context.query = Self::persisted_search_query(&job.query);
                state.saved.context.source_paths = results
                    .values()
                    .map(|result| encode_persisted_path(&result.path))
                    .collect();
                state.context = self.search_tab_result_context(
                    &state.saved.context,
                    job.query.clone(),
                    results,
                    matcher,
                );
                completed = true;
                if let Some(notice) = notice {
                    window.notify_message(format!("{}: {notice}", state.title()), cx);
                }
            }
            Ok(SearchTabOutput::Cancelled) => {}
            Err(error) => {
                window.notify_message(
                    crate::tr_args!(
                        "{}：搜索失败，可重新搜索。{}",
                        "{}: Search failed; retry the search. {}",
                        state.title(),
                        error
                    ),
                    cx,
                );
            }
        }
        if completed {
            state.saved.completed = Some(SearchTabQuery::from_query(&job.query));
            state.facade_dirty = true;
            self.record_search_history(&job.query.text, window, cx);
        }
        *self.search_tabs.state_mut(job.owner, job.id).unwrap() = state;
        if visible && completed {
            self.install_search_tab(job.owner, job.id, window, cx);
        }
        if let SearchTabOwner::File(document_id) = job.owner {
            self.schedule_checkpoint(document_id, window, cx);
        }
        self.schedule_workspace_search_state_save(window, cx);
    }

    fn search_tab_result_context(
        &self,
        saved: &PersistedGlobalSearchContext,
        query: SearchQuery,
        results: GlobalSearchResults,
        matcher: Option<SearchMatcher>,
    ) -> SearchSessionState {
        let key = |key: &PersistedSearchRowKey| {
            self.global_document_id_for_path(&results, &key.path)
                .map(|id| match key.source_row {
                    Some(source_row) => LogRowKey::Row {
                        document_id: id,
                        source_row,
                    },
                    None => LogRowKey::FileGroup { document_id: id },
                })
        };
        let selected_row = saved
            .selected_row
            .as_ref()
            .and_then(key)
            .map(|key| match key {
                LogRowKey::Row {
                    document_id,
                    source_row,
                } => GlobalSearchRow::Match {
                    document_id,
                    source_row,
                },
                LogRowKey::FileGroup { document_id } => GlobalSearchRow::Group { document_id },
            });
        SearchSessionState {
            query,
            collapsed_document_ids: saved
                .collapsed_paths
                .iter()
                .filter_map(|path| self.global_document_id_for_path(&results, path))
                .collect(),
            selection: saved
                .selection
                .iter()
                .filter_map(|selection| {
                    self.global_document_id_for_path(&results, &selection.path)
                        .map(|id| (id, selection.decoded_rows()))
                })
                .collect(),
            selected_row,
            viewport: saved.viewport.as_ref().and_then(|viewport| {
                key(&viewport.key).map(|key| ViewportAnchor {
                    key,
                    viewport_y: px(viewport.viewport_y()),
                    at_end: viewport.at_end,
                    fallback_ix: viewport.fallback_ix,
                })
            }),
            horizontal_offset: saved
                .viewport
                .as_ref()
                .map_or(0., PersistedSearchViewport::horizontal_offset),
            keyword_color_rules: saved.keyword_color_rules.clone(),
            resolved_color_rules: resolve_color_rules(
                &saved.keyword_color_rules,
                &self.color_labels,
            ),
            color_exclusions: crate::search_color_exclusions::SearchColorExclusions::restore(
                &saved.cleared_color_keywords,
                &saved.keyword_color_rules,
                &self.color_labels,
            ),
            initialized: true,
            results,
            matcher,
            result_mode: ResultMode::from_database(saved.result_mode),
            results_visible: true,
            word_wrap: saved.word_wrap,
            active: saved.active,
            visible_lines: None,
        }
    }
}
