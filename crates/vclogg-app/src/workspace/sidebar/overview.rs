use super::*;

pub(super) const OVERVIEW_BINS: usize = 1024;
pub(super) const COLOR_TRACKS_PER_PAGE: usize = 8;

pub(super) struct OverviewTrack {
    pub(super) group: Option<ColorGroup>,
    pub(super) rows: CompressedRows,
    pub(super) counts: Arc<Vec<usize>>,
}

impl OverviewTrack {
    pub(super) fn label(&self) -> String {
        self.group.as_ref().map_or_else(
            || crate::tr!("搜索", "Search").to_owned(),
            |group| group.label.clone(),
        )
    }

    pub(super) fn target(&self, range: Range<usize>, row: usize) -> usize {
        if let Some(ix) = self.rows.nearest_position(row) {
            let nearest = self.rows.get(ix).unwrap_or(row);
            if range.contains(&nearest) {
                return nearest;
            }
            let next = if nearest < range.start {
                self.rows.get(ix + 1)
            } else {
                ix.checked_sub(1).and_then(|ix| self.rows.get(ix))
            };
            if let Some(next) = next.filter(|next| range.contains(next)) {
                return next;
            }
        }
        row
    }
}

pub(super) struct OverviewSnapshot {
    pub(super) position_track: bool,
    pub(super) range: Range<usize>,
    pub(super) bins: usize,
    pub(super) tracks: Vec<OverviewTrack>,
}

impl OverviewSnapshot {
    pub(super) fn lane_count(&self) -> usize {
        self.tracks.len() + usize::from(self.position_track)
    }

    // Integer boundaries keep the last row and very large source row IDs exact.
    pub(super) fn rows(&self, bins: Range<usize>) -> Range<usize> {
        overview_row_range(&self.range, self.bins, bins)
    }

    pub(super) fn stride(&self, bounds: Bounds<Pixels>, scale: f32) -> usize {
        let height = (f32::from(bounds.size.height) * scale).floor().max(1.) as usize;
        self.bins.div_ceil(height).max(1)
    }
}

pub(super) fn overview_row_range(
    range: &Range<usize>,
    count: usize,
    bins: Range<usize>,
) -> Range<usize> {
    let boundary =
        |ix: usize| range.start + (range.len() as u128 * ix as u128 / count as u128) as usize;
    boundary(bins.start)..boundary(bins.end)
}

impl SidebarState {
    pub(super) fn invalidate_overview(&mut self, panel: SidebarPanelId, cx: &App) {
        let state = self.overview_state_mut(panel);
        state.job.take();
        state.request += 1;
        state.dirty = true;
        state.hover = None;
        state.drag = None;
        tasks::release_in_background(state.snapshot.take(), cx);
    }

    pub(super) fn sync_overview_search(&mut self, rows: &CompressedRows, cx: &App) {
        if self.minimap.search.shares_storage(rows)
            || (self.minimap.search.is_empty() && rows.is_empty())
        {
            return;
        }
        tasks::release_in_background(
            std::mem::replace(&mut self.minimap.search, rows.clone()),
            cx,
        );
        self.invalidate_overview(SidebarPanelId::Minimap, cx);
    }

    pub(super) fn prepare_overview(&mut self, panel: SidebarPanelId, cx: &mut Context<Self>) {
        if !self.is_showing(panel) {
            return;
        }
        let Some((_, document)) = self.document.as_ref() else {
            return;
        };
        if !document.has_complete_line_index() || document.source_line_count() == 0 {
            return;
        }
        let range = self.overview_range(panel);
        let tracks = if panel == SidebarPanelId::Minimap {
            vec![OverviewTrack {
                group: None,
                rows: self.minimap.search.clone(),
                counts: Arc::default(),
            }]
        } else {
            if self.colors.is_empty() {
                return;
            }
            let page = self
                .color_overview
                .color_page
                .min(self.colors.len().saturating_sub(1) / COLOR_TRACKS_PER_PAGE);
            self.color_overview.color_page = page;
            self.colors
                .iter()
                .skip(page * COLOR_TRACKS_PER_PAGE)
                .take(COLOR_TRACKS_PER_PAGE)
                .map(|group| OverviewTrack {
                    group: Some(group.clone()),
                    rows: group.rows.clone(),
                    counts: if range == (0..document.source_line_count()) {
                        group.counts.clone()
                    } else {
                        self.color_overview
                            .snapshot
                            .as_ref()
                            .filter(|old| old.range == range)
                            .and_then(|old| {
                                old.tracks.iter().find(|track| {
                                    track.group.as_ref().is_some_and(|old| old.id == group.id)
                                        && track.rows.shares_storage(&group.rows)
                                })
                            })
                            .map_or_else(Arc::default, |track| track.counts.clone())
                    },
                })
                .collect()
        };
        let mut snapshot = OverviewSnapshot {
            position_track: panel == SidebarPanelId::Minimap,
            bins: range.len().min(OVERVIEW_BINS),
            range,
            tracks,
        };
        let state = self.overview_state_mut(panel);
        state.job.take();
        state.request += 1;
        let request = state.request;
        if panel == SidebarPanelId::Colors
            && snapshot
                .tracks
                .iter()
                .all(|track| track.counts.len() == snapshot.bins)
        {
            self.install_color_overview(snapshot, cx);
            return;
        }
        let generation = self.generation;
        let cancellation = CancellationToken::default();
        let worker = cancellation.clone();
        let task = cx.spawn(async move |this, cx| {
            let snapshot = cx
                .background_spawn(async move {
                    for track in &mut snapshot.tracks {
                        if track.counts.len() == snapshot.bins {
                            continue;
                        }
                        let mut counts = Vec::with_capacity(snapshot.bins);
                        for ix in 0..snapshot.bins {
                            if worker.is_cancelled() {
                                return None;
                            }
                            counts.push(track.rows.count_in_range(overview_row_range(
                                &snapshot.range,
                                snapshot.bins,
                                ix..ix + 1,
                            )));
                        }
                        track.counts = Arc::new(counts);
                    }
                    Some(snapshot)
                })
                .await;
            _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.overview_state(panel).request != request {
                    tasks::release_in_background(snapshot, cx);
                    return;
                }
                this.overview_state_mut(panel).job = None;
                if let Some(snapshot) = snapshot {
                    if this
                        .overview_state(panel)
                        .snapshot
                        .as_ref()
                        .is_some_and(|old| old.range != snapshot.range)
                    {
                        this.overview_state_mut(panel).hover = None;
                    }
                    if panel == SidebarPanelId::Colors {
                        this.install_color_overview(snapshot, cx);
                    } else {
                        tasks::release_in_background(
                            this.overview_state_mut(panel)
                                .snapshot
                                .replace(Arc::new(snapshot)),
                            cx,
                        );
                        this.overview_state_mut(panel).dirty = false;
                    }
                }
                cx.notify();
            });
        });
        self.overview_state_mut(panel).job = Some(SidebarJob {
            cancellation,
            _task: task,
        });
    }
}
