use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::settings::HideMode;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum FancyToolbarSide {
    Top,
    Bottom,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
pub struct FancyToolbarSettings {
    pub enabled: bool,
    pub position: FancyToolbarSide,
    pub hide_mode: HideMode,
    pub height: u32,
}

impl Default for FancyToolbarSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            position: FancyToolbarSide::Top,
            hide_mode: HideMode::Never,
            height: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ByWidgetSettings {
    pub fancy_toolbar: FancyToolbarSettings,
}
