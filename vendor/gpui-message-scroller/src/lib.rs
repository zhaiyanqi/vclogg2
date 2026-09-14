// Copyright GPUI Kit contributors. SPDX-License-Identifier: Apache-2.0
// Adapted from gpui-kit MessageScroller; see ../README.md for provenance and changes.

use std::ops::Range;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, FollowMode, InteractiveElement as _, IntoElement,
    ListAlignment, ListOffset, ListState, ParentElement as _, RenderOnce, Role, SharedString,
    StatefulInteractiveElement as _, StyleRefinement, Styled, Window, div, list,
    prelude::FluentBuilder as _, px, rems,
};

use gpui_component::{ActiveTheme as _, IconName, StyledExt as _, button::Button};
use gpui_component::{button::ButtonVariants as _, scroll::ScrollableElement as _};

const LIST_OVERDRAW: gpui::Pixels = px(400.);

/// The entity-owned scrolling state for a [`MessageScroller`].
///
/// The state owns only GPUI's virtual-list bookkeeping. Message data remains
/// with the caller and is read by the row renderer passed to
/// [`MessageScroller::new`].
pub struct MessageScrollerState {
    list_state: ListState,
}

impl MessageScrollerState {
    /// Create a state for `item_count` rows and enable tail following.
    ///
    /// The constructor receives the entity context so the list's scroll
    /// handler can safely defer its entity update until GPUI has released the
    /// list's internal borrow.
    pub fn new(item_count: usize, cx: &mut Context<Self>) -> Self {
        let list_state = ListState::new(item_count, ListAlignment::Top, LIST_OVERDRAW);
        list_state.set_follow_mode(FollowMode::Tail);

        let weak_state = cx.weak_entity();
        list_state.set_scroll_handler(move |_, _, cx| {
            let weak_state = weak_state.clone();

            cx.defer(move |cx| {
                let _ = weak_state.update(cx, |_, cx| cx.notify());
            });
        });

        Self { list_state }
    }

    /// Return the current number of rows known by the virtual list.
    pub fn item_count(&self) -> usize {
        self.list_state.item_count()
    }

    /// Return whether the user has scrolled away from the latest content.
    pub fn is_scrolled_up(&self) -> bool {
        self.list_state.max_offset_for_scrollbar().y > px(0.)
            && !self.list_state.is_following_tail()
            && !self.list_state.is_scrolled_to_end().unwrap_or(false)
    }

    /// Return whether the list is actively following its tail.
    pub fn is_following_tail(&self) -> bool {
        self.list_state.is_following_tail()
    }

    /// Reset the list to `item_count` rows.
    pub fn reset(&mut self, item_count: usize, cx: &mut Context<Self>) {
        self.list_state.reset(item_count);
        self.list_state.set_follow_mode(FollowMode::Tail);
        cx.notify();
    }

    /// Replace `old_range` with `count` new rows.
    ///
    /// Returns `false` when the range is outside the current list and leaves
    /// the state unchanged.
    pub fn splice(
        &mut self,
        old_range: Range<usize>,
        count: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.valid_range(&old_range) {
            return false;
        }

        let neighbor = old_range.start.checked_sub(1);
        self.list_state.splice(old_range, count);

        // The default row wrapper pads every row except the last, so a row
        // whose "last" status may have flipped carries a stale measured
        // height. Remeasure the new last row and the survivor next to the
        // splice.
        if let Some(last) = self.list_state.item_count().checked_sub(1) {
            self.list_state.remeasure_items(last..last + 1);
            if let Some(neighbor) = neighbor.filter(|neighbor| *neighbor != last) {
                self.list_state.remeasure_items(neighbor..neighbor + 1);
            }
        }

        cx.notify();
        true
    }

    /// Append `count` rows to the end of the list.
    pub fn append(&mut self, count: usize, cx: &mut Context<Self>) -> bool {
        let item_count = self.list_state.item_count();
        self.splice(item_count..item_count, count, cx)
    }

    /// Prepend `count` rows while preserving the current scroll anchor.
    pub fn prepend(&mut self, count: usize, cx: &mut Context<Self>) -> bool {
        self.splice(0..0, count, cx)
    }

