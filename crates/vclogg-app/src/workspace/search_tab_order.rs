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
