use super::*;

impl Workspace {
    pub(super) fn open_quick_find(
        &mut self,
        _: &OpenQuickFind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active_document() else {
            return;
        };
        if tab.load_state != DocumentLoadState::Ready {
            window.push_notification(
                crate::tr!(
                    "完整索引建立后即可查找",
                    "Find will be available after the full index is built"
                ),
                cx,
            );
            return;
        }
        let (target, anchor) = match self.last_user_log_region {
            LogRegion::GlobalResults
                if self.global_search.scope != SearchScope::CurrentFile
                    && self.global_search.results_visible =>
            {
                (
                    QuickFindTarget::GlobalResults,
                    self.global_table
                        .read(cx)
                        .active_log_row()
                        .unwrap_or_default(),
                )
            }
            LogRegion::CurrentResults
                if self.global_search.scope == SearchScope::CurrentFile && tab.results_visible =>
            {
                (
                    QuickFindTarget::Results(tab.id),
                    tab.result_table
                        .read(cx)
                        .active_log_row()
                        .unwrap_or_default(),
                )
            }
            _ => (
                QuickFindTarget::Log(tab.id),
                tab.log_table.read(cx).active_log_row().unwrap_or_default(),
            ),
        };

        self.quick_find.open(target, anchor);
        self.update_quick_find_matcher(window, cx);
        let focus = self.quick_find.query.focus_handle(cx);
        self.quick_find
            .query
            .update(cx, |state, cx| state.select_all(window, cx));
        window.defer(cx, move |window, cx| focus.focus(window, cx));
        if !self.quick_find.query.read(cx).value().is_empty() {
            self.start_quick_find(QuickFindDirection::Forward, true, window, cx);
        }
        cx.notify();
    }

