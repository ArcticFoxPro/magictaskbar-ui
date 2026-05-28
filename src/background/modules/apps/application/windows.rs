use libs_core::system_state::UserAppWindow;
use log;
use std::time::Duration;
use windows::Win32::UI::WindowsAndMessaging::{
    WS_CHILD, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_MINIMIZEBOX,
};

use crate::{
    hook::HookManager,
    modules::apps::application::{UserAppsEvent, UserAppsManager, USER_APPS_MANAGER},
    windows_api::{
        window::{event::WinEvent, Window},
        WindowEnumerator, WindowsApi,
    },
};

/// Helper function to handle UAC window restoration and focusing
fn handle_uac_window(window: &Window, context: &str) {
    log::debug!(
        "[UAC {}] Found UAC window HWND: {:?}",
        context,
        window.address()
    );
    // Try to focus the window
    let _ = window.focus();
}

impl UserAppsManager {
    pub fn reconcile_closed_window(window: &Window, reason: &str) -> bool {
        if !USER_APPS_MANAGER.contains_win(window) {
            return false;
        }

        let process = window.process();
        let process_exists =
            process.open_limited_handle().is_ok() || process.exe_name_by_snapshot().is_ok();
        let still_interactable = process_exists && is_interactable_and_not_hidden(window);

        if still_interactable {
            log::trace!(
                "[UserAppsManager][CloseReconcile] window still interactable reason={} hwnd={:?}, title='{}', class='{}'",
                reason,
                window.address(),
                window.title(),
                window.class()
            );
            return false;
        }

        log::info!(
            "[UserAppsManager][CloseReconcile] removing stale window reason={} hwnd={:?}, title='{}', class='{}', process_exists={}, is_window={}, is_visible={}",
            reason,
            window.address(),
            window.title(),
            window.class(),
            process_exists,
            window.is_window(),
            window.is_visible()
        );
        USER_APPS_MANAGER.remove_win(window);
        Self::send(UserAppsEvent::WinRemoved(window.address()));
        true
    }

    pub fn reconcile_observed_window(window: &Window, reason: &str) {
        if USER_APPS_MANAGER.contains_win(window) {
            return;
        }

        if window.class() == "Tauri Window" {
            return;
        }

        if !is_interactable_and_not_hidden(window) {
            log::trace!(
                "[UserAppsManager][Reconcile] observed window not interactable reason={} hwnd={:?}, title='{}', class='{}'",
                reason,
                window.address(),
                window.title(),
                window.class()
            );
            return;
        }

        if USER_APPS_MANAGER.add_win_if_missing(window) {
            log::info!(
                "[UserAppsManager][Reconcile] added missing window reason={} hwnd={:?}, title='{}', class='{}'",
                reason,
                window.address(),
                window.title(),
                window.class()
            );
            Self::send(UserAppsEvent::WinAdded(window.address()));
        }
    }

    pub(super) fn init_listing_app_windows() -> Vec<UserAppWindow> {
        // Start with empty vector and perform progressive enumeration in background
        let initial = Vec::new();

        // Subscribe to window events first
        HookManager::subscribe(|(event, window)| {
            // Special handling for UAC windows
            if let Some(process_name) = get_process_name_with_fallback(&window) {
                if process_name == "consent.exe" {
                    handle_uac_window(&window, "Event");
                }
            }

            Self::on_win_event(event, window)
        });

        // Perform progressive window enumeration in background thread
        std::thread::spawn(|| {
            log::info!("[Progressive Loading] Starting progressive window enumeration");
            let _ = WindowEnumerator::new().for_each(|window| {
                // Special handling for existing UAC windows during initial enumeration
                if let Some(process_name) = get_process_name_with_fallback(&window) {
                    if process_name == "consent.exe" {
                        handle_uac_window(&window, "Initial Enumeration");
                    }
                }

                if is_interactable_and_not_hidden(&window) {
                    // Add window and send event immediately for progressive loading
                    USER_APPS_MANAGER.add_win(&window);
                    Self::send(UserAppsEvent::WinAdded(window.address()));
                }
            });
            log::info!("[Progressive Loading] Progressive window enumeration completed");
        });

        initial
    }

