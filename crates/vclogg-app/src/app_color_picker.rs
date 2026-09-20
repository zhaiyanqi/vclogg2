use gpui_kit::Entity;
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerState};

/// The upstream picker keys featured swatches by color value. Repeated theme
/// colors then produce duplicate accessibility node IDs and panic in debug
/// builds. The full palette and sliders remain available without that row.
pub(crate) fn new(state: &Entity<ColorPickerState>) -> ColorPicker {
    ColorPicker::new(state).featured_colors(Vec::new())
}
