use std::sync::Arc;

use gpui::{Global, PlatformTextSystem};

/// Shares the application's font backend without adding GPUI's fallback aliases
/// to the settings picker. Font files are loaded by the backend when used.
pub(crate) struct SystemFonts(Arc<dyn PlatformTextSystem>);

impl Global for SystemFonts {}

impl SystemFonts {
    pub(crate) fn new(text_system: Arc<dyn PlatformTextSystem>) -> Self {
        Self(text_system)
    }

    pub(crate) fn available_families(&self) -> Vec<String> {
        let mut names = self.0.all_font_names();
        names.retain(|name| !name.trim().is_empty() && !name.starts_with('.'));
        names.sort_unstable();
        names.dedup();
        names
    }
}
