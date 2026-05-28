#[cfg(test)]
use crate::modules::bluetooth::BluetoothDevice;

#[cfg(test)]
use crate::{resource::*, state::by_monitor::MonitorConfiguration, state::*, system_state::*};
#[cfg(test)]
use std::{collections::HashMap, path::PathBuf};

// Test-only shim: provide the argument shape for LanguageActivateInputProfile
// so the TS bindings generator can compile without depending on the app crate.
#[cfg(test)]
#[derive(serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct ActivateInputArgs {
    pub guid_profile: String,
}

// Test-only shim: provide the argument shape for LanguageActivateKeyboardLayout
// so the TS bindings generator can compile without depending on the app crate.
#[cfg(test)]
#[derive(serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct ActivateKeyboardLayoutArgs {
    pub klid: String,
}

#[cfg(test)]
#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct ContextMenuState {
    pub item_id: Option<String>,
    pub display_name: Option<String>,
}

macro_rules! slu_commands_declaration {
    ($($key:ident = $fn_name:ident($($args:tt)*) $(-> $return_type:ty)?,)*) => {
        #[cfg(test)]
        pub struct FuncCommand;

        #[cfg(test)]
        impl FuncCommand {
            #[cfg(feature = "gen-binds")]
            pub(crate) fn generate_ts_file(path: &str) {
                let mut content: Vec<String> = std::vec::Vec::new();

                content.push("// This file was generated via rust macros. Don't modify manually.".to_owned());
                content.push("export enum FuncCommand {".to_owned());
                $(
                    content.push(format!("  {} = '{}',", stringify!($key), stringify!($fn_name)));
                )*
                content.push("}\n".to_owned());

                std::fs::write(path, content.join("\n")).unwrap();
            }
        }

        paste::paste! {
            $(
                $crate::__switch! {
                    if { $($args)* }
                    do {
                        #[cfg(test)]
                        #[derive(Deserialize, TS)]
                        #[serde(rename_all = "camelCase")]
                        #[allow(dead_code)]
                        struct [<FuncCommand $key Args>] {
                            $($args)*
                        }
                    }
                    else {}
                }
            )*

            /// Internal used as mapping of commands to their arguments
            #[cfg(test)]
            #[allow(non_camel_case_types, dead_code)]
            #[derive(Deserialize, TS)]
            #[cfg_attr(feature = "gen-binds", ts(export))]
            enum FuncCommandArgument {
                $(
                    #[allow(non_snake_case)]
                    $fn_name(Box<$crate::__switch! {
                        if { $($args)* }
                        do { [<FuncCommand $key Args>] }
                        else { () }
                    }>),
                )*
            }

            /// Internal used as mapping of commands to their return types
            #[cfg(test)]
            #[allow(non_camel_case_types, dead_code)]
            #[derive(Deserialize, TS)]
            #[cfg_attr(feature = "gen-binds", ts(export))]
            enum FuncCommandReturn {
                $(
                    #[allow(non_snake_case)]
                    $fn_name($crate::__switch! {
                        if { $($return_type)? }
                        do { $($return_type)? }
                        else { () }
                    }),
                )*
            }
        }
        #[macro_export]
        macro_rules! command_handler_list {
            () => {
                tauri::generate_handler![
                    $(
                        $fn_name,
                    )*
                ]
            };
        }

        pub use command_handler_list;
    };
}

