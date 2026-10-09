//! Presentation-only enter/exit motion; closed documents are released immediately.
use super::*;

const FRAME: Duration = Duration::from_millis(16);

#[derive(Default)]
pub(super) struct TabMotionState {
    opening: BTreeMap<WorkspaceTabId, Duration>,
    pub(super) closing: Vec<ClosingWorkspaceTab>,
    task: Option<Task<()>>,
}

pub(super) struct ClosingWorkspaceTab {
    pub(super) id: WorkspaceTabId,
    pub(super) index: usize,
    pub(super) width: Pixels,
    pub(super) presentation: super::tab_view::WorkspaceTabPresentation,
    elapsed: Duration,
    opacity: f32,
}

fn progress(elapsed: Duration) -> f32 {
    ease_out_cubic(
        (elapsed.as_secs_f32() / super::tab_drag::ANIMATION_DURATION.as_secs_f32()).min(1.),
    )
}

impl ClosingWorkspaceTab {
    pub(super) fn remaining(&self) -> f32 {
        1. - progress(self.elapsed)
    }

    pub(super) fn opacity(&self) -> f32 {
        self.opacity * self.remaining()
    }
}

impl Workspace {
    pub(super) fn workspace_tab_visual_index(&self, index: usize) -> usize {
        self.tab_motion
            .closing
            .iter()
            .fold(index, |index, closing| {
                index + usize::from(closing.index <= index)
            })
    }

    pub(super) fn workspace_tab_opacity(&self, id: WorkspaceTabId) -> f32 {
        self.tab_motion
            .opening
            .get(&id)
            .copied()
            .map(progress)
            .unwrap_or(1.)
    }

    pub(super) fn animate_workspace_tab_open(
        &mut self,
        id: WorkspaceTabId,
        cx: &mut Context<Self>,
    ) {
        if cx.reduce_motion() {
            return;
        }
        self.tab_motion.opening.insert(id, Duration::ZERO);
        self.start_workspace_tab_motion(cx);
    }

    pub(super) fn animate_workspace_tabs_close(
        &mut self,
        ids: &BTreeSet<WorkspaceTabId>,
        cx: &mut Context<Self>,
    ) {
        if cx.reduce_motion() {
            return;
        }
        let closing = self
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(index, id)| {
                if !ids.contains(id) {
                    return None;
                }
                let width = self.tab_drag.file_bounds.borrow().get(id)?.size.width;
                Some(ClosingWorkspaceTab {
                    id: *id,
                    index: self.workspace_tab_visual_index(index),
                    width,
                    presentation: self.workspace_tab_presentation(*id),
                    elapsed: Duration::ZERO,
                    opacity: self.workspace_tab_opacity(*id),
                })
            })
            .collect::<Vec<_>>();
        self.tab_motion.closing.extend(closing);
        self.tab_motion.closing.sort_by_key(|tab| tab.index);
        self.tab_motion.opening.retain(|id, _| !ids.contains(id));
        self.start_workspace_tab_motion(cx);
    }

    fn start_workspace_tab_motion(&mut self, cx: &mut Context<Self>) {
        if self.tab_motion.task.is_some() {
            return;
        }
        self.tab_motion.task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(FRAME).await;
                let keep_running = this.update(cx, |this, cx| {
                    let motion = &mut this.tab_motion;
                    if cx.reduce_motion() {
                        motion.opening.clear();
                        motion.closing.clear();
                    } else {
                        motion.opening.retain(|id, elapsed| {
                            *elapsed += FRAME;
                            this.tabs.contains(id) && *elapsed < super::tab_drag::ANIMATION_DURATION
                        });
                        let mut removed = 0;
                        motion.closing.retain_mut(|tab| {
                            tab.elapsed += FRAME;
                            if tab.elapsed >= super::tab_drag::ANIMATION_DURATION {
                                removed += 1;
                                false
                            } else {
                                tab.index -= removed;
                                true
                            }
                        });
                    }
                    let running = !motion.opening.is_empty() || !motion.closing.is_empty();
                    if !running {
                        motion.task = None;
                    }
                    cx.notify();
                    running
                });
                if !matches!(keep_running, Ok(true)) {
                    break;
                }
            }
        }));
    }
}
