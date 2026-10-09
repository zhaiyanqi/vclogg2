//! Shared ordering and measured geometry for the search strip.
use super::search_tabs::{SearchTabId, SearchTabOwner};
use super::*;

type Key = (SearchTabOwner, SearchTabId);

#[derive(Default)]
pub(super) struct SearchTabLayout {
    pub(super) track: Option<Bounds<Pixels>>,
    pub(super) slots: BTreeMap<Key, Bounds<Pixels>>,
    pub(super) painted: BTreeMap<Key, Bounds<Pixels>>,
}

impl SearchTabLayout {
    pub(super) fn target(&self, keys: &[Key], dragged: Key, x: Pixels) -> Option<usize> {
        let from = keys.iter().position(|key| *key == dragged)?;
        let mut target = from;
        for (ix, key) in keys.iter().enumerate() {
            let Some(bounds) = self.slots.get(key) else {
                continue;
            };
            let center = bounds.center().x;
            if ix < from && x <= center {
                return Some(ix);
            }
            if ix > from && x >= center {
                target = ix;
            }
        }
        (target != from).then_some(target)
    }
}

impl Workspace {
    pub(super) fn animate_search_tab_close(
        &mut self,
        key: Key,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if cx.reduce_motion() || self.search_tabs.strip_collapsed {
            return;
        }
        let Some(width) = self
            .search_tabs
            .layout
            .borrow()
            .slots
            .get(&key)
            .map(|bounds| bounds.size.width)
        else {
            return;
        };
        let Some(state) = self.search_tabs.state(key.0, key.1) else {
            return;
        };
        let title = self.search_tab_title(key.0, state);
        let mut index = index;
        for closing in &self.search_tabs.closing {
            if closing.index <= index {
                index += 1;
            }
        }
        self.search_tabs
            .closing
            .push(super::search_tabs::ClosingSearchTab {
                key,
                index,
                title,
                selected: self.active_search_tab_key() == Some(key),
                width,
            });
        self.search_tabs.closing.sort_by_key(|tab| tab.index);
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(super::tab_drag::ANIMATION_DURATION)
                .await;
            _ = this.update(cx, |this, cx| {
                if let Some(ix) = this
                    .search_tabs
                    .closing
                    .iter()
                    .position(|tab| tab.key == key)
                {
                    let removed = this.search_tabs.closing.remove(ix);
                    for tab in &mut this.search_tabs.closing {
                        if tab.index > removed.index {
                            tab.index -= 1;
                        }
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn move_search_tab_drag(
        &mut self,
        x: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.search_tabs.dragging {
            return;
        }
        let Some(key) = self.search_tabs.hidden_drag else {
            return;
        };
        if let Some(bounds) = self.search_tabs.layout.borrow().track {
            let edge = window.rem_size() * 1.5;
            let step = if x < bounds.left() + edge {
                window.rem_size()
            } else if x > bounds.right() - edge {
                -window.rem_size()
            } else {
                px(0.)
            };
            let scroll = &self.search_tabs.scroll;
            let offset = scroll.offset();
            let next_x = (offset.x + step).clamp(-scroll.max_offset().x.max(px(0.)), px(0.));
            if next_x != offset.x {
                scroll.set_offset(point(next_x, offset.y));
                cx.notify();
            }
        }
        let keys = self.visible_search_tab_keys();
        let target = self.search_tabs.layout.borrow().target(&keys, key, x);
        if let Some(target) = target {
            self.reorder_search_tab(key, target, window, cx);
        }
    }

    pub(super) fn reorder_search_tab(
        &mut self,
        key: Key,
        target: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.visible_search_tab_keys();
        if !self.search_tabs.reorder(key, target) {
            return;
        }
        let after = self.visible_search_tab_keys();
        let mut layout = self.search_tabs.layout.borrow_mut();
        self.search_tabs.motion_offsets.clear();
        if let Some(mut x) = before
            .first()
            .and_then(|key| layout.slots.get(key))
            .map(|bounds| bounds.left())
        {
            for key in &after {
                if let Some(slot) = layout.slots.get(key) {
                    let painted = layout.painted.get(key).unwrap_or(slot);
                    self.search_tabs
                        .motion_offsets
                        .insert(*key, painted.left() - x);
                    x += slot.size.width;
                }
            }
        }
        // Closed files and tabs must not retain measurement snapshots indefinitely.
        layout.slots.retain(|key, _| after.contains(key));
        layout.painted.retain(|key, _| after.contains(key));
        drop(layout);
        self.search_tabs.order_revision = self.search_tabs.order_revision.wrapping_add(1);
        let documents = self
            .search_tabs
            .groups
            .keys()
            .filter_map(|owner| match owner {
                SearchTabOwner::File(id) => Some(*id),
                _ => None,
            })
            .collect::<Vec<_>>();
        for id in documents {
            self.schedule_checkpoint(id, window, cx);
        }
        self.schedule_workspace_search_state_save(window, cx);
        cx.notify();
    }
}
