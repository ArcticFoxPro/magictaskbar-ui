use std::path::PathBuf;

use crate::system_state::MonitorId;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FocusedApp {
    pub hwnd: isize,
    pub monitor: MonitorId,
    pub title: String,
    pub name: String,
    pub exe: Option<PathBuf>, // todo remove this and refactor taskbar items to a shared windows state
    pub umid: Option<String>, // todo remove this and refactor taskbar items to a shared windows state
    pub is_maximized: bool,
    pub is_fullscreened: bool,
    pub is_bar_overlay: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UserApplication {
    pub name: String,
    pub path: PathBuf,
    pub umid: Option<String>,
    pub is_in_start_menu: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UserAppWindow {
    pub hwnd: isize,
    pub monitor: MonitorId,
    pub title: String,
    pub is_zoomed: bool,
    pub is_iconic: bool,
    pub is_fullscreen: bool,
    /// Window icon as base64-encoded PNG (ManagedShell-style)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_png_base64: Option<String>,
    /// Whether the icon should show a backplate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_approximately_square: Option<bool>,
    /// Whether the icon is from local icon directory
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_from_local: Option<bool>,
}
