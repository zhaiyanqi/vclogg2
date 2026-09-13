use super::*;

impl SidebarState {
    pub(super) fn prepare_overview_marks(&mut self, cx: &mut Context<Self>) {
        if !self.is_showing(SidebarPanelId::Minimap) {
            return;
        }
        let Some(summary) = self.summary.clone() else {
            return;
        };
        self.overview_job.take();
        let colors = self.colors.clone();
        let generation = self.generation;
        self.overview_request += 1;
        let request = self.overview_request;
        let cancellation = CancellationToken::default();
        let worker = cancellation.clone();
        let task = cx.spawn(async move |this, cx| {
            let marks = cx
                .background_spawn(async move {
                    let mut marks = vec![None; summary.buckets().len()];
                    for (group_ix, group) in colors.iter().enumerate() {
                        for (ix, mark) in marks.iter_mut().enumerate() {
                            if worker.is_cancelled() {
                                return None;
                            }
                            let start = ix * summary.bucket_rows();
                            if let Some(position) = group.rows.nearest_position(start) {
                                let row = group.rows.get(position).unwrap_or(usize::MAX);
                                let row = if row < start {
                                    group.rows.get(position + 1).unwrap_or(usize::MAX)
                                } else {
                                    row
                                };
                                if row >= start && row < start.saturating_add(summary.bucket_rows())
                                {
                                    *mark = Some(group_ix);
                                }
                            }
                        }
                    }
                    Some(marks)
                })
                .await;
            _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.overview_request != request {
                    return;
                }
                this.overview_job = None;
                if let Some(marks) = marks {
                    this.overview_marks = marks;
                    this.overview_dirty = false;
                }
                cx.notify();
            });
        });
        self.overview_job = Some(SidebarJob {
            cancellation,
            _task: task,
        });
    }
}
