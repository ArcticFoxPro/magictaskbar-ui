pub mod hook;

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use crate::{
    app::get_app_handle,
    cli::ServicePipe,
    error::Result,
    log_error,
    state::application::FULL_STATE,
    widgets::WebviewArgs,
    windows_api::{window::Window, AppBarData, NativeAppBarWindow, WindowsApi},
};
use base64::Engine;
use libs_core::{
    handlers::FuncEvent,
    state::{FancyToolbarSide, HideMode},
};
use serde::Serialize;
use slu_ipc::messages::SvcAction;
use tauri::{Emitter, WebviewWindow};
use windows::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Gdi::HMONITOR,
    UI::WindowsAndMessaging::{
        RegisterWindowMessageW, SWP_ASYNCWINDOWPOS, SW_HIDE, SW_SHOWNOACTIVATE,
    },
};

fn request_gpu_wake() {
    let _ = ServicePipe::request(SvcAction::ExecuteBackendCommand {
        command: "gpu_wake".to_string(),
        args: serde_json::json!({}),
    });
}

static TOOLBAR_APPBAR_CALLBACK_MSG: LazyLock<u32> = LazyLock::new(|| unsafe {
    RegisterWindowMessageW(windows::core::w!("MagicTaskbar.Toolbar.AppBarCallback"))
});

pub fn schedule_initial_reposition(toolbar: Arc<parking_lot::Mutex<Option<FancyToolbar>>>) {
    let toolbar_ptr = Arc::into_raw(toolbar) as usize;
    std::thread::spawn(move || {
        let toolbar: Arc<parking_lot::Mutex<Option<FancyToolbar>>> =
            unsafe { Arc::from_raw(toolbar_ptr as *const _) };

        for delay in [
            std::time::Duration::from_millis(300),
            std::time::Duration::from_millis(1500),
            std::time::Duration::from_millis(4000),
            std::time::Duration::from_millis(8000),
        ] {
            std::thread::sleep(delay);

            let Some(mut guard) = toolbar.try_lock_for(std::time::Duration::from_millis(200))
            else {
                log::warn!(
                    target: "toolbar",
                    "Initial delayed toolbar reposition skipped: toolbar lock busy"
                );
                continue;
            };

            if let Some(tl) = guard.as_mut() {
                if let Err(e) = tl.reposition_if_needed() {
                    log::error!(
                        target: "toolbar",
                        "Initial delayed toolbar reposition failed: {e:?}"
                    );
                }
            }
        }
    });
}

pub fn schedule_deferred_fixed_appbar_restore(
    toolbar: Arc<parking_lot::Mutex<Option<FancyToolbar>>>,
    delay: Duration,
) {
    let toolbar_ptr = Arc::into_raw(toolbar) as usize;
    std::thread::spawn(move || {
        std::thread::sleep(delay);

        let toolbar: Arc<parking_lot::Mutex<Option<FancyToolbar>>> =
            unsafe { Arc::from_raw(toolbar_ptr as *const _) };
        let Some(mut guard) = toolbar.try_lock_for(Duration::from_millis(500)) else {
            log::warn!(
                target: "toolbar",
                "Deferred fixed-mode restore skipped: toolbar lock busy"
            );
            return;
        };

        let Some(tl) = guard.as_mut() else {
            return;
        };

        tl.defer_fixed_appbar_until = None;
        tl.fixed_appbar_restore_scheduled = false;

        let result = if let Some(monitor) = tl.current_monitor {
            tl.set_position(monitor)
        } else {
            let hwnd = tl.hwnd();
            hwnd.and_then(|hwnd| tl.set_position(WindowsApi::monitor_from_window(hwnd)))
        };

        if let Err(e) = result {
            log::error!(
                target: "toolbar",
                "Deferred fixed-mode restore failed: {e:?}"
            );
        }
    });
}

// ============================================================================
// ToolbarAppBar - 閻欘剛鐝涢惃?AppBar 閸楃姳缍呯粣妤€褰涢敍鍫モ偓蹇旀閵嗕線绱堕弽鍥┾敍闁骏绱?// ============================================================================

pub struct ToolbarAppBar {
    window: NativeAppBarWindow,
    appbar_registered: bool,
    pub rect: RECT,
}