    fn on_win_event(event: WinEvent, window: Window) {
        let mut is_interactable = USER_APPS_MANAGER.contains_win(&window);

        match event {
            WinEvent::ObjectCreate | WinEvent::ObjectShow => {
                if !is_interactable && is_interactable_and_not_hidden(&window) {
                    USER_APPS_MANAGER.add_win(&window);
                    Self::send(UserAppsEvent::WinAdded(window.address()));
                }
            }
            WinEvent::ObjectFocus | WinEvent::SystemForeground => {
                // 对于 UWP 窗口，焦点事件可能在标题设置后触发，重新检查
                if !is_interactable && is_interactable_and_not_hidden(&window) {
                    USER_APPS_MANAGER.add_win(&window);
                    Self::send(UserAppsEvent::WinAdded(window.address()));
                }
            }
            WinEvent::ObjectNameChange => {
                let was_interactable = is_interactable;
                is_interactable = is_interactable_and_not_hidden(&window);
                match (was_interactable, is_interactable) {
                    (false, true) => {
                        USER_APPS_MANAGER.add_win(&window);
                        Self::send(UserAppsEvent::WinAdded(window.address()));
                    }
                    (true, false) => {
                        USER_APPS_MANAGER.remove_win(&window);
                        Self::send(UserAppsEvent::WinRemoved(window.address()));
                    }
                    _ => {}
                }
            }
            WinEvent::ObjectParentChange => {
                // re-check for UWP apps that on creation starts without a parent
                if let Some(parent) = window.parent() {
                    if !USER_APPS_MANAGER.contains_win(&parent)
                        && parent.is_interactable_and_not_hidden()
                    {
                        USER_APPS_MANAGER.add_win(&parent);
                        Self::send(UserAppsEvent::WinAdded(parent.address()));
                    }
                }
            }
            WinEvent::ObjectHide => {
                // UWP ApplicationFrameHosts are always hidden on minimize
                if is_interactable && !window.is_frame().unwrap_or(false) {
                    USER_APPS_MANAGER.remove_win(&window);
                    Self::send(UserAppsEvent::WinRemoved(window.address()));
                }
            }
            WinEvent::SystemMinimizeStart => {
                if window.title() == "抖音"
                    && !is_interactable_and_not_hidden(&window)
                    && is_interactable
                {
                    log::debug!("[Window Filter] douyin.exe SystemMinimizeEnd - filtering out");
                    USER_APPS_MANAGER.remove_win(&window);
                    Self::send(UserAppsEvent::WinRemoved(window.address()));
                }
            }
            WinEvent::ObjectDestroy => {
                if is_interactable {
                    USER_APPS_MANAGER.remove_win(&window);
                    Self::send(UserAppsEvent::WinRemoved(window.address()));
                }
            }
            _ => {}
        }

        // update cases on UserAppWindow
        if is_interactable
            && matches!(
                event,
                WinEvent::ObjectNameChange
                    | WinEvent::SystemMinimizeStart
                    | WinEvent::SystemMinimizeEnd
                    | WinEvent::SyntheticFullscreenStart
                    | WinEvent::SyntheticFullscreenEnd
                    | WinEvent::SyntheticMonitorChanged
            )
        {
            USER_APPS_MANAGER.interactable_windows.for_each(|w| {
                if w.hwnd == window.address() {
                    *w = window.to_serializable();
                }
            });
            Self::send(UserAppsEvent::WinUpdated(window.address()));
        }
    }
}

/// The idea with this module is contain all the logic under the filteriong of windows
/// that can be considered as applications windows, it means windows that are interactable
/// for the users.
///
/// As windows properties can change, this should be reevaluated on every change.

/// 获取窗口进程名的辅助函数，带 IPC fallback
///
/// 首先尝试直接获取进程名，如果失败（通常是权限不足），
/// 则通过 IPC 向具有高权限的 service 进程请求。
/// 这对于访问 UAC 窗口（如 consent.exe）等系统敏感进程非常有用。
pub fn get_process_name_with_fallback(window: &Window) -> Option<String> {
    // 先尝试直接获取
    if let Ok(name) = window.process().program_exe_name() {
        return Some(name);
    }

    // 如果失败，通过 IPC 获取（service 进程有 SE_DEBUG_NAME 权限）
    use crate::cli::ServicePipe;
    use slu_ipc::messages::SvcAction;

    let result = ServicePipe::request_with_response_blocking(
        SvcAction::GetProcessName {
            hwnd: window.address(),
        },
        Duration::from_millis(800),
    );

    if let Ok(Some(name)) = result {
        return Some(name);
    }

    // 兜底逻辑：如果 IPC 也拿不到进程名，无条件使用窗口标题，确保高权限进程不被识别为 unknown
    let title = window.title();
    if !title.is_empty() {
        Some(title)
    } else {
        Some("Unknown App".to_string())
    }
}

