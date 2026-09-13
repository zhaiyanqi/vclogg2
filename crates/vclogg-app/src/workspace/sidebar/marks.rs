//! Current-file marks and bounded, asynchronously decoded list previews.
use super::*;
use crate::log_tags::{RowTags, source_digest};
use crate::virtual_log_lines::DEFAULT_MAX_LINE_SOURCE_BYTES;

mod navigation;
pub(in super::super) use navigation::PendingMarkResultJump;

const PREVIEW_NEIGHBORS: usize = 16;
const MAX_PREVIEW_ITEMS: usize = 512;
const SUMMARY_CHARS: usize = 256;

#[derive(Default)]
pub(super) struct MarksState {
    pub(super) rows: CompressedRows,
    marked: CompressedRows,
    tags: RowTags,
    previews: BTreeMap<usize, MarkPreview>,
    range: Range<usize>,
    revision: u64,
    request: u64,
    visible_request: u64,
    job: Option<SidebarJob>,
}

struct MarkPreview {
    summary: String,
    labels: String,
    stale_tags: bool,
    available: bool,
}

impl MarksState {
    pub(super) fn release_previews(&mut self) {
        if self.job.is_some() || !self.range.is_empty() || !self.previews.is_empty() {
            self.revision = self.revision.wrapping_add(1);
            self.job.take();
            self.previews.clear();
            self.range = 0..0;
        }
    }
}

impl SidebarState {
    pub(super) fn sync_marks(
        &mut self,
        source: Option<(u64, CompressedRows, RowTags)>,
        source_changed: bool,
        cx: &mut Context<Self>,
    ) {
        let Some((id, marked, tags)) = source else {
            self.marks = MarksState::default();
            self.selected.remove(&SidebarPanelId::Marks);
            return;
        };
        let marks_changed = !self.marks.marked.shares_storage(&marked);
        let tags_changed = !self.marks.tags.shares_storage(&tags);
        if !source_changed && !marks_changed && !tags_changed {
            return;
        }
        self.marks.release_previews();
        self.marks.revision = self.marks.revision.wrapping_add(1);
        self.marks.marked = marked;
        self.marks.tags = tags;
        self.marks.rows = self.marks.marked.union(self.marks.tags.source_rows());
        if self.document.as_ref().map(|(id, _)| *id) != Some(id) {
            self.selected.remove(&SidebarPanelId::Marks);
            self.scrolls
                .insert(SidebarPanelId::Marks, UniformListScrollHandle::new());
        } else if self
            .selected
            .get(&SidebarPanelId::Marks)
            .is_some_and(|key| self.mark_index(key).is_none())
        {
            self.selected.remove(&SidebarPanelId::Marks);
        }
        cx.notify();
    }

    pub(super) fn mark_key(&self, ix: usize) -> Option<String> {
        Some(format!(
            "mark-{}-{}",
            self.document.as_ref()?.0,
            self.marks.rows.get(ix)?
        ))
    }

    pub(super) fn mark_index(&self, key: &str) -> Option<usize> {
        let (id, row) = key.strip_prefix("mark-")?.split_once('-')?;
        (id.parse::<u64>().ok()? == self.document.as_ref()?.0)
            .then(|| self.marks.rows.position(row.parse().ok()?))?
    }

    /// The list reports visibility during layout; start I/O only after that frame's
    /// entity updates have unwound. Generation checks also reject measurement callbacks
    /// belonging to a file or marks snapshot that has since been replaced.
    pub(super) fn defer_mark_previews(
        &mut self,
        range: Range<usize>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.marks.visible_request = self.marks.visible_request.wrapping_add(1);
        let visible_request = self.marks.visible_request;
        let generation = self.generation;
        let revision = self.marks.revision;
        cx.defer_in(window, move |this, _, cx| {
            if this.generation == generation
                && this.marks.revision == revision
                && this.marks.visible_request == visible_request
            {
                this.load_mark_previews(range, cx);
            }
        });
    }

