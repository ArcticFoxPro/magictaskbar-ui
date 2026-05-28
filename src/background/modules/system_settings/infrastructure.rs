use libs_core::{handlers::FuncEvent, system_state::UIColors};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;
use tauri::Manager;
use windows::Win32::Foundation::HWND;

use crate::{
    app::get_app_handle,
    cli::ServicePipe,
    error::Result,
    log_error,
    modules::system_settings::application::{SystemSettings, SystemSettingsEvent},
    trace_lock,
    windows_api::WindowsApi,
};

use super::application::SYSTEM_SETTINGS;

/// 最新的主题状态：true=深色, false=浅色
/// 由 UISettings ColorValuesChanged 事件更新，初始値由注册表读取
pub static CURRENT_IS_DARK: AtomicBool = AtomicBool::new(false);

fn emit_colors(colors: &UIColors) {
    get_app_handle()
        .emit(FuncEvent::ColorsChanged, colors)
        .expect("failed to emit");
}

/// 切换主题后更新所有 preview/contextmenu 窗口的亚克力颜色，并通知前端
fn update_acrylic_for_theme(is_dark: bool) {
    let app = get_app_handle();
    let gradient_color = if is_dark {
        0xA8303030u32
    } else {
        0xA8FDFBFAu32
    };
    // 注意：preview/contextmenu 的 label 是 base64 编码的，
    // @magic/preview    -> QG1hZ2ljL3ByZXZpZXc
    // @magic/contextmenu -> QG1hZ2ljL2NvbnRleHRtZW51
    for (label, window) in app.webview_windows() {
        if label.starts_with("QG1hZ2ljL3ByZXZpZXc") || label.starts_with("QG1hZ2ljL2NvbnRleHRtZW51")
        {
            if let Ok(raw) = window.hwnd() {
                let hwnd = HWND(raw.0);
                let _ = WindowsApi::apply_acrylic_effect(hwnd, Some(gradient_color));
                log::info!(
                    "[SystemSettings] Applied acrylic is_dark={} to window: {}",
                    is_dark,
                    label
                );
            }
        }
    }
    // 通知前端主题已切换
    let _ = app.emit("theme::changed", serde_json::json!({ "is_dark": is_dark }));
}

/// 获取当前系统是否为深色模式
/// 使用 AtomicBool 全局状态（由 UISettings 事件更新），比注册表更可靠
pub fn current_is_dark_mode() -> bool {
    CURRENT_IS_DARK.load(Ordering::SeqCst)
}

/// 初始化全局主题状态（从 UISettings 读取初始値）
fn init_theme_state() {
    if let Ok(colors) = trace_lock!(SYSTEM_SETTINGS).get_colors() {
        let is_dark = colors
            .background
            .get(1..3)
            .and_then(|s| u8::from_str_radix(s, 16).ok())
            .map(|r| r < 128)
            .unwrap_or(false);
        CURRENT_IS_DARK.store(is_dark, Ordering::SeqCst);
    }
}

/// 获取当前系统是否为深色模式（供前端查询）
#[tauri::command(async)]
pub fn get_is_dark_mode() -> bool {
    current_is_dark_mode()
}

pub fn register_system_settings_events() {
    std::thread::spawn(move || {
        log_error!(trace_lock!(SYSTEM_SETTINGS).initialize());
        // 初始化主题状态
        init_theme_state();
        SystemSettings::subscribe(|event| {
            if event == SystemSettingsEvent::ColorChanged {
                // CURRENT_IS_DARK 已在 ColorValuesChanged 回调里更新，这里直接用
                let is_dark = CURRENT_IS_DARK.load(Ordering::SeqCst);
                if let Ok(colors) = trace_lock!(SYSTEM_SETTINGS).get_colors() {
                    emit_colors(&colors);
                }
                update_acrylic_for_theme(is_dark);
            }
        });
    });
}

pub fn release_colors_events() {
    log_error!(trace_lock!(SYSTEM_SETTINGS).release());
}

#[tauri::command(async)]
pub fn get_system_colors() -> Result<UIColors> {
    trace_lock!(SYSTEM_SETTINGS).get_colors()
}

#[tauri::command(async)]
pub async fn system_open_power_settings() -> Result<()> {
    ServicePipe::request_with_response(slu_ipc::messages::SvcAction::ExecuteBackendCommand {
        command: "system_open_power_settings".to_string(),
        args: serde_json::json!({}),
    })
    .await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn system_open_language_settings() -> Result<()> {
    ServicePipe::request_with_response(slu_ipc::messages::SvcAction::ExecuteBackendCommand {
        command: "system_open_language_settings".to_string(),
        args: serde_json::json!({}),
    })
    .await?;
    Ok(())
}
