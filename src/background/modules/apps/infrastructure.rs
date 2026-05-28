use libs_core::system_state::FocusedApp;
use windows::Win32::UI::WindowsAndMessaging::SW_MINIMIZE;

use crate::modules::input::Mouse;
use crate::{
    error::ResultLogExt, modules::apps::application::USER_APPS_MANAGER, windows_api::window::Window,
};

pub fn register_app_win_events() {
    // App windows events are tracked internally by UserAppsManager
    // No UI notifications needed for dock/virtual desktop functionality
}

#[tauri::command(async)]
pub fn get_focused_app() -> FocusedApp {
    Window::get_foregrounded().as_focused_app_information()
}

#[tauri::command(async)]
pub fn get_mouse_position() -> [i32; 2] {
    let point = Mouse::get_cursor_pos().unwrap_or_default();
    [point.x(), point.y()]
}

#[tauri::command(async)]
pub fn show_desktop() {
    USER_APPS_MANAGER.interactable_windows.for_each(|data| {
        let win = Window::from(data.hwnd);
        if !win.is_minimized() {
            win.show_window_async(SW_MINIMIZE).log_error();
        }
    });
}
