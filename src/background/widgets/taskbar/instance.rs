use base64::Engine;
use libs_core::handlers::FuncEvent;
use serde::Serialize;
use tauri::{Emitter, WebviewWindow, Wry};
use windows::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Gdi::HMONITOR,
    UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, SWP_ASYNCWINDOWPOS, WS_EX_NOACTIVATE,
    },
};

use crate::{
    app::get_app_handle,
    cli::ServicePipe,
    error::Result,
    log_error,
    state::application::FULL_STATE,
    utils::are_overlaped,
    widgets::WebviewArgs,
    windows_api::{window::Window, AppBarData, WindowsApi},
};

use super::glass_effect::TaskbarGlassEffect;

pub struct Taskbar {
    pub window: WebviewWindow<Wry>,
    /// This is the webview/window rect
    pub webview_rect: RECT,
    pub overlaped_by: Option<Window>,
    pub hidden: bool,
    /// 液态玻璃亚克力效果（覆盖在 WebView 下层）
    pub glass_effect: Option<TaskbarGlassEffect>,
    /// glass effect 创建时的 DPI（用于检测 DPI 变化）
    pub last_dpi: f64,
    /// glass effect 创建时的父窗口尺寸（用于检测分辨率变化）
    pub last_glass_size: (i32, i32),
}

impl Drop for Taskbar {
    fn drop(&mut self) {
        log::info!("Dropping {}", self.window.label());
        // 先销毁玻璃效果（触发 DLL 资源释放）
        self.glass_effect.take();
        if let Ok(hwnd) = self.hwnd() {
            AppBarData::from_handle(hwnd).unregister_bar();
        }
        log_error!(self.window.destroy());
    }
}

impl Taskbar {
    pub const TITLE: &'static str = "HonorTaskbar";
    pub const TARGET: &'static str = "@magic/taskbar";
    /// Taskbar 与屏幕底部的间距（像素），前后端需要保持一致
    pub const BOTTOM_MARGIN: i32 = 0;
    /// 窗口右边比容器右边多的 buffer（CSS 像素），需要乘以 DPI
    pub const WINDOW_BUFFER_PX: i32 = 50;
    /// 容器底部与窗口底部的间距（CSS 像素），需要乘以 DPI，与前端 CSS margin-bottom 保持一致
    pub const CONTAINER_BOTTOM_MARGIN_CSS: i32 = 5;

    pub fn hwnd(&self) -> Result<HWND> {
        Ok(HWND(self.window.hwnd()?.0))
    }

    fn create_window(monitor_id: &str) -> Result<WebviewWindow> {
        let manager = get_app_handle();
        let label = format!("{}?monitorId={}", Self::TARGET, monitor_id);
        let args = WebviewArgs::new().disable_gpu();

        log::info!("Creating {label}");
        let label = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&label);

