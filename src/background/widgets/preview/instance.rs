use base64::Engine;
use tauri::{Emitter, Manager, WebviewWindow, Wry};
use windows::Win32::{
    Foundation::{HWND, RECT},
    Graphics::{Dwm::DWM_WINDOW_CORNER_PREFERENCE, Gdi::HMONITOR},
    UI::WindowsAndMessaging::SWP_ASYNCWINDOWPOS,
};

use crate::{
    app::get_app_handle, error::Result, log_error,
    modules::system_settings::infrastructure::current_is_dark_mode, widgets::WebviewArgs,
    windows_api::WindowsApi,
};

/// Preview 窗口 - 用于显示 Taskbar item 的窗口预览列表
pub struct Preview {
    pub window: WebviewWindow<Wry>,
    pub monitor_id: String,
    /// 窗口矩形区域
    pub rect: RECT,
    /// 是否可见
    pub visible: bool,
}

impl Drop for Preview {
    fn drop(&mut self) {
        log::info!("Dropping Preview: {}", self.window.label());
        log_error!(self.window.destroy());
    }
}

impl Preview {
    pub const TITLE: &'static str = "MagicPreview";
    pub const TARGET: &'static str = "@magic/preview";

    /// 默认窗口宽度
    const DEFAULT_WIDTH: i32 = 280;
    /// 默认窗口高度
    const DEFAULT_HEIGHT: i32 = 320;

    pub fn hwnd(&self) -> Result<HWND> {
        Ok(HWND(self.window.hwnd()?.0))
    }

    fn create_window(monitor_id: &str) -> Result<WebviewWindow> {
        let manager = get_app_handle();
        let label = format!("{}?monitorId={}", Self::TARGET, monitor_id);
        let args = WebviewArgs::new().disable_gpu();

        log::info!("[Preview] Creating window: {}", label);
        let label = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&label);

        let window = tauri::WebviewWindowBuilder::new(
            manager,
            label,
            tauri::WebviewUrl::App("preview/index.html".into()),
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
        .devtools(false)
        .build()
        .map_err(|e| {
            log::error!(
                "[Preview] Failed to create preview webview for monitor_id={monitor_id}: {e:?}"
            );
            e
        })?;

        window.set_ignore_cursor_events(false)?;

        // 异步设置窗口样式，避免 hwnd() 阻塞
        let window_clone = window.clone();
        std::thread::spawn(move || {
            if let Ok(raw) = window_clone.hwnd() {
                let hwnd = HWND(raw.0);

                // 1. 应用亚克力效果（根据系统主题选色）
                let gradient_color = if current_is_dark_mode() {
                    0xA8303030u32 // 深色: 66% 深色磨砂
                } else {
                    0xA8FDFBFAu32 // 浅色: 66% 白色磨砂
                };
                let _ = WindowsApi::apply_acrylic_effect(hwnd, Some(gradient_color));

                // 2. 设置窗口圆角 (Windows 11+)
                let _ =
                    WindowsApi::set_window_corner_preference(hwnd, DWM_WINDOW_CORNER_PREFERENCE(2));
                // 隐藏边框
                let _ = WindowsApi::set_window_border_color(hwnd, 0xFFFFFFFE);
            }
        });

        Ok(window)
    }

    pub fn new(monitor_id: &str) -> Result<Self> {
        log::info!(
            "[Preview] Creating Preview instance for monitor_id={}",
            monitor_id
        );

        match Self::create_window(monitor_id) {
            Ok(window) => {
                let preview = Self {
                    window,
                    monitor_id: monitor_id.to_string(),
                    rect: RECT::default(),
                    visible: false,
                };
                log::info!(
                    "[Preview] Preview instance created successfully: {}",
                    preview.window.label()
                );
                Ok(preview)
            }
            Err(e) => {
                log::error!(
                    "[Preview] Failed to create Preview instance for monitor_id={monitor_id}: {e:?}"
                );
                Err(e)
            }
        }
    }

    /// 设置预览窗口的位置和大小
    pub fn set_position(&mut self, x: i32, y: i32, width: i32, height: i32) -> Result<()> {
        let hwnd = self.hwnd()?;

        self.rect = RECT {
            left: x,
            top: y,
            right: x + width,
            bottom: y + height,
        };

        WindowsApi::set_position(hwnd, None, &self.rect, SWP_ASYNCWINDOWPOS)?;

        log::debug!(
            "[Preview] Position set to: x={}, y={}, width={}, height={}",
            x,
            y,
            width,
            height
        );

        Ok(())
    }

    /// 根据锚点位置和弹出方向计算并设置窗口位置
    #[allow(dead_code)]
    pub fn position_relative_to(
        &mut self,
        anchor_x: i32,
        anchor_y: i32,
        placement: &str,
        monitor: HMONITOR,
    ) -> Result<()> {
        let monitor_info = WindowsApi::monitor_info(monitor)?;
        let monitor_rect = monitor_info.monitorInfo.rcMonitor;
        let monitor_dpi = WindowsApi::get_monitor_scale_factor(monitor)?;

        let width = (Self::DEFAULT_WIDTH as f64 * monitor_dpi) as i32;
        let height = (Self::DEFAULT_HEIGHT as f64 * monitor_dpi) as i32;

        let (x, y) = match placement {
            "top" => {
                // 在锚点上方显示
                let x = (anchor_x - width / 2)
                    .max(monitor_rect.left)
                    .min(monitor_rect.right - width);
                let y = anchor_y - height - 8; // 8px 间隙
                (x, y)
            }
            "bottom" => {
                // 在锚点下方显示
                let x = (anchor_x - width / 2)
                    .max(monitor_rect.left)
                    .min(monitor_rect.right - width);
                let y = anchor_y + 8;
                (x, y)
            }
            "left" => {
                // 在锚点左侧显示
                let x = anchor_x - width - 8;
                let y = (anchor_y - height / 2)
                    .max(monitor_rect.top)
                    .min(monitor_rect.bottom - height);
                (x, y)
            }
            "right" => {
                // 在锚点右侧显示
                let x = anchor_x + 8;
                let y = (anchor_y - height / 2)
                    .max(monitor_rect.top)
                    .min(monitor_rect.bottom - height);
                (x, y)
            }
            _ => {
                // 默认在上方
                let x = (anchor_x - width / 2)
                    .max(monitor_rect.left)
                    .min(monitor_rect.right - width);
                let y = anchor_y - height - 8;
                (x, y)
            }
        };

        self.set_position(x, y, width, height)
    }

