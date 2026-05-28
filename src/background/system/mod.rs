use crate::{
    app::get_app_handle,
    cli::ServicePipe,
    error::Result,
    exposed, log_error,
    modules::{
        apps::infrastructure::register_app_win_events,
        language::register_language_events,
        monitors::infrastructure::register_monitor_webview_events,
        network::infrastructure::register_network_events,
        system_settings::infrastructure::{register_system_settings_events, release_colors_events},
    },
    widgets::taskbar::Taskbar,
    windows_api::event_window::subscribe_to_background_window,
};
use libs_core::handlers::{CheckUpdateResult, DownloadUpdateResult, FuncEvent};
use slu_ipc::messages::SvcAction;
use std::time::Duration;
use tauri::Emitter;
use windows::Win32::UI::WindowsAndMessaging::WM_USER;

// todo replace this by self module lazy initilization
pub fn declare_system_events_handlers() -> Result<()> {
    register_app_win_events();
    log_error!(register_monitor_webview_events());
    register_system_settings_events();
    register_network_events();
    register_language_events();
    Taskbar::ensure_taskbar_hidden_on_startup();

    // 注册处理来自其他进程的自定义消息
    register_toolbar_custom_messages();

    Ok(())
}

pub fn release_system_events_handlers() {
    release_colors_events();
}

fn read_pending_update_version_from_service() -> String {
    let data = ServicePipe::request_with_response_blocking(
        SvcAction::ExecuteBackendCommand {
            command: "system_get_pending_update_version".to_string(),
            args: serde_json::json!({}),
        },
        Duration::from_secs(2),
    );

    data.ok()
        .flatten()
        .and_then(|data| serde_json::from_str::<serde_json::Value>(&data).ok())
        .and_then(|value| {
            value
                .get("value")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default()
}

// 为了让其他进程可以发送消息给toolbar，我们注册一个消息监听器
fn register_toolbar_custom_messages() {
    subscribe_to_background_window(|msg, wparam, lparam| {
        const WM_TOOLBAR_CUSTOM_MSG: u32 = WM_USER + 100;
        const WM_MAGICVISUALS_CHECK_UPDATE: u32 = WM_USER + 3;
        const WM_MAGICVISUALS_DOWNLOAD_UPDATE: u32 = WM_USER + 4;
        const WM_OPEN_PUREMODE_SETTINGS: u32 = WM_USER + 11;

        if msg == WM_MAGICVISUALS_CHECK_UPDATE {
            log::info!(
                "[CheckUpdateMessage] Received check update message: wParam={}, lParam={}",
                wparam,
                lparam
            );

            // wParam为0表示无新版本或检查失败，重置UI到默认状态
            if wparam == 0 {
                let result = CheckUpdateResult {
                    has_new_version: false,
                    new_version: String::new(),
                };
                if let Err(e) =
                    get_app_handle().emit(FuncEvent::CheckUpdateMessageReceived, &result)
                {
                    log::warn!(
                        "[CheckUpdateMessage] Failed to emit check update event: {}",
                        e
                    );
                }
                return Ok(());
            }

            // wParam为1表示有新版本，读取新的版本号
            let has_new_version = wparam == 1;
            let new_version = if has_new_version {
                read_pending_update_version_from_service()
            } else {
                String::new()
            };

            // 发送事件给前端
            let result = CheckUpdateResult {
                has_new_version,
                new_version,
            };
            if let Err(e) = get_app_handle().emit(FuncEvent::CheckUpdateMessageReceived, &result) {
                log::warn!(
                    "[CheckUpdateMessage] Failed to emit check update event: {}",
                    e
                );
            }
        } else if msg == WM_MAGICVISUALS_DOWNLOAD_UPDATE {
            log::info!(
                "[DownloadUpdateMessage] Received download update message: wParam={}, lParam={}",
                wparam,
                lparam
            );

            // 下载状态码定义：
            // 1002 = 下载失败
            // 1003 = 下载进度，lParam 包含百分比
            // 1004 = 下载完成
            const DOWNLOAD_ERROR: u32 = 1002;
            const DOWNLOAD_PROCESS: u32 = 1003;
            const DOWNLOAD_FINISH: u32 = 1004;

            let status = wparam as u32;
            let progress = if status == DOWNLOAD_PROCESS {
                lparam as u32
            } else {
                0
            };

            let result = DownloadUpdateResult { status, progress };
            if let Err(e) = get_app_handle().emit(FuncEvent::DownloadUpdateMessageReceived, &result)
            {
                log::warn!(
                    "[DownloadUpdateMessage] Failed to emit download update event: {}",
                    e
                );
            }
        } else if msg == WM_TOOLBAR_CUSTOM_MSG {
            log::info!(
                "[ToolbarCustomMsg] Received custom message: wParam={}, lParam={}",
                wparam,
                lparam
            );
        } else if msg == WM_OPEN_PUREMODE_SETTINGS {
            log::info!("[OpenPureModeSettings] 收到打开纯净模式设置的消息");

            // 获取 AppHandle 并在主线程上打开设置窗口到纯净模式
            let app_handle = get_app_handle();

            // 创建异步任务来打开设置窗口
            let app_handle_clone = app_handle.clone();
            app_handle
                .run_on_main_thread(move || {
                    let app_handle = app_handle_clone.clone();
                    tauri::async_runtime::spawn(async move {
                        // 调用 system_open_puremode_settings，它会处理窗口的创建、恢复和显示
                        if let Err(e) =
                            exposed::system_open_puremode_settings(app_handle.clone()).await
                        {
                            log::warn!(
                                "[OpenPureModeSettings] Failed to open settings window: {}",
                                e
                            );
                        }
                    });
                })
                .ok();
        } else if msg == WM_USER + 101 {
            // WM_MAGIC_TSF_CHANGED: 收到来自服务进程的输入法切换通知
            log::info!(
                "[TSF-Message] Received TSF change notification from service: index={}",
                wparam
            );
            use crate::modules::language::tsf;
            let index = wparam as i32;
            tsf::handle_tsf_change_from_service(index);
        } else if msg == WM_USER + 401 {
            let mode = match wparam {
                1 => Some("中"),
                2 => Some("英"),
                3 => Some("A"),
                4 => Some("EN"),
                _ => None,
            };

            if let Some(mode) = mode {
                log::info!(
                    "[InputMethodMode] Received toolbar mode message: wParam={}, mode={}",
                    wparam,
                    mode
                );
                if let Err(e) = get_app_handle().emit("input_method_toolbar_mode_changed", mode) {
                    log::warn!("[InputMethodMode] Failed to emit toolbar mode event: {}", e);
                }
            } else {
                log::warn!(
                    "[InputMethodMode] Ignoring unknown toolbar mode message: wParam={}",
                    wparam
                );
            }
        } else if msg == WM_USER + 21 {
            // WM_NOTIFICATION_ICON_CHANGE: 收到通知图标变更消息
            log::info!(
                "[NotificationIcon] Received notification icon change message: wParam={}",
                wparam
            );
            // wParam: 0 = no notification (nonotify), 1 = has notification (havenotify)
            let has_notification = wparam != 0;
            if let Err(e) =
                get_app_handle().emit(FuncEvent::NotificationIconChanged, has_notification)
            {
                log::warn!(
                    "[NotificationIcon] Failed to emit notification icon change event: {}",
                    e
                );
            }
        }
        Ok(())
    });
}