impl Drop for ToolbarAppBar {
    fn drop(&mut self) {
        log::info!("Dropping ToolbarAppBar");
        if self.appbar_registered {
            // AppBar 濞夈劑鏀㈣箛鍛淬€忛崷銊х崶閸欙綁鏀㈠В浣稿鐎瑰本鍨?            AppBarData::from_handle(self.window.hwnd()).unregister_bar();
        }
    }
}

impl ToolbarAppBar {
    pub fn hwnd(&self) -> Result<HWND> {
        Ok(self.window.hwnd())
    }

    pub fn new(monitor_id: &str) -> Result<Self> {
        log::info!(
            "Creating ToolbarAppBar (native Win32) for monitor {}",
            monitor_id
        );
        let window = NativeAppBarWindow::new(monitor_id)?;
        let _ = WindowsApi::ignore_close(window.hwnd());

        log::info!(
            "[ToolbarAppBar] Native window created successfully for monitor {}",
            monitor_id
        );
        Ok(Self {
            window,
            appbar_registered: false,
            rect: RECT::default(),
        })
    }

    pub fn register_appbar(&mut self, rect: RECT, edge: FancyToolbarSide) -> Result<()> {
        let hwnd = self.hwnd()?;
        self.rect = rect;

        log::info!(
            target: "toolbar",
            "ToolbarAppBar register_appbar called: rect={:?}, edge={:?}",
            rect,
            edge
        );

        WindowsApi::move_window(hwnd, &rect)?;
        WindowsApi::show_window_async(hwnd, SW_SHOWNOACTIVATE)?;

        let mut abd = AppBarData::from_handle(hwnd);
        abd.set_callback_message(*TOOLBAR_APPBAR_CALLBACK_MSG);
        abd.set_edge(edge.into());
        abd.set_rect(rect);
        abd.register_as_new_bar();

        self.appbar_registered = true;
        log::info!(target: "toolbar", "ToolbarAppBar registered successfully");
        Ok(())
    }

    pub fn unregister_appbar(&mut self) -> Result<()> {
        if !self.appbar_registered {
            return Ok(());
        }

        let hwnd = self.hwnd()?;
        AppBarData::from_handle(hwnd).unregister_bar();
        self.appbar_registered = false;
        WindowsApi::show_window_async(hwnd, SW_HIDE)?;

        log::info!(target: "toolbar", "ToolbarAppBar unregistered");
        Ok(())
    }

    pub fn update_rect(&mut self, rect: RECT, edge: FancyToolbarSide) -> Result<()> {
        if !self.appbar_registered {
            return self.register_appbar(rect, edge);
        }

        let hwnd = self.hwnd();
        if hwnd.is_err() {
            return self.register_appbar(rect, edge);
        }
        let hwnd = hwnd?;

        if self.rect == rect {
            return Ok(());
        }

        self.rect = rect;
        WindowsApi::move_window(hwnd, &rect)?;

        let mut abd = AppBarData::from_handle(hwnd);
        abd.set_rect(rect);
        abd.set_edge(edge.into());
        abd.register_as_new_bar();

        log::debug!(target: "toolbar", "ToolbarAppBar rect updated: {:?}", rect);
        Ok(())
    }

    pub fn is_registered(&self) -> bool {
        self.appbar_registered
    }
}

// ============================================================================
// FancyToolbar - UI 閺勫墽銇氱粣妤€褰?// ============================================================================

use crate::widgets::popup_glass_effect::PopupGlassEffect;

pub struct FancyToolbar {
    window: WebviewWindow,
    appbar: Option<ToolbarAppBar>,
    /// This is the GUI rect of the dock, not used as webview window rect
    pub theoretical_rect: RECT,
    /// This is the webview/window rect
    pub webview_rect: RECT,
    overlaped_by: Option<Window>,
    hidden: bool,
    positioned: bool,
    current_monitor: Option<HMONITOR>,
    // Caches to avoid redundant updates
    pub(crate) last_overlaped_hwnd: Option<HWND>,
    pub(crate) last_refresh_at: Option<std::time::Instant>,
    pub(crate) last_state_update_at: Option<std::time::Instant>,
    pub(crate) last_has_maximized_window: Option<bool>,
    defer_fixed_appbar_until: Option<Instant>,
    fixed_appbar_restore_scheduled: bool,
    // HWNDS of windows we forced to TOPMOST to avoid being covered by other apps
    // Instance-level processing guard to avoid re-entrancy
    pub(crate) is_processing: bool,
    /// Popup 閻滆崵鎷戝Ο锛勭ˇ閺佸牊鐏夐敍鍫熸暜閹镐礁顦挎稉顏勬倱閺冭泛鐡ㄩ崷顭掔礆
    pub popup_glasses: HashMap<String, PopupGlassEffect>,
}

