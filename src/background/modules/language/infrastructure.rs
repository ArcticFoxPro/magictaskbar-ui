use serde::Deserialize;

use crate::error::Result;

use super::tsf;

pub fn register_language_events() {
    // 请求服务进程（管理员权限）启动 TSF 钩子
    // 这样可以确保在 release 版本中也能正确监听输入法切换事件
    use crate::cli::ServicePipe;
    use crate::windows_api::event_window::BACKGROUND_HWND;
    use slu_ipc::messages::SvcAction;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    let hwnd = BACKGROUND_HWND.load(Ordering::Relaxed);
    log::info!("[TSF] register_language_events: hwnd=0x{:X}", hwnd);

    // 先检查 srv 是否已就绪
    if ServicePipe::is_running() {
        // srv 已就绪，直接请求
        let _ = ServicePipe::request(SvcAction::StartTsfWatcher { hwnd });
        log::info!("[TSF] TSF watcher request sent (srv already running)");
    } else {
        // srv 还未就绪，启动后台重试线程
        log::warn!("[TSF] Service not running yet, starting retry thread...");
        std::thread::spawn(move || {
            for attempt in 1..=5 {
                std::thread::sleep(Duration::from_millis(500));

                let hwnd = BACKGROUND_HWND.load(Ordering::Relaxed);

                if ServicePipe::is_running() {
                    log::info!(
                        "[TSF] Service is now available (attempt {}), sending TSF watcher request",
                        attempt
                    );
                    let _ = ServicePipe::request(SvcAction::StartTsfWatcher { hwnd });
                    // 等待一小会儿让 srv 处理请求
                    std::thread::sleep(Duration::from_millis(200));

                    // 验证一下 srv 是否真的收到了
                    // 通过打印日志确认（srv 端会打印 updated UI HWND）
                    log::info!(
                        "[TSF] TSF watcher init request sent, waiting for srv confirmation..."
                    );
                    break;
                } else {
                    log::trace!("[TSF] Service not yet available (attempt {}/20)", attempt);
                }
            }
        });
    }
}

#[tauri::command(async)]
pub fn get_system_languages() -> Result<Vec<tsf::TsfProfile>> {
    // Map to TSF profiles for the new scheme
    tsf::get_installed_input_profiles()
}

#[tauri::command(async)]
pub fn set_system_keyboard_layout(id: String, _handle: String) -> Result<()> {
    // Bridge to TSF activation. 'id' is used as guid_profile if applicable.
    tsf::activate_input_profile(id).map(|_| ())
}

// === TSF exposed helpers (wrappers) ===

#[tauri::command(async)]
pub fn get_active_input_profile() -> Result<Option<tsf::TsfActiveProfile>> {
    tsf::get_active_input_profile()
}

#[tauri::command(async)]
pub fn get_installed_input_profiles() -> Result<Vec<tsf::TsfProfile>> {
    tsf::get_installed_input_profiles()
}

#[tauri::command(async)]
pub fn get_installed_keyboard_layouts() -> Result<Vec<tsf::KeyboardLayoutProfile>> {
    tsf::get_installed_keyboard_layouts()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivateKeyboardLayoutArgs {
    klid: String,
}

#[tauri::command(async)]
pub fn activate_keyboard_layout(
    args: ActivateKeyboardLayoutArgs,
) -> Result<Option<tsf::KeyboardLayoutProfile>> {
    tsf::activate_keyboard_layout(args.klid)
}

#[tauri::command(async)]
pub fn get_last_active_input_profile_cached() -> Option<tsf::TsfActiveProfile> {
    tsf::get_last_active_input_profile_cached()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivateInputArgs {
    #[serde(alias = "guid_profile")]
    guid_profile: String,
}

#[tauri::command(async)]
pub fn activate_input_profile(args: ActivateInputArgs) -> Result<Option<tsf::TsfActiveProfile>> {
    tsf::activate_input_profile(args.guid_profile)
}

#[tauri::command(async)]
pub fn activate_keyboard_layout_via_tsf(id: String, _handle: String) -> Result<()> {
    tsf::activate_input_profile(id).map(|_| ())
}

#[tauri::command(async)]
pub fn activate_input_profile_by_name(name: String) -> Result<()> {
    tsf::activate_input_profile_by_name(name)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSimpleModeArgs {
    pub enabled: bool,
}

#[tauri::command(async)]
pub fn set_tsf_simple_mode(args: SetSimpleModeArgs) -> Result<()> {
    tsf::set_simple_mode(args.enabled);
    Ok(())
}

// Stubs for IME mode (chi/eng) to avoid frontend breakage if it still calls these,
// but returning empty/default to keep it "simple" and "not complex".
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImeModeInfo {
    pub mode: String,
    pub open: bool,
    pub conversion: u32,
}

#[tauri::command(async)]
pub fn get_ime_mode() -> Result<ImeModeInfo> {
    Ok(ImeModeInfo {
        mode: "eng".into(),
        open: false,
        conversion: 0,
    })
}

#[tauri::command(async)]
pub fn toggle_ime_mode() -> Result<()> {
    // Simple toggle logic can be added later if needed.
    // For now, we follow the "simplify" mandate.
    Ok(())
}