fn should_filter_by_title_and_process(_window: &Window, process_name: &str, title: &str) -> bool {
    // 过滤自家任务栏/工具栏窗口
    if title == "HonorTaskbar" || title == "HonorToolbar" {
        return true;
    }

    // 过滤回收站窗口（回收站不应该作为应用显示在任务栏，它有专门的图标）
    if title.starts_with("回收站")
        || title.starts_with("Recycle Bin")
        || title.starts_with("资源回收筒")
    {
        log::info!("[Window Filter] 过滤回收站窗口: {}", title);
        return true;
    }

    // 过滤特定有问题的进程
    if process_name == "douyin_widget.exe" {
        return true;
    }

    // 过滤迅雷的悬浮球窗口
    if process_name == "Thunder.exe" && title == "悬浮球" {
        return true;
    }

    // 过滤优酷播放器子窗口（ykplayer），Windows原生任务栏也不显示这些窗口
    let title_lower = title.to_lowercase();
    if title_lower == "ykplayer" || title_lower.starts_with("ykplayer ") {
        return true;
    }

    if process_name == "Hihonornote.exe" && title == "HnCollectCenter" {
        return true;
    }

    if process_name == "PowerToys.MonacoPreviewHandler.exe" && title == "" {
        return true;
    }

    false
}

pub fn is_interactable_and_not_hidden(window: &Window) -> bool {
    if !window.is_window() || !window.is_visible() {
        return false;
    }

    // Get window properties once to avoid repeated API calls
    let class = window.class();
    let process = window.process();
    let title = window.title();

    // 获取进程名，如果直接获取失败则通过 IPC fallback
    let process_name =
        get_process_name_with_fallback(window).unwrap_or_else(|| "unknown".to_string());

    // this class is used for edge tabs to be shown as independent windows on alt + tab
    // this only applies when the new tab is created it is binded to explorer.exe for some reason
    // maybe we can search/learn more about edge tabs later.
    if class == "Windows.Internal.Shell.TabProxyWindow" {
        return false;
    }

    if class == "Ghost" && process_name == "dwm.exe" {
        return false;
    }

    if should_filter_by_title_and_process(window, &process_name, &title) {
        return false;
    }

    // Filter out frozen processes
    if process.is_frozen().unwrap_or(false) {
        return false;
    }

    let style = WindowsApi::get_styles(window.hwnd());
    let ex_style = WindowsApi::get_ex_styles(window.hwnd());

    // Handle special window classes with custom logic
    let is_special_window = class == "ApplicationFrameWindow"
        || class == "Windows.UI.Core.CoreWindow"
        || class == "StartMenuSizingFrame"
        || class == "Shell_LightDismissOverlay";

    if is_immersive_shell_window(window) {
        return false;
    }

    if is_special_window {
        // Process special logic for UWP windows
        if process_uwp_window(window, &class) {
            return false;
        }
    } else {
        // For non-special windows, check cloaked normally
        if window.is_cloaked() {
            return false;
        }
    }

    // Window style checks
    if !ex_style.contains(WS_EX_APPWINDOW) {
        // It must not be owned by another window
        if style.contains(WS_CHILD) || window.owner().is_some() {
            return false;
        }

        // Discard tool windows without WS_EX_APPWINDOW
        if ex_style.contains(WS_EX_TOOLWINDOW) || ex_style.contains(WS_EX_NOACTIVATE) {
            return false;
        }
    }

    let process = window.process();
    // unmanageable window, these probably are system processes
    match process.open_limited_handle() {
        Ok(handle) => {
            let _wrapper = crate::windows_api::HandleWrapper::new(handle);
        }
        Err(_e) => {
            // Protected process (Access Denied).
            // 1. If it's Cloaked (hidden by system), it's definitely not a game/app we want to show.
            if window.is_cloaked() {
                return false;
            }

            // 2. Strict style check for high-privilege windows:
            // A valid Taskbar App MUST:
            // - Have a Title (System UIs like InputHost often have empty/internal titles)
            // - Have a Minimize Box (WS_MINIMIZEBOX) OR be an App Window (WS_EX_APPWINDOW)
            // - NOT be a Tool Window (WS_EX_TOOLWINDOW)
            // - NOT be NoActivate (WS_EX_NOACTIVATE)
            if title.is_empty()
                || (!style.contains(WS_MINIMIZEBOX) && !ex_style.contains(WS_EX_APPWINDOW))
                || ex_style.contains(WS_EX_TOOLWINDOW)
                || ex_style.contains(WS_EX_NOACTIVATE)
            {
                log::debug!("[Window Filter] Skip high-privilege system UI (style mismatch) - process: {}, class: {}", process_name, class);
                return false;
            }

            log::debug!("[Window Filter] Allowing high-privilege application window - process: {}, title: '{}'", process_name, title);
        }
    }

    // Internal behaviour for seelen ui widgets:
    // Discard unminimizable windows (they have no caption/title bar)
    if !style.contains(WS_MINIMIZEBOX) && process.is_taskbar() {
        return false;
    }

    // Check for empty title windows with special filtering
    if title.is_empty() {
        // Filter out specific problematic empty-title windows
        // explorer.exe + ApplicationFrameWindow is typically a hidden framework window
        // Also filter out ApplicationFrameHost.exe + ApplicationFrameWindow as they are usually pre-loaded frames
        if (process_name == "explorer.exe" && class == "ApplicationFrameWindow")
            || (process_name == "ApplicationFrameHost.exe" && class == "ApplicationFrameWindow")
            || (process_name == "csrss.exe" && class == "#32769")
        {
            log::debug!(
                "[Window Filter] Empty title {} ApplicationFrameWindow - filtering out",
                process_name
            );
            return false;
        }

        // Log other empty-title windows for debugging but allow them through
        log::debug!(
            "[Window Filter] Empty title window - process: {}, class: {} (allowed)",
            process_name,
            class
        );
    }

    if process_name == "douyin.exe"
        && WindowsApi::get_prop(window.hwnd(), "ITaskList_Deleted").is_some()
    {
        log::debug!("[Window Filter] douyin.exe with ITaskList_Deleted - filtering out");
        return false;
    }
    // Print the current filtered process
    log::info!(
        "[Filtered Process] Process: {}, Window Title: '{}', Class: {}, HWND: {:?}",
        process_name,
        title,
        class,
        window.address()
    );

    true
}