impl Drop for FancyToolbar {
    fn drop(&mut self) {
        log::info!("Dropping {}", self.window.label());
        // AppBar cleanup is handled by the independent appbar window.
        log_error!(self.window.destroy());
    }
}

impl FancyToolbar {
    pub fn hwnd(&self) -> Result<HWND> {
        Ok(HWND(self.window.hwnd()?.0))
    }

    pub fn window_label(&self) -> &str {
        self.window.label()
    }

    pub fn is_overlaped(&self) -> bool {
        self.overlaped_by.is_some()
    }

    pub fn is_positioned(&self) -> bool {
        self.positioned
    }

    pub fn window(&self) -> WebviewWindow {
        self.window.clone()
    }

    pub fn set_topmost(&self, on: bool) {
        let _ = self.window.set_always_on_top(on);
    }

    pub fn new(monitor: &str) -> Result<Self> {
        match Self::create_window(monitor) {
            Ok(window) => {
                // Register WebView2 ProcessFailed handler - notifies srv to restart UI on crash
                crate::webview_recovery::register_process_failed_handler(&window, "Toolbar");

                let appbar = match ToolbarAppBar::new(monitor) {
                    Ok(ab) => Some(ab),
                    Err(e) => {
                        log::error!("[Toolbar] Failed to create ToolbarAppBar: {:?}", e);
                        None
                    }
                };
                let defer_fixed_appbar_until =
                    if FULL_STATE.load().settings.by_widget.fancy_toolbar.hide_mode
                        == HideMode::Never
                    {
                        Some(Instant::now() + Duration::from_secs(6))
                    } else {
                        None
                    };

                Ok(Self {
                    window,
                    appbar,
                    theoretical_rect: RECT::default(),
                    webview_rect: RECT::default(),
                    overlaped_by: None,
                    hidden: true,
                    positioned: false,
                    current_monitor: None,
                    last_overlaped_hwnd: None,
                    last_refresh_at: None,
                    last_state_update_at: None,
                    last_has_maximized_window: None,
                    defer_fixed_appbar_until,
                    fixed_appbar_restore_scheduled: false,
                    is_processing: false,
                    popup_glasses: HashMap::new(),
                })
            }
            Err(e) => {
                log::error!(
                    "[Toolbar] Failed to create FancyToolbar webview for monitor_id={monitor}: {e:?}"
                );
                Err(e)
            }
        }
    }

    pub fn emit<S: Serialize + Clone>(&self, event: &str, payload: S) -> Result<()> {
        self.window.emit_to(self.window.label(), event, payload)?;
        Ok(())
    }

    pub fn set_current_monitor_hint(&mut self, monitor: HMONITOR) {
        self.current_monitor = Some(monitor);
    }

    fn should_defer_fixed_appbar(&self, monitor: HMONITOR) -> bool {
        if monitor == WindowsApi::primary_monitor() {
            return false;
        }

        self.defer_fixed_appbar_until
            .map(|until| Instant::now() < until)
            .unwrap_or(false)
    }

    pub fn take_deferred_fixed_appbar_restore_delay(
        &mut self,
        monitor: HMONITOR,
    ) -> Option<Duration> {
        if self.fixed_appbar_restore_scheduled || !self.should_defer_fixed_appbar(monitor) {
            return None;
        }

        let delay = self
            .defer_fixed_appbar_until
            .map(|until| until.saturating_duration_since(Instant::now()))?;
        self.fixed_appbar_restore_scheduled = true;
        Some(delay)
    }

