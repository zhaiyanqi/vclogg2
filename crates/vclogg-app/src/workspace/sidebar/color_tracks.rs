use super::overview::{OVERVIEW_BINS, OverviewSnapshot, overview_row_range};
use super::*;

#[derive(Clone, PartialEq)]
pub(super) struct ColorSpec {
    label: ColorLabel,
    rules: Vec<(String, bool)>,
}

pub(super) struct ColorScanJob {
    request: u64,
    _job: SidebarJob,
}

impl SidebarState {
    pub(super) fn reset_color_tracks(&mut self, cx: &App) {
        self.color_jobs.clear();
        self.color_specs.clear();
        self.color_errors.clear();
        self.color_error = None;
        tasks::release_in_background(std::mem::take(&mut self.color_cache), cx);
        tasks::release_in_background(std::mem::take(&mut self.colors), cx);
    }

    pub(super) fn sync_color_tracks(&mut self, cx: &mut Context<Self>) {
        let mut specs: BTreeMap<String, ColorSpec> = BTreeMap::new();
        for (ix, rule) in self
            .rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| rule.enabled && !rule.keyword.is_empty())
        {
            // Preserve the effective assignment semantics of the previous color scanner.
            if rule.case_sensitive
                && self.rules[ix + 1..].iter().any(|later| {
                    later.enabled && later.case_sensitive && later.keyword == rule.keyword
                })
            {
                continue;
            }
            let Some(label) = rule
                .label_id
                .as_ref()
                .and_then(|id| self.labels.iter().find(|label| &label.id == id))
            else {
                continue;
            };
            specs
                .entry(format!("label-{}", label.id))
                .or_insert_with(|| ColorSpec {
                    label: label.clone(),
                    rules: Vec::new(),
                })
                .rules
                .push((rule.keyword.clone(), rule.case_sensitive));
        }
        for spec in specs.values_mut() {
            spec.rules.sort();
            spec.rules.dedup();
        }
        if specs == self.color_specs {
            return;
        }
        for (id, old) in &self.color_specs {
            if specs.get(id).is_none_or(|spec| spec.rules != old.rules) {
                self.color_jobs.remove(id);
                self.color_errors.remove(id);
                tasks::release_in_background(self.color_cache.remove(id), cx);
            }
        }
        self.color_specs = specs;
        // Name and swatch edits reuse both the compressed rows and the full-file counts.
        for (id, group) in &mut self.color_cache {
            if let Some(spec) = self.color_specs.get(id) {
                group.label = spec.label.localized_name();
                group.color = spec.label.background_color;
                group.alpha = spec.label.background_alpha;
            }
        }
        self.publish_color_tracks(cx);
    }

    pub(super) fn retry_colors(&mut self, cx: &mut Context<Self>) {
        self.color_errors.clear();
        self.color_error = None;
        self.start_colors(cx);
        cx.notify();
    }

    pub(super) fn start_colors(&mut self, cx: &mut Context<Self>) {
        if !self.is_showing(SidebarPanelId::Colors) {
            return;
        }
        let Some((_, document)) = self
            .document
            .clone()
            .filter(|(_, document)| document.has_complete_line_index())
        else {
            return;
        };
        // Keep independent jobs bounded; deleting one label never cancels another label's scan.
        let pending = self
            .color_specs
            .iter()
            .filter(|(id, _)| {
                !self.color_cache.contains_key(*id)
                    && !self.color_jobs.contains_key(*id)
                    && !self.color_errors.contains_key(*id)
            })
            .take(2_usize.saturating_sub(self.color_jobs.len()))
            .map(|(id, spec)| (id.clone(), spec.clone()))
            .collect::<Vec<_>>();
        for (id, spec) in pending {
            self.color_request += 1;
            let request = self.color_request;
            let generation = self.generation;
            let source = document.clone();
            let cancellation = CancellationToken::default();
            let worker = cancellation.clone();
            let task_id = id.clone();
            let task = cx.spawn(async move |this, cx| {
                let worker_id = task_id.clone();
                let result = cx
                    .background_spawn(async move {
                        let mut rows = CompressedRows::default();
                        for (keyword, case_sensitive) in &spec.rules {
                            if worker.is_cancelled() {
                                return Ok(None);
                            }
                            let matcher = SearchMatcher::literal(keyword, *case_sensitive)?;
                            match vclogg_core::search_with_compiled_matcher(
                                &source,
                                matcher.as_ref(),
                                None,
                                &worker,
                            ) {
                                SearchRun::Completed(result) => {
                                    rows.insert_rows(&result.line_indices)
                                }
                                SearchRun::Cancelled => return Ok(None),
                                SearchRun::SourceChanged => {
                                    anyhow::bail!("Source changed while indexing color labels")
                                }
                            }
                        }
                        let range = 0..source.source_line_count();
                        let bins = range.len().min(OVERVIEW_BINS);
                        let mut counts = Vec::new();
                        if !rows.is_empty() {
                            counts.reserve(bins);
                            for ix in 0..bins {
                                if worker.is_cancelled() {
                                    return Ok(None);
                                }
                                counts.push(rows.count_in_range(overview_row_range(
                                    &range,
                                    bins,
                                    ix..ix + 1,
                                )));
                            }
                        }
                        anyhow::ensure!(
                            !source.source_changed()?,
                            "Source changed while indexing color labels"
                        );
                        Ok::<_, anyhow::Error>(Some(ColorGroup {
                            id: worker_id,
                            label: spec.label.localized_name(),
                            color: spec.label.background_color,
                            alpha: spec.label.background_alpha,
                            rows,
                            counts: Arc::new(counts),
                        }))
                    })
                    .await;
                _ = this.update(cx, |this, cx| {
                    if this.generation != generation
                        || this
                            .color_jobs
                            .get(&task_id)
                            .is_none_or(|job| job.request != request)
                    {
                        tasks::release_in_background(result, cx);
                        return;
                    }
                    this.color_jobs.remove(&task_id);
                    match result {
                        Ok(Some(mut group)) => {
                            // Metadata may have changed while the unchanged match rules were scanning.
                            if let Some(spec) = this.color_specs.get(&task_id) {
                                group.label = spec.label.localized_name();
                                group.color = spec.label.background_color;
                                group.alpha = spec.label.background_alpha;
                                tasks::release_in_background(
                                    this.color_cache.insert(task_id.clone(), group),
                                    cx,
                                );
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            this.color_errors.insert(task_id.clone(), error.to_string());
                        }
                    }
                    this.publish_color_tracks(cx);
                    this.start_colors(cx);
                    cx.notify();
                });
            });
            self.color_jobs.insert(
                id,
                ColorScanJob {
                    request,
                    _job: SidebarJob {
                        cancellation,
                        _task: task,
                    },
                },
            );
        }
    }

    fn publish_color_tracks(&mut self, cx: &mut Context<Self>) {
        self.color_error = self.color_errors.values().next().cloned();
        let groups = self
            .color_cache
            .values()
            .filter(|group| !group.rows.is_empty())
            .cloned()
            .collect::<Vec<_>>();
        if groups.len() == self.colors.len()
            && groups
                .iter()
                .zip(self.colors.iter())
                .all(|(a, b)| same_group(a, b))
        {
            return;
        }
        tasks::release_in_background(std::mem::replace(&mut self.colors, Arc::new(groups)), cx);
        self.color_overview.dirty = true;
        if self.colors.is_empty() {
            self.invalidate_overview(SidebarPanelId::Colors, cx);
        } else {
            // Install only a new arrangement of cached tracks; never replace the page with Loading.
            self.prepare_overview(SidebarPanelId::Colors, cx);
        }
    }

    pub(super) fn install_color_overview(&mut self, snapshot: OverviewSnapshot, cx: &App) {
        let state = &mut self.color_overview;
        state.dirty = false;
        if let Some(old) = &state.snapshot {
            if old.range == snapshot.range
                && old.tracks.len() == snapshot.tracks.len()
                && old.tracks.iter().zip(&snapshot.tracks).all(|(a, b)| {
                    a.group
                        .as_ref()
                        .zip(b.group.as_ref())
                        .is_some_and(|(a, b)| same_group(a, b))
                })
            {
                return;
            }
            // A removed or inserted preceding track must not silently change the hovered label.
            state.hover = state.hover.as_ref().and_then(|hover| {
                if old.range != snapshot.range {
                    return None;
                }
                let id = &old.tracks.get(hover.lane)?.group.as_ref()?.id;
                let lane = snapshot
                    .tracks
                    .iter()
                    .position(|track| track.group.as_ref().is_some_and(|group| &group.id == id))?;
                Some(minimap::OverviewHover {
                    lane,
                    bins: hover.bins.clone(),
                })
            });
        }
        tasks::release_in_background(state.snapshot.replace(Arc::new(snapshot)), cx);
    }
}

fn same_group(a: &ColorGroup, b: &ColorGroup) -> bool {
    a.id == b.id
        && a.label == b.label
        && a.color == b.color
        && a.alpha == b.alpha
        && a.rows.shares_storage(&b.rows)
        && Arc::ptr_eq(&a.counts, &b.counts)
}