/// Helper function to process UWP window specific logic
fn process_uwp_window(window: &Window, class: &str) -> bool {
    // Skip position/size checks for minimized windows as they may legitimately be off-screen
    if !window.is_minimized() {
        if let Ok(rect) = window.outer_rect() {
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;

            // Check if window is hidden off-screen or has zero size
            let is_offscreen = rect.left < -10000
                || rect.top < -10000
                || rect.right < -10000
                || rect.bottom < -10000;
            let is_zero_size = width <= 0 || height <= 0;

            if is_offscreen || is_zero_size {
                return true;
            }
        }
    }

    // Additional validation for cloaked UWP windows
    if window.is_cloaked() && !window.is_minimized() {
        if let Ok(rect) = window.outer_rect() {
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;

            // Check if window has zero size
            let is_zero_size = width <= 0 || height <= 0;
            if is_zero_size {
                return true;
            }

            // Check if window is positioned in a way that suggests it's not meant to be visible
            let is_suspicious_position = rect.left <= -30000 && rect.top <= -30000;
            if is_suspicious_position {
                return true;
            }
        }
    }

    // Additional check for UWP windows: verify they are actually active/visible to user
    if class == "ApplicationFrameWindow" && !window.is_focused() && !window.is_minimized() {
        // Check if this is likely a pre-loaded window by verifying it's not the foreground window
        // and doesn't have focus
        if WindowsApi::get_foreground_window() != window.hwnd() && window.is_cloaked() {
            return true;
        }
    }

    false
}

fn is_immersive_shell_window(window: &Window) -> bool {
    let class = window.class();
    let ex_style = WindowsApi::get_ex_styles(window.hwnd());
    let has_accept_files = (ex_style.0 & 0x100) != 0;

    // Check for common Immersive Shell window classes
    if class == "ApplicationFrameWindow"
        || class == "Windows.UI.Core.CoreWindow"
        || class == "StartMenuSizingFrame"
        || class == "Shell_LightDismissOverlay"
    {
        if !has_accept_files {
            return true;
        }
    }

    if class == "ImmersiveBackgroundWindow"
        || class == "SearchPane"
        || class == "NativeHWNDHost"
        || class == "Shell_CharmWindow"
        || class == "ImmersiveLauncher"
    {
        if let Ok(exe) = window.process().program_path() {
            if let Some(exe_str) = exe.to_str() {
                if exe_str.to_lowercase().contains("explorer.exe") {
                    return true;
                }
            }
        }
    }

    false
}
