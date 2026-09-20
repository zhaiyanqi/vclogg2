use gpui_kit::component::button::Button;
use gpui_kit::{SharedString, StatefulInteractiveElement as _};

// Button owns a stable ID but does not expose StatefulInteractiveElement.
// Borrow its interactivity to name the button without another rendered node.
struct ButtonAccessibility<'a>(&'a mut Button);

impl gpui_kit::InteractiveElement for ButtonAccessibility<'_> {
    fn interactivity(&mut self) -> &mut gpui_kit::Interactivity {
        self.0.interactivity()
    }
}

impl gpui_kit::StatefulInteractiveElement for ButtonAccessibility<'_> {}

pub(crate) fn with_label(mut button: Button, label: impl Into<SharedString>) -> Button {
    ButtonAccessibility(&mut button).aria_label(label);
    button
}