    pub(super) fn quick_find_input_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.quick_find.open && self.quick_find.query.focus_handle(cx).is_focused(window)
    }

    pub(super) fn close_quick_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.quick_find.close();
        self.refresh_quick_find_highlights(cx);
        match target {
            Some(QuickFindTarget::Log(_)) => self.log_viewer.focus_handle.focus(window, cx),
            Some(QuickFindTarget::Results(_)) => {
                self.search_results_viewer.focus_handle.focus(window, cx)
            }
            Some(QuickFindTarget::GlobalResults) => {
                self.search_results_viewer.focus_handle.focus(window, cx)
            }
            None => self.focus_handle.focus(window, cx),
        }
        cx.notify();
    }

    pub(super) fn cancel_quick_find_work(&mut self) {
        self.quick_find.cancel_work();
    }

    pub(super) fn update_quick_find_matcher(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = self.quick_find.query.read(cx).value().to_string();
        match SearchMatcher::quick_find(
            &query,
            self.quick_find.case_sensitive,
            self.quick_find.whole_word,
            self.quick_find.regex,
        ) {
            Ok(matcher) => {
                self.quick_find.matcher = matcher;
                self.quick_find.error = None;
            }
            Err(error) => {
                self.quick_find.matcher = None;
                self.quick_find.error = Some(error.to_string().into());
            }
        }
        self.refresh_quick_find_highlights(cx);
    }

    pub(super) fn focus_quick_find_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.quick_find.query.focus_handle(cx);
        window.defer(cx, move |window, cx| focus.focus(window, cx));
    }

    pub(super) fn toggle_quick_find_case_sensitive(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.quick_find.case_sensitive = !self.quick_find.case_sensitive;
        self.schedule_incremental_quick_find(window, cx);
        self.focus_quick_find_input(window, cx);
    }

    pub(super) fn toggle_quick_find_whole_word(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.quick_find.whole_word = !self.quick_find.whole_word;
        self.schedule_incremental_quick_find(window, cx);
        self.focus_quick_find_input(window, cx);
    }

    pub(super) fn toggle_quick_find_regex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.quick_find.regex = !self.quick_find.regex;
        self.schedule_incremental_quick_find(window, cx);
        self.focus_quick_find_input(window, cx);
    }

    pub(super) fn refresh_quick_find_highlights(&mut self, cx: &mut Context<Self>) {
        let matcher = self.quick_find.matcher.clone();
        for tab in &self.documents {
            tab.log_table.update(cx, |table, cx| {
                table.delegate_mut().set_quick_find_matcher(matcher.clone());
                table.refresh(cx);
            });
            tab.result_table.update(cx, |table, cx| {
                table.delegate_mut().set_quick_find_matcher(matcher.clone());
                table.refresh(cx);
            });
        }
        self.global_table.update(cx, |table, cx| {
            table.delegate_mut().set_quick_find_matcher(matcher);
            table.refresh(cx);
        });
    }

    pub(super) fn schedule_incremental_quick_find(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.quick_find.open {
            return;
        }
        self.cancel_quick_find_work();
        self.quick_find.clear_match();
        self.quick_find.no_match = false;
        self.quick_find.boundary = None;
        self.update_quick_find_matcher(window, cx);
        if self.quick_find.query.read(cx).value().is_empty() || self.quick_find.matcher.is_none() {
            cx.notify();
            return;
        }
        let revision = self.quick_find.revision;
        self.quick_find.task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                if this.quick_find.open && this.quick_find.revision == revision {
                    this.start_quick_find(QuickFindDirection::Forward, true, window, cx);
                }
            });
        }));
        cx.notify();
    }

    pub(super) fn quick_find_source(
        &self,
        target: QuickFindTarget,
        cx: &App,
    ) -> Option<(QuickFindSource, usize, QuickFindSourceVersion)> {
        match target {
            QuickFindTarget::Log(document_id) => {
                let tab = self.documents.iter().find(|tab| tab.id == document_id)?;
                let row_count = tab.document.line_count();
                Some((
                    QuickFindSource::Document {
                        document: tab.document.clone(),
                        rows: None,
                        row_count,
                    },
                    row_count,
                    QuickFindSourceVersion::Document {
                        document: tab.document.clone(),
                        rows: None,
                    },
                ))
            }
            QuickFindTarget::Results(document_id) => {
                let tab = self.documents.iter().find(|tab| tab.id == document_id)?;
                let rows = tab.result_rows(cx);
                let row_count = rows.len();
                Some((
                    QuickFindSource::Document {
                        document: tab.document.clone(),
                        rows: Some(rows.clone()),
                        row_count,
                    },
                    row_count,
                    QuickFindSourceVersion::Document {
                        document: tab.document.clone(),
                        rows: Some(rows),
                    },
                ))
            }
            QuickFindTarget::GlobalResults => {
                let table = self.global_table.read(cx);
                let row_count = table.delegate().rows_len();
                Some((
                    QuickFindSource::Global(table.delegate().quick_find_groups()),
                    row_count,
                    QuickFindSourceVersion::Global {
                        content_revision: table.delegate().content_revision(),
                        layout_revision: table.delegate().layout_revision(),
                    },
                ))
            }
        }
    }

    pub(super) fn quick_find_source_version(
        &self,
        target: QuickFindTarget,
        cx: &App,
    ) -> Option<QuickFindSourceVersion> {
        match target {
            QuickFindTarget::Log(document_id) => {
                let tab = self.documents.iter().find(|tab| tab.id == document_id)?;
                Some(QuickFindSourceVersion::Document {
                    document: tab.document.clone(),
                    rows: None,
                })
            }
            QuickFindTarget::Results(document_id) => {
                let tab = self.documents.iter().find(|tab| tab.id == document_id)?;
                Some(QuickFindSourceVersion::Document {
                    document: tab.document.clone(),
                    rows: Some(tab.result_rows(cx)),
                })
            }
            QuickFindTarget::GlobalResults => {
                let table = self.global_table.read(cx);
                Some(QuickFindSourceVersion::Global {
                    content_revision: table.delegate().content_revision(),
                    layout_revision: table.delegate().layout_revision(),
                })
            }
        }
    }

    pub(super) fn start_quick_find(
        &mut self,
        direction: QuickFindDirection,
        incremental: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.quick_find.open {
            return;
        }
        let query = self.quick_find.query.read(cx).value().to_string();
        if query.is_empty() {
            self.cancel_quick_find_work();
            self.quick_find.clear_match();
            self.quick_find.no_match = false;
            self.quick_find.boundary = None;
            cx.notify();
            return;
        }
        let Some(target) = self.quick_find.target else {
            return;
        };
        let Some(matcher) = self.quick_find.matcher.clone() else {
            self.update_quick_find_matcher(window, cx);
            let Some(matcher) = self.quick_find.matcher.clone() else {
                return;
            };
            return self.start_quick_find_with_matcher(
                target,
                matcher,
                direction,
                incremental,
                window,
                cx,
            );
        };
        self.start_quick_find_with_matcher(target, matcher, direction, incremental, window, cx);
    }

    pub(super) fn start_quick_find_with_matcher(
        &mut self,
        target: QuickFindTarget,
        matcher: SearchMatcher,
        direction: QuickFindDirection,
        incremental: bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((source, row_count, source_version)) = self.quick_find_source(target, cx) else {
            return;
        };
        if row_count == 0 {
            self.quick_find.busy = false;
            self.quick_find.direction = None;
            self.quick_find.no_match |= incremental;
            self.quick_find.boundary = Some(match direction {
                QuickFindDirection::Forward => QuickFindBoundary::End,
                QuickFindDirection::Backward => QuickFindBoundary::Start,
            });
            cx.notify();
            return;
        }

        let requested_boundary = match direction {
            QuickFindDirection::Forward => QuickFindBoundary::End,
            QuickFindDirection::Backward => QuickFindBoundary::Start,
        };
        if !incremental && self.quick_find.boundary == Some(requested_boundary) {
            self.quick_find.busy = false;
            self.quick_find.direction = None;
            return;
        }

        let matched_source_is_current = self
            .quick_find
            .matched_source_version
            .as_ref()
            .is_some_and(|version| version.is_same_as(&source_version));
        if self.quick_find.matched.is_some() && !matched_source_is_current {
            self.quick_find.clear_match();
        }
        let current_match = (!incremental)
            .then_some(self.quick_find.matched)
            .flatten()
            .filter(|matched| matched.target == target);
        let start = match (direction, current_match) {
            (QuickFindDirection::Forward, Some(matched)) => {
                (matched.view_row + 1 < row_count).then_some(matched.view_row + 1)
            }
            (QuickFindDirection::Backward, Some(matched)) => matched.view_row.checked_sub(1),
            (QuickFindDirection::Forward, None) => Some(self.quick_find.anchor.min(row_count - 1)),
            (QuickFindDirection::Backward, None) => {
                self.quick_find.anchor.min(row_count - 1).checked_sub(1)
            }
        };
        let Some(start) = start else {
            self.quick_find.busy = false;
            self.quick_find.direction = None;
            self.quick_find.no_match |= incremental;
            self.quick_find.boundary = Some(match direction {
                QuickFindDirection::Forward => QuickFindBoundary::End,
                QuickFindDirection::Backward => QuickFindBoundary::Start,
            });
            cx.notify();
            return;
        };

        self.cancel_quick_find_work();
        let revision = self.quick_find.revision;
        let cancellation = SearchCancellation::default();
        self.quick_find.cancellation = Some(cancellation.clone());
        self.quick_find.busy = true;
        self.quick_find.direction = Some(direction);
        self.quick_find.boundary = None;
        cx.notify();
        self.quick_find.task = Some(cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_spawn(async move {
                    Self::find_quick_match(source, target, matcher, direction, start, cancellation)
                })
                .await;
            _ = this.update(cx, |this, cx| {
                if !this.quick_find.open
                    || this.quick_find.revision != revision
                    || this.quick_find.target != Some(target)
                {
                    return;
                }
                if this
                    .quick_find_source_version(target, cx)
                    .is_none_or(|current| !current.is_same_as(&source_version))
                {
                    this.quick_find.busy = false;
                    this.quick_find.direction = None;
                    this.quick_find.cancellation = None;
                    this.quick_find.task = None;
                    this.quick_find.clear_match();
                    this.quick_find.no_match = false;
                    this.quick_find.boundary = None;
                    cx.notify();
                    return;
                }
                this.quick_find.busy = false;
                this.quick_find.direction = None;
                this.quick_find.cancellation = None;
                this.quick_find.task = None;
                match outcome {
                    DocumentLineTask::Completed(Some(matched)) => {
                        this.quick_find.matched = Some(matched);
                        this.quick_find.matched_source_version = Some(source_version);
                        this.quick_find.no_match = false;
                        this.quick_find.boundary = None;
                        this.apply_quick_find_match(matched, cx);
                    }
                    DocumentLineTask::Completed(None) => {
                        this.quick_find.no_match |= incremental;
                        this.quick_find.boundary = Some(match direction {
                            QuickFindDirection::Forward => QuickFindBoundary::End,
                            QuickFindDirection::Backward => QuickFindBoundary::Start,
                        });
                    }
                    DocumentLineTask::Cancelled => return,
                    DocumentLineTask::SourceUnavailable => {
                        this.quick_find.clear_match();
                        this.quick_find.no_match = false;
                        this.quick_find.boundary = None;
                        this.quick_find.error = Some(
                            crate::tr!(
                                "文件内容已改变，请重新加载后再查找",
                                "The file changed. Reload it before finding."
                            )
                            .into(),
                        );
                    }
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn find_quick_match(
        source: QuickFindSource,
        target: QuickFindTarget,
        matcher: SearchMatcher,
        direction: QuickFindDirection,
        start: usize,
        cancellation: SearchCancellation,
    ) -> DocumentLineTask<Option<QuickFindMatch>> {
        let inspect = |view_row: usize,
                       source_row: usize,
                       document: &Arc<LogDocument>,
                       reader: &mut LineReader| {
            if cancellation.is_cancelled() {
                return DocumentLineTask::Cancelled;
            }
            let Some(line) = reader.line(document, source_row) else {
                return DocumentLineTask::SourceUnavailable;
            };
            DocumentLineTask::Completed((!matcher.matching_ranges(&line).is_empty()).then_some(
                QuickFindMatch {
                    target,
                    view_row,
                    source_row,
                },
            ))
        };

        match source {
            QuickFindSource::Document {
                document,
                rows,
                row_count,
            } => {
                let mut reader = LineReader::default();
                match direction {
                    QuickFindDirection::Forward => {
                        for view_row in start..row_count {
                            if cancellation.is_cancelled() {
                                return DocumentLineTask::Cancelled;
                            }
                            let source_row = rows
                                .as_ref()
                                .and_then(|rows| rows.get(view_row))
                                .unwrap_or(view_row);
                            match inspect(view_row, source_row, &document, &mut reader) {
                                DocumentLineTask::Completed(Some(matched)) => {
                                    return DocumentLineTask::Completed(Some(matched));
                                }
                                DocumentLineTask::Completed(None) => {}
                                outcome => return outcome,
                            }
                        }
                        DocumentLineTask::Completed(None)
                    }
                    QuickFindDirection::Backward => {
                        if row_count == 0 {
                            return DocumentLineTask::Completed(None);
                        }
                        for view_row in (0..=start.min(row_count - 1)).rev() {
                            if cancellation.is_cancelled() {
                                return DocumentLineTask::Cancelled;
                            }
                            let source_row = rows
                                .as_ref()
                                .and_then(|rows| rows.get(view_row))
                                .unwrap_or(view_row);
                            match inspect(view_row, source_row, &document, &mut reader) {
                                DocumentLineTask::Completed(Some(matched)) => {
                                    return DocumentLineTask::Completed(Some(matched));
                                }
                                DocumentLineTask::Completed(None) => {}
                                outcome => return outcome,
                            }
                        }
                        DocumentLineTask::Completed(None)
                    }
                }
            }
            QuickFindSource::Global(groups) => match direction {
                QuickFindDirection::Forward => {
                    for group in &groups {
                        let mut reader = LineReader::default();
                        let first = start.saturating_sub(group.view_start).min(group.rows.len());
                        for result_ix in first..group.rows.len() {
                            let Some(source_row) = group.rows.get(result_ix) else {
                                continue;
                            };
                            let view_row = group.view_start.saturating_add(result_ix);
                            match inspect(view_row, source_row, &group.document, &mut reader) {
                                DocumentLineTask::Completed(Some(matched)) => {
                                    return DocumentLineTask::Completed(Some(matched));
                                }
                                DocumentLineTask::Completed(None) => {}
                                outcome => return outcome,
                            }
                        }
                    }
                    DocumentLineTask::Completed(None)
                }
                QuickFindDirection::Backward => {
                    for group in groups.iter().rev() {
                        let mut reader = LineReader::default();
                        if group.rows.is_empty() || start < group.view_start {
                            continue;
                        }
                        let last = start
                            .saturating_sub(group.view_start)
                            .min(group.rows.len().saturating_sub(1));
                        for result_ix in (0..=last).rev() {
                            let Some(source_row) = group.rows.get(result_ix) else {
                                continue;
                            };
                            let view_row = group.view_start.saturating_add(result_ix);
                            match inspect(view_row, source_row, &group.document, &mut reader) {
                                DocumentLineTask::Completed(Some(matched)) => {
                                    return DocumentLineTask::Completed(Some(matched));
                                }
                                DocumentLineTask::Completed(None) => {}
                                outcome => return outcome,
                            }
                        }
                    }
                    DocumentLineTask::Completed(None)
                }
            },
        }
    }

    pub(super) fn apply_quick_find_match(
        &mut self,
        matched: QuickFindMatch,
        cx: &mut Context<Self>,
    ) {
        match matched.target {
            QuickFindTarget::Log(document_id) => {
                let Some(tab) = self.documents.iter_mut().find(|tab| tab.id == document_id) else {
                    return;
                };
                tab.view.auto_follow = false;
                tab.view.selection_table = SelectionTable::Log;
                tab.log_table.update(cx, |table, cx| {
                    table.set_active_log_row(matched.view_row, cx);
                });
                tab.log_viewport.center_row(matched.view_row);
                self.active_log_region = LogRegion::Body;
                self.selected_source_row = Some(matched.source_row);
            }
            QuickFindTarget::Results(document_id) => {
                let Some(tab) = self.documents.iter_mut().find(|tab| tab.id == document_id) else {
                    return;
                };
                tab.view.auto_follow = false;
                tab.view.selection_table = SelectionTable::Results;
                tab.result_table.update(cx, |table, cx| {
                    table.set_active_log_row(matched.view_row, cx);
                });
                tab.result_viewport.center_row(matched.view_row);
                self.active_log_region = LogRegion::CurrentResults;
                self.selected_source_row = Some(matched.source_row);
            }
            QuickFindTarget::GlobalResults => {
                self.global_table.update(cx, |table, cx| {
                    table.set_active_log_row(matched.view_row, cx);
                });
                self.global_viewport.center_row(matched.view_row);
                self.active_log_region = LogRegion::GlobalResults;
            }
        }
    }
}
