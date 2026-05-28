mod app_bar;
mod com;
pub mod device;
pub mod event_window;
pub mod hdc;
pub mod icon_extractor;
mod iterator;
pub mod monitor;
pub mod native_window;
pub mod process;
pub mod string_utils;
pub mod traits;
pub mod types;
pub mod window;

pub use app_bar::*;
pub use com::*;
pub use iterator::*;
use itertools::Itertools;
pub use native_window::NativeAppBarWindow;
use process::ProcessInformationFlag;
use string_utils::WindowsString;
use windows_core::Interface;

use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    thread::sleep,
    time::Duration,
};

use windows::{
    core::{BSTR, GUID, PCWSTR},
    ApplicationModel::AppInfo,
    Wdk::System::{
        SystemServices::PROCESS_EXTENDED_BASIC_INFORMATION,
        Threading::{NtQueryInformationProcess, ProcessBasicInformation},
    },
    Win32::{
        Foundation::{CloseHandle, HANDLE, HMODULE, HWND, LPARAM, RECT, STATUS_SUCCESS, WPARAM},
        Graphics::{
            Dwm::{
                DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_CLOAKED,
                DWMWA_EXTENDED_FRAME_BOUNDS, DWMWA_VISIBLE_FRAME_BORDER_THICKNESS,
                DWMWINDOWATTRIBUTE, DWM_CLOAKED_APP, DWM_CLOAKED_INHERITED, DWM_CLOAKED_SHELL,
                DWM_WINDOW_CORNER_PREFERENCE,
            },
            Gdi::{
                EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow,
                HMONITOR, MONITORENUMPROC, MONITORINFOEXW, MONITOR_DEFAULTTOPRIMARY,
            },
        },
        Security::{
            GetTokenInformation, TokenElevation, TokenLogonSid, TOKEN_ADJUST_PRIVILEGES,
            TOKEN_ELEVATION, TOKEN_GROUPS, TOKEN_QUERY,
        },
        Storage::{
            EnhancedStorage::{
                PKEY_AppUserModel_ID, PKEY_AppUserModel_PreventPinning,
                PKEY_AppUserModel_RelaunchCommand, PKEY_AppUserModel_RelaunchDisplayNameResource,
                PKEY_AppUserModel_RelaunchIconResource, PKEY_AppUserModel_ToastActivatorCLSID,
                PKEY_FileDescription,
            },
            FileSystem::WIN32_FIND_DATAW,
        },
        System::{
            Com::{IPersistFile, STGM_READ},
            Environment::ExpandEnvironmentStringsW,
            LibraryLoader::GetModuleHandleW,
            Threading::{
                AttachThreadInput, GetCurrentProcess, GetCurrentThreadId, OpenProcess,
                OpenProcessToken, QueryFullProcessImageNameW, PROCESS_ACCESS_RIGHTS,
                PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            Shell::{
                IShellItem2, IShellLinkW, IVirtualDesktopManager,
                PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow, GPS_DEFAULT},
                SHCreateItemFromParsingName, SHGetKnownFolderPath, SHLoadIndirectString, ShellLink,
                VirtualDesktopManager, KF_FLAG_DEFAULT, SIGDN_NORMALDISPLAY,
            },
            WindowsAndMessaging::{
                BringWindowToTop, CallWindowProcW, DefWindowProcW, FindWindowExW, FindWindowW,
                GetClassNameW, GetDesktopWindow, GetForegroundWindow, GetParent, GetPropW,
                GetWindow, GetWindowLongW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
                IsIconic, IsWindow, IsWindowVisible, IsZoomed, PostMessageW, SetForegroundWindow,
                SetWindowLongPtrW, SetWindowPos, ShowWindow, ShowWindowAsync, GWLP_WNDPROC,
                GWL_EXSTYLE, GWL_STYLE, GW_OWNER, SC_CLOSE, SET_WINDOW_POS_FLAGS, SHOW_WINDOW_CMD,
                SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, WINDOW_EX_STYLE,
                WINDOW_STYLE, WM_CLOSE, WM_SYSCOMMAND, WNDPROC, WS_SIZEBOX, WS_THICKFRAME,
            },
        },
    },
    UI::ViewManagement::UISettings,
};

use crate::{
    error::{Result, WindowsResultExt},
    hook::HookManager,
    modules::input::{domain::Point, Keyboard, Mouse},
    windows_api::window::{event::WinEvent, Window},
};

/// A wrapper for Windows HANDLE that automatically closes the handle when dropped
pub struct HandleWrapper(HANDLE);

impl HandleWrapper {
    pub fn new(handle: HANDLE) -> Option<Self> {
        if handle.is_invalid() {
            None
        } else {
            Some(HandleWrapper(handle))
        }
    }

    pub fn handle(&self) -> HANDLE {
        self.0
    }
}

