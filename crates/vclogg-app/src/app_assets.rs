use std::borrow::Cow;

use gpui_kit::component::IconNamed;
use gpui_kit::{AssetSource, Result, SharedString};

/// Additional icons rendered by the standard Icon component.
/// Lucide 1.27.0 SVGs: https://github.com/lucide-icons/lucide/tree/1.27.0/icons.
/// License: assets/icons/lucide-LICENSE.txt.
/// Tabler Letter M: https://github.com/tabler/tabler-icons.
/// License: assets/icons/tabler-LICENSE.txt.
#[derive(Clone, Copy)]
pub(crate) enum AppIcon {
    History,
    Refresh,
    FollowEnd,
    Save,
    ColorLabel,
    LetterM,
}

impl AppIcon {
    const ALL: [Self; 6] = [
        Self::History,
        Self::Refresh,
        Self::FollowEnd,
        Self::Save,
        Self::ColorLabel,
        Self::LetterM,
    ];

    fn asset_path(self) -> &'static str {
        match self {
            Self::History => "vclogg/icons/history.svg",
            Self::Refresh => "vclogg/icons/refresh-cw.svg",
            Self::FollowEnd => "vclogg/icons/arrow-down-to-line.svg",
            Self::Save => "vclogg/icons/save.svg",
            Self::ColorLabel => "vclogg/icons/tag.svg",
            Self::LetterM => "vclogg/icons/letter-m.svg",
        }
    }

    fn data(self) -> &'static [u8] {
        match self {
            Self::History => include_bytes!("../assets/icons/history.svg"),
            Self::Refresh => include_bytes!("../assets/icons/refresh-cw.svg"),
            Self::FollowEnd => include_bytes!("../assets/icons/arrow-down-to-line.svg"),
            Self::Save => include_bytes!("../assets/icons/save.svg"),
            Self::ColorLabel => include_bytes!("../assets/icons/tag.svg"),
            Self::LetterM => include_bytes!("../assets/icons/letter-m.svg"),
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
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = gpui_kit::assets::Assets.list(path)?;
        assets.extend(
            AppIcon::ALL
                .into_iter()
                .filter(|icon| icon.asset_path().starts_with(path))
                .map(IconNamed::path),
        );
        Ok(assets)
    }
}
