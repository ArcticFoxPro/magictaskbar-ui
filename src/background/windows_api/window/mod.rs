pub mod cache;
pub mod event;

use crate::utils::icon_whitelist::is_generic_installer;
use libs_core::{rect::Rect, system_state::MonitorId};
use slu_ipc::messages::SvcAction;
use std::{
    fmt::{Debug, Display},
    path::PathBuf,
    sync::LazyLock,
    time::Duration,
};

use windows::{
    ApplicationModel::AppInfo,
    Win32::{
        Foundation::{HWND, RECT},
        UI::{
            Shell::FOLDERID_System,
            WindowsAndMessaging::{SET_WINDOW_POS_FLAGS, SHOW_WINDOW_CMD, SW_RESTORE},
        },
    },
};

use crate::{
    cli::ServicePipe,
    error::Result,
    modules::{
        apps::application::is_interactable_and_not_hidden, start::application::START_MENU_MANAGER,
    },
    widgets::{taskbar::Taskbar, toolbar::FancyToolbar},
};

use super::{
    monitor::Monitor, process::Process, types::AppUserModelId, HandleWrapper, WindowEnumerator,
    WindowsApi,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Window(HWND);
unsafe impl Send for Window {}
unsafe impl Sync for Window {}

impl From<HWND> for Window {
    fn from(hwnd: HWND) -> Self {
        Self(hwnd)
    }
}

impl From<isize> for Window {
    fn from(addr: isize) -> Self {
        Self(HWND(addr as _))
    }
}

impl Debug for Window {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Window")
            .field("handle", &self.0 .0)
            .field(
                "exe",
                &self.process().program_exe_name().unwrap_or_default(),
            )
            .field("class", &self.class())
            .field("title", &self.title())
            .finish()
    }
}

impl Display for Window {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Window({:?})", self.0 .0)
    }
}

static APP_FRAME_HOST_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    WindowsApi::known_folder(FOLDERID_System)
        .expect("Failed to get system folder")
        .join("ApplicationFrameHost.exe")
});

impl Window {
    pub fn get_foregrounded() -> Window {
        Window(WindowsApi::get_foreground_window())
    }

    pub fn hwnd(&self) -> HWND {
        self.0
    }

    pub fn address(&self) -> isize {
        self.0 .0 as isize
    }

    pub fn is_electron(&self) -> bool {
        self.class() == "Chrome_WidgetWin_1"
    }

    /// Application user model id asigned to the window via property-store or inherited from the process
    ///
    /// https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-id
    pub fn app_user_model_id(&self) -> Option<AppUserModelId> {
        if let Ok(umid) = WindowsApi::get_window_app_user_model_id(self.0) {
            return match WindowsApi::is_uwp_package_id(&umid) {
                true => Some(AppUserModelId::Appx(umid)),
                false => Some(AppUserModelId::PropertyStore(umid)),
            };
        }

        let process = self.process();
        if let Ok(umid) = process.package_app_user_model_id() {
            return Some(umid);
        }

        if self.is_electron() {
            let path = process.program_path().ok()?;

            // special manual case like there's no way to call GetCurrentProcessExplicitAppUserModelID without code injection
            if path.file_name()?.to_string_lossy().to_lowercase() == "discord.exe" {
                return Some(AppUserModelId::PropertyStore(
                    "com.squirrel.Discord.Discord".to_string(),
                ));
            }

            let guard = START_MENU_MANAGER.load();
            let item = guard.get_by_target(&path)?;
            Some(AppUserModelId::PropertyStore(item.umid.clone()?))
        } else {
            None
        }
    }

    /// https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-preventpinning
    pub fn prevent_pinning(&self) -> bool {
        WindowsApi::get_window_prevent_pinning(self.0).unwrap_or(false)
    }

    /// https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-relaunchcommand
    pub fn relaunch_command(&self) -> Option<String> {
        WindowsApi::get_window_relaunch_command(self.0).ok()
    }

    /// https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-relaunchdisplaynameresource
    pub fn relaunch_display_name(&self) -> Option<String> {
        if let Ok(name) = WindowsApi::get_window_relaunch_display_name(self.0) {
            if name.starts_with("@") {
                return WindowsApi::resolve_indirect_string(&name).ok();
            }
            return Some(name);
        }
        None
    }

