use std::borrow::Cow;

use gpui_kit::component::IconNamed;
use gpui_kit::{AssetSource, Result, SharedString};

/// Additional icons rendered by the standard Icon component.
/// Lucide SVGs: existing icons from 1.27.0; file-pen-line and save-check from 1.43.0.
/// License: assets/icons/lucide-LICENSE.txt.
/// Tabler Letter M: https://github.com/tabler/tabler-icons.
/// License: assets/icons/tabler-LICENSE.txt.
#[derive(Clone, Copy)]
pub(crate) enum AppIcon {
    History,
    Refresh,
    FollowEnd,
    SaveCheck,
    Edit,
    ColorLabel,
    LetterM,
}

impl AppIcon {
    const ALL: [Self; 7] = [
        Self::History,
        Self::Refresh,
        Self::FollowEnd,
        Self::SaveCheck,
        Self::Edit,
        Self::ColorLabel,
        Self::LetterM,
    ];

    fn asset_path(self) -> &'static str {
        match self {
            Self::History => "vclogg/icons/history.svg",
            Self::Refresh => "vclogg/icons/refresh-cw.svg",
            Self::FollowEnd => "vclogg/icons/arrow-down-to-line.svg",
            Self::SaveCheck => "vclogg/icons/save-check.svg",
            Self::Edit => "vclogg/icons/file-pen-line.svg",
            Self::ColorLabel => "vclogg/icons/tag.svg",
            Self::LetterM => "vclogg/icons/letter-m.svg",
        }
    }

    fn data(self) -> &'static [u8] {
        match self {
            Self::History => include_bytes!("../assets/icons/history.svg"),
            Self::Refresh => include_bytes!("../assets/icons/refresh-cw.svg"),
            Self::FollowEnd => include_bytes!("../assets/icons/arrow-down-to-line.svg"),
            Self::SaveCheck => include_bytes!("../assets/icons/save-check.svg"),
            Self::Edit => include_bytes!("../assets/icons/file-pen-line.svg"),
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

gpui_kit::assets::icon_assets!(SearchInputAssets, [TextWrap]);

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(icon) = SearchInputAssets.load(path)? {
            return Ok(Some(icon));
        }
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
        assets.extend(SearchInputAssets.list(path)?);
        assets.extend(
            AppIcon::ALL
                .into_iter()
                .filter(|icon| icon.asset_path().starts_with(path))
                .map(IconNamed::path),
        );
        Ok(assets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiline_search_icon_is_bundled() {
        let path = gpui_kit::assets::IconName::TextWrap.path();
        assert!(Assets.load(&path).unwrap().is_some());
        assert!(Assets.list("icons/").unwrap().contains(&path));
    }

    #[test]
    fn edit_action_icon_is_available_from_application_assets() {
        let path = AppIcon::Edit.asset_path();
        assert!(Assets.load(path).unwrap().is_some());
        assert!(
            Assets
                .list("vclogg/icons/")
                .unwrap()
                .iter()
                .any(|item| item.as_ref() == path)
        );
    }
}
