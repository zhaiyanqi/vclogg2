use serde::{Deserialize, Serialize};

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SidebarPanelId {
    Files,
    Favorites,
    History,
    Minutes,
    Colors,
    Minimap,
}

impl SidebarPanelId {
    pub(super) const ALL: [Self; 6] = [
        Self::Files,
        Self::Favorites,
        Self::History,
        Self::Minutes,
        Self::Colors,
        Self::Minimap,
    ];
    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Files => crate::tr!("文件", "Files"),
            Self::Favorites => crate::tr!("收藏", "Favorites"),
            Self::History => crate::tr!("历史", "History"),
            Self::Minutes => crate::tr!("时间分组", "Time groups"),
            Self::Colors => crate::tr!("颜色标签", "Color labels"),
            Self::Minimap => crate::tr!("文件缩略图", "Minimap"),
        }
    }
    pub(super) fn icon(self) -> AnyElement {
        if self == Self::History {
            return svg()
                .data(include_bytes!("../../../assets/icons/history.svg"))
                .size_4()
                .flex_shrink_0()
                .into_any_element();
        }
        Icon::new(match self {
            Self::Files => IconName::Folder,
            Self::Favorites => IconName::Star,
            Self::History => unreachable!("history uses its own icon"),
            Self::Minutes => IconName::Calendar,
            Self::Colors => IconName::Palette,
            Self::Minimap => IconName::Map,
        })
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
    pub(super) sides: [SidebarPlacement; 2],
}

impl Default for SidebarLayout {
    fn default() -> Self {
        Self {
            version: 1,
            sides: [
                SidebarPlacement {
                    panels: vec![
                        SidebarPanelId::Files,
                        SidebarPanelId::Favorites,
                        SidebarPanelId::History,
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
        let panels = layout
            .sides
            .iter()
            .flat_map(|side| side.panels.iter().copied())
            .collect::<BTreeSet<_>>();
        if layout.version != 1
            || panels.len() != 6
            || layout
                .sides
                .iter()
                .map(|side| side.panels.len())
                .sum::<usize>()
                != 6
        {
            return Self::default();
        }
        for side in &mut layout.sides {
            side.width = if side.width.is_finite() {
                side.width.clamp(12., 32.)
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

    /// Resolve temporary constraints without changing persisted user preferences.
    pub(super) fn fit(&self, available: f32) -> [Option<f32>; 2] {
        let mut widths = self
            .sides
            .each_ref()
            .map(|side| side.visible.then_some(side.width));
        for ix in [1, 0] {
            let used = 24. + widths.iter().flatten().map(|width| width + 3.).sum::<f32>();
            if used > available
                && let Some(width) = &mut widths[ix]
            {
                *width = (*width - (used - available)).max(12.);
            }
        }
        for ix in [1, 0] {
            if 24. + widths.iter().flatten().map(|width| width + 3.).sum::<f32>() > available {
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