impl Drop for HandleWrapper {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[macro_export]
macro_rules! pcstr {
    ($s:literal) => {
        windows::core::s!($s)
    };
}

#[macro_export]
macro_rules! pcwstr {
    ($s:literal) => {
        windows::core::w!($s)
    };
}

#[macro_export]
macro_rules! hstring {
    ($s:literal) => {
        windows::core::h!($s)
    };
}

pub struct WindowsApi {}
impl WindowsApi {
    /// Adds a WNDPROC filter for a window. The filter may intercept messages and return a result.
    ///
    /// Internally this installs a single subclass WNDPROC per HWND and chains multiple filters.
    pub fn add_wndproc_filter<F>(hwnd: HWND, key: u64, filter: F) -> Result<()>
    where
        F: Fn(HWND, u32, WPARAM, LPARAM) -> Option<isize> + Send + Sync + 'static,
    {
        use parking_lot::Mutex as ParkingMutex;
        use std::collections::HashMap as StdHashMap;
        use std::sync::LazyLock;

        #[derive(Clone, Copy)]
        struct StoredFilter {
            key: u64,
        }

        type FilterFn =
            Box<dyn Fn(HWND, u32, WPARAM, LPARAM) -> Option<isize> + Send + Sync + 'static>;

        // Store original WNDPROC per window
        static ORIGINAL_WNDPROC: LazyLock<ParkingMutex<StdHashMap<isize, isize>>> =
            LazyLock::new(|| ParkingMutex::new(StdHashMap::new()));
        // Store filter chain per window
        static FILTERS: LazyLock<ParkingMutex<StdHashMap<isize, Vec<(StoredFilter, FilterFn)>>>> =
            LazyLock::new(|| ParkingMutex::new(StdHashMap::new()));

        unsafe extern "system" fn chain_proc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> isize {
            // Run filters first
            let maybe_result = {
                let map = FILTERS.lock();
                map.get(&(hwnd.0 as isize)).and_then(|filters| {
                    for (_meta, cb) in filters.iter() {
                        if let Some(res) = cb(hwnd, msg, wparam, lparam) {
                            return Some(res);
                        }
                    }
                    None
                })
            };
            if let Some(res) = maybe_result {
                return res;
            }

            // forward to original proc
            let orig = {
                let map = ORIGINAL_WNDPROC.lock();
                map.get(&(hwnd.0 as isize)).copied()
            };
            if let Some(op) = orig {
                // SAFETY: op was returned by SetWindowLongPtrW as the previous WNDPROC
                let prev_proc: WNDPROC = std::mem::transmute(op);
                unsafe { CallWindowProcW(prev_proc, hwnd, msg, wparam, lparam).0 }
            } else {
                // Fallback to default window proc if original is unknown
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam).0 }
            }
        }

        unsafe {
            // Ensure subclass installed once
            let hwnd_key = hwnd.0 as isize;
            let already_subclassed = ORIGINAL_WNDPROC.lock().contains_key(&hwnd_key);
            if !already_subclassed {
                let prev = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, chain_proc as *const () as isize);
                ORIGINAL_WNDPROC.lock().insert(hwnd_key, prev);
            }

