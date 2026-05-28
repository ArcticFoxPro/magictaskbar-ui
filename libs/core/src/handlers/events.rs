use crate::state::*;
use crate::system_state::*;

#[derive(Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CheckUpdateResult {
    pub has_new_version: bool,
    pub new_version: String,
}

#[derive(Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DownloadUpdateResult {
    pub status: u32,
    /// 下载进度百分比 (0-100)，仅当 status=1003 时有效
    #[serde(default)]
    pub progress: u32,
}

macro_rules! slu_events_declaration {
    ($($name:ident$(($payload:ty))? as $value:literal,)*) => {
        pub struct FuncEvent;

        #[allow(non_upper_case_globals)]
        impl FuncEvent {
            $(
                pub const $name: &'static str = $value;
            )*

            #[allow(dead_code)]
            pub(crate) fn generate_ts_file(path: &str) {
                let content: Vec<String> = vec![
                    "// This file was generated via rust macros. Don't modify manually.".to_owned(),
                    "export enum FuncEvent {".to_owned(),
                    $(
                        format!("  {} = '{}',", stringify!($name), Self::$name),
                    )*
                    "}\n".to_owned(),
                ];
                std::fs::write(path, content.join("\n")).unwrap();
            }
        }

        #[derive(Serialize, TS)]
        #[cfg_attr(feature = "gen-binds", ts(export))]
        pub enum FuncEventPayload {
            $(
                #[serde(rename = $value)]
                $name($crate::__switch! {
                    if { $($payload)? }
                    do { Box<$($payload)?> }
                    else { () }
                }),
            )*
        }
    };
}

slu_events_declaration! {
    GlobalFocusChanged(FocusedApp) as "global-focus-changed",
    GlobalMouseMove([i32; 2]) as "global-mouse-move",

    HandleLayeredHitboxes(bool) as "handle-layered",

    SystemMonitorsChanged(Vec<PhysicalMonitor>) as "system::monitors-changed",
    SystemLanguagesChanged(Vec<SystemLanguage>) as "system::languages-changed",
    SystemNetworksChanged(Vec<crate::system_state::WifiNetwork>) as "system::networks-changed",
    SystemNetworkShareDevicesChanged(Vec<crate::system_state::NetworkShareDevice>) as "system::network-share-devices-changed",
    SystemVolumeChanged(crate::system_state::VolumeState) as "system::volume-changed",

    ColorsChanged(UIColors) as "colors-changed",

    TaskbarOverlaped(bool) as "set-auto-hide",
    // Notify frontend to refresh container position (DPI/monitor config changed)
    // Payload: (screen_center_x, dpi) - screen center X in physical pixels and DPI scale factor * 100 (to avoid float)
    TaskbarContainerRefresh((i32, u32)) as "taskbar::container-refresh",
    // alias for toolbar overlap to keep backward/library compatibility
    ToolbarOverlaped(bool) as "toolbar-overlaped",
    ToolbarHasMaximizedWindow(bool) as "toolbar-has-maximized-window",

    StateSettingsChanged(Settings) as "settings-changed",
    StateTaskbarItemsChanged as "taskbar-items",
    StateThemesChanged(Vec<Theme>) as "themes",
    StateIconPacksChanged(Vec<IconPack>) as "icon-packs",

    StatePerformanceModeChanged(PerformanceMode) as "state::performance-mode-changed",

    // Preview 窗口事件
    PreviewShow(PreviewShowPayload) as "preview::show",
    PreviewHide as "preview::hide",

    CheckUpdateMessageReceived(CheckUpdateResult) as "check-update-message",
    DownloadUpdateMessageReceived(DownloadUpdateResult) as "download-update-message",
    SystemBluetoothStateChanged(bool) as "system::bluetooth-state-changed",
    SystemBluetoothDevicesChanged(Vec<BluetoothDevice>) as "system::bluetooth-devices-changed",
    NotificationIconChanged(bool) as "notification-icon-changed",
    AppNotification(crate::system_state::AppNotification) as "app-notification",
    GameFullscreenChanged(bool) as "game-fullscreen-changed",
}