    fn load_mark_previews(&mut self, visible: Range<usize>, cx: &mut Context<Self>) {
        if !self.is_showing(SidebarPanelId::Marks) || visible.is_empty() {
            return;
        }
        let Some((_, source)) = &self.document else {
            return;
        };
        if !source.has_complete_line_index() {
            return;
        }
        let start = visible.start.saturating_sub(PREVIEW_NEIGHBORS);
        let range = start
            ..visible
                .end
                .saturating_add(PREVIEW_NEIGHBORS)
                .min(self.marks.rows.len())
                .min(start.saturating_add(MAX_PREVIEW_ITEMS));
        if range == self.marks.range {
            return;
        }
        self.marks.job.take();
        self.marks.request = self.marks.request.wrapping_add(1);
        self.marks.range = range.clone();
        let rows = &self.marks.rows;
        self.marks
            .previews
            .retain(|row, _| rows.position(*row).is_some_and(|ix| range.contains(&ix)));
        let missing = range
            .filter_map(|ix| self.marks.rows.get(ix))
            .filter(|row| !self.marks.previews.contains_key(row))
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return;
        }
        let source = source.clone();
        let tags = self.marks.tags.clone();
        let generation = self.generation;
        let revision = self.marks.revision;
        let request = self.marks.request;
        let cancellation = CancellationToken::default();
        let cancelled = cancellation.clone();
        let task = cx.spawn(async move |this, cx| {
            let previews = cx
                .background_spawn(async move {
                    let mut reader = LinePreviewReader::default();
                    let mut previews = BTreeMap::new();
                    for row in missing {
                        if cancelled.is_cancelled() {
                            break;
                        }
                        let Some(preview) =
                            reader.line_preview(&source, row, DEFAULT_MAX_LINE_SOURCE_BYTES)
                        else {
                            previews.insert(
                                row,
                                MarkPreview {
                                    summary: String::new(),
                                    labels: String::new(),
                                    stale_tags: false,
                                    available: false,
                                },
                            );
                            continue;
                        };
                        let (mut text, truncated) = preview.into_parts();
                        if truncated {
                            text.push('…');
                        }
                        let digest = source_digest(&text);
                        let labels = tags
                            .row(row)
                            .filter(|(_, tag)| tag.source_digest == digest)
                            .map(|(_, tag)| tag.label.as_str())
                            .collect::<Vec<_>>()
                            .join(" · ");
                        let stale_tags = tags.row(row).any(|(_, tag)| tag.source_digest != digest);
                        let mut summary = text
                            .chars()
                            .take(SUMMARY_CHARS)
                            .map(|ch| if ch.is_control() { ' ' } else { ch })
                            .collect::<String>();
                        if text.chars().nth(SUMMARY_CHARS).is_some() {
                            summary.push('…');
                        }
                        previews.insert(
                            row,
                            MarkPreview {
                                summary,
                                labels,
                                stale_tags,
                                available: true,
                            },
                        );
                    }
                    previews
                })
                .await;
            _ = this.update(cx, |this, cx| {
                if this.generation != generation
                    || this.marks.revision != revision
                    || this.marks.request != request
                {
                    return;
                }
                this.marks.job = None;
                this.marks.previews.extend(previews);
                cx.notify();
            });
        });
        self.marks.job = Some(SidebarJob {
            cancellation,
            _task: task,
        });
    }

    pub(super) fn activate_mark(&self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.marks.rows.get(ix) else {
            return;
        };
        let Some(preview) = self.marks.previews.get(&row) else {
            return;
        };
        if !preview.available || (!self.marks.marked.contains(row) && preview.labels.is_empty()) {
            return;
        }
        let Some((id, source)) = &self.document else {
            return;
        };
        let id = *id;
        let source = source.clone();
        _ = self.workspace.update(cx, |workspace, cx| {
            workspace.navigate_to_mark(id, source, row, window, cx);
        });
    }

    pub(super) fn render_mark(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.marks.rows.get(ix) else {
            return div().into_any_element();
        };
        let Some(key) = self.mark_key(ix) else {
            return div().into_any_element();
        };
        let preview = self.marks.previews.get(&row);
        let marked = self.marks.marked.contains(row);
        let enabled = preview
            .is_some_and(|preview| preview.available && (marked || !preview.labels.is_empty()));
        let summary = match preview {
            Some(preview) if preview.available => preview.summary.clone(),
            Some(_) => {
                crate::tr!("该行已不存在或无法读取", "Line missing or unavailable").to_owned()
            }
            None => crate::tr!("加载中…", "Loading…").to_owned(),
        };
        let detail = match preview {
            Some(preview) if preview.stale_tags && preview.labels.is_empty() => crate::tr!(
                "文字标记已失效：日志内容已变化",
                "Text marks are stale: log content changed"
            )
            .to_owned(),
            Some(preview) if preview.stale_tags => format!(
                "{} · {}",
                preview.labels,
                crate::tr!("部分文字标记已失效", "Some text marks are stale")
            ),
            Some(preview) if !preview.labels.is_empty() => preview.labels.clone(),
            _ if marked => crate::tr!("行标记", "Row mark").to_owned(),
            _ => crate::tr!("文字标记", "Text mark").to_owned(),
        };
        let generation = self.generation;
        let revision = self.marks.revision;
        let item_key = key.clone();
        Button::new(SharedString::from(format!("sidebar-{key}")))
            .ghost().w_full().h(rems(3.25)).py_1().justify_start()
            .selected(self.selected.get(&SidebarPanelId::Marks) == Some(&key))
            .disabled(!enabled)
            .tooltip(detail.clone())
            .child(v_flex().min_w_0().flex_1().items_start().gap_0p5()
                .child(h_flex().w_full().min_w_0().h(rems(1.25)).flex_shrink_0().gap_2()
                    .text_sm().line_height(rems(1.25))
                    .child(div().flex_shrink_0().text_color(cx.theme().muted_foreground).child((row + 1).to_string()))
                    .child(div().flex_1().min_w_0().truncate().child(summary)))
                .child(div().w_full().h(rems(1.125)).flex_shrink_0().text_xs()
                    .line_height(rems(1.125)).truncate().text_color(cx.theme().muted_foreground).child(detail)))
            .on_click(cx.listener(move |this, event, window, cx| {
                if matches!(event, ClickEvent::Mouse(event) if event.down.button != MouseButton::Left || event.up.button != MouseButton::Left) {
                    return;
                }
                if this.generation != generation || this.marks.revision != revision { return; }
                if let Some(ix) = this.mark_index(&item_key) {
                    this.activate_item(SidebarPanelId::Marks, ix, window, cx);
                }
            })).into_any_element()
    }
}
