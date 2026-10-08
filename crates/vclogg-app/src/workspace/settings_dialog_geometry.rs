use super::*;

// Keep reading the existing size preference when upgrading from an embedded dialog.
#[derive(Default)]
pub(super) struct SettingsDialogGeometry {
    pub(super) preferred_size: Option<Size<Pixels>>,
}

impl Workspace {
    pub(super) fn restore_settings_dialog_size(
        &mut self,
        stored: Option<[f32; 2]>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.settings_dialog_geometry.preferred_size =
            stored.map(|[width, height]| size(px(width), px(height)));
    }
}
