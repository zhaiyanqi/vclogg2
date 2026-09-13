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
}
