use serde::{Deserialize, Serialize};

use super::*;

pub(super) const SIDEBAR_RAIL_WIDTH_REM: f32 = 2.25;
pub(super) const SIDEBAR_MIN_WIDTH_REM: f32 = 8.;
pub(super) const SIDEBAR_MAX_WIDTH_REM: f32 = 48.;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SidebarPanelId {
    Files,
    Favorites,
    History,
    Marks,
    Minutes,
    Colors,
    LogColoring,
    Minimap,
    Tabs,
}

impl SidebarPanelId {
    pub(super) const ALL: [Self; 9] = [
        Self::Files,
        Self::Favorites,
        Self::History,
        Self::Marks,
        Self::Minutes,
        Self::Colors,
        Self::LogColoring,
        Self::Minimap,
        Self::Tabs,
    ];
    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Files => crate::tr!("文件", "Files"),
            Self::Favorites => crate::tr!("收藏", "Favorites"),
            Self::History => crate::tr!("历史", "History"),
            Self::Marks => crate::tr!("标记", "Marks"),
            Self::Minutes => crate::tr!("时间分组", "Time groups"),
            Self::LogColoring => crate::tr!("日志着色", "Log coloring"),
            Self::Colors => crate::tr!("颜色标签", "Color labels"),
            Self::Minimap => crate::tr!("文件缩略图", "Minimap"),
            Self::Tabs => crate::tr!("标签页", "Tabs"),
        }
    }
    pub(super) fn icon(self) -> AnyElement {
        if self == Self::Marks {
            return Icon::new(crate::app_assets::AppIcon::LetterM)
                .small()
                .into_any_element();
        }
        if self == Self::History {
            return Icon::new(crate::app_assets::AppIcon::History)
                .small()
                .into_any_element();
        }
        Icon::new(match self {
            Self::Files => IconName::Folder,
            Self::Favorites => IconName::Star,
            Self::History => unreachable!("history uses its own icon"),
            Self::Marks => unreachable!("marks use the Letter M icon"),
            Self::Minutes => IconName::Calendar,
            Self::Colors => IconName::Palette,
            Self::LogColoring => IconName::Palette,
            Self::Minimap => IconName::Map,
            Self::Tabs => IconName::File,
        })
        .small()
        .into_any_element()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(in super::super) enum SidebarSide {
    Left,
    Right,
}

impl SidebarSide {
    pub(super) fn ix(self) -> usize {
        if self == Self::Left { 0 } else { 1 }
    }
    pub(super) fn other(self) -> Self {
        if self == Self::Left {
            Self::Right
        } else {
            Self::Left
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct SidebarPlacement {
    pub panels: Vec<SidebarPanelId>,
    pub active: Option<SidebarPanelId>,
    pub visible: bool,
    pub width: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in super::super) struct SidebarLayout {
    version: u32,
    #[serde(default)]
    pub(super) vertical_tabs: bool,
    pub(super) sides: [SidebarPlacement; 2],
}

impl Default for SidebarLayout {
    fn default() -> Self {
        Self {
            version: 3,
            vertical_tabs: false,
            sides: [
                SidebarPlacement {
                    panels: vec![
                        SidebarPanelId::Files,
                        SidebarPanelId::Favorites,
                        SidebarPanelId::History,
                        SidebarPanelId::Marks,
                        SidebarPanelId::LogColoring,
                    ],
                    active: Some(SidebarPanelId::Files),
                    visible: false,
                    width: 18.,
                },
                SidebarPlacement {
                    panels: vec![
                        SidebarPanelId::Minutes,
                        SidebarPanelId::Colors,
                        SidebarPanelId::Minimap,
                    ],
                    active: Some(SidebarPanelId::Minutes),
                    visible: false,
                    width: 18.,
                },
            ],
        }
    }
}

impl SidebarLayout {
    pub(super) fn decode(value: &str) -> Self {
        let Ok(mut layout) = serde_json::from_str::<Self>(value) else {
            return Self::default();
        };
        if layout.version == 1 {
            if !layout
                .sides
                .iter()
                .any(|side| side.panels.contains(&SidebarPanelId::LogColoring))
            {
                layout.sides[0].panels.push(SidebarPanelId::LogColoring);
            }
            layout.version = 2;
        }
        if layout.version == 2 {
            if !layout
                .sides
                .iter()
                .any(|side| side.panels.contains(&SidebarPanelId::Marks))
            {
                let left = &mut layout.sides[0].panels;
                let ix = left
                    .iter()
                    .position(|panel| *panel == SidebarPanelId::History)
                    .map_or(left.len(), |ix| ix + 1);
                left.insert(ix, SidebarPanelId::Marks);
            }
            layout.version = 3;
        }
        let panels = layout
            .sides
            .iter()
            .flat_map(|side| side.panels.iter().copied())
            .collect::<BTreeSet<_>>();
        let expected = if layout.vertical_tabs { 9 } else { 8 };
        if layout.version != 3
            || panels.len() != expected
            || panels.contains(&SidebarPanelId::Tabs) != layout.vertical_tabs
            || layout
                .sides
                .iter()
                .map(|side| side.panels.len())
                .sum::<usize>()
                != expected
        {
            return Self::default();
        }
        for side in &mut layout.sides {
            side.width = if side.width.is_finite() {
                side.width
                    .clamp(SIDEBAR_MIN_WIDTH_REM, SIDEBAR_MAX_WIDTH_REM)
            } else {
                18.
            };
            if !side
                .active
                .is_some_and(|active| side.panels.contains(&active))
            {
                side.active = side.panels.first().copied();
            }
        }
        layout
    }

    pub(super) fn move_panel(
        &mut self,
        panel: SidebarPanelId,
        side: SidebarSide,
        before: Option<SidebarPanelId>,
    ) {
        if before == Some(panel) {
            return;
        }
        for placement in &mut self.sides {
            if let Some(ix) = placement.panels.iter().position(|id| *id == panel) {
                placement.panels.remove(ix);
                if placement.active == Some(panel) {
                    placement.active = placement
                        .panels
                        .get(ix)
                        .or_else(|| placement.panels.last())
                        .copied();
                }
                if placement.panels.is_empty() {
                    placement.visible = false;
                }
            }
        }
        let placement = &mut self.sides[side.ix()];
        let ix = before
            .and_then(|before| placement.panels.iter().position(|id| *id == before))
            .unwrap_or(placement.panels.len());
        placement.panels.insert(ix, panel);
        placement.active = Some(panel);
        placement.visible = true;
    }

    pub(super) fn set_vertical_tabs(&mut self, enabled: bool) {
        self.vertical_tabs = enabled;
        if enabled {
            let left = &mut self.sides[0];
            if !left.panels.contains(&SidebarPanelId::Tabs) {
                left.panels.insert(0, SidebarPanelId::Tabs);
            }
            left.active = Some(SidebarPanelId::Tabs);
            left.visible = true;
        } else {
            for side in &mut self.sides {
                side.panels.retain(|panel| *panel != SidebarPanelId::Tabs);
                if side.active == Some(SidebarPanelId::Tabs) {
                    side.active = side.panels.first().copied();
                }
                if side.panels.is_empty() {
                    side.visible = false;
                }
            }
        }
    }

    /// Resolve temporary constraints without changing persisted user preferences.
    pub(super) fn fit(&self, available: f32) -> [Option<f32>; 2] {
        let mut widths = self
            .sides
            .each_ref()
            .map(|side| side.visible.then_some(side.width));
        for ix in [1, 0] {
            let used = 24.
                + widths
                    .iter()
                    .flatten()
                    .map(|width| width + SIDEBAR_RAIL_WIDTH_REM)
                    .sum::<f32>();
            if used > available
                && let Some(width) = &mut widths[ix]
            {
                *width = (*width - (used - available)).max(SIDEBAR_MIN_WIDTH_REM);
            }
        }
        for ix in [1, 0] {
            if 24.
                + widths
                    .iter()
                    .flatten()
                    .map(|width| width + SIDEBAR_RAIL_WIDTH_REM)
                    .sum::<f32>()
                > available
            {
                widths[ix] = None;
            }
        }
        widths
    }
}

#[derive(Clone)]
pub(in super::super) struct DraggedSidebarPanel {
    pub(super) owner: gpui::EntityId,
    pub(super) panel: SidebarPanelId,
}

impl Render for DraggedSidebarPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_2()
            .p_2()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .child(self.panel.icon())
            .child(self.panel.title())
    }
}
