use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};
use gpui_component::IconNamed;

/// Additional Lucide 1.27.0 icons, rendered by the standard Icon component.
/// Unmodified SVGs from https://github.com/lucide-icons/lucide/tree/1.27.0/icons.
/// License: assets/icons/lucide-LICENSE.txt.
#[derive(Clone, Copy)]
pub(crate) enum AppIcon {
    History,
    Refresh,
    FollowEnd,
}

impl AppIcon {
    const ALL: [Self; 3] = [Self::History, Self::Refresh, Self::FollowEnd];

    fn asset_path(self) -> &'static str {
        match self {
            Self::History => "vclogg/icons/history.svg",
            Self::Refresh => "vclogg/icons/refresh-cw.svg",
            Self::FollowEnd => "vclogg/icons/arrow-down-to-line.svg",
        }
    }

    fn data(self) -> &'static [u8] {
        match self {
            Self::History => include_bytes!("../assets/icons/history.svg"),
            Self::Refresh => include_bytes!("../assets/icons/refresh-cw.svg"),
            Self::FollowEnd => include_bytes!("../assets/icons/arrow-down-to-line.svg"),
        }
    }
}

impl IconNamed for AppIcon {
    fn path(self) -> SharedString {
        self.asset_path().into()
    }
}

pub(crate) struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(icon) = AppIcon::ALL
            .into_iter()
            .find(|icon| icon.asset_path() == path)
        {
            return Ok(Some(Cow::Borrowed(icon.data())));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = gpui_component_assets::Assets.list(path)?;
        assets.extend(
            AppIcon::ALL
                .into_iter()
                .filter(|icon| icon.asset_path().starts_with(path))
                .map(IconNamed::path),
        );
        Ok(assets)
    }
}
