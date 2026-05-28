use crate::app::get_app_handle;
use crate::windows_api::process::Process;
use libs_core::handlers::FuncEvent;
use libs_core::system_state::AppNotification;
use log::{error, info, warn};
use serde::Deserialize;
use std::fs::File;
use std::io::Read;
use tauri::Emitter;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowTextW, GetWindowThreadProcessId,
};

use super::domain::{AppNotificationEvent, NotificationWhitelist};

#[derive(Deserialize)]
pub struct WhitelistConfig {
    pub notifications: Vec<String>,
}

/// 加载通知白名单
pub fn load_notification_whitelist() -> NotificationWhitelist {
    use crate::utils::constants::VAR_COMMON;

    let resource_dir = VAR_COMMON.app_resource_dir();
    let config_path = resource_dir.join("static/app_notification_whitelist.yml");
    let mut whitelist = NotificationWhitelist::new();

    if config_path.exists() {
        match File::open(config_path) {
            Ok(mut file) => {
                let mut content = String::new();
                if file.read_to_string(&mut content).is_ok() {
                    match serde_yaml::from_str::<WhitelistConfig>(&content) {
                        Ok(config) => {
                            for app in config.notifications {
                                log::debug!("Added to notification whitelist: {}", app);
                                whitelist.add(app);
                            }
                            log::debug!(
                                "Notification whitelist loaded with {} applications",
                                whitelist.applications.len()
                            );
                        }
                        Err(e) => {
                            log::error!("Failed to parse notification whitelist: {:?}", e);
                        }
                    }
                } else {
                    log::error!("Failed to read notification whitelist file");
                }
            }
            Err(e) => {
                log::error!("Failed to open notification whitelist file: {:?}", e);
            }
        }
    } else {
        log::warn!(
            "Notification whitelist config file not found: {:?}",
            config_path
        );
    }

    whitelist
}

/// 处理 Shell Hook 事件
pub fn handle_shell_hook_event(wparam: u32, lparam: isize, whitelist: &NotificationWhitelist) {
    // HSHELL_FLASH = 0x8006 = 32774
    if wparam == 0x8006 {
        let hwnd = HWND(lparam as _);
        info!("HSHELL_FLASH event from HWND: {:?}", hwnd);

        if let Some(event) = get_app_notification_event(hwnd) {
            info!(
                "Got notification event: process_name={}, window_title={}, process_id={}",
                event.process_name, event.window_title, event.process_id
            );

            if whitelist.contains(&event.process_name) {
                // 上报通知事件
                let app_notification = AppNotification {
                    id: event.process_id,
                    app_umid: event.process_name.clone(),
                    app_name: event.process_name.clone(),
                    app_description: event.window_title,
                    date: chrono::Utc::now().timestamp(),
                    content: libs_core::system_state::Toast::default(),
                };

                if let Err(e) = get_app_handle().emit(FuncEvent::AppNotification, &app_notification)
                {
                    error!("Failed to emit app notification event: {:?}", e);
                }
            } else {
                info!(
                    "Notification from app NOT in whitelist: {}",
                    event.process_name
                );
            }
        } else {
            info!("Failed to get notification event for HWND: {:?}", hwnd);
        }
    }
}

/// 从窗口句柄获取应用通知事件
fn get_app_notification_event(hwnd: HWND) -> Option<AppNotificationEvent> {
    // 获取进程 ID
    let mut process_id = 0;
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    if thread_id == 0 || process_id == 0 {
        warn!("Failed to get process ID for window: {:?}", hwnd);
        return None;
    }

    // 获取进程名
    use crate::windows_api::window::Window;
    let window = Window::from(hwnd);
    let process = Process::from_window(&window);

    let process_name = match process.program_exe_name() {
        Ok(name) => name,
        Err(e) => {
            warn!("Failed to get process name for PID {}: {:?}", process_id, e);
            return None;
        }
    };

    // 获取窗口标题
    let mut title_buffer = [0u16; 256];
    let title_length = unsafe { GetWindowTextW(hwnd, &mut title_buffer) };
    let window_title = String::from_utf16_lossy(&title_buffer[..title_length as usize]);

    // 获取窗口类名
    let mut class_buffer = [0u16; 256];
    let class_length = unsafe { GetClassNameW(hwnd, &mut class_buffer) };
    let window_class = String::from_utf16_lossy(&class_buffer[..class_length as usize]);

    Some(AppNotificationEvent {
        hwnd,
        process_id,
        process_name,
        window_title,
        window_class,
    })
}