    /// 显示预览窗口
    pub fn show(&mut self) -> Result<()> {
        if !self.visible {
            self.window.show()?;
            self.visible = true;
            log::debug!("[Preview] Window shown");

            // 通知前端窗口已打开（广播到所有 taskbar 窗口）
            let _ = get_app_handle().emit(
                "preview::window_open",
                serde_json::json!({ "open": true, "monitorId": self.monitor_id }),
            );
        }
        Ok(())
    }

    /// 隐藏预览窗口
    pub fn hide(&mut self) -> Result<()> {
        if self.visible {
            self.window.hide()?;
            self.visible = false;
            log::debug!("[Preview] Window hidden");

            // 通知前端窗口已关闭（广播到所有 taskbar 窗口）
            let _ = get_app_handle().emit(
                "preview::window_open",
                serde_json::json!({ "open": false, "monitorId": self.monitor_id }),
            );
        }
        Ok(())
    }

    /// 设置预览窗口在指定显示器上的初始位置（屏幕外）
    pub fn set_initial_position(&mut self, monitor: HMONITOR) -> Result<()> {
        let monitor_info = WindowsApi::monitor_info(monitor)?;
        let monitor_rect = monitor_info.monitorInfo.rcMonitor;

        // 初始位置设置在屏幕外
        self.set_position(
            monitor_rect.left - Self::DEFAULT_WIDTH - 100,
            monitor_rect.top,
            Self::DEFAULT_WIDTH,
            Self::DEFAULT_HEIGHT,
        )
    }
}

// ==================== Preview Manager（带 pending 机制）====================

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

#[derive(PartialEq, Debug)]
pub enum PreviewState {
    /// 窗口已创建但前端尚未就绪
    NotReady,
    /// 窗口已就绪，可以接收事件
    Ready,
}

#[derive(Debug)]
struct PreviewWindowState {
    state: PreviewState,
    /// 待发送的 payload（JSON），窗口就绪后发送
    pending_payload: Option<serde_json::Value>,
}

pub struct PreviewManager {
    windows: HashMap<String, PreviewWindowState>,
}

pub static PREVIEW_MANAGER: LazyLock<Mutex<PreviewManager>> = LazyLock::new(|| {
    Mutex::new(PreviewManager {
        windows: HashMap::new(),
    })
});

/// 触发预览窗口显示：存储 payload，等窗口就绪后发送事件
pub fn preview_manager_show(preview_label: &str, payload: serde_json::Value) -> Result<()> {
    let should_emit_now = {
        let mut mgr = PREVIEW_MANAGER
            .lock()
            .map_err(|e| format!("Lock poisoned: {e}"))?;

        let window_state = mgr
            .windows
            .entry(preview_label.to_string())
            .or_insert_with(|| PreviewWindowState {
                state: PreviewState::NotReady,
                pending_payload: None,
            });

        match window_state.state {
            PreviewState::Ready => {
                window_state.pending_payload = None;
                true
            }
            PreviewState::NotReady => {
                log::info!("[Preview] Window not ready yet, storing pending payload");
                window_state.pending_payload = Some(payload.clone());
                false
            }
        }
    };

    if should_emit_now {
        log::info!("[Preview] Window ready, emitting show event directly");
        get_app_handle().emit_to(preview_label, "preview::show", &payload)?;
    }

    Ok(())
}

/// 前端挂载完成后调用，标记就绪并发送 pending payload
pub fn preview_manager_ready(preview_label: &str) -> Result<()> {
    let pending_payload = {
        let mut mgr = PREVIEW_MANAGER
            .lock()
            .map_err(|e| format!("Lock poisoned: {e}"))?;

        let window_state = mgr
            .windows
            .entry(preview_label.to_string())
            .or_insert_with(|| PreviewWindowState {
                state: PreviewState::NotReady,
                pending_payload: None,
            });

        if window_state.state != PreviewState::NotReady {
            return Ok(());
        }

        window_state.state = PreviewState::Ready;
        log::info!("[Preview] Frontend ready: {}", preview_label);
        window_state.pending_payload.take()
    };

    // 窗口就绪时同步主题和亚克力，确保两者一致
    let is_dark = current_is_dark_mode();
    if let Some(window) = get_app_handle().get_webview_window(preview_label) {
        if let Ok(raw) = window.hwnd() {
            let hwnd = HWND(raw.0);
            let gradient_color = if is_dark {
                0xA8303030u32
            } else {
                0xA8FDFBFAu32
            };
            let _ = WindowsApi::apply_acrylic_effect(hwnd, Some(gradient_color));
        }
    }
    let _ = get_app_handle().emit_to(
        preview_label,
        "theme::changed",
        serde_json::json!({ "is_dark": is_dark }),
    );

    if let Some(payload) = pending_payload {
        log::info!("[Preview] Sending pending payload to window");
        get_app_handle().emit_to(preview_label, "preview::show", &payload)?;
    }

    Ok(())
}
