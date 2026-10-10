//! Shared drag visual and return flight for both tab strips.
use super::search_tabs::{SearchTabId, SearchTabOwner};
use super::*;
use gpui_kit::AnyView;
use gpui_kit::base::TestSupportExt as _;

const FRAME: Duration = Duration::from_millis(16);
const FRAMES: u32 = 10;
pub(super) const ANIMATION_DURATION: Duration = FRAME.saturating_mul(FRAMES);

#[derive(Clone, Copy)]
pub(super) enum TabDragKey {
    File(WorkspaceTabId),
    Search(SearchTabOwner, SearchTabId),
}

pub(super) struct TabDragVisual {
    key: TabDragKey,
    view: AnyView,
    cursor_offset: Point<Pixels>,
}

pub(super) struct TabFlight {
    visual: TabDragVisual,
    from: Point<Pixels>,
    to: Point<Pixels>,
    progress: f32,
}

#[derive(Default)]
pub(super) struct TabDragState {
    visual: Option<TabDragVisual>,
    pub(super) flight: Option<TabFlight>,
    flight_task: Option<Task<()>>,
    pub(super) file_hidden: Option<WorkspaceTabId>,
    pub(super) file_bounds: Rc<RefCell<BTreeMap<WorkspaceTabId, Bounds<Pixels>>>>,
    pub(super) file_offsets: BTreeMap<WorkspaceTabId, Pixels>,
    pub(super) file_progress: f32,
    file_task: Option<Task<()>>,
}