    /// https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-relaunchiconresource
    #[allow(dead_code)]
    pub fn relaunch_icon(&self) -> Option<String> {
        WindowsApi::get_window_relaunch_icon_resource(self.0).ok()
    }

    pub fn title(&self) -> String {
        WindowsApi::get_window_text(self.0)
    }

    pub fn class(&self) -> String {
        WindowsApi::get_class(self.0).unwrap_or_default()
    }

    pub fn process(&self) -> Process {
        Process::from_window(self)
    }

    pub fn app_display_name(&self) -> Result<String> {
        // 先注释，目前不需要用到准确的应用名称了
        if let Some(AppUserModelId::Appx(umid)) = self.app_user_model_id() {
            let info = AppInfo::GetFromAppUserModelId(&umid.into())?;
            return Ok(info.DisplayInfo()?.DisplayName()?.to_string_lossy());
        }

        self.process().program_display_name()
    }

    #[allow(dead_code)]
    pub fn outer_rect(&self) -> Result<Rect> {
        let rect = WindowsApi::get_outer_window_rect(self.hwnd())?;
        Ok(Rect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        })
    }

    pub fn inner_rect(&self) -> Result<Rect> {
        let rect = WindowsApi::get_inner_window_rect(self.hwnd())?;
        Ok(Rect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        })
    }

    pub fn parent(&self) -> Option<Window> {
        match WindowsApi::get_parent(self.0) {
            Ok(parent) => Some(Window::from(parent)),
            Err(_) => None,
        }
    }

    pub fn owner(&self) -> Option<Window> {
        WindowsApi::get_owner(self.0).ok().map(Window::from)
    }

    pub fn children(&self) -> Result<Vec<Window>> {
        WindowEnumerator::new()
            .with_parent(self.0)
            .map(Window::from)
    }

    pub fn monitor(&self) -> Monitor {
        Monitor::from(WindowsApi::monitor_from_window(self.0))
    }

    pub fn monitor_id(&self) -> MonitorId {
        self.monitor()
            .stable_id2()
            .unwrap_or_else(|_| MonitorId("null".to_string()))
    }

    pub fn is_window(&self) -> bool {
        WindowsApi::is_window(self.0)
    }

    pub fn is_visible(&self) -> bool {
        WindowsApi::is_window_visible(self.0)
    }

    pub fn is_minimized(&self) -> bool {
        WindowsApi::is_iconic(self.0)
    }

    pub fn is_maximized(&self) -> bool {
        WindowsApi::is_zoomed(self.0)
    }

    pub fn is_cloaked(&self) -> bool {
        WindowsApi::is_cloaked(self.0).unwrap_or(false)
    }

    pub fn is_focused(&self) -> bool {
        WindowsApi::get_foreground_window() == self.0
    }

    pub fn is_fullscreen(&self) -> bool {
        WindowsApi::is_fullscreen(self.0).unwrap_or(false) && !self.is_desktop()
    }

    /// is the window an Application Frame Host
    pub fn is_frame(&self) -> Result<bool> {
        Ok(self
            .process()
            .program_path()?
            .as_os_str()
            .eq_ignore_ascii_case(&*APP_FRAME_HOST_PATH))
    }

    /// will fail if the window is not a frame
    pub fn get_frame_creator(&self) -> Result<Option<Window>> {
        if !self.is_frame()? {
            return Err("Window is not a frame".into());
        }

        let title = self.title();
        let class = "Windows.UI.Core.CoreWindow";

        // Use WindowEnumerator to find CoreWindow directly, bypassing filters
        // This ensures we can find CoreWindow even if it was filtered by is_interactable_and_not_hidden
        let mut found: Option<Window> = None;
        let _ = crate::windows_api::WindowEnumerator::new().for_each(|w| {
            if found.is_none() && w.class() == class && w.title() == title {
                found = Some(w);
            }
        });

        if let Some(creator) = found {
            return Ok(Some(creator));
        }

        Ok(None)
    }

    /// this means all windows that are part of the UI desktop not the real desktop window
    pub fn is_desktop(&self) -> bool {
        WindowsApi::get_desktop_window() == self.0 || {
            let class = self.class();
            let title = self.title();
            // 新增XZDesktopWnd和HonorMagicDesktop窗口识别为桌面窗口
            class == "Progman"
                || class == "XZDesktopWnd"
                || title == "XZDesktopWnd"
                || class == "HonorMagicDesktop"
                || title == "HonorMagicDesktop"
                || {
                    class == "WorkerW"
                        && self.children().is_ok_and(|children| {
                            children
                                .iter()
                                .any(|child| child.class() == "SHELLDLL_DefView")
                        })
                }
        }
    }

    pub fn is_bar_overlay(&self) -> bool {
        if let Ok(exe) = self.process().program_path() {
            return exe.ends_with("magictaskbar-ui.exe")
                && [FancyToolbar::TITLE, Taskbar::TITLE].contains(&self.title().as_str());
        }
        false
    }

    /// read inner called doc for more info
    pub fn is_interactable_and_not_hidden(&self) -> bool {
        is_interactable_and_not_hidden(self)
    }

    pub fn show_window(&self, command: SHOW_WINDOW_CMD) -> Result<()> {
        // Try direct call first if we can open process handle
        if let Ok(handle) = self.process().open_handle() {
            let _wrapper = HandleWrapper::new(handle);
            if WindowsApi::show_window(self.hwnd(), command).is_ok() {
                return Ok(());
            }
        }

        // Fall back to service pipe (either because direct call failed or couldn't open handle)
        ServicePipe::request(SvcAction::ShowWindow {
            hwnd: self.address(),
            command: command.0,
        })
    }

    pub fn show_window_async(&self, command: SHOW_WINDOW_CMD) -> Result<()> {
        if let Ok(handle) = self.process().open_handle() {
            let _wrapper = HandleWrapper::new(handle);
            WindowsApi::show_window_async(self.hwnd(), command)
        } else {
            ServicePipe::request(SvcAction::ShowWindowAsync {
                hwnd: self.address(),
                command: command.0,
            })
        }
    }

    #[allow(dead_code)]
    pub fn set_position(&self, rect: &RECT, flags: SET_WINDOW_POS_FLAGS) -> Result<()> {
        if let Ok(handle) = self.process().open_handle() {
            let _wrapper = HandleWrapper::new(handle);
            WindowsApi::set_position(self.hwnd(), None, rect, flags)
        } else {
            ServicePipe::request(SvcAction::SetWindowPosition {
                hwnd: self.address(),
                rect: Rect {
                    top: rect.top,
                    left: rect.left,
                    right: rect.right,
                    bottom: rect.bottom,
                },
                flags: flags.0,
            })
        }
    }

    pub fn focus(&self) -> Result<()> {
        if self.is_minimized() {
            self.show_window(SW_RESTORE)?;
        }

        WindowsApi::set_foreground(self.hwnd())
    }

    /// Check if this is a UWP application (line 99)
    pub fn is_uwp(&self) -> bool {
        matches!(
            self.app_user_model_id(),
            Some(crate::windows_api::types::AppUserModelId::Appx(_))
        )
    }

    /// Get window icon as PNG bytes (ManagedShell-style)
    /// Implements ApplicationWindow.Icon logic (lines 176-192, 472-573)
    pub fn get_icon_png(
        &self,
        size: crate::windows_api::icon_extractor::IconSize,
    ) -> Option<Vec<u8>> {
        use base64::Engine as _;

        let large = size == crate::windows_api::icon_extractor::IconSize::Large;
        let encoded = ServicePipe::request_with_response_blocking(
            SvcAction::GetWindowIconPng {
                hwnd: self.address(),
                large,
            },
            Duration::from_millis(1200),
        )
        .ok()
        .flatten()?;
        let mut png_data = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()?;
        self.fix_war3_icon_alpha(&mut png_data);
        Some(png_data)
    }

    fn fix_war3_icon_alpha(&self, png_data: &mut Vec<u8>) {
        if !self.title().to_lowercase().contains("warcraft")
            && !self
                .process()
                .program_path()
                .map(|p| p.to_string_lossy().to_lowercase().contains("war3"))
                .unwrap_or(false)
        {
            return;
        }

        log::info!(
            "[War3Fix] Detected War3, original PNG size: {} bytes",
            png_data.len()
        );
        if let Ok(img) = image::load_from_memory(png_data) {
            let mut rgba = img.to_rgba8();
            log::info!(
                "[War3Fix] PNG decoded, dimensions: {}x{}",
                rgba.width(),
                rgba.height()
            );

            for pixel in rgba.pixels_mut() {
                pixel[3] = 255;
            }

            let mut buf = std::io::Cursor::new(Vec::new());
            if image::DynamicImage::ImageRgba8(rgba)
                .write_to(&mut buf, image::ImageFormat::Png)
                .is_ok()
            {
                *png_data = buf.into_inner();
                log::info!(
                    "[War3Fix] Transparency fixed! New PNG size: {} bytes",
                    png_data.len()
                );
            } else {
                log::error!("[War3Fix] Failed to re-encode PNG");
            }
        } else {
            log::error!("[War3Fix] Failed to decode PNG");
        }
    }

    /// Get icon from executable file for Win32 apps (line 548)
    /// Implements IconHelper.GetIconByFilename fallback
    pub fn get_icon_from_exe(
        &self,
        size: crate::windows_api::icon_extractor::IconSize,
    ) -> Option<Vec<u8>> {
        let exe_path = self.process().program_path().ok()?;

        let hicon = crate::windows_api::icon_extractor::IconExtractor::get_icon_by_filename(
            &exe_path, size,
        )?;

        match crate::windows_api::icon_extractor::IconExtractor::hicon_to_png(hicon) {
            Ok(png_data) => {
                let _ = crate::windows_api::icon_extractor::IconExtractor::destroy_icon(hicon);
                Some(png_data)
            }
            Err(_) => {
                let _ = crate::windows_api::icon_extractor::IconExtractor::destroy_icon(hicon);
                None
            }
        }
    }

    /// Smart serialization: only whitelist apps call to_serializable_with_icon for performance
    /// Non-whitelist apps use lightweight to_serializable to avoid expensive icon extraction
    pub fn to_smart_serializable(
        &self,
        display_name: Option<&str>,
    ) -> libs_core::system_state::UserAppWindow {
        let process_name = self.process().program_exe_name().unwrap_or_default();
        // Check both process whitelist and window whitelist
        let (is_whitelisted, _requires_backplate) =
            crate::utils::icon_whitelist::is_app_or_window_whitelisted(
                &process_name,
                &self.class(),
                &self.title(),
            );

        // 🔧 修复：generic installer 也需要调用 to_serializable_with_icon 来获取窗口图标
        let is_generic = is_generic_installer(&process_name);

        // Only whitelist apps and Control Panel apps call the expensive to_serializable_with_icon
        if is_whitelisted
            || is_generic
            || display_name
                .map(|name| name.contains("控制面板"))
                .unwrap_or(false)
        {
            self.to_serializable_with_icon(display_name)
        } else {
            // Non-whitelist apps use lightweight serialization without icon extraction
            self.to_serializable()
        }
    }

    /// Convert window to UserAppWindow with icon (ManagedShell-style)
    /// Implements ApplicationWindow.Icon getter logic
    pub fn to_serializable_with_icon(
        &self,
        display_name: Option<&str>,
    ) -> libs_core::system_state::UserAppWindow {
        let current_settings = crate::state::application::FULL_STATE.load();
        use libs_core::state::IconBackplateStyle;
        let use_local_icon = current_settings.settings.taskbar.icon_backplate_style
            == IconBackplateStyle::Transparent;

        let process_name = self
            .process()
            .program_exe_name()
            .unwrap_or_else(|_| "unknown".to_string());

        if is_generic_installer(&process_name) {
            if let Ok(exe_path) = self.process().program_path() {
                if let Some(hicon) =
                    crate::windows_api::icon_extractor::IconExtractor::get_icon_by_filename(
                        &exe_path,
                        crate::windows_api::icon_extractor::IconSize::Large,
                    )
                {
                    if let Ok(png_data) =
                        crate::windows_api::icon_extractor::IconExtractor::hicon_to_png(hicon)
                    {
                        let icon_base64 = base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            &png_data,
                        );
                        let _ =
                            crate::windows_api::icon_extractor::IconExtractor::destroy_icon(hicon);
                        let mut user_app_window = self.to_serializable();
                        user_app_window.icon_png_base64 = Some(icon_base64);
                        return user_app_window;
                    } else {
                        let _ =
                            crate::windows_api::icon_extractor::IconExtractor::destroy_icon(hicon);
                        // 转换失败，继续执行后续本地图标和白名单逻辑
                    }
                }
            }
            // 🔧 修复：generic installer 文件不存在时，fallback 到从窗口句柄获取图标
            // 这是安装程序的常见场景：临时目录中的 setup.exe 已被删除
            log::info!("[to_serializable_with_icon] Generic installer file not found, trying window icon for hwnd={:?}", self.hwnd());
            if let Some(png_data) =
                self.get_icon_png(crate::windows_api::icon_extractor::IconSize::Large)
            {
                let icon_base64 =
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_data);
                let mut user_app_window = self.to_serializable();
                user_app_window.icon_png_base64 = Some(icon_base64);
                return user_app_window;
            }
            log::warn!("[to_serializable_with_icon] service window icon returned None for generic installer");
        }

        let name_to_check = display_name.unwrap_or(&process_name);
        let local_png = if use_local_icon {
            crate::utils::icon_whitelist::get_local_process_icon(&process_name)
                .or_else(|| crate::utils::icon_whitelist::get_local_process_icon(name_to_check))
        } else {
            crate::utils::icon_whitelist::get_local_process_icon_white(&process_name).or_else(
                || crate::utils::icon_whitelist::get_local_process_icon_white(name_to_check),
            )
        };

        if let Some(png_data) = local_png {
            let icon_base64 =
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_data);
            return libs_core::system_state::UserAppWindow {
                icon_png_base64: Some(icon_base64),
                is_approximately_square: Some(true),
                is_from_local: Some(true),
                ..self.to_serializable()
            };
        }

        let (is_whitelisted, _) = crate::utils::icon_whitelist::is_app_or_window_whitelisted(
            &process_name,
            &self.class(),
            &self.title(),
        );
        let is_control_panel = display_name
            .map(|n| n.contains("控制面板"))
            .unwrap_or(false);

        let png_opt = if is_whitelisted || is_control_panel {
            if is_control_panel {
                let path = std::path::Path::new(r"C:\Windows\System32\control.exe");
                if let Some(hicon) =
                    crate::windows_api::icon_extractor::IconExtractor::get_icon_by_filename(
                        path,
                        crate::windows_api::icon_extractor::IconSize::Large,
                    )
                {
                    let png =
                        crate::windows_api::icon_extractor::IconExtractor::hicon_to_png(hicon).ok();
                    let _ = crate::windows_api::icon_extractor::IconExtractor::destroy_icon(hicon);
                    png
                } else {
                    None
                }
            } else if self.is_uwp() {
                self.get_icon_png(crate::windows_api::icon_extractor::IconSize::Large)
                    .or_else(|| {
                        self.get_icon_png(crate::windows_api::icon_extractor::IconSize::Small)
                    })
            } else {
                self.get_icon_png(crate::windows_api::icon_extractor::IconSize::Large)
                    .or_else(|| {
                        self.get_icon_png(crate::windows_api::icon_extractor::IconSize::Small)
                    })
                    .or_else(|| {
                        self.get_icon_from_exe(crate::windows_api::icon_extractor::IconSize::Large)
                    })
                    .or_else(|| {
                        self.get_icon_from_exe(crate::windows_api::icon_extractor::IconSize::Small)
                    })
            }
        } else {
            None
        };

        libs_core::system_state::UserAppWindow {
            icon_png_base64: png_opt.map(|png| {
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png)
            }),
            is_approximately_square: Some(false),
            is_from_local: None,
            ..self.to_serializable()
        }
    }
}
