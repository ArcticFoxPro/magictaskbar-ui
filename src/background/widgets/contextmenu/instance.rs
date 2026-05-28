use std::sync::Mutex;

use base64::Engine;
use tauri::{Emitter, WebviewWindow, Wry};
use windows::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Dwm::DWM_WINDOW_CORNER_PREFERENCE,
    UI::WindowsAndMessaging::SWP_ASYNCWINDOWPOS,
};

use crate::{
    app::get_app_handle, error::Result, log_error,
    modules::system_settings::infrastructure::current_is_dark_mode, widgets::WebviewArgs,
    windows_api::WindowsApi,
};

// ==================== ContextMenu 窗口实例 ====================

/// ContextMenu 窗口 - 用于显示右键菜单的独立弹出窗口
pub struct ContextMenu {
    pub window: WebviewWindow<Wry>,
    pub rect: RECT,
    pub visible: bool,
}

impl Drop for ContextMenu {
    fn drop(&mut self) {
        log::info!(
            "[ContextMenu] Dropping ContextMenu: {}",
            self.window.label()
        );
        log_error!(self.window.destroy());
    }
}

impl ContextMenu {
    pub const TITLE: &'static str = "MagicContextMenu";
    pub const TARGET: &'static str = "@magic/contextmenu";

    pub fn hwnd(&self) -> Result<HWND> {
        Ok(HWND(self.window.hwnd()?.0))
    }

    fn create_window(monitor_id: &str) -> Result<WebviewWindow> {
        let manager = get_app_handle();
        let label = format!("{}?monitorId={}", Self::TARGET, monitor_id);
        let args = WebviewArgs::new().disable_gpu();

        log::info!("[ContextMenu] Creating window: {}", label);
        let label = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&label);

        let window = tauri::WebviewWindowBuilder::new(
            manager,
            label,
            tauri::WebviewUrl::App("contextmenu/index.html".into()),
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
                "[ContextMenu] Failed to create contextmenu webview for monitor_id={monitor_id}: {e:?}"
            );
            e
        })?;

        window.set_ignore_cursor_events(false)?;

        // 应用亚克力效果和窗口样式
        if let Ok(raw) = window.hwnd() {
            let hwnd = HWND(raw.0);

            // 1. 应用亚克力效果（根据系统主题选色）
            let gradient_color = if current_is_dark_mode() {
                0xA8303030u32 // 深色: 66% 深色磨砂
            } else {
                0xA8FDFBFAu32 // 浅色: 66% 白色磨砂
            };
            let _ = WindowsApi::apply_acrylic_effect(hwnd, Some(gradient_color));

            unsafe {
                // 2. 移除窗口阴影
                use windows::Win32::UI::WindowsAndMessaging::{
                    GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_DLGMODALFRAME,
                };

                let mut ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                ex_style |= WS_EX_DLGMODALFRAME.0 as isize;
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style);
            }

            // 3. 设置窗口圆角 (Windows 11+)
            let _ = WindowsApi::set_window_corner_preference(hwnd, DWM_WINDOW_CORNER_PREFERENCE(2));
            // 隐藏边框
            let _ = WindowsApi::set_window_border_color(hwnd, 0xFFFFFFFE);
        }

        Ok(window)
    }

    pub fn new(monitor_id: &str) -> Result<Self> {
        log::info!(
            "[ContextMenu] Creating ContextMenu instance for monitor_id={}",
            monitor_id
        );

        let window = Self::create_window(monitor_id)?;
        let contextmenu = Self {
            window,
            rect: RECT::default(),
            visible: false,
        };
        log::info!(
            "[ContextMenu] ContextMenu instance created successfully: {}",
            contextmenu.window.label()
        );
        Ok(contextmenu)
    }

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
            "[ContextMenu] Position set to: x={}, y={}, width={}, height={}",
            x,
            y,
            width,
            height
        );

        Ok(())
    }

    pub fn show(&mut self) -> Result<()> {
        if !self.visible {
            self.window.show()?;
            self.visible = true;
            log::debug!("[ContextMenu] Window shown");
        }
        Ok(())
    }

    pub fn hide(&mut self) -> Result<()> {
        if self.visible {
            self.window.hide()?;
            self.visible = false;
            log::debug!("[ContextMenu] Window hidden");
        }
        Ok(())
    }
}

// ==================== 全局 ContextMenu 管理器（懒创建 + 延迟销毁） ====================

#[derive(PartialEq, Debug)]
pub enum ContextMenuState {
    /// 无窗口实例
    Idle,
    /// 正在创建窗口（WebView 尚未就绪）
    Creating,
    /// 窗口已就绪，可以接收事件
    Ready,
}

pub struct ContextMenuManager {
    state: ContextMenuState,
    instance: Option<ContextMenu>,
    /// 待发送的 payload（JSON），窗口就绪后发送
    pending_payload: Option<serde_json::Value>,
    active_monitor_id: Option<String>,
}

pub static CONTEXTMENU_MANAGER: Mutex<ContextMenuManager> = Mutex::new(ContextMenuManager {
    state: ContextMenuState::Idle,
    instance: None,
    pending_payload: None,
    active_monitor_id: None,
});