        let window = tauri::WebviewWindowBuilder::new(
            manager,
            label,
            tauri::WebviewUrl::App(format!("taskbar/index.html?monitorId={}", monitor_id).into()),
        )
        .title(Self::TITLE)
        .minimizable(false)
        .maximizable(false)
        .closable(false)
        .resizable(false)
        .visible(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .data_directory(args.data_directory())
        .additional_browser_args(&args.to_string())
        // 显式禁用开发工具（F12），防止用户误触调出调试窗口
        .devtools(false)
        .build()
        .map_err(|e| {
            log::error!(
                "[Taskbar] Failed to create taskbar webview for monitor_id={monitor_id}: {e:?}"
            );
            e
        })?;

        window.set_ignore_cursor_events(true)?;
        log::info!("[Taskbar] initial ignore_cursor_events = true");

        // 异步设置窗口样式，避免 hwnd() 阻塞
        let window_clone = window.clone();
        std::thread::spawn(move || {
            if let Ok(raw) = window_clone.hwnd() {
                let hwnd = HWND(raw.0);
                let _ = WindowsApi::ignore_close(hwnd);
                unsafe {
                    let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style | WS_EX_NOACTIVATE.0 as isize);
                }
            }
        });
        Ok(window)
    }

    pub fn new(postfix: &str) -> Result<Self> {
        log::info!("[Taskbar] creating instance, monitor_id={}", postfix);
        match Self::create_window(postfix) {
            Ok(window) => {
                // Register WebView2 ProcessFailed handler - notifies srv to restart UI on crash
                crate::webview_recovery::register_process_failed_handler(&window, "Taskbar");

                let taskbar = Self {
                    window,
                    overlaped_by: None,
                    webview_rect: RECT::default(),
                    hidden: true,
                    glass_effect: None,
                    last_dpi: 0.0,
                    last_glass_size: (0, 0),
                };
                log::info!(
                    "[Taskbar] Taskbar 实例创建成功: {}, hidden=true, visible=false",
                    taskbar.window.label()
                );
                Ok(taskbar)
            }
            Err(e) => {
                log::error!(
                    "[Taskbar] Failed to create Taskbar instance for monitor_id={postfix}: {e:?}"
                );
                Err(e)
            }
        }
    }

    fn emit<S: Serialize + Clone>(&self, event: &str, payload: S) -> Result<()> {
        self.window.emit_to(self.window.label(), event, payload)?;
        Ok(())
    }

    fn is_overlapping(&self, window: &Window) -> Result<bool> {
        // 检查窗口句柄是否有效
        if !window.is_window() {
            return Ok(false);
        }

        let monitor = WindowsApi::monitor_from_window(self.hwnd()?);
        let window_monitor = WindowsApi::monitor_from_window(window.hwnd());
        if window_monitor != monitor {
            return Ok(false);
        }

        let window_rect = WindowsApi::get_inner_window_rect(window.hwnd())?;
        let mut dock_rect = self.webview_rect;
        let dock_size = Self::get_taskbar_size_on_monitor(monitor)?;
        dock_rect.top = dock_rect.bottom - dock_size;
        Ok(are_overlaped(&dock_rect, &window_rect))
    }

    /// 检查是否有任何窗口与任务栏区域重叠，返回第一个重叠的窗口
    fn has_overlapping_window(&self) -> Result<Option<Window>> {
        use crate::utils::constants::NATIVE_UI_POPUP_CLASSES;
        use crate::widgets::taskbar::taskbar_items_impl::get_taskbar_windows;

        let windows = get_taskbar_windows();

        for window in windows {
            if window.is_minimized() {
                continue;
            }

            // 检查是否重叠且满足条件
            if let Ok(true) = self.is_overlapping(&window) {
                if !NATIVE_UI_POPUP_CLASSES.contains(&window.class().as_str()) {
                    // 打印找到的重叠窗口信息
                    log::info!(
                        "[has_overlapping_window] Found overlapping window: hwnd={:?}",
                        window.address()
                    );
                    // 找到第一个重叠窗口，立即返回
                    return Ok(Some(window));
                }
            }
        }
        Ok(None)
    }

    pub fn set_overlaped(&mut self, overlaped_by: Option<Window>) -> Result<()> {
        if self.overlaped_by != overlaped_by {
            log::info!(
                "[set_overlaped] Emitting TaskbarOverlaped event, is_overlapped: {:?}",
                overlaped_by.is_some()
            );
            self.emit(FuncEvent::TaskbarOverlaped, overlaped_by.is_some())?;
        }
        self.overlaped_by = overlaped_by;
        Ok(())
    }
    /// 检查单个窗口是否应该被视为遮挡任务栏的有效窗口
    fn is_valid_overlapping_window(&self, window: &Window) -> bool {
        use crate::widgets::taskbar::taskbar_items_impl::get_taskbar_windows;

        // 检查窗口句柄是否有效
        if !window.is_window() {
            return false;
        }

        // 跳过不可见窗口（如被隐藏但未销毁的窗口）
        if !window.is_visible() {
            return false;
        }

        // 跳过最小化窗口
        if window.is_minimized() {
            return false;
        }

        // 检查窗口是否在任务栏项中（排除不在任务栏管理列表中的未知窗口）
        let taskbar_windows = get_taskbar_windows();
        let in_taskbar = taskbar_windows.iter().any(|w| w.hwnd() == window.hwnd());
        if !in_taskbar {
            return false;
        }

        true
    }

    pub fn handle_overlaped_status(&mut self, window: &Window) -> Result<()> {
        if self.handle_overlaped_status_by_service(window)? {
            return Ok(());
        }

        // 优化：优先检查触发事件的窗口是否与任务栏重叠
        // 避免每次都全量枚举所有窗口
        if self.is_valid_overlapping_window(window) {
            if let Ok(true) = self.is_overlapping(window) {
                log::info!("[Taskbar] is overlapped");
                return self.set_overlaped(Some(*window));
            }
        }

        // 如果触发事件的窗口不是遮挡窗口，检查之前的遮挡窗口是否还在遮挡
        if let Some(prev_overlapping) = &self.overlaped_by {
            // 如果之前的遮挡窗口仍然有效且仍在遮挡，保持状态
            if self.is_valid_overlapping_window(prev_overlapping) {
                if let Ok(true) = self.is_overlapping(prev_overlapping) {
                    return Ok(()); // 保持当前遮挡状态
                }
            }
            // 之前的遮挡窗口不再遮挡，清除状态
            log::debug!("[Taskbar] previous overlapping window no longer overlapping");
            self.set_overlaped(None)?;
        }

        // 全量枚举（较少发生）
        if self.overlaped_by.is_none() {
            if let Some(overlapping_window) = self.has_overlapping_window()? {
                log::info!(
                    "[Taskbar] is overlapped by (full scan): hwnd={:?}",
                    overlapping_window.address()
                );
                return self.set_overlaped(Some(overlapping_window));
            }
        }

        Ok(())
    }

    fn handle_overlaped_status_by_service(&mut self, window: &Window) -> Result<bool> {
        use crate::widgets::taskbar::taskbar_items_impl::get_taskbar_windows;

        let widget_hwnd = self.hwnd()?;
        let monitor = WindowsApi::monitor_from_window(widget_hwnd);
        let dock_size = Self::get_taskbar_size_on_monitor(monitor)?;
        let mut overlap_rect = self.webview_rect;
        overlap_rect.top = overlap_rect.bottom - dock_size;

        let candidate_hwnds: Vec<isize> = get_taskbar_windows()
            .iter()
            .map(|window| window.address())
            .collect();
        let previous_hwnd = self.overlaped_by.map(|window| window.address());

        let args = serde_json::json!({
            "widget_kind": "taskbar",
            "widget_hwnd": widget_hwnd.0 as isize,
            "trigger_hwnd": window.address(),
            "previous_hwnd": previous_hwnd,
            "overlap_rect": {
                "left": overlap_rect.left,
                "top": overlap_rect.top,
                "right": overlap_rect.right,
                "bottom": overlap_rect.bottom,
            },
            "candidate_hwnds": candidate_hwnds,
        });

        let data = match ServicePipe::request_with_response_blocking(
            slu_ipc::messages::SvcAction::ExecuteBackendCommand {
                command: "check_widget_overlap".to_string(),
                args,
            },
            std::time::Duration::from_millis(120),
        ) {
            Ok(Some(data)) => data,
            Ok(None) => return Ok(false),
            Err(err) => {
                log::warn!(
                    "[Taskbar] service overlap check failed, falling back to UI path: {:?}",
                    err
                );
                return Ok(false);
            }
        };

        let value: serde_json::Value = serde_json::from_str(&data)?;
        let overlapped = value
            .get("overlapped")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let overlapped_by = if overlapped {
            value
                .get("hwnd")
                .and_then(|value| value.as_i64())
                .map(|hwnd| Window::from(hwnd as isize))
        } else {
            None
        };
        self.set_overlaped(overlapped_by)?;
        Ok(true)
    }

    pub fn get_taskbar_size_on_monitor(monitor: HMONITOR) -> Result<i32> {
        let state = FULL_STATE.load();
        let settings = &state.settings.taskbar;
        let monitor_dpi = WindowsApi::get_monitor_scale_factor(monitor)?;
        let text_scale_factor = WindowsApi::get_text_scale_factor()?;
        let total_size = (settings.total_size() as f64 * monitor_dpi * text_scale_factor) as i32;
        Ok(total_size)
    }

    pub fn set_position(&mut self, monitor: HMONITOR) -> Result<()> {
        log::info!("[Taskbar::set_position] start");

        let step = std::time::Instant::now();
        let hwnd = HWND(self.hwnd()?.0);
        log::info!(
            "[Taskbar::set_position] hwnd resolved in {:.3}s, hwnd={:?}",
            step.elapsed().as_secs_f64(),
            hwnd
        );

        self.set_position_with_hwnd(monitor, hwnd)
    }

    pub fn set_position_with_hwnd(&mut self, monitor: HMONITOR, hwnd: HWND) -> Result<()> {
        let monitor_info = WindowsApi::monitor_info(monitor)?;
        let monitor_dpi = WindowsApi::get_monitor_scale_factor(monitor)?;

        self.webview_rect = monitor_info.monitorInfo.rcMonitor;

        let dock_size = Self::get_taskbar_size_on_monitor(monitor)?;

        // Taskbar 与屏幕底部的间距
        self.webview_rect.bottom = monitor_info.monitorInfo.rcMonitor.bottom - Self::BOTTOM_MARGIN;
        // dock栏图标放大上移时会超出dock容器边界，所以这里额外减去 20 像素
        self.webview_rect.top = self.webview_rect.bottom - dock_size - (20.0 * monitor_dpi) as i32;

        let screen_center_x = (monitor_info.monitorInfo.rcMonitor.left
            + monitor_info.monitorInfo.rcMonitor.right)
            / 2;

        // 设置窗口位置
        WindowsApi::move_window(hwnd, &self.webview_rect)?;

        WindowsApi::set_position(hwnd, None, &self.webview_rect, SWP_ASYNCWINDOWPOS)?;

        // 初始化或同步移动玻璃窗口
        self.ensure_glass_effect(&self.webview_rect.clone(), hwnd, monitor_dpi);

        // Notify frontend to refresh container position with screen center X and DPI
        // DPI is multiplied by 100 to avoid floating point issues
        // set position之后通知前端刷新亚克力和容器位置
        let dpi_times_100 = (monitor_dpi * 100.0) as u32;
        self.window.emit_to(
            self.window.label(),
            FuncEvent::TaskbarContainerRefresh,
            (screen_center_x, dpi_times_100),
        )?;
        Ok(())
    }

    pub fn emit_container_refresh_for_monitor(&mut self, monitor: HMONITOR) -> Result<()> {
        let monitor_info = WindowsApi::monitor_info(monitor)?;
        let monitor_dpi = WindowsApi::get_monitor_scale_factor(monitor)?;

        self.webview_rect = monitor_info.monitorInfo.rcMonitor;
        let dock_size = Self::get_taskbar_size_on_monitor(monitor)?;
        self.webview_rect.bottom = monitor_info.monitorInfo.rcMonitor.bottom - Self::BOTTOM_MARGIN;
        self.webview_rect.top = self.webview_rect.bottom - dock_size - (20.0 * monitor_dpi) as i32;

        let screen_center_x = (monitor_info.monitorInfo.rcMonitor.left
            + monitor_info.monitorInfo.rcMonitor.right)
            / 2;
        let dpi_times_100 = (monitor_dpi * 100.0) as u32;
        log::info!(
            "[Taskbar] emit container refresh without hwnd label={}, center_x={}, dpi={}, rect={:?}",
            self.window.label(),
            screen_center_x,
            monitor_dpi,
            self.webview_rect
        );
        self.window.emit_to(
            self.window.label(),
            FuncEvent::TaskbarContainerRefresh,
            (screen_center_x, dpi_times_100),
        )?;
        Ok(())
    }

    /// 确保玻璃效果子窗口已创建并同步尺寸
    fn ensure_glass_effect(&mut self, rect: &RECT, taskbar_hwnd: HWND, monitor_dpi: f64) {
        let win_w = rect.right - rect.left;
        let win_h = rect.bottom - rect.top;
        let top_padding = (20.0 * monitor_dpi) as i32;
        let bottom_padding = (Self::CONTAINER_BOTTOM_MARGIN_CSS as f64 * monitor_dpi) as i32;

        if self.glass_effect.is_none() {
            // 首次创建：子窗口覆盖整个父窗口，模糊区域排除顶部 padding 和底部间距
            match TaskbarGlassEffect::new(
                taskbar_hwnd,
                win_w,
                win_h,
                0.0,
                top_padding as f32,
                win_w as f32,
                (win_h - top_padding - bottom_padding) as f32,
                18.0 * monitor_dpi as f32,
            ) {
                Ok(glass) => {
                    log::info!("[Taskbar] 玻璃效果子窗口已创建");
                    self.sync_new_glass_visibility(&glass);
                    self.glass_effect = Some(glass);
                    self.last_dpi = monitor_dpi;
                    self.last_glass_size = (win_w, win_h);
                }
                Err(e) => {
                    log::warn!("[Taskbar] 玻璃效果创建失败（非致命）: {e:?}");
                }
            }
        } else {
            // glass 已存在：检查是否需要重建
            // DesktopWindowTarget 的 Composition 坐标系与创建时的显示上下文绑定，
            // DPI 变化或分辨率变化（窗口尺寸改变）后，仅 resize 不会更新坐标系，
            // 导致后续 blur offset 被旧上下文错误解释（偏移 ~28px）。
            let dpi_changed = self.last_dpi > 0.0 && (self.last_dpi - monitor_dpi).abs() > 0.001;
            let (last_w, last_h) = self.last_glass_size;
            let size_changed = last_w > 0 && last_h > 0 && (last_w != win_w || last_h != win_h);

            if dpi_changed || size_changed {
                // 重建 glass effect 以获取正确的 Composition 坐标上下文
                self.glass_effect = None;
                match TaskbarGlassEffect::new(
                    taskbar_hwnd,
                    win_w,
                    win_h,
                    0.0,
                    top_padding as f32,
                    win_w as f32,
                    (win_h - top_padding - bottom_padding) as f32,
                    18.0 * monitor_dpi as f32,
                ) {
                    Ok(glass) => {
                        log::info!("[Taskbar] 玻璃效果子窗口已重建");
                        self.sync_new_glass_visibility(&glass);
                        self.glass_effect = Some(glass);
                        self.last_dpi = monitor_dpi;
                        self.last_glass_size = (win_w, win_h);
                    }
                    Err(e) => {
                        log::warn!("[Taskbar] 玻璃效果重建失败（非致命）: {e:?}");
                    }
                }
            } else {
                // 同 DPI 同尺寸：无需任何操作（blur 由前端独占管理）
            }
        }
    }

    /// A recreated WS_CHILD glass window starts as WS_VISIBLE. If the React dock is
    /// already auto-hidden, no new CSS transition will fire to hide the child again.
    fn sync_new_glass_visibility(&self, glass: &TaskbarGlassEffect) {
        if !self.hidden {
            return;
        }

        let parent_visible = self.window.is_visible().unwrap_or(false);
        if parent_visible {
            log::info!("[Taskbar] Dock is hidden; hiding newly created glass child window");
            glass.hide();
        }
    }

    pub fn reposition_if_needed(&mut self) -> Result<()> {
        let hwnd = self.hwnd()?;
        let outer_rect = WindowsApi::get_outer_window_rect(hwnd)?;
        if self.webview_rect == outer_rect {
            return Ok(()); // position is ok no need to reposition
        }
        log::info!(
            "[reposition_if_needed] Webview rect: {:?}, Outer rect: {:?}, repositioning",
            self.webview_rect,
            outer_rect
        );
        self.set_position(WindowsApi::monitor_from_window(hwnd))?;
        Ok(())
    }
}
