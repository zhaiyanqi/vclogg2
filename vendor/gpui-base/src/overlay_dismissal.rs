use std::rc::Rc;

use gpui::{App, ElementId, Global, WeakEntity, Window, WindowId};

type Dismiss = Rc<dyn Fn(&mut Window, &mut App) -> bool>;

struct OverlayDismissal {
    priority: usize,
    dismiss: Option<Dismiss>,
}

#[derive(Default)]
struct OverlayDismissals(Vec<(WindowId, WeakEntity<OverlayDismissal>)>);

impl Global for OverlayDismissals {}

/// Register a rendered surface's normal cancellation path. Keyed element state
/// owns the callback; the registry must not keep an unmounted surface alive.
pub(crate) fn register_overlay_dismissal(
    id: impl Into<ElementId>,
    priority: usize,
    window: &mut Window,
    cx: &mut App,
    dismiss: impl Fn(&mut Window, &mut App) -> bool + 'static,
) {
    let window_id = window.window_handle().window_id();
    let state = window.use_keyed_state(id, cx, |_, _| OverlayDismissal {
        priority,
        dismiss: None,
    });
    let registry = cx.default_global::<OverlayDismissals>();
    registry.0.retain(|(_, state)| state.is_upgradable());
    if !registry
        .0
        .iter()
        .any(|(_, registered)| registered.entity_id() == state.entity_id())
    {
        registry.0.push((window_id, state.downgrade()));
    }
    state.update(cx, |state, _| {
        state.priority = priority;
        state.dismiss = Some(Rc::new(dismiss));
    });
}

/// Cancel rendered dialogs, popovers, and sheets in this window, front to back.
/// Runs each surface's existing callbacks and focus restoration. Returns false
/// if a dialog vetoes cancellation; callers should then keep the current task.
/// Call outside a view update, because callbacks may update their owning view.
pub fn dismiss_window_overlays(window: &mut Window, cx: &mut App) -> bool {
    let Some(registry) = cx.try_global::<OverlayDismissals>() else {
        return true;
    };
    let window_id = window.window_handle().window_id();
    let mut surfaces = registry
        .0
        .iter()
        .filter(|(id, _)| *id == window_id)
        .filter_map(|(_, state)| state.upgrade())
        .collect::<Vec<_>>();
    surfaces.sort_by_key(|state| state.read(cx).priority);
    for state in surfaces.into_iter().rev() {
        let dismiss = state.update(cx, |state, _| state.dismiss.take());
        if let Some(dismiss) = dismiss {
            if !dismiss(window, cx) {
                state.update(cx, |state, _| state.dismiss = Some(dismiss));
                return false;
            }
        }
    }
    true
}