    /// Mark all rows for remeasurement while preserving a proportional anchor.
    pub fn remeasure(&mut self, cx: &mut Context<Self>) {
        self.list_state.remeasure();
        cx.notify();
    }

    /// Mark rows in `range` for remeasurement while preserving an item anchor.
    ///
    /// Returns `false` when the range is outside the current list.
    pub fn remeasure_items(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> bool {
        if !self.valid_range(&range) {
            return false;
        }

        self.list_state.remeasure_items(range);
        cx.notify();
        true
    }

    /// Scroll to the row at `index`, if it exists.
    pub fn scroll_to_item(&mut self, index: usize, cx: &mut Context<Self>) -> bool {
        if index >= self.list_state.item_count() {
            return false;
        }

        self.list_state.scroll_to(ListOffset {
            item_ix: index,
            offset_in_item: px(0.),
        });
        cx.notify();
        true
    }

    /// Resume tail following and scroll to the latest row.
    pub fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        self.list_state.set_follow_mode(FollowMode::Tail);
        self.list_state.scroll_to_end();
        cx.notify();
    }

    fn valid_range(&self, range: &Range<usize>) -> bool {
        range.start <= range.end && range.end <= self.list_state.item_count()
    }
}

type MessageRenderer = dyn FnMut(usize, &mut Window, &mut App) -> AnyElement;

/// A virtualized message list with optional scrollbar and jump-to-latest UI.
#[derive(IntoElement)]
pub struct MessageScroller {
    id: ElementId,
    state: Entity<MessageScrollerState>,
    renderer: Box<MessageRenderer>,
    style: StyleRefinement,
    content_style: StyleRefinement,
    list_style: StyleRefinement,
    row_style: StyleRefinement,
    jump_button_style: StyleRefinement,
    jump_button_renderer: Option<Box<dyn FnOnce(Button) -> Button>>,
    scrollbar: bool,
    jump_button: bool,
    jump_button_label: SharedString,
}

impl MessageScroller {
    /// Create a message scroller with a renderer for each row.
    pub fn new<E>(
        id: impl Into<ElementId>,
        state: Entity<MessageScrollerState>,
        renderer: impl FnMut(usize, &mut Window, &mut App) -> E + 'static,
    ) -> Self
    where
        E: IntoElement,
    {
        let mut renderer = renderer;
        Self {
            id: id.into(),
            state,
            renderer: Box::new(move |index, window, cx| {
                renderer(index, window, cx).into_any_element()
            }),
            style: StyleRefinement::default(),
            content_style: StyleRefinement::default(),
            list_style: StyleRefinement::default(),
            row_style: StyleRefinement::default(),
            jump_button_style: StyleRefinement::default(),
            jump_button_renderer: None,
            scrollbar: true,
            jump_button: true,
            jump_button_label: "Jump to latest".into(),
        }
    }

    /// Enable or disable the virtual-list scrollbar.
    pub fn scrollbar(mut self, scrollbar: bool) -> Self {
        self.scrollbar = scrollbar;
        self
    }

    /// Enable or disable the built-in jump-to-latest button.
    pub fn jump_button(mut self, jump_button: bool) -> Self {
        self.jump_button = jump_button;
        self
    }

    /// Set the label used by the built-in jump-to-latest button.
    pub fn with_jump_button_label(mut self, label: impl Into<SharedString>) -> Self {
        self.jump_button_label = label.into();
        self
    }

    /// Refine the viewport that contains the list and scrollbar.
    pub fn with_content_style(mut self, style: StyleRefinement) -> Self {
        self.content_style = style;
        self
    }

    /// Refine the GPUI list element used to render rows.
    pub fn with_list_style(mut self, style: StyleRefinement) -> Self {
        self.list_style = style;
        self
    }

    /// Refine the full-width wrapper around every rendered row.
    pub fn with_row_style(mut self, style: StyleRefinement) -> Self {
        self.row_style = style;
        self
    }

    /// Refine the built-in jump-to-latest button after its defaults.
    pub fn with_jump_button_style(mut self, style: StyleRefinement) -> Self {
        self.jump_button_style = style;
        self
    }