impl Workspace {
    pub(super) fn render_tab_drag_observer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let moving = cx.weak_entity();
        let ending = moving.clone();
        canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase.capture() {
                        _ = moving.update(cx, |this, cx| {
                            if cx
                                .global::<WorkspaceWindowRegistry>()
                                .cross_window_tab_drag
                                .is_some()
                            {
                                return;
                            }
                            if cx.has_active_drag() {
                                this.move_search_tab_drag(event.position.x, window, cx);
                                this.move_file_tab_drag(event.position.x, window, cx);
                            } else if this.tab_drag.visual.is_some() {
                                this.cancel_tab_drag(window, cx);
                            }
                        });
                    }
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase.capture() && event.button == MouseButton::Left {
                        _ = ending.update(cx, |this, cx| {
                            if cx
                                .global::<WorkspaceWindowRegistry>()
                                .cross_window_tab_drag
                                .is_none()
                            {
                                this.finish_tab_drag(event.position, window, cx);
                            }
                        });
                    }
                });
            },
        )
        .absolute()
        .size_0()
    }

    pub(super) fn begin_tab_drag(
        &mut self,
        key: TabDragKey,
        view: AnyView,
        cursor_offset: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.clear_tab_drag(cx);
        match key {
            TabDragKey::File(id) => self.tab_drag.file_hidden = Some(id),
            TabDragKey::Search(owner, id) => {
                self.search_tabs.hidden_drag = Some((owner, id));
                self.search_tabs.dragging = true;
            }
        }
        self.tab_drag.visual = Some(TabDragVisual {
            key,
            view,
            cursor_offset,
        });
        cx.notify();
    }

    pub(super) fn clear_tab_drag(&mut self, cx: &mut Context<Self>) {
        self.tab_drag.visual = None;
        self.tab_drag.flight = None;
        self.tab_drag.flight_task = None;
        self.tab_drag.file_hidden = None;
        self.search_tabs.hidden_drag = None;
        self.search_tabs.dragging = false;
        cx.notify();
    }

    pub(super) fn cancel_tab_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if cx
            .global::<WorkspaceWindowRegistry>()
            .cross_window_tab_drag
            .is_some()
        {
            cx.stop_active_drag(window);
            Self::cancel_cross_window_tab_drag(cx.entity().entity_id(), cx);
        }
        if self.tab_drag.visual.is_some() || self.tab_drag.flight.is_some() {
            cx.stop_active_drag(window);
            Self::cancel_cross_window_tab_drag(cx.entity().entity_id(), cx);
            self.clear_tab_drag(cx);
        }
    }

    pub(super) fn finish_tab_drag(
        &mut self,
        position: Point<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(visual) = self.tab_drag.visual.take() else {
            return;
        };
        self.search_tabs.dragging = false;
        let bounds = match visual.key {
            TabDragKey::Search(owner, id) => self
                .search_tabs
                .layout
                .borrow()
                .slots
                .get(&(owner, id))
                .copied(),
            TabDragKey::File(id) => self
                .tabs
                .iter()
                .position(|candidate| *candidate == id)
                .and_then(|ix| self.tab_drop_layout.borrow().tabs.get(ix).copied()),
        };
        if cx.reduce_motion() || bounds.is_none() {
            self.clear_tab_drag(cx);
            return;
        }
        self.tab_drag.flight = Some(TabFlight {
            from: position - visual.cursor_offset,
            to: bounds.unwrap().origin,
            visual,
            progress: 0.,
        });
        self.tab_drag.flight_task = Some(cx.spawn(async move |this, cx| {
            for frame in 1..=FRAMES {
                cx.background_executor().timer(FRAME).await;
                if this
                    .update(cx, |this, cx| {
                        if frame == FRAMES {
                            this.tab_drag.flight = None;
                            this.tab_drag.file_hidden = None;
                            this.search_tabs.hidden_drag = None;
                        } else if let Some(flight) = &mut this.tab_drag.flight {
                            flight.progress = ease_out_cubic(frame as f32 / FRAMES as f32);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
        cx.notify();
    }

    pub(super) fn render_tab_flight(&self) -> AnyElement {
        let Some(flight) = &self.tab_drag.flight else {
            return div().into_any_element();
        };
        let position = flight.from + (flight.to - flight.from) * flight.progress;
        super::render_shell::deferred_workspace_overlay(
            div()
                .id("tab-return-flight")
                .test_support()
                .absolute()
                .left(position.x)
                .top(position.y)
                .child(flight.visual.view.clone()),
        )
        .into_any_element()
    }

    pub(super) fn move_file_tab_drag(
        &mut self,
        x: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(TabDragVisual {
            key: TabDragKey::File(id),
            ..
        }) = &self.tab_drag.visual
        else {
            return;
        };
        let id = *id;
        let Some(from) = self.tabs.iter().position(|key| *key == id) else {
            return;
        };
        let layout = self.tab_drop_layout.borrow();
        let mut target = from;
        for (ix, bounds) in layout.tabs.iter().enumerate() {
            if ix < from && x <= bounds.center().x {
                target = ix;
                break;
            }
            if ix > from && x >= bounds.center().x {
                target = ix;
            }
        }
        drop(layout);
        if target == from {
            return;
        }
        let before = self.tab_drag.file_bounds.borrow().clone();
        self.reorder_tab(
            id,
            if target > from { target + 1 } else { target },
            window,
            cx,
        );
        self.tab_drag.file_offsets.clear();
        if !cx.reduce_motion() {
            let layout = self.tab_drop_layout.borrow();
            let gap = layout
                .tabs
                .windows(2)
                .next()
                .map(|pair| pair[1].left() - pair[0].right())
                .unwrap_or_default();
            if let Some(mut x) = layout.tabs.first().map(|bounds| bounds.left()) {
                for key in &self.tabs {
                    if let Some(bounds) = before.get(key) {
                        self.tab_drag.file_offsets.insert(*key, bounds.left() - x);
                        x += bounds.size.width + gap;
                    }
                }
            }
        }
        self.tab_drag.file_progress = 0.;
        self.tab_drag.file_task = Some(cx.spawn(async move |this, cx| {
            for frame in 1..=FRAMES {
                cx.background_executor().timer(FRAME).await;
                if this
                    .update(cx, |this, cx| {
                        this.tab_drag.file_progress = ease_out_cubic(frame as f32 / FRAMES as f32);
                        if frame == FRAMES {
                            this.tab_drag.file_offsets.clear();
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
        cx.notify();
    }
}
