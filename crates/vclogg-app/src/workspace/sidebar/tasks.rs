use super::*;

pub(super) fn release_in_background(value: impl Send + 'static, cx: &App) {
    cx.background_executor()
        .spawn(async move {
            drop(value);
        })
        .detach();
}

static LAYOUT_REVISION: AtomicU64 = AtomicU64::new(0);
static LAYOUT_WRITE: Mutex<()> = Mutex::new(());
static LATEST_LAYOUT: Mutex<Option<String>> = Mutex::new(None);

pub(super) fn record_layout(layout: &SidebarLayout) -> u64 {
    let revision = LAYOUT_REVISION.fetch_add(1, Ordering::SeqCst) + 1;
    if let Ok(mut latest) = LATEST_LAYOUT.lock() {
        *latest = serde_json::to_string(layout).ok();
    }
    revision
}

impl SidebarState {
    pub(super) fn load_layout(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        if self.layout_loaded {
            return;
        }
        self.layout_loaded = true;
        self.persistence_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { store.load_sidebar_layout() })
                .await;
            _ = this.update(cx, |this, cx| {
                this.persistence_task = None;
                if !this.layout_modified {
                    match result {
                        Ok(Some(layout)) => this.layout = SidebarLayout::decode(&layout),
                        Ok(None) => {}
                        Err(error) => this.persistence_error = Some(error.to_string()),
                    }
                    if let Some(latest) =
                        LATEST_LAYOUT.lock().ok().and_then(|latest| latest.clone())
                    {
                        this.layout = SidebarLayout::decode(&latest);
                    }
                    this.ensure_visible_data(cx);
                    if this.is_showing(SidebarPanelId::History) {
                        this.refresh_history(cx);
                    }
                    cx.emit(SidebarChanged);
                    cx.notify();
                } else {
                    this.save_layout(cx);
                }
            });
        }));
    }

    pub(super) fn save_layout(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let Ok(value) = serde_json::to_string(&self.layout) else {
            return;
        };
        let revision = self.layout_revision;
        // The revision is issued at the user action, not at write completion. A slow
        // older window can never overwrite the newest explicit layout preference.
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let _guard = LAYOUT_WRITE
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Layout write lock unavailable"))?;
                    if LAYOUT_REVISION.load(Ordering::SeqCst) == revision {
                        store.save_sidebar_layout(&value)?;
                    }
                    Ok::<_, anyhow::Error>(())
                })
                .await;
            _ = this.update(cx, |this, cx| {
                if this.layout_revision != revision {
                    return;
                }
                this.persistence_error = result.err().map(|error| error.to_string());
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn refresh_history(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        self.history_error = None;
        self.history_request += 1;
        let request = self.history_request;
        self.history_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let mut rows = store.session_history()?;
                    rows.sort_by_key(|row| std::cmp::Reverse(row.last_opened_at));
                    Ok::<_, anyhow::Error>(rows)
                })
                .await;
            _ = this.update(cx, |this, cx| {
                if this.history_request != request {
                    release_in_background(result, cx);
                    return;
                }
                this.history_task = None;
                match result {
                    Ok(rows) => {
                        release_in_background(std::mem::replace(&mut this.history, rows), cx);
                    }
                    Err(error) => this.history_error = Some(error.to_string()),
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn start_summary(&mut self, cx: &mut Context<Self>) {
        let Some((_, document)) = self.document.clone() else {
            return;
        };
        let appended = self
            .append_pair
            .as_ref()
            .filter(|(old, new)| {
                Arc::ptr_eq(new, &document)
                    && self
                        .summary_source
                        .as_ref()
                        .is_some_and(|source| Arc::ptr_eq(source, old))
            })
            .and(self.summary.clone());
        let generation = self.generation;
        let cancellation = CancellationToken::default();
        let worker_cancellation = cancellation.clone();
        let progress = Arc::new(AtomicUsize::new(0));
        self.progress = progress.clone();
        self.summary_error = None;
        let source = document.clone();
        self.summary_request += 1;
        let request = self.summary_request;
        let task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    vclogg_core::summarize_navigation(
                        &source,
                        appended.as_deref(),
                        &worker_cancellation,
                        &progress,
                    )
                })
                .await;
            _ = this.update(cx, |this, cx| {
                if this.summary_request != request || this.generation != generation {
                    release_in_background(result, cx);
                    return;
                }
                this.summary_job = None;
                this.progress_task = None;
                match result {
                    Ok(Some(summary)) => {
                        release_in_background(this.summary.replace(Arc::new(summary)), cx);
                        this.summary_source = Some(document);
                        this.append_pair = None;
                        this.overview_dirty = true;
                        this.prepare_overview_marks(cx);
                    }
                    Ok(None) => {}
                    Err(error) => this.summary_error = Some(error.to_string()),
                }
                cx.notify();
            });
        });
        self.summary_job = Some(SidebarJob {
            cancellation,
            _task: task,
        });
        self.progress_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                if !this
                    .update(cx, |this, cx| {
                        cx.notify();
                        this.summary_job.is_some()
                    })
                    .unwrap_or(false)
                {
                    break;
                }
            }
        }));
    }

    pub(super) fn start_colors(&mut self, cx: &mut Context<Self>) {
        let Some((_, document)) = self
            .document
            .clone()
            .filter(|(_, document)| document.has_complete_line_index())
        else {
            return;
        };
        let generation = self.generation;
        let revision = self.color_revision;
        let rules = self.rules.clone();
        let labels = self.labels.clone();
        let cancellation = CancellationToken::default();
        let worker_cancellation = cancellation.clone();
        self.color_request += 1;
        let request = self.color_request;
        let task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let mut groups: BTreeMap<String, ColorGroup> = BTreeMap::new();
                    for (rule_ix, rule) in rules
                        .iter()
                        .enumerate()
                        .filter(|(_, rule)| rule.enabled && !rule.keyword.is_empty())
                    {
                        if rule.case_sensitive
                            && rules[rule_ix + 1..].iter().any(|later| {
                                later.enabled
                                    && later.case_sensitive
                                    && later.keyword == rule.keyword
                            })
                        {
                            continue;
                        }
                        if worker_cancellation.is_cancelled() {
                            return Ok(None);
                        }
                        // Only explicit, still-existing color-label assignments belong here.
                        // Plain keyword rules and text annotations are separate features.
                        let Some(label) = rule
                            .label_id
                            .as_ref()
                            .and_then(|id| labels.iter().find(|label| &label.id == id))
                        else {
                            continue;
                        };
                        let color = label.background_color;
                        let alpha = label.background_alpha;
                        let id = format!("label-{}", label.id);
                        let matcher = SearchMatcher::literal(&rule.keyword, rule.case_sensitive)?;
                        let rows = match vclogg_core::search_with_compiled_matcher(
                            &document,
                            matcher.as_ref(),
                            None,
                            &worker_cancellation,
                        ) {
                            SearchRun::Completed(result) => result.line_indices,
                            SearchRun::Cancelled => return Ok(None),
                            SearchRun::SourceChanged => {
                                anyhow::bail!("Source changed while indexing color labels")
                            }
                        };
                        groups
                            .entry(id.clone())
                            .or_insert_with(|| ColorGroup {
                                id,
                                label: label.localized_name(),
                                color,
                                alpha,
                                rows: CompressedRows::default(),
                            })
                            .rows
                            .insert_rows(&rows);
                    }
                    Ok::<_, anyhow::Error>(Some(
                        groups
                            .into_values()
                            .filter(|group| !group.rows.is_empty())
                            .collect::<Vec<_>>(),
                    ))
                })
                .await;
            _ = this.update(cx, |this, cx| {
                if this.color_request != request
                    || this.generation != generation
                    || this.color_revision != revision
                {
                    release_in_background(result, cx);
                    return;
                }
                this.color_job = None;
                match result {
                    Ok(Some(groups)) => {
                        release_in_background(
                            std::mem::replace(&mut this.colors, Arc::new(groups)),
                            cx,
                        );
                        this.colors_loaded = true;
                        this.overview_dirty = true;
                        this.prepare_overview_marks(cx);
                        this.request_previews(cx);
                    }
                    Ok(None) => {}
                    Err(error) => this.color_error = Some(error.to_string()),
                }
                cx.notify();
            });
        });
        self.color_job = Some(SidebarJob {
            cancellation,
            _task: task,
        });
    }

    pub(super) fn color_item(&self, mut ix: usize) -> Option<(usize, Option<usize>)> {
        for (group_ix, group) in self.colors.iter().enumerate() {
            if ix == 0 {
                return Some((group_ix, None));
            }
            ix -= 1;
            if !self.collapsed_colors.contains(&group.id) {
                if ix < group.rows.len() {
                    return Some((group_ix, group.rows.get(ix)));
                }
                ix -= group.rows.len();
            }
        }
        None
    }

    pub(super) fn color_item_count(&self) -> usize {
        self.colors
            .iter()
            .map(|group| {
                1 + if self.collapsed_colors.contains(&group.id) {
                    0
                } else {
                    group.rows.len()
                }
            })
            .sum()
    }

    pub(super) fn request_previews(&mut self, cx: &mut Context<Self>) {
        let Some((_, document)) = self.document.clone() else {
            return;
        };
        // The list supplies its actual visible range after layout, including keyboard scrolling.
        let top = self.color_visible_start;
        let range = top..top.saturating_add(96).min(self.color_item_count());
        if self.preview_range.as_ref() == Some(&range) {
            return;
        }
        self.preview_range = Some(range.clone());
        let rows = range
            .filter_map(|ix| self.color_item(ix).and_then(|(_, row)| row))
            .collect::<BTreeSet<_>>();
        let generation = self.generation;
        let revision = self.color_revision;
        let cancellation = CancellationToken::default();
        let worker_cancellation = cancellation.clone();
        self.preview_request += 1;
        let request = self.preview_request;
        let task = cx.spawn(async move |this, cx| {
            let previews = cx
                .background_spawn(async move {
                    let mut reader = LinePreviewReader::default();
                    let mut previews = BTreeMap::new();
                    for row in rows {
                        if worker_cancellation.is_cancelled() {
                            return None;
                        }
                        let text = reader
                            .line_preview(&document, row, 256)
                            .map(|line| line.text().to_owned())
                            .unwrap_or_else(|| {
                                crate::tr!(
                                    "该行已不可用，请重新加载",
                                    "Line unavailable; reload the file"
                                )
                                .to_string()
                            });
                        previews.insert(row, text);
                    }
                    Some(previews)
                })
                .await;
            _ = this.update(cx, |this, cx| {
                if this.preview_request != request {
                    return;
                }
                if this.generation != generation || this.color_revision != revision {
                    return;
                }
                if let Some(previews) = previews {
                    this.previews = previews;
                }
                this.preview_job = None;
                cx.notify();
            });
        });
        self.preview_job = Some(SidebarJob {
            cancellation,
            _task: task,
        });
    }
}