    /// Customize the built-in jump button without replacing its scroll action.
    ///
    /// The callback receives the fully configured Button, so its variant,
    /// semantic size, icon, tooltip, or instance styling may be adjusted.
    pub fn with_jump_button_renderer(
        mut self,
        renderer: impl FnOnce(Button) -> Button + 'static,
    ) -> Self {
        self.jump_button_renderer = Some(Box::new(renderer));
        self
    }
}

impl Styled for MessageScroller {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for MessageScroller {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let root_id = self.id.clone();
        let (list_state, scrolled_up) = {
            let state = self.state.read(cx);
            (state.list_state.clone(), state.is_scrolled_up())
        };
        let show_jump_button = self.jump_button && scrolled_up;
        let tokens = cx.theme().semantic_tokens();
        let row_style = self.row_style;
        let jump_button_style = self.jump_button_style;
        let jump_button_renderer = self.jump_button_renderer;
        let mut renderer = self.renderer;

        // GPUI's `list` lays rows out at the full list width and offsets them
        // only by vertical padding, so the horizontal component of the list
        // style must be carried by every row wrapper instead.
        let mut list_style = self.list_style;
        let row_inset_left = list_style.padding.left.take();
        let row_inset_right = list_style.padding.right.take();

        // Read the count outside the row closure: the list holds a mutable
        // borrow of its state while rendering rows, so the closure must not
        // borrow it again. The count is stable within one render pass.
        let item_count = list_state.item_count();
        let list = list(list_state.clone(), move |index, window, cx| {
            div()
                .w_full()
                .min_w_0()
                .px_3()
                // Spacing between rows only, like a CSS gap: the list's own
                // bottom padding owns the gap after the last row.
                .when(index + 1 < item_count, |this| this.pb_8())
                .when_some(row_inset_left, |this, left| this.pl(left))
                .when_some(row_inset_right, |this, right| this.pr(right))
                .refine_style(&row_style)
                .child(renderer(index, window, cx))
                .into_any_element()
        })
        .size_full()
        .min_h_0()
        .py_2()
        .refine_style(&list_style);

        let viewport = div()
            .id((root_id.clone(), "viewport"))
            // Announce appended rows as a log region, like shadcn's
            // `role="log"` transcript content.
            .role(Role::Log)
            .size_full()
            .min_h_0()
            .min_w_0()
            .child(list)
            .when(self.scrollbar, |this| this.vertical_scrollbar(&list_state))
            .refine_style(&self.content_style);

        div()
            .id(root_id.clone())
            .relative()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .child(viewport)
            .when(show_jump_button, |this| {
                let state = self.state.clone();

                this.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom(rems(1.))
                        .flex()
                        .justify_center()
                        .child(
                            // No explicit width or height: Button sizes an
                            // icon-only button as a square on its own, and a
                            // renderer that adds a label or another semantic
                            // size must be able to change the layout.
                            Button::new((root_id, "jump-to-latest"))
                                .secondary()
                                .icon(IconName::ArrowDown)
                                .tooltip(self.jump_button_label)
                                .rounded(cx.theme().radius_full())
                                .border_1()
                                .border_color(tokens.colors.border)
                                .bg(tokens.colors.background)
                                .text_color(tokens.colors.foreground)
                                .refine_style(&jump_button_style)
                                .on_click(move |_, _, cx| {
                                    state.update(cx, |state, cx| state.scroll_to_end(cx));
                                })
                                .when_some(jump_button_renderer, |button, renderer| {
                                    renderer(button)
                                }),
                        ),
                )
            })
            .refine_style(&self.style)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext as _;
    use gpui_component::Sizable as _;

    #[gpui::test]
    fn test_message_scroller_state_builder(cx: &mut gpui::TestAppContext) {
        let state = cx.new(|cx| MessageScrollerState::new(3, cx));

        cx.update(|cx| {
            assert_eq!(state.read(cx).item_count(), 3);
            assert!(!state.read(cx).is_scrolled_up());
            assert!(state.read(cx).is_following_tail());

            state.update(cx, |state, cx| {
                assert!(!state.scroll_to_item(3, cx));
                assert!(state.append(2, cx));
                assert_eq!(state.item_count(), 5);
                assert!(state.prepend(1, cx));
                assert_eq!(state.item_count(), 6);
                assert!(!state.splice(5..7, 0, cx));
                assert!(state.remeasure_items(0..6, cx));
                assert!(!state.remeasure_items(6..7, cx));
                assert!(state.scroll_to_item(2, cx));
                assert!(!state.is_scrolled_up());
                assert!(!state.is_following_tail());
                state.scroll_to_end(cx);
                assert!(state.is_following_tail());
                state.reset(2, cx);
                assert_eq!(state.item_count(), 2);
                assert!(state.is_following_tail());
            });
        });
    }