    pub fn set_overlaped(&mut self, overlaped_by: Option<Window>) -> Result<()> {
        let prev = self.overlaped_by.is_some();
        let curr = overlaped_by.is_some();
        if prev != curr {
            if curr {
                let exe = overlaped_by
                    .as_ref()
                    .and_then(|w| w.process().program_exe_name().ok())
                    .unwrap_or_else(|| "<none>".to_string());
                log::info!(target: "toolbar", "[set_overlaped] Overlap toggled -> emitting true, exe={}", exe);
            } else {
                log::info!(target: "toolbar", "[set_overlaped] Overlap toggled -> emitting false");
            }
            self.emit(FuncEvent::ToolbarOverlaped, curr)?;
        } else {
            log::debug!(
                target: "toolbar",
                "[set_overlaped] state unchanged (overlaped={}), skip emit",
                curr
            );
        }
        self.overlaped_by = overlaped_by;
        Ok(())
    }

    pub fn handle_overlaped_status(&mut self, window: &Window) -> Result<()> {
        self.handle_overlaped_status_by_service(window)
    }

    fn handle_overlaped_status_by_service(&mut self, window: &Window) -> Result<()> {
        use crate::widgets::taskbar::taskbar_items_impl::get_taskbar_windows;

        let widget_hwnd = self.hwnd()?;
        let candidate_hwnds: Vec<isize> = get_taskbar_windows()
            .iter()
            .map(|window| window.address())
            .collect();
        let previous_hwnd = self.overlaped_by.map(|window| window.address());

        let args = serde_json::json!({
            "widget_kind": "toolbar",
            "widget_hwnd": widget_hwnd.0 as isize,
            "trigger_hwnd": window.address(),
            "previous_hwnd": previous_hwnd,
            "overlap_rect": {
                "left": self.theoretical_rect.left,
                "top": self.theoretical_rect.top,
                "right": self.theoretical_rect.right,
                "bottom": self.theoretical_rect.bottom,
            },
            "candidate_hwnds": candidate_hwnds,
        });

        let data = ServicePipe::request_with_response_blocking(
            slu_ipc::messages::SvcAction::ExecuteBackendCommand {
                command: "check_widget_overlap".to_string(),
                args,
            },
            std::time::Duration::from_millis(200),
        )?
        .ok_or("check_widget_overlap returned no data")?;

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
        Ok(())
    }

    pub fn hide(&mut self) -> Result<()> {
        let hwnd = self.hwnd()?;
        if self.hidden {
            if !WindowsApi::is_window_visible(hwnd) {
                log::debug!(target: "toolbar", "hide: no-op (already hidden) label={}", self.window.label());
                return Ok(());
            }
            log::warn!(
                target: "toolbar",
                "hide: state says hidden but window is visible; forcing SW_HIDE label={}",
                self.window.label()
            );
        }
        log::info!(target: "toolbar", "hide: SW_HIDE + HandleLayeredHitboxes=false label={}", self.window.label());
        WindowsApi::show_window_async(hwnd, SW_HIDE)?;
        self.hidden = true;
        self.window
            .emit_to(self.window.label(), FuncEvent::HandleLayeredHitboxes, false)?;
        Ok(())
    }

    pub fn show(&mut self) -> Result<()> {
        let hwnd = self.hwnd()?;
        if !self.hidden {
            if WindowsApi::is_window_visible(hwnd) {
                log::debug!(target: "toolbar", "show: no-op (already visible) label={}", self.window.label());
                return Ok(());
            }
            log::warn!(
                target: "toolbar",
                "show: state says visible but window is not visible; forcing SW_SHOWNOACTIVATE label={}",
                self.window.label()
            );
        }
        log::info!(target: "toolbar", "show: SW_SHOWNOACTIVATE + HandleLayeredHitboxes=true label={}", self.window.label());
        WindowsApi::show_window_async(hwnd, SW_SHOWNOACTIVATE)?;
        self.hidden = false;
        self.window
            .emit_to(self.window.label(), FuncEvent::HandleLayeredHitboxes, true)?;
        Ok(())
    }
}

// statics
impl FancyToolbar {
    pub const TITLE: &'static str = "HonorToolbar";
    pub const TARGET: &'static str = "@magic/fancy-toolbar";

    pub fn decoded_label(monitor_id: &str) -> String {
        format!("{}?monitorId={}", Self::TARGET, monitor_id)
    }