            // Add filter if missing
            let mut map = FILTERS.lock();
            let entry = map.entry(hwnd_key).or_default();
            if entry.iter().any(|(meta, _)| meta.key == key) {
                return Ok(());
            }
            entry.push((StoredFilter { key }, Box::new(filter)));
        }

        Ok(())
    }

    /// Install a window procedure hook on the given HWND that ignores WM_CLOSE and SC_CLOSE.
    /// This prevents close gestures (e.g., touchpad) from closing toolbar/taskbar windows.
    pub fn ignore_close(hwnd: HWND) -> Result<()> {
        const FILTER_KEY_IGNORE_CLOSE: u64 = 0x4D_5442_49_47_4E_4F_52; // "MTBIGNOR" (arbitrary)
        Self::add_wndproc_filter(hwnd, FILTER_KEY_IGNORE_CLOSE, |_, msg, wparam, _| {
            if msg == WM_CLOSE || (msg == WM_SYSCOMMAND && (wparam.0 as u32 & 0xFFF0) == SC_CLOSE) {
                return Some(0);
            }
            None
        })
    }
    pub fn module_handle_w() -> Result<HMODULE> {
        Ok(unsafe { GetModuleHandleW(None) }?)
    }

    pub fn enum_display_monitors(
        callback: MONITORENUMPROC,
        callback_data_address: isize,
    ) -> Result<()> {
        unsafe {
            EnumDisplayMonitors(None, None, callback, LPARAM(callback_data_address))
                .ok()
                .filter_fake_error()?;
        }
        Ok(())
    }

    pub fn post_message(hwnd: HWND, message: u32, wparam: usize, lparam: isize) -> Result<()> {
        unsafe { PostMessageW(Some(hwnd), message, WPARAM(wparam), LPARAM(lparam))? };
        Ok(())
    }

    pub fn get_monitor_scale_factor(hmonitor: HMONITOR) -> Result<f64> {
        let mut dpi_x: u32 = 0;
        let mut _dpi_y: u32 = 0;
        unsafe { GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut _dpi_y)? };
        // 96 is the default DPI value on Windows
        Ok(dpi_x as f64 / 96_f64)
    }

    pub fn get_text_scale_factor() -> Result<f64> {
        Ok(UISettings::new()?.TextScaleFactor()?)
    }

    /// Behaviour is undefined if an invalid HWND is given
    /// https://docs.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowthreadprocessid
    pub fn window_thread_process_id(hwnd: HWND) -> (u32, u32) {
        let mut process_id: u32 = 0;

        let thread_id = unsafe {
            GetWindowThreadProcessId(hwnd, Option::from(std::ptr::addr_of_mut!(process_id)))
        };

        (process_id, thread_id)
    }

    pub fn find_window(
        parent: Option<HWND>,
        after: Option<HWND>,
        title: Option<String>,
        class: Option<String>,
    ) -> Result<HWND> {
        let title = WindowsString::from(title.unwrap_or_default());
        let class = WindowsString::from(class.unwrap_or_default());
        let found = unsafe {
            FindWindowExW(
                parent,
                after,
                if class.is_empty() {
                    PCWSTR::null()
                } else {
                    class.as_pcwstr()
                },
                if title.is_empty() {
                    PCWSTR::null()
                } else {
                    title.as_pcwstr()
                },
            )
        }?;
        Ok(found)
    }

    pub fn wait_for_native_shell() {
        log::info!("Waiting for native shell...");
        let mut attempt = 0;
        let class = WindowsString::from_str("Shell_TrayWnd");
        unsafe {
            // Wait for Explorer's native taskbar until 50 attempts or 5 seconds.
            while FindWindowW(class.as_pcwstr(), None).is_err() && attempt < 50 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                attempt += 1;
            }
        }
        if attempt == 50 {
            log::warn!("Native shell not found");
        } else {
            log::info!("Native shell found, continuing setup...");
        }
    }

    pub fn current_process() -> HANDLE {
        unsafe { GetCurrentProcess() }
    }

    #[allow(dead_code)]
    pub fn current_thread_id() -> u32 {
        unsafe { GetCurrentThreadId() }
    }

    /// https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getforegroundwindow
    pub fn get_foreground_window() -> HWND {
        let mut hwnd = unsafe { GetForegroundWindow() };
        // based on windows doc, get foreground can return null while window is losing activation
        // so we wait until we get a valid window
        while hwnd.is_invalid() {
            hwnd = unsafe { GetForegroundWindow() };
        }
        hwnd
    }

    pub fn is_window(hwnd: HWND) -> bool {
        unsafe { IsWindow(Some(hwnd)) }.into()
    }

    pub fn is_window_visible(hwnd: HWND) -> bool {
        unsafe { IsWindowVisible(hwnd) }.into()
    }

    pub fn is_iconic(hwnd: HWND) -> bool {
        unsafe { IsIconic(hwnd) }.into()
    }

    pub fn is_zoomed(hwnd: HWND) -> bool {
        unsafe { IsZoomed(hwnd) }.into()
    }

    pub fn is_fullscreen(hwnd: HWND) -> Result<bool> {
        let styles = WindowsApi::get_styles(hwnd);
        if styles.contains(WS_THICKFRAME) {
            return Ok(false);
        }

        let rc_monitor = WindowsApi::monitor_rect(WindowsApi::monitor_from_window(hwnd))?;
        let window_rect = WindowsApi::get_inner_window_rect(hwnd)?;
        Ok(window_rect.left <= rc_monitor.left
            && window_rect.top <= rc_monitor.top
            && window_rect.right >= rc_monitor.right
            && window_rect.bottom >= rc_monitor.bottom)
    }
    /// Sets the visibility state of a window created by the calling thread (could cause a deadlock)
    ///
    /// The deadlock occurs if show_window is called for a window created on a different thread but in same process.
    /// Is safe to use for windows created by other processes
    ///
    /// Use this only if you need wait for the window to be visible, otherwise use show_window_async
    ///
    /// https://stackoverflow.com/questions/16881820/win32-api-deadlocks-while-using-different-threads
    /// https://stackoverflow.com/questions/15637124/whats-the-difference-between-showwindow-and-showwindowasync
    pub fn show_window(hwnd: HWND, command: SHOW_WINDOW_CMD) -> Result<()> {
        // BOOL is returned but does not signify whether or not the operation was succesful
        // https://docs.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showwindow
        unsafe { ShowWindow(hwnd, command) }
            .ok()
            .filter_fake_error()?;
        Ok(())
    }

    pub fn show_window_async(hwnd: HWND, command: SHOW_WINDOW_CMD) -> Result<()> {
        // BOOL is returned but does not signify whether or not the operation was succesful
        // https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showwindowasync
        unsafe { ShowWindowAsync(hwnd, command) }
            .ok()
            .filter_fake_error()?;
        Ok(())
    }

    pub fn get_styles(hwnd: HWND) -> WINDOW_STYLE {
        WINDOW_STYLE(unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32)
    }

    pub fn get_ex_styles(hwnd: HWND) -> WINDOW_EX_STYLE {
        WINDOW_EX_STYLE(unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32)
    }

    fn _set_position(
        hwnd: HWND,
        order: Option<HWND>,
        rect: RECT,
        flags: SET_WINDOW_POS_FLAGS,
    ) -> Result<()> {
        unsafe {
            SetWindowPos(
                hwnd,
                order,
                rect.left,
                rect.top,
                (rect.right - rect.left).abs(),
                (rect.bottom - rect.top).abs(),
                flags,
            )
            .filter_fake_error()?;
        }
        Ok(())
    }

    /// Similar to ShowWindow could cause a deadlock if the window is created on a different thread.
    ///
    /// Add the flag `SWP_ASYNCWINDOWPOS` to avoid that of if you don't need to wait for the window position to be set
    pub fn set_position(
        hwnd: HWND,
        order: Option<HWND>,
        rect: &RECT,
        flags: SET_WINDOW_POS_FLAGS,
    ) -> Result<()> {
        let flags = match order {
            Some(_) => flags,
            None => SWP_NOZORDER | flags,
        } | SWP_NOACTIVATE;
        Self::_set_position(hwnd, order, *rect, flags)
    }

    pub fn move_window(hwnd: HWND, rect: &RECT) -> Result<()> {
        Self::set_position(hwnd, None, rect, SWP_NOSIZE | SWP_ASYNCWINDOWPOS)
    }

    #[allow(dead_code)]
    pub fn bring_to_top(hwnd: HWND) -> Result<()> {
        unsafe { BringWindowToTop(hwnd)? };
        Ok(())
    }

    #[allow(dead_code)]
    pub fn attach_thread_input(thread_id: u32, attach_to: u32, attach: bool) -> Result<()> {
        unsafe { AttachThreadInput(thread_id, attach_to, attach).ok()? };
        Ok(())
    }

    pub fn set_foreground(hwnd: HWND) -> Result<()> {
        let window = Window::from(hwnd);

        if !unsafe { SetForegroundWindow(hwnd).as_bool() } {
            // https://stackoverflow.com/questions/10740346/setforegroundwindow-only-working-while-visual-studio-is-open
            let keyboard = Keyboard::new();
            keyboard.send_keys("{alt}")?;
            // this can fail but still be successful.
            let _ = unsafe { SetForegroundWindow(hwnd) };
        }

        // extra validation
        if Window::get_foregrounded() != window {
            return Err("Failed to set foreground window".into());
        }

        // event sometimes is not emitted, so we manually emit it, this will cause 2 foreground events
        // if original was recieved, btw having it twice is better than nothing
        HookManager::event_tx().send((WinEvent::SystemForeground, window))?;
        Ok(())
    }

    fn open_process(
        access_rights: PROCESS_ACCESS_RIGHTS,
        inherit_handle: bool,
        process_id: u32,
    ) -> Result<HANDLE> {
        unsafe { Ok(OpenProcess(access_rights, inherit_handle, process_id)?) }
    }

    pub fn open_current_process_token() -> Result<HANDLE> {
        let mut token_handle = HANDLE::default();
        unsafe {
            OpenProcessToken(
                Self::current_process(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token_handle,
            )?;
        }
        if token_handle.is_invalid() {
            return Err("OpenProcessToken failed".into());
        }
        Ok(token_handle)
    }

    #[allow(dead_code)]
    pub fn get_current_process_info() -> Result<()> {
        let token_handle = Self::open_current_process_token()?;
        let mut returnlength = 0;
        unsafe {
            let data = TOKEN_GROUPS::default();

            GetTokenInformation(
                token_handle,
                TokenLogonSid,
                Some(&data as *const _ as *mut _),
                std::mem::size_of::<TOKEN_GROUPS>() as u32,
                &mut returnlength,
            )?;
        }
        Ok(())
    }

    pub fn get_parent(hwnd: HWND) -> Result<HWND> {
        Ok(unsafe { GetParent(hwnd)? })
    }

    pub fn get_owner(hwnd: HWND) -> Result<HWND> {
        Ok(unsafe { GetWindow(hwnd, GW_OWNER)? })
    }

    pub fn is_cloaked(hwnd: HWND) -> Result<bool> {
        let mut cloaked: u32 = 0;
        Self::dwm_get_window_attribute(hwnd, DWMWA_CLOAKED, &mut cloaked)?;
        Ok(matches!(
            cloaked,
            DWM_CLOAKED_APP | DWM_CLOAKED_SHELL | DWM_CLOAKED_INHERITED
        ))
    }

    pub fn get_desktop_window() -> HWND {
        unsafe { GetDesktopWindow() }
    }

    pub fn is_process_frozen(process_id: u32) -> Result<bool> {
        let handle = Self::open_process(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id)?;
        let _handle_wrapper = HandleWrapper::new(handle);
        let is_frozen = unsafe {
            let mut buffer: [PROCESS_EXTENDED_BASIC_INFORMATION; 1] = std::mem::zeroed();
            let status = NtQueryInformationProcess(
                handle,
                ProcessBasicInformation,
                buffer.as_mut_ptr() as _,
                std::mem::size_of::<PROCESS_EXTENDED_BASIC_INFORMATION>() as _,
                0u32 as _,
            );

            if status != STATUS_SUCCESS {
                return Err(format!(
                    "NtQueryInformationProcess failed with status: {:x}",
                    status.0
                )
                .into());
            }

            let data = buffer[0];
            data.Anonymous.Flags & ProcessInformationFlag::IsFrozen as u32 != 0
        };
        Ok(is_frozen)
    }

    pub fn exe_path_by_process(process_id: u32) -> Result<OsString> {
        let mut path = WindowsString::new_to_fill(1024);
        let handle = Self::open_process(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id)?;
        let _handle_wrapper = HandleWrapper::new(handle);
        let mut size = 1024u32;
        unsafe {
            QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, path.as_pwstr(), &mut size)?;
        }
        // 验证返回的 size 是否在有效范围内
        if size as usize > path.inner.len() {
            return Err("Process path too long".into());
        }
        Ok(path.to_os_string())
    }

    pub fn get_class(hwnd: HWND) -> Result<String> {
        let mut text: [u16; 512] = [0; 512];
        let len = unsafe { GetClassNameW(hwnd, &mut text) };
        let length = usize::try_from(len).unwrap_or(0);
        Ok(String::from_utf16(&text[..length])?)
    }

    pub fn get_shell_item(path: &Path) -> Result<IShellItem2> {
        let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let item = unsafe { SHCreateItemFromParsingName(PCWSTR(wide_path.as_ptr()), None)? };
        Ok(item)
    }

    pub fn get_property_store_for_window(hwnd: HWND) -> Result<IPropertyStore> {
        Ok(unsafe { SHGetPropertyStoreForWindow(hwnd)? })
    }

    /// https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-id
    pub fn get_window_app_user_model_id(hwnd: HWND) -> Result<String> {
        let store = Self::get_property_store_for_window(hwnd)?;
        let value = unsafe { store.GetValue(&PKEY_AppUserModel_ID)? };
        if value.is_empty() {
            return Err("No AppUserModel_ID".into());
        }
        Ok(BSTR::try_from(&value)?.to_string())
    }

    pub fn get_window_prevent_pinning(hwnd: HWND) -> Result<bool> {
        let store = Self::get_property_store_for_window(hwnd)?;
        let value = unsafe { store.GetValue(&PKEY_AppUserModel_PreventPinning)? };
        if value.is_empty() {
            return Err("No AppUserModel_PreventPinning".into());
        }
        Ok(bool::try_from(&value)?)
    }

    /// https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-relaunchcommand
    pub fn get_window_relaunch_command(hwnd: HWND) -> Result<String> {
        let store = Self::get_property_store_for_window(hwnd)?;
        let value = unsafe { store.GetValue(&PKEY_AppUserModel_RelaunchCommand)? };
        if value.is_empty() {
            return Err("No AppUserModel_RelaunchCommand".into());
        }
        Ok(BSTR::try_from(&value)?.to_string())
    }

    /// https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-relaunchdisplaynameresource
    pub fn get_window_relaunch_display_name(hwnd: HWND) -> Result<String> {
        let store = Self::get_property_store_for_window(hwnd)?;
        let value = unsafe { store.GetValue(&PKEY_AppUserModel_RelaunchDisplayNameResource)? };
        if value.is_empty() {
            return Err("No AppUserModel_RelaunchDisplayName".into());
        }
        Ok(BSTR::try_from(&value)?.to_string())
    }

    /// https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-relaunchiconresource
    pub fn get_window_relaunch_icon_resource(hwnd: HWND) -> Result<String> {
        let store = Self::get_property_store_for_window(hwnd)?;
        let value = unsafe { store.GetValue(&PKEY_AppUserModel_RelaunchIconResource)? };
        if value.is_empty() {
            return Err("No AppUserModel_RelaunchIconResource".into());
        }
        Ok(BSTR::try_from(&value)?.to_string())
    }

    pub fn is_uwp_package_id(package_id: &str) -> bool {
        Self::get_uwp_app_info(package_id).is_ok()
    }

    pub fn get_uwp_app_info(umid: &str) -> Result<AppInfo> {
        let app_info = AppInfo::GetFromAppUserModelId(&umid.into())?;
        Ok(app_info)
    }

    pub fn create_temp_shortcut(
        program: &Path,
        args: &str,
        working_dir: Option<&Path>,
    ) -> Result<PathBuf> {
        let working_dir = working_dir.or_else(|| program.parent());

        Com::run_with_context(|| unsafe {
            let shell_link: IShellLinkW = Com::create_instance(&ShellLink)?;

            let program = WindowsString::from_os_string(program.as_os_str());
            shell_link.SetPath(program.as_pcwstr())?;

            let arguments = WindowsString::from_str(args);
            shell_link.SetArguments(arguments.as_pcwstr())?;

            if let Some(working_dir) = working_dir {
                let working_dir = WindowsString::from_os_string(working_dir.as_os_str());
                shell_link.SetWorkingDirectory(working_dir.as_pcwstr())?;
            }

            let temp_dir = std::env::temp_dir();
            let lnk_path = temp_dir.join(format!("{}.lnk", uuid::Uuid::new_v4()));
            let lnk_path_wide = WindowsString::from_os_string(lnk_path.as_os_str());

            let persist_file: IPersistFile = shell_link.cast()?;
            persist_file.Save(lnk_path_wide.as_pcwstr(), true)?;
            Ok(lnk_path)
        })
    }

    /// return the program and arguments
    pub fn resolve_lnk_target(lnk_path: &Path) -> Result<(PathBuf, OsString)> {
        Com::run_with_context(|| {
            let shell_link: IShellLinkW = Com::create_instance(&ShellLink)?;
            let lnk_wide = lnk_path
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect_vec();

            let persist_file: IPersistFile = shell_link.cast()?;
            unsafe { persist_file.Load(PCWSTR(lnk_wide.as_ptr()), STGM_READ)? };

            let mut target_path = WindowsString::new_to_fill(1024);
            let mut idk = WIN32_FIND_DATAW::default();
            unsafe { shell_link.GetPath(target_path.as_mut_slice(), &mut idk, 0)? };
            target_path = Self::resolve_environment_variables(&target_path)?;

            let mut arguments = WindowsString::new_to_fill(1024);
            unsafe { shell_link.GetArguments(arguments.as_mut_slice())? };

            Ok((target_path.to_os_string().into(), arguments.to_os_string()))
        })
    }

    /// 返回 (图标路径, 图标索引)。索引 > 0 表示图标在 .exe/.dll 中的非默认位置。
    pub fn resolve_lnk_custom_icon_path(lnk_path: &Path) -> Result<(PathBuf, i32)> {
        Com::run_with_context(|| {
            let shell_link: IShellLinkW = Com::create_instance(&ShellLink)?;
            let lnk_wide = lnk_path
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect_vec();

            let persist_file: IPersistFile = shell_link.cast()?;
            unsafe { persist_file.Load(PCWSTR(lnk_wide.as_ptr()), STGM_READ)? };

            let mut icon_path = WindowsString::new_to_fill(1024);
            let mut icon_idx = 0;
            unsafe { shell_link.GetIconLocation(icon_path.as_mut_slice(), &mut icon_idx)? };

            if icon_path.is_empty() {
                return Err("There is no custom icon for this link file".into());
            }

            icon_path = Self::resolve_environment_variables(&icon_path)?;
            Ok((PathBuf::from(icon_path.to_os_string()), icon_idx))
        })
    }

    /// https://learn.microsoft.com/en-us/windows/win32/api/shlwapi/nf-shlwapi-shloadindirectstring
    /// Extracts a specified text resource when given that resource in the form of an indirect string
    /// (a string that begins with the '@' symbol).
    pub fn resolve_indirect_string(text: &str) -> Result<String> {
        let source = WindowsString::from_str(text);
        let mut out = WindowsString::new_to_fill(1024);
        unsafe { SHLoadIndirectString(source.as_pcwstr(), out.as_mut_slice(), None)? };
        Ok(out.to_string())
    }

    /// https://learn.microsoft.com/en-us/windows/win32/api/processenv/nf-processenv-expandenvironmentstringsw
    /// Expands all environment variables in a string (for example, %PATH%).
    pub fn resolve_environment_variables(source: &WindowsString) -> Result<WindowsString> {
        let len = unsafe { ExpandEnvironmentStringsW(source.as_pcwstr(), None) };
        let mut out = WindowsString::new_to_fill(len as usize);
        unsafe { ExpandEnvironmentStringsW(source.as_pcwstr(), Some(out.as_mut_slice())) };
        Ok(out)
    }

    pub fn get_executable_display_name(path: &Path) -> Result<String> {
        Com::run_with_context(|| unsafe {
            let shell_item = Self::get_shell_item(path)?;
            let text = shell_item
                .GetString(&PKEY_FileDescription)
                .or_else(|_| shell_item.GetDisplayName(SIGDN_NORMALDISPLAY))?;
            Ok(text.to_string()?)
        })
    }

    pub fn get_file_umid(path: &Path) -> Result<String> {
        Com::run_with_context(|| unsafe {
            let shell_item = Self::get_shell_item(path)?;
            let store: IPropertyStore = shell_item.GetPropertyStore(GPS_DEFAULT)?;
            let value = store.GetValue(&PKEY_AppUserModel_ID)?;
            if value.is_empty() {
                return Err("No AppUserModel_ID".into());
            }
            Ok(value.to_string())
        })
    }

    pub fn get_file_toast_activator(path: &Path) -> Result<String> {
        Com::run_with_context(|| unsafe {
            let shell_item = Self::get_shell_item(path)?;
            let store: IPropertyStore = shell_item.GetPropertyStore(GPS_DEFAULT)?;
            let value = store.GetValue(&PKEY_AppUserModel_ToastActivatorCLSID)?;
            if value.is_empty() {
                return Err("No AppUserModel ToastActivator CLSID".into());
            }
            Ok(value
                .to_string()
                .trim_start_matches("{")
                .trim_end_matches("}")
                .to_owned())
        })
    }

    pub fn get_window_text(hwnd: HWND) -> String {
        let mut text: [u16; 512] = [0; 512];
        let len = unsafe { GetWindowTextW(hwnd, &mut text) };
        let length = usize::try_from(len).unwrap_or(0);
        String::from_utf16(&text[..length]).unwrap_or("".to_owned())
    }

    pub fn dwm_get_window_attribute<T>(
        hwnd: HWND,
        attribute: DWMWINDOWATTRIBUTE,
        value: &mut T,
    ) -> Result<()> {
        unsafe {
            DwmGetWindowAttribute(
                hwnd,
                attribute,
                (value as *mut T).cast(),
                u32::try_from(std::mem::size_of::<T>())?,
            )?;
        }
        Ok(())
    }

    /// Set window corner preference (Windows 11+)
    pub fn set_window_corner_preference(
        hwnd: HWND,
        preference: DWM_WINDOW_CORNER_PREFERENCE,
    ) -> Result<()> {
        unsafe {
            // DWMWA_WINDOW_CORNER_PREFERENCE = 33
            let attribute = DWMWINDOWATTRIBUTE(33);
            DwmSetWindowAttribute(
                hwnd,
                attribute,
                (&preference as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
                u32::try_from(std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>())?,
            )?;
        }
        Ok(())
    }

    /// Set window border color (Windows 11+)
    /// Use DWMWA_COLOR_NONE (0xFFFFFFFE) to hide border
    pub fn set_window_border_color(hwnd: HWND, color: u32) -> Result<()> {
        unsafe {
            // DWMWA_BORDER_COLOR = 34
            let attribute = DWMWINDOWATTRIBUTE(34);
            DwmSetWindowAttribute(
                hwnd,
                attribute,
                (&color as *const u32).cast(),
                u32::try_from(std::mem::size_of::<u32>())?,
            )?;
        }
        Ok(())
    }

    /// Apply acrylic blur effect to window using Win32 API
    ///
    /// This provides high-quality acrylic blur-behind effect using SetWindowCompositionAttribute.
    ///
    /// # Arguments
    /// * `hwnd` - Window handle
    /// * `gradient_color` - ARGB color (default: 0x50FFFFFF for white semi-transparent)
    ///
    /// # Example
    /// ```ignore
    /// // Apply default acrylic effect (white semi-transparent)
    /// WindowsApi::apply_acrylic_effect(hwnd, None)?;
    ///
    /// // Apply custom tint color
    /// WindowsApi::apply_acrylic_effect(hwnd, Some(0x50FF0000))?; // Red tint
    /// ```
    pub fn apply_acrylic_effect(hwnd: HWND, gradient_color: Option<u32>) -> Result<()> {
        unsafe {
            #[repr(C)]
            struct AccentPolicy {
                accent_state: u32,
                accent_flags: u32,
                gradient_color: u32,
                animation_id: u32,
            }

            #[repr(C)]
            struct WindowCompositionAttributeData {
                attribute: u32,
                data: *mut std::ffi::c_void,
                size_of_data: usize,
            }

            let mut policy = AccentPolicy {
                accent_state: 4, // ACCENT_ENABLE_ACRYLICBLURBEHIND
                accent_flags: 0,
                gradient_color: gradient_color.unwrap_or(0x50FFFFFF), // Default: Alpha=0x50 (80/255) white semi-transparent
                animation_id: 0,
            };

            let mut data = WindowCompositionAttributeData {
                attribute: 19, // WCA_ACCENT_POLICY
                data: &mut policy as *mut _ as *mut std::ffi::c_void,
                size_of_data: std::mem::size_of::<AccentPolicy>(),
            };

            let user32 = windows::Win32::System::LibraryLoader::GetModuleHandleW(
                windows::core::w!("user32.dll"),
            )
            .map_err(|e| format!("Failed to load user32.dll: {:?}", e))?;

            if user32.is_invalid() {
                return Err("user32.dll handle is invalid".into());
            }

            type SetWindowCompositionAttributeFn =
                unsafe extern "system" fn(
                    HWND,
                    *mut WindowCompositionAttributeData,
                ) -> windows::Win32::Foundation::BOOL;

            let proc = windows::Win32::System::LibraryLoader::GetProcAddress(
                user32,
                windows::core::s!("SetWindowCompositionAttribute"),
            )
            .ok_or("SetWindowCompositionAttribute not found in user32.dll")?;

            let set_wca: SetWindowCompositionAttributeFn = std::mem::transmute(proc);
            let result = set_wca(hwnd, &mut data);

            if !result.as_bool() {
                log::warn!(
                    "[WindowsApi] SetWindowCompositionAttribute returned false for hwnd: {:?}",
                    hwnd
                );
            } else {
                log::info!(
                    "[WindowsApi] Acrylic effect applied successfully to hwnd: {:?}",
                    hwnd
                );
            }
        }
        Ok(())
    }

    /// Get the window rect including drop shadow
    pub fn get_outer_window_rect(hwnd: HWND) -> Result<RECT> {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut rect)? };
        Ok(rect)
    }

    fn get_window_thickness(hwnd: HWND) -> u32 {
        let mut thickness = 0u32;
        let _ = Self::dwm_get_window_attribute(
            hwnd,
            DWMWA_VISIBLE_FRAME_BORDER_THICKNESS,
            &mut thickness,
        );
        thickness
    }

    /// return the window rect excluding drop shadow & thick border
    /// https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowrect#remarks
    pub fn get_inner_window_rect(hwnd: HWND) -> Result<RECT> {
        let mut rect = RECT::default();
        if Self::dwm_get_window_attribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut rect).is_err() {
            rect = Self::get_outer_window_rect(hwnd)?;
        }

        let styles = Self::get_styles(hwnd);
        if styles.contains(WS_THICKFRAME) || styles.contains(WS_SIZEBOX) {
            let thickness = Self::get_window_thickness(hwnd) as i32;
            rect.left += thickness;
            rect.top += thickness;
            rect.right -= thickness;
            rect.bottom -= thickness;
        }

        Ok(rect)
    }

    pub fn monitor_from_window(hwnd: HWND) -> HMONITOR {
        unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY) }
    }

    pub fn monitor_from_cursor_point() -> HMONITOR {
        if let Ok(point) = Mouse::get_cursor_pos() {
            return unsafe { MonitorFromPoint(*point.as_ref(), MONITOR_DEFAULTTOPRIMARY) };
        }
        Self::primary_monitor()
    }

    pub fn monitor_from_point(point: &Point) -> HMONITOR {
        unsafe { MonitorFromPoint(*point.as_ref(), MONITOR_DEFAULTTOPRIMARY) }
    }

    pub fn primary_monitor() -> HMONITOR {
        unsafe { MonitorFromWindow(GetDesktopWindow(), MONITOR_DEFAULTTOPRIMARY) }
    }

    pub fn monitor_info(hmonitor: HMONITOR) -> Result<MONITORINFOEXW> {
        let mut ex_info = MONITORINFOEXW::default();
        ex_info.monitorInfo.cbSize = u32::try_from(std::mem::size_of::<MONITORINFOEXW>())?;
        unsafe { GetMonitorInfoW(hmonitor, &mut ex_info.monitorInfo).ok() }?;
        Ok(ex_info)
    }

    pub fn monitor_rect(hmonitor: HMONITOR) -> Result<RECT> {
        Ok(Self::monitor_info(hmonitor)?.monitorInfo.rcMonitor)
    }

    pub fn shadow_rect(hwnd: HWND) -> Result<RECT> {
        let outer_rect = Self::get_outer_window_rect(hwnd)?;
        let inner_rect = Self::get_inner_window_rect(hwnd)?;
        Ok(RECT {
            left: outer_rect.left - inner_rect.left,
            top: outer_rect.top - inner_rect.top,
            right: outer_rect.right - inner_rect.right,
            bottom: outer_rect.bottom - inner_rect.bottom,
        })
    }

    pub fn _get_virtual_desktop_manager() -> Result<IVirtualDesktopManager> {
        Com::create_instance(&VirtualDesktopManager)
    }

    pub fn _get_virtual_desktop_id(hwnd: HWND) -> Result<GUID> {
        let manager = Self::_get_virtual_desktop_manager()?;
        let mut desktop_id = GUID::zeroed();
        let mut attempt = 0;
        while desktop_id.to_u128() == 0 && attempt < 10 {
            attempt += 1;
            sleep(Duration::from_millis(30));
            if let Ok(desktop) = unsafe { manager.GetWindowDesktopId(hwnd) } {
                desktop_id = desktop
            }
        }
        if desktop_id.to_u128() == 0 {
            return Err(format!("Failed to get desktop id for: {hwnd:?}").into());
        }
        Ok(desktop_id)
    }

    pub fn is_elevated() -> Result<bool> {
        unsafe {
            let mut elevation = TOKEN_ELEVATION::default();
            let mut ret_len = 0;

            let token_handle = Self::open_current_process_token()?;

            GetTokenInformation(
                token_handle,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut _),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut ret_len,
            )?;

            CloseHandle(token_handle)?;

            Ok(elevation.TokenIsElevated != 0)
        }
    }

    /// https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid
    pub fn known_folder(folder_id: windows::core::GUID) -> Result<PathBuf> {
        let path = unsafe { SHGetKnownFolderPath(&folder_id, KF_FLAG_DEFAULT, None)? };
        Ok(PathBuf::from(OsString::from_wide(unsafe {
            path.as_wide()
        })))
    }

    /// 获取窗口属性
    pub fn get_prop(hwnd: HWND, prop_name: &str) -> Option<isize> {
        let prop_name_wide = WindowsString::from_str(prop_name);
        unsafe {
            let result = GetPropW(hwnd, prop_name_wide.as_pcwstr());
            // GetPropW 在属性不存在时返回 NULL，而不是 INVALID_HANDLE_VALUE
            if result.0.is_null() {
                None
            } else {
                Some(result.0 as isize)
            }
        }
    }

    pub fn get_recycle_bin_hwnd() -> Option<HWND> {
        // 直接遍历所有 CabinetWClass 窗口，避免 find_window 模糊匹配
        // 获取第一个 CabinetWClass 窗口
        let first_class = WindowsString::from_str("CabinetWClass");
        let mut hwnd =
            match unsafe { FindWindowExW(None, None, first_class.as_pcwstr(), PCWSTR::null()) } {
                Ok(h) => h,
                Err(_) => {
                    return None;
                }
            };

        while !hwnd.is_invalid() {
            let title = WindowsApi::get_window_text(hwnd);

            // 检查标题是否匹配回收站（精确匹配开头）
            // 中文系统：回收站 或 回收站 - 文件资源管理器
            // 英文系统：Recycle Bin 或 Recycle Bin - File Explorer
            // 繁体系统：資源回收筒
            let is_recycle_bin = title.starts_with("回收站")
                || title.starts_with("Recycle Bin")
                || title.starts_with("資源回收筒");

            if is_recycle_bin {
                return Some(hwnd);
            }

            // 继续查找下一个 CabinetWClass 窗口
            match unsafe {
                FindWindowExW(None, Some(hwnd), first_class.as_pcwstr(), PCWSTR::null())
            } {
                Ok(h) => hwnd = h,
                Err(_) => break,
            }
        }

        None
    }
}