    #[gpui::test]
    fn test_message_scroller_builder(cx: &mut gpui::TestAppContext) {
        let state = cx.new(|cx| MessageScrollerState::new(0, cx));
        let scroller = MessageScroller::new("message-scroller", state, |_, _, _| div())
            .scrollbar(false)
            .jump_button(false)
            .with_jump_button_label("Latest")
            .with_content_style(StyleRefinement::default())
            .with_list_style(StyleRefinement::default())
            .with_row_style(StyleRefinement::default())
            .with_jump_button_style(StyleRefinement::default())
            .with_jump_button_renderer(|button| button.large());

        assert!(!scroller.scrollbar);
        assert!(!scroller.jump_button);
        assert_eq!(scroller.jump_button_label, "Latest");
        assert!(scroller.jump_button_renderer.is_some());
    }
}

#[cfg(test)]
mod streaming_tests {
    use super::*;
    use gpui::{AppContext as _, Render, point, size};
    use std::{cell::RefCell, rc::Rc};

    struct Transcript {
        state: Entity<MessageScrollerState>,
        heights: Rc<RefCell<Vec<f32>>>,
    }
    impl Render for Transcript {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let heights = self.heights.clone();
            MessageScroller::new("transcript", self.state.clone(), move |ix, _, _| {
                div().w_full().h(px(heights.borrow()[ix]))
            })
            .with_list_style(StyleRefinement::default().p_0())
            .with_row_style(StyleRefinement::default().p_0())
        }
    }

    #[gpui::test]
    fn growing_reply_keeps_tail_or_reading_anchor(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let cx = cx.add_empty_window();
        let heights = Rc::new(RefCell::new(vec![50.; 10]));
        let (state, view) = cx.update(|_, cx| {
            let state = cx.new(|cx| MessageScrollerState::new(10, cx));
            let view = cx.new(|_| Transcript {
                state: state.clone(),
                heights: heights.clone(),
            });
            (state, view)
        });
        let draw = |cx: &mut gpui::VisualTestContext| {
            cx.draw(point(px(0.), px(0.)), size(px(240.), px(200.)), |_, _| {
                view.clone().into_any_element()
            });
        };
        draw(cx);
        heights.borrow_mut()[9] = 250.;
        cx.update(|_, cx| {
            state.update(cx, |s, cx| {
                s.remeasure_items(9..10, cx);
            })
        });
        draw(cx);
        cx.update(|_, cx| {
            let s = state.read(cx);
            assert!(s.is_following_tail());
            assert!(s.list_state.is_scrolled_to_end().unwrap());
        });
        cx.update(|_, cx| {
            state.update(cx, |s, cx| {
                s.scroll_to_item(2, cx);
            })
        });
        draw(cx);
        let before = cx.update(|_, cx| state.read(cx).list_state.logical_scroll_top());
        heights.borrow_mut()[9] = 650.;
        cx.update(|_, cx| {
            state.update(cx, |s, cx| {
                s.remeasure_items(9..10, cx);
            })
        });
        draw(cx);
        heights.borrow_mut().push(100.);
        cx.update(|_, cx| {
            state.update(cx, |s, cx| {
                s.append(1, cx);
            })
        });
        draw(cx);
        cx.update(|_, cx| {
            let s = state.read(cx);
            let after = s.list_state.logical_scroll_top();
            assert_eq!(before.item_ix, after.item_ix);
            assert_eq!(before.offset_in_item, after.offset_in_item);
            assert!(s.is_scrolled_up());
            assert!(!s.is_following_tail());
        });
        cx.update(|_, cx| state.update(cx, |s, cx| s.scroll_to_end(cx)));
        draw(cx);
        cx.update(|_, cx| {
            assert!(state.read(cx).is_following_tail());
            assert!(state.read(cx).list_state.is_scrolled_to_end().unwrap());
        });
    }
}