    pub fn label(monitor_id: &str) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Self::decoded_label(monitor_id))
    }

    fn create_window(monitor_id: &str) -> Result<WebviewWindow> {
        let manager = get_app_handle();
        let args = WebviewArgs::new().disable_gpu();

        log::info!("Creating {}", Self::decoded_label(monitor_id));

        let window = tauri::WebviewWindowBuilder::new(
            manager,
            Self::label(monitor_id),
            tauri::WebviewUrl::App("toolbar/index.html".into()),
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
        // Default not topmost; toggle in set_position according to mode
        .always_on_top(false)
        .data_directory(args.data_directory())
        .additional_browser_args(&args.to_string())
        // 閺勬儳绱＄粋浣烘暏瀵偓閸欐垵浼愰崗鍑ょ礄F12閿涘绱濋梼鍙夘剾閻劍鍩涚拠顖澬曠拫鍐ㄥ毉鐠嬪啳鐦粣妤€褰?        .devtools(false)
        .build()
        .map_err(|e| {
            log::error!(
                "[Toolbar] Failed to create toolbar webview for monitor_id={monitor_id}: {e:?}"
            );
            e
        })?;

        window.set_ignore_cursor_events(true)?;

        let window_clone = window.clone();
        std::thread::spawn(move || {
            if let Ok(raw) = window_clone.hwnd() {
                let _ = WindowsApi::ignore_close(HWND(raw.0));
            }
        });

        Ok(window)
    }

    pub fn get_toolbar_height_on_monitor(monitor: HMONITOR) -> Result<i32> {
        let state = FULL_STATE.load();
        let settings = &state.settings.by_widget.fancy_toolbar;
        let monitor_scale_factor = WindowsApi::get_monitor_scale_factor(monitor)?;
        let text_scale_factor = WindowsApi::get_text_scale_factor()?;
        Ok((settings.height as f64 * monitor_scale_factor * text_scale_factor) as i32)
    }

    fn calculate_position_rects(monitor: HMONITOR) -> Result<(RECT, RECT, FancyToolbarSide)> {
        let state = FULL_STATE.load();
        let settings = &state.settings.by_widget.fancy_toolbar;

        let monitor_info = WindowsApi::monitor_info(monitor)?;
        let rc_monitor = monitor_info.monitorInfo.rcMonitor;
        let real_height = Self::get_toolbar_height_on_monitor(monitor)?;

        let mut theoretical_rect = rc_monitor;
        let mut real_rect = rc_monitor;
        match settings.position {
            FancyToolbarSide::Top => {
                theoretical_rect.bottom = rc_monitor.top + real_height;
                real_rect.bottom -= (rc_monitor.bottom - rc_monitor.top) / 3;
            }
            FancyToolbarSide::Bottom => {
                theoretical_rect.top = rc_monitor.bottom - real_height;
                real_rect.top += (rc_monitor.bottom - rc_monitor.top) / 3;
            }
        }

        Ok((theoretical_rect, real_rect, settings.position))
    }

    pub fn reserve_appbar_position(&mut self, monitor: HMONITOR) -> Result<()> {
        let state = FULL_STATE.load();
        let settings = &state.settings.by_widget.fancy_toolbar;
        if settings.hide_mode != HideMode::Never {
            return Ok(());
        }

        let step = std::time::Instant::now();
        let (theoretical_rect, _, position) = Self::calculate_position_rects(monitor)?;
        self.theoretical_rect = theoretical_rect;
        self.current_monitor = Some(monitor);

        if let Some(ref mut appbar) = self.appbar {
            request_gpu_wake();
            appbar.update_rect(theoretical_rect, position)?;
            log::info!(
                target: "toolbar",
                "[Toolbar::reserve_appbar_position] completed in {:.3}s, rect={:?}",
                step.elapsed().as_secs_f64(),
                theoretical_rect
            );
        }

        Ok(())
    }

    pub fn set_position(&mut self, monitor: HMONITOR) -> Result<()> {
        log::info!(target: "toolbar", "[Toolbar::set_position] start");

        let step = std::time::Instant::now();
        let hwnd = HWND(self.hwnd()?.0);
        log::info!(
            target: "toolbar",
            "[Toolbar::set_position] hwnd resolved in {:.3}s, hwnd={:?}",
            step.elapsed().as_secs_f64(),
            hwnd
        );

        self.set_position_with_hwnd(monitor, hwnd)
    }

    pub fn set_position_with_hwnd(&mut self, monitor: HMONITOR, hwnd: HWND) -> Result<()> {
        let state = FULL_STATE.load();
        let settings = &state.settings.by_widget.fancy_toolbar;

        let (theoretical_rect, real_rect, position) = Self::calculate_position_rects(monitor)?;
        self.theoretical_rect = theoretical_rect;
        self.current_monitor = Some(monitor);

        let defer_fixed_appbar =
            settings.hide_mode == HideMode::Never && self.should_defer_fixed_appbar(monitor);

        match settings.hide_mode {
            HideMode::Never if defer_fixed_appbar => {
                if let Some(ref mut appbar) = self.appbar {
                    request_gpu_wake();
                    log::info!(
                        target: "toolbar",
                        "Deferring fixed AppBar registration on external monitor for 6s"
                    );
                    appbar.unregister_appbar()?;
                }
                self.set_topmost(true);
            }
            HideMode::Never => {
                self.defer_fixed_appbar_until = None;
                if let Some(ref mut appbar) = self.appbar {
                    request_gpu_wake();
                    log::debug!(target: "toolbar", "AppBar updating with rect={:?}, position={:?}", self.theoretical_rect, position);
                    appbar.update_rect(self.theoretical_rect, position)?;
                }
                self.set_topmost(false);
            }
            HideMode::OnOverlap => {
                if let Some(ref mut appbar) = self.appbar {
                    request_gpu_wake();
                    log::debug!(target: "toolbar", "AppBar unregistering (OnOverlap mode)");
                    appbar.unregister_appbar()?;
                }
                self.set_topmost(true);
            }
            HideMode::Always => {
                if let Some(ref mut appbar) = self.appbar {
                    request_gpu_wake();
                    log::debug!(target: "toolbar", "AppBar unregistering (Always mode)");
                    appbar.unregister_appbar()?;
                }
                self.set_topmost(true);
            }
        };

        WindowsApi::move_window(hwnd, &real_rect)?;
        WindowsApi::set_position(hwnd, None, &real_rect, SWP_ASYNCWINDOWPOS)?;

        self.webview_rect = real_rect;
        self.positioned = true;
        Ok(())
    }

    pub fn reposition_if_needed(&mut self) -> Result<()> {
        let hwnd = self.hwnd()?;
        let current_window_rect = WindowsApi::get_outer_window_rect(hwnd)?;
        if self.webview_rect == current_window_rect {
            log::info!(target: "toolbar", "ToolbarPosition is ok no need to reposition");
            return Ok(()); // position is ok no need to reposition
        }
        log::debug!(target: "toolbar", "XXXXXPosition is not ok need to reposition");
        self.set_position(WindowsApi::monitor_from_window(hwnd))?;
        Ok(())
    }

    pub fn reregister_appbar(&mut self) -> Result<()> {
        let state = FULL_STATE.load();
        let hide_mode = state.settings.by_widget.fancy_toolbar.hide_mode;

        if hide_mode != HideMode::Never {
            log::debug!(
                target: "toolbar",
                "reregister_appbar: skipping (hide_mode={:?})",
                hide_mode
            );
            return Ok(());
        }

        if let Some(ref mut appbar) = self.appbar {
            if appbar.is_registered() {
                log::info!(target: "toolbar", "reregister_appbar: unregistering old AppBar");
                appbar.unregister_appbar()?;
            }

            // 闁插秵鏌婂▔銊ュ斀 AppBar
            let position = state.settings.by_widget.fancy_toolbar.position;
            log::info!(
                target: "toolbar",
                "reregister_appbar: registering new AppBar with rect={:?}, position={:?}",
                self.theoretical_rect, position
            );
            appbar.register_appbar(self.theoretical_rect, position)?;
            log::info!(target: "toolbar", "reregister_appbar: AppBar re-registered successfully");
        } else {
            log::warn!(target: "toolbar", "reregister_appbar: no AppBar instance available");
        }

        Ok(())
    }
}
