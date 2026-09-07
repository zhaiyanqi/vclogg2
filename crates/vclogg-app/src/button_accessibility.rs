use gpui::{SharedString, StatefulInteractiveElement as _};
use gpui_component::button::Button;

// Button owns a stable ID but does not expose StatefulInteractiveElement.
// Borrow its interactivity to name the button without another rendered node.
struct ButtonAccessibility<'a>(&'a mut Button);

impl gpui::InteractiveElement for ButtonAccessibility<'_> {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.0.interactivity()
    }
}

impl gpui::StatefulInteractiveElement for ButtonAccessibility<'_> {}

pub(crate) fn with_label(mut button: Button, label: impl Into<SharedString>) -> Button {
    ButtonAccessibility(&mut button).aria_label(label);
    button
}
