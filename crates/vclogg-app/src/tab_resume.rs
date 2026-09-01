use serde::{Deserialize, Serialize};

pub(crate) const TAB_RESUME_VERSION: u32 = 1;
const PIXEL_SCALE: f32 = 1_000.;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PersistedLogRegion {
    #[default]
    Body,
    CurrentResults,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ViewportBookmark {
    pub anchor_source_row: usize,
    pub anchor_viewport_y_milli: i64,
    /// Measured wrapped height of the anchor row. Older bookmarks leave this at zero.
    pub anchor_row_height_milli: i64,
    pub horizontal_offset_milli: i64,
    pub at_end: bool,
}

impl ViewportBookmark {
    pub fn new(
        anchor_source_row: usize,
        anchor_viewport_y: f32,
        horizontal_offset: f32,
        at_end: bool,
    ) -> Self {
        Self {
            anchor_source_row,
            anchor_viewport_y_milli: encode_pixels(anchor_viewport_y),
            anchor_row_height_milli: 0,
            horizontal_offset_milli: encode_pixels(horizontal_offset),
            at_end,
        }
    }

    pub fn with_anchor_row_height(mut self, anchor_row_height: f32) -> Self {
        self.anchor_row_height_milli = encode_pixels(anchor_row_height);
        self
    }

    pub fn anchor_viewport_y(self) -> f32 {
        decode_pixels(self.anchor_viewport_y_milli)
    }

    pub fn anchor_row_height(self) -> Option<f32> {
        let height = decode_pixels(self.anchor_row_height_milli);
        (height > 0.).then_some(height)
    }

    pub fn horizontal_offset(self) -> f32 {
        decode_pixels(self.horizontal_offset_milli)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ViewerResumeState {
    pub viewport: Option<ViewportBookmark>,
    pub auto_follow: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct CurrentSearchResumeState {
    pub results_visible: bool,
    pub selected_source_row: Option<usize>,
    pub selected_result_ix: Option<usize>,
    pub viewport: Option<ViewportBookmark>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct TabResumeState {
    pub version: u32,
    pub viewer: ViewerResumeState,
    pub current_search: CurrentSearchResumeState,
    pub active_region: PersistedLogRegion,
}

impl Default for TabResumeState {
    fn default() -> Self {
        Self {
            version: TAB_RESUME_VERSION,
            viewer: ViewerResumeState::default(),
            current_search: CurrentSearchResumeState::default(),
            active_region: PersistedLogRegion::Body,
        }
    }
}

impl TabResumeState {
    pub fn is_compatible(&self) -> bool {
        self.version == TAB_RESUME_VERSION
    }

    pub fn from_legacy(selected_row: Option<usize>, results_visible: bool) -> Self {
        Self {
            current_search: CurrentSearchResumeState {
                results_visible,
                ..CurrentSearchResumeState::default()
            },
            ..Self::default()
        }
        .with_legacy_selected_row(selected_row)
    }

    fn with_legacy_selected_row(mut self, selected_row: Option<usize>) -> Self {
        if let Some(row) = selected_row {
            self.viewer.viewport = Some(ViewportBookmark::new(row, 0., 0., false));
        }
        self
    }
}

fn encode_pixels(value: f32) -> i64 {
    if value.is_finite() {
        (value * PIXEL_SCALE).round() as i64
    } else {
        0
    }
}

fn decode_pixels(value: i64) -> f32 {
    value as f32 / PIXEL_SCALE
}