/// 触发右键菜单：存储 payload，懒创建窗口，就绪后发送事件
pub fn contextmenu_manager_trigger(payload: serde_json::Value) -> Result<()> {
    let monitor_id = payload
        .get("monitorId")
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    let needs_create;
    {
        let mut mgr = CONTEXTMENU_MANAGER
            .lock()
            .map_err(|e| format!("Lock poisoned: {e}"))?;

        match mgr.state {
            ContextMenuState::Ready => {
                mgr.active_monitor_id = monitor_id;
                // 窗口已就绪，直接 emit
                if let Some(instance) = &mgr.instance {
                    log::info!("[ContextMenu] Window ready, emitting show event directly");
                    get_app_handle().emit_to(
                        instance.window.label(),
                        "contextmenu::show",
                        &payload,
                    )?;
                }
                mgr.pending_payload = None;
                return Ok(());
            }
            ContextMenuState::Creating => {
                mgr.active_monitor_id = monitor_id;
                // 正在创建中，只更新 pending payload
                log::info!("[ContextMenu] Window creating, storing pending payload");
                mgr.pending_payload = Some(payload);
                return Ok(());
            }
            ContextMenuState::Idle => {
                mgr.active_monitor_id = monitor_id;
                // 需要创建窗口
                log::info!("[ContextMenu] Window idle, will create lazily");
                mgr.state = ContextMenuState::Creating;
                mgr.pending_payload = Some(payload);
                needs_create = true;
            }
        }
    } // 释放锁

    if needs_create {
        // 在锁外创建窗口（避免长时间持锁）
        match ContextMenu::new("global") {
            Ok(instance) => {
                let mut mgr = CONTEXTMENU_MANAGER
                    .lock()
                    .map_err(|e| format!("Lock poisoned: {e}"))?;
                log::info!("[ContextMenu] Window created, waiting for frontend ready signal");
                mgr.instance = Some(instance);
                // 不设 Ready —— 等前端调用 contextmenu_ready
            }
            Err(e) => {
                let mut mgr = CONTEXTMENU_MANAGER
                    .lock()
                    .map_err(|e| format!("Lock poisoned: {e}"))?;
                mgr.state = ContextMenuState::Idle;
                mgr.pending_payload = None;
                mgr.active_monitor_id = None;
                log::error!("[ContextMenu] Failed to create window: {e:?}");
                return Err(e);
            }
        }
    }

    Ok(())
}

/// 前端挂载完成后调用，标记就绪并发送 pending payload
pub fn contextmenu_manager_ready() -> Result<()> {
    let mut mgr = CONTEXTMENU_MANAGER
        .lock()
        .map_err(|e| format!("Lock poisoned: {e}"))?;
    mgr.state = ContextMenuState::Ready;
    log::info!("[ContextMenu] Frontend ready");

    // 窗口就绪后立即同步主题，并更新亚克力
    if let Some(instance) = &mgr.instance {
        let is_dark = current_is_dark_mode();
        if let Ok(raw) = instance.window.hwnd() {
            let hwnd = HWND(raw.0);
            let gradient_color = if is_dark {
                0xA8303030u32
            } else {
                0xA8FDFBFAu32
            };
            let _ = WindowsApi::apply_acrylic_effect(hwnd, Some(gradient_color));
        }
        let _ = get_app_handle().emit_to(
            instance.window.label(),
            "theme::changed",
            serde_json::json!({ "is_dark": is_dark }),
        );
    }

    // 发送 pending payload
    if let Some(payload) = mgr.pending_payload.take() {
        if let Some(instance) = &mgr.instance {
            log::info!("[ContextMenu] Sending pending payload to window");
            get_app_handle().emit_to(instance.window.label(), "contextmenu::show", &payload)?;
        }
    }

    Ok(())
}

/// 销毁窗口，重置为 Idle 状态
pub fn contextmenu_manager_destroy() -> Result<()> {
    let mut mgr = CONTEXTMENU_MANAGER
        .lock()
        .map_err(|e| format!("Lock poisoned: {e}"))?;
    log::info!("[ContextMenu] Destroying window (lazy cleanup)");
    mgr.instance = None; // Drop 触发 window.destroy()
    mgr.state = ContextMenuState::Idle;
    mgr.pending_payload = None;
    mgr.active_monitor_id = None;
    Ok(())
}

/// 设置窗口位置（从前端 contextmenu 窗口调用）
pub fn contextmenu_manager_set_position(x: i32, y: i32, width: i32, height: i32) -> Result<()> {
    let mut mgr = CONTEXTMENU_MANAGER
        .lock()
        .map_err(|e| format!("Lock poisoned: {e}"))?;
    if let Some(instance) = &mut mgr.instance {
        instance.set_position(x, y, width, height)?;
    }
    Ok(())
}

/// 显示窗口
pub fn contextmenu_manager_show() -> Result<()> {
    let mut mgr = CONTEXTMENU_MANAGER
        .lock()
        .map_err(|e| format!("Lock poisoned: {e}"))?;
    let monitor_id = mgr.active_monitor_id.clone();
    if let Some(instance) = &mut mgr.instance {
        instance.show()?;

        // 通知前端窗口已打开（广播到所有 taskbar 窗口）
        let _ = get_app_handle().emit(
            "contextmenu::window_open",
            serde_json::json!({ "open": true, "monitorId": monitor_id }),
        );
    }
    Ok(())
}

/// 隐藏窗口
pub fn contextmenu_manager_hide() -> Result<()> {
    let mut mgr = CONTEXTMENU_MANAGER
        .lock()
        .map_err(|e| format!("Lock poisoned: {e}"))?;
    let monitor_id = mgr.active_monitor_id.clone();
    if let Some(instance) = &mut mgr.instance {
        instance.hide()?;

        // 通知前端窗口已关闭（广播到所有 taskbar 窗口）
        let _ = get_app_handle().emit(
            "contextmenu::window_open",
            serde_json::json!({ "open": false, "monitorId": monitor_id }),
        );
    }
    Ok(())
}