slu_commands_declaration! {
    // General
    Run = run(program: PathBuf, args: Option<RelaunchArguments>, working_dir: Option<PathBuf>),
    RunAsAdmin = run_as_admin(program: PathBuf, args: Option<RelaunchArguments>),

    CheckPCManagerExists = check_pc_manager_exists() -> bool,

    GetFocusedApp = get_focused_app() -> FocusedApp,
    GetMousePosition = get_mouse_position() -> [i32; 2],
    IsDevMode = is_dev_mode() -> bool,
    IsAppxPackage = is_appx_package() -> bool,
    CheckTaskbarOverlapStatus = check_taskbar_overlap_status() -> bool,
    OpenFile = open_file(path: PathBuf),
    SelectFileOnExplorer = select_file_on_explorer(path: PathBuf),
    HonorCalendarWidgetOpen = honor_calendar_widget_open() -> bool,
    HonorMessageCenterUiOpen = honor_message_center_ui_open() -> bool,
    GetUserEnvs = get_user_envs() -> HashMap<String, String>,
    SendKeys = send_keys(keys: String),
    GetIcon = get_icon(
        #[ts(optional = nullable)]
        path: Option<PathBuf>,
        #[ts(optional = nullable)]
        umid: Option<String>
    ),
    SimulateFullscreen = simulate_fullscreen(),
    ShowDesktop = show_desktop(),
    GetLocalIcon = get_local_icon(process_name: String) -> Option<String>,
    GetLocalIconWhite = get_local_icon_white(process_name: String) -> Option<String>,
    IconExtractWithFallback = icon_extract_with_fallback(path: Option<String>, umid: Option<String>) -> (),

    // miscellaneous
    TranslateText = translate_text(source: String, source_lang: String, target_lang: String) -> String,

    // System
    SystemGetForegroundWindowColor = get_foreground_window_color() -> Color,
    SystemGetMonitors = get_connected_monitors() -> Vec<PhysicalMonitor>,
    SystemGetColors = get_system_colors() -> UIColors,
    SystemGetIsDarkMode = get_is_dark_mode() -> bool,
    SystemGetLanguages = get_system_languages() -> Vec<SystemLanguage>,
    GetForegroundWindowInfo = get_foreground_window_info() -> Option<(String, String)>,
    SystemGetWifiNetworks = system_get_wifi_networks() -> Vec<WifiNetwork>,
    SystemGetNetworkShareDevices = system_get_network_share_devices() -> Vec<NetworkShareDevice>,
    SystemConnectNetworkShareDevice = system_connect_network_share_device(device_id: String, device_name: String) -> (),
    SystemDisconnectNetworkShareDevice = system_disconnect_network_share_device(device_id: String, device_name: String) -> (),
    SystemGetWlanEnabled = system_get_wlan_enabled() -> bool,
    SystemSetWlanEnabled = system_set_wlan_enabled(enabled: bool) -> (),
    SystemConnectWifi = system_connect_wifi(args: WifiConnectArgs) -> (),
    SystemGetWifiAutoconnect = system_get_wifi_autoconnect(profile_name: String) -> bool,
    SystemSetWifiAutoconnect = system_set_wifi_autoconnect(profile_name: String, enabled: bool) -> (),
    SystemDisconnectWifi = system_disconnect_wifi() -> (),
    SystemForgetWifi = system_forget_wifi(profile_name: String) -> (),
    SystemOpenLocationSettings = system_open_location_settings() -> (),
    SystemOpenWifiSettings = system_open_wifi_settings() -> (),
    SystemOpenWlanFlyout = system_open_wlan_flyout() -> (),
    SystemCheckCaptivePortal = system_check_captive_portal() -> bool,
    SystemGetBluetoothEnabled = system_get_bluetooth_enabled() -> bool,
    SystemGetBluetoothDevices = system_get_bluetooth_devices() -> Vec<BluetoothDevice>,
    SystemSetBluetoothEnabled = system_set_bluetooth_enabled(enabled: bool),
    SystemOpenPowerSettings = system_open_power_settings() -> (),
    SystemOpenLanguageSettings = system_open_language_settings() -> (),
    SystemGetMasterVolume = system_get_master_volume() -> u8,
    SystemSetMasterVolume = system_set_master_volume(volume: u8),
    SystemGetMasterMuted = system_get_master_muted() -> bool,
    SystemSetMasterMuted = system_set_master_muted(muted: bool),
    SystemOpenVolumeMixer = system_open_volume_mixer() -> (),
    ControlCenterPostTrayClick = control_center_post_tray_click() -> (),
    ControlCenterIsVisible = control_center_is_visible() -> bool,
    CalendarIsVisible = calendar_is_visible() -> bool,
    MessageCenterIsVisible = message_center_is_visible() -> bool,
    YoyoLaunchAssistant = yoyo_launch_assistant() -> (),
    AiRecommendIconClicked = ai_recommend_icon_clicked(
        btnId: String,
        #[ts(optional = nullable)]
        windowTitle: Option<String>
    ) -> (),
    AiRecommendSendScreenRecognition = ai_recommend_send_screen_recognition(btnId: String, aiFunctionNames: String) -> (),
    SystemSetKeyboardLayout = set_system_keyboard_layout(id: String, handle: String),
    LanguageGetActiveInputProfile = get_active_input_profile(),
    LanguageGetInstalledInputProfiles = get_installed_input_profiles(),
    LanguageGetInstalledKeyboardLayouts = get_installed_keyboard_layouts(),
    LanguageGetLastActiveInputProfileCached = get_last_active_input_profile_cached(),
    LanguageActivateInputProfile = activate_input_profile(args: ActivateInputArgs),
    LanguageActivateKeyboardLayout = activate_keyboard_layout(args: ActivateKeyboardLayoutArgs),
    LanguageActivateInputProfileByName = activate_input_profile_by_name(name: String),
    LanguageActivateKeyboardLayoutViaTsf = activate_keyboard_layout_via_tsf(id: String, handle: String),
    LanguageGetImeMode = get_ime_mode(),
    LanguageToggleImeMode = toggle_ime_mode(),

    // System Tray
    ShowNativeTrayOverflow = show_native_tray_overflow(anchor_center_x: i32, anchor_top_y: i32, gap: i32) -> (bool, bool),
    IsTrayOverflowVisible = is_tray_overflow_visible() -> bool,
    HideTrayOverflow = hide_tray_overflow() -> bool,

    // Settings
    StateGetDefaultSettings = state_get_default_settings() -> Settings,
    StateGetDefaultMonitorSettings = state_get_default_monitor_settings() -> MonitorConfiguration,

    RemoveResource = remove_resource(id: ResourceId, kind: ResourceKind),

    StateGetThemes = state_get_themes() -> Vec<Theme>,
    StateGetIconPacks = state_get_icon_packs() -> Vec<IconPack>,
    StateGetTaskbarItems = state_get_taskbar_items(monitor_id: Option<MonitorId>) -> TaskbarItems,
    StateWriteTaskbarItems = state_write_taskbar_items(items: TaskbarItems),
    StateGetSettings = state_get_settings(path: Option<PathBuf>) -> Settings,
    StateWriteSettings = state_write_settings(settings: Settings),
    StateDeleteCachedIcons = state_delete_cached_icons(),
    StateGetPerformanceMode = state_get_performance_mode() -> PerformanceMode,

    // Defender
    GetDefenderDisabledFromRegistry = get_defender_disabled_from_registry() -> bool,
    SystemToggleDefender = system_toggle_defender(disabled: bool) -> (),

    // 服务体验优化（StopWU）
    GetStopWuFromRegistry = get_stop_wu_from_registry() -> bool,
    SystemToggleStopWu = system_toggle_stop_wu(enabled: bool) -> (),

    // 浏览器体验增强（StopEdgeAds）
    GetBrowserEnhanceFromRegistry = get_browser_enhance_from_registry() -> bool,
    SystemToggleBrowserEnhance = system_toggle_browser_enhance(enabled: bool) -> (),

    // 升级管理
    GetUpgradeModeFromRegistry = get_upgrade_mode_from_registry() -> bool,
    SystemToggleUpgradeMode = system_toggle_upgrade_mode(enabled: bool) -> (),

    // Shortcut
    ShortcutGetKeys = shortcut_get_keys() -> Vec<String>,
    ShortcutSaveKeys = shortcut_save_keys(shortcut_ids: Vec<String>),

    // Shell

    // Taskbar
    TaskbarCloseApp = taskbar_close_app(hwnd: isize),
    TaskbarKillApp = taskbar_kill_app(hwnd: isize),
    TaskbarToggleWindowState = taskbar_toggle_window_state(hwnd: isize, was_focused: bool),
    SetForegroundWindow = set_foreground_window(hwnd: isize),
    TaskbarRequestUpdatePreviews = taskbar_request_update_previews(handles: Vec<isize>),
    TaskbarPinItem = taskbar_pin_item(
        umid: Option<String>,
        relaunch_program: String,
        display_name: String,
        path: PathBuf,
        original_id: Option<String>,
        relaunch_args: Option<String>,
        target_index: Option<usize>
    ),
    TaskbarUnpinItem = taskbar_unpin_item(umid: Option<String>, relaunch_program: String),
    TaskbarGetWebviewHwnd = taskbar_get_webview_hwnd() -> isize,
    TaskbarSaveWindowCoordinates = taskbar_save_window_coordinates(logs: String),
    TaskbarBringToFront = taskbar_bring_to_front(),
    TaskbarUpdateWindowSize = taskbar_update_window_size(width: i32, container_left: i32, container_top: i32, container_height: i32),
    TaskbarHideGlassEffect = taskbar_hide_glass_effect(),
    TaskbarShowGlassEffect = taskbar_show_glass_effect(),

    // Preview,
    PreviewTriggerShow = preview_trigger_show(payload: serde_json::Value, monitor_id: Option<String>),
    PreviewSetPosition = preview_set_position(x: i32, y: i32, width: i32, height: i32, monitor_id: Option<String>),
    PreviewShow = preview_show(monitor_id: Option<String>),
    PreviewHide = preview_hide(monitor_id: Option<String>),
    PreviewReady = preview_ready(monitor_id: Option<String>),

    // ContextMenu (懒创建 + 延迟销毁)
    ContextMenuTrigger = contextmenu_trigger(payload: serde_json::Value),
    ContextMenuReady = contextmenu_ready(),
    ContextMenuDestroy = contextmenu_destroy(),
    ContextMenuSetPosition = contextmenu_set_position(x: i32, y: i32, width: i32, height: i32),
    ContextMenuShow = contextmenu_show(),
    ContextMenuHide = contextmenu_hide(),
    ContextMenuSetState = contextmenu_set_state(item_id: Option<String>, display_name: Option<String>),
    ContextMenuGetState = contextmenu_get_state() -> ContextMenuState,

    SystemLockScreen = system_lock_screen() -> (),
    SystemRecycleFiles = system_recycle_files(paths: Vec<PathBuf>),
    SystemEmptyRecycleBin = system_empty_recycle_bin(),
    SystemOpenRecycleBin = system_open_recycle_bin(),
    SystemIsRecycleBinOpen = system_is_recycle_bin_open() -> bool,
    SystemIsRecycleBinEmpty = system_is_recycle_bin_empty() -> bool,
    SystemCloseRecycleBin = system_close_recycle_bin(),
    SystemGetRecycleBinHwnd = system_get_recycle_bin_hwnd() -> i64,

    SystemSleep = system_sleep() -> (),
    SystemHibernate = system_hibernate() -> (),
    SystemShutdown = system_shutdown() -> (),
    SystemRestart = system_restart() -> (),
    SystemExitToDesktop = system_exit_to_desktop() -> (),
    GpuWakeAsync = gpu_wake_async() -> (),
    SystemCheckUpdate = system_check_update() -> String,
    SystemSendCheckUpdateToMagicvisuals = system_send_check_update_to_magicvisuals() -> (),
    SystemSendDownloadUpdateToMagicvisuals = system_send_download_update_to_magicvisuals() -> (),
    SystemSendStartInstallToMagicvisuals = system_send_start_install_to_magicvisuals() -> (),
    SystemSendLoginAccountToMagicvisuals = system_send_login_account_to_magicvisuals() -> (),
    SystemOpenCheckUpdateWindow = system_open_check_update_window() -> (),
    SystemOpenSettingsWindow = system_open_settings_window() -> (),
    SystemOpenAboutWindow = system_open_about_window() -> (),
    SystemOpenFeedbackWindow = system_open_feedback_window() -> (),
    SystemGetAppVersion = system_get_app_version() -> String,
    SystemIsGameFullscreenBlocked = system_is_game_fullscreen_blocked() -> bool,

    // Web
    OpenUrl = open_url(url: String) -> (),

    // Reporting
    ReportClickComponent = report_click_component(content: String) -> (),
    ReportToolbarMode = report_toolbar_mode(content: String) -> (),
    ReportSettingsClick = report_settings_click(content: String) -> (),
    ReportShortcutOperation = report_shortcut_operation(operation: String, tool_name: String) -> (),
    ReportAiRecommendFunction = report_ai_recommend_function(content: String) -> (),
    ReportFeedback = report_feedback(feedback_types: String, description: String, contact_info: String, upload_logs: bool) -> (),

    // Shortcut
    SendShortcutMessage = send_shortcut_message(shortcut_id: String) -> (),

    RequestPairBluetoothDevice = request_pair_bluetooth_device(id: String) -> DevicePairingNeededAction,
    ConfirmBluetoothDevicePairing = confirm_bluetooth_device_pairing(id: String, answer: DevicePairingAnswer),
    ConnectBluetoothDevice = connect_bluetooth_device(id: String),
    DisconnectBluetoothDevice = disconnect_bluetooth_device(id: String),
    SystemOpenBluetoothSettings = system_open_bluetooth_settings() -> (),
    ForgetBluetoothDevice = forget_bluetooth_device(id: String),
    StopBluetoothScanning = stop_bluetooth_scanning(),
    StartBluetoothScanning = start_bluetooth_scanning() -> (),
    SystemChangeTheme = system_change_theme(theme_type: u32) -> (),
    SystemSwitchIconBackplateStyle = system_switch_icon_backplate_style(style: String) -> (),
    SystemToggleMinimizeAnimation = system_toggle_minimize_animation(enabled: bool) -> (),
    SystemGetMinimizeAnimationFromRegistry = get_minimize_animation_from_registry() -> bool,
    SystemGetIconThemeFromRegistry = get_icon_theme_from_registry() -> u32,
    SystemGetUserExperiencePlanFromRegistry = get_user_experience_plan_from_registry() -> bool,
    SystemToggleUserExperiencePlan = system_toggle_user_experience_plan(enabled: bool) -> (),
    SystemToggleCleanMode = system_toggle_clean_mode(enabled: bool) -> (),
    SystemGetCleanModeFromRegistry = get_clean_mode_from_registry() -> bool,
    SendMessageToMagicSpaceTurbo = send_message_to_magic_space_turbo() -> (),
    SendMessageToAppStartup = send_message_to_app_startup() -> (),
    SendAppStartupStatus = send_app_startup_status(name: String, display_name: String, display_name_utf8: String, description: String, description_utf8: String, status: bool) -> (),
    SendThirdPartyAppStatus = send_third_party_app_status(category: String, app_name: String, status: String) -> (),
    SystemOpenFile = system_open_file(file_path: String) -> (),
    SystemStopService = system_stop_service() -> (),
    SystemGetTextScaleFactor = get_text_scale_factor() -> f64,

    // Popup Glass Effect
    PopupGlassShow = popup_glass_show(x: i32, y: i32, width: i32, height: i32, corner_radius: f32) -> (),
    PopupGlassHide = popup_glass_hide() -> (),

    // Toolbar
    ToolbarGetOverlapState = toolbar_get_overlap_state() -> bool,
    ToolbarGetMaximizedState = toolbar_get_maximized_state() -> bool,
}
