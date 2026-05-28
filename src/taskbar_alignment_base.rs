use slu_ipc::messages::TaskbarAlignment;
use winreg::{
    enums::{HKEY_CURRENT_USER, KEY_READ},
    RegKey,
};

pub const TASKBAR_ALIGNMENT_PATH: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";
pub const TASKBAR_ALIGNMENT_VALUE: &str = "TaskbarAl";

pub fn read_alignment_from_registry() -> Result<Option<TaskbarAlignment>, std::io::Error> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey_with_flags(TASKBAR_ALIGNMENT_PATH, KEY_READ)?;

    match key.get_value::<u32, _>(TASKBAR_ALIGNMENT_VALUE) {
        Ok(value) => Ok(Some(TaskbarAlignment::from(value))),
        Err(_) => Ok(None), // 未找到值
    }
}

pub fn refresh_taskbar() {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_SETTINGCHANGE};

    let taskbar_class: Vec<u16> = "Shell_TrayWnd\0".encode_utf16().collect();

    unsafe {
        if let Ok(hwnd) = FindWindowW(PCWSTR(taskbar_class.as_ptr()), PCWSTR::null()) {
            if !hwnd.is_invalid() {
                for _ in 0..3 {
                    let _ = PostMessageW(Some(hwnd), WM_SETTINGCHANGE, WPARAM(0), LPARAM(0));
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(100));
}
