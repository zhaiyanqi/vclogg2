//! Scroll state owned by the AI transcript. Message content stays in `AiPanel`.
use std::ops::Range;

#[cfg(test)]
use gpui_kit::ListOffset;
use gpui_kit::{Context, FollowMode, ListAlignment, ListState, Pixels, px};

pub(super) struct TranscriptScroll {
    pub(super) list: ListState,
}

impl TranscriptScroll {
    pub(super) fn new(count: usize, cx: &mut Context<Self>) -> Self {
        let list = ListState::new(count, ListAlignment::Top, px(400.));
        list.set_follow_mode(FollowMode::Tail);
        let owner = cx.weak_entity();
        list.set_scroll_handler(move |_, _, cx| {
            let owner = owner.clone();
            cx.defer(move |cx| {
                _ = owner.update(cx, |_, cx| cx.notify());
            });
        });
        Self { list }
    }

    #[cfg(test)]
    pub(super) fn item_count(&self) -> usize {
        self.list.item_count()
    }

    #[cfg(test)]
    pub(super) fn is_following_tail(&self) -> bool {
        self.list.is_following_tail()
    }

    pub(super) fn is_scrolled_up(&self) -> bool {
        self.list.max_offset_for_scrollbar().y > Pixels::ZERO
            && !self.list.is_following_tail()
            && !self.list.is_scrolled_to_end().unwrap_or(false)
    }

    pub(super) fn reset(&mut self, count: usize, cx: &mut Context<Self>) {
        self.list.reset(count);
        self.list.set_follow_mode(FollowMode::Tail);
        cx.notify();
    }

    pub(super) fn append(&mut self, count: usize, cx: &mut Context<Self>) -> bool {
        let old = self.list.item_count();
        self.list.splice(old..old, count);
        if old > 0 {
            self.list.remeasure_items(old - 1..old);
        }
        cx.notify();
        true
    }

    pub(super) fn remeasure(&mut self, cx: &mut Context<Self>) {
        self.list.remeasure();
        cx.notify();
    }

    pub(super) fn remeasure_items(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> bool {
        if range.start > range.end || range.end > self.list.item_count() {
            return false;
        }
        self.list.remeasure_items(range);
        cx.notify();
        true
    }

    #[cfg(test)]
    pub(super) fn scroll_to_item(&mut self, index: usize, cx: &mut Context<Self>) -> bool {
        if index >= self.list.item_count() {
            return false;
        }
        self.list.scroll_to(ListOffset {
            item_ix: index,
            offset_in_item: Pixels::ZERO,
        });
        cx.notify();
        true
    }

    pub(super) fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        self.list.set_follow_mode(FollowMode::Tail);
        self.list.scroll_to_end();
        cx.notify();
    }
}
