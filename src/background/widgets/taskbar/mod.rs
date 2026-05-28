pub mod cli;
pub mod glass_effect;
pub mod handler;
pub mod hook;
pub mod instance;
pub mod taskbar_items_impl;

pub use instance::Taskbar;

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use image::{DynamicImage, RgbaImage};
use taskbar_items_impl::TASKBAR_STATE;
use win_screenshot::capture::capture_window;
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL},
};

use crate::{
    error::Result,
    state::application::FULL_STATE,
    trace_lock,
    utils::sleep_millis,
    windows_api::{window::Window, AppBarData, AppBarDataState, WindowEnumerator, WindowsApi},
};

static IS_HIDING_TASKBAR: AtomicBool = AtomicBool::new(false);

impl Taskbar {
    pub fn contains_app(window: &Window) -> bool {
        trace_lock!(TASKBAR_STATE).contains(window)
    }

    pub fn foregrounded_app(window: &Window) -> Result<()> {
        let mut taskbar = trace_lock!(TASKBAR_STATE);
        taskbar.update_window_activation(window);
        taskbar.emit_to_webview()?;
        Ok(())
    }

    pub fn update_app(window: &Window) -> Result<()> {
        let mut taskbar = trace_lock!(TASKBAR_STATE);
        taskbar.update_window_info(window);
        taskbar.emit_to_webview()?;
        Ok(())
    }

    pub fn emit_hide_indicator(process_name: &str) {
        use crate::app::get_app_handle;
        use tauri::Emitter;

        log::info!(
            "[PrelaunchApp] Emitting hide_indicator event for process: '{}'",
            process_name
        );

        let handle = get_app_handle();
        match handle.emit("prelaunch::hide_indicator", process_name) {
            Ok(_) => log::info!(
                "[PrelaunchApp] Successfully emitted hide_indicator for: '{}'",
                process_name
            ),
            Err(e) => log::error!("[PrelaunchApp] Failed to emit hide_indicator: {:?}", e),
        }
    }

    pub fn capture_window(hwnd: HWND) -> Option<DynamicImage> {
        capture_window(hwnd.0 as isize).ok().map(|buf| {
            let image = RgbaImage::from_raw(buf.width, buf.height, buf.pixels).unwrap_or_default();
            DynamicImage::ImageRgba8(image)
        })
    }
}

// ====================
// TASKBAR HIDDEN LOGIC
// ====================

pub static TASKBAR_CLASS: [&str; 2] = ["Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

pub fn get_taskbars_handles() -> Result<Vec<HWND>> {
    let mut founds = Vec::new();
    WindowEnumerator::new().for_each(|w| {
        if TASKBAR_CLASS.contains(&w.class().as_str()) {
            founds.push(w.hwnd());
        }
    })?;
    Ok(founds)
}

impl Taskbar {
    pub fn hide_taskbar() -> Option<JoinHandle<()>> {
        // 如果已经在执行隐藏操作，则直接返回，避免线程爆炸
        if IS_HIDING_TASKBAR.swap(true, Ordering::SeqCst) {
            return None;
        }

        Some(std::thread::spawn(move || {
            match get_taskbars_handles() {
                Ok(handles) => {
                    if FULL_STATE.load().is_taskbar_enabled() {
                        for handle in &handles {
                            let app_bar = AppBarData::from_handle(*handle);
                            app_bar.set_state(AppBarDataState::AutoHide);
                            let _ = WindowsApi::show_window_async(*handle, SW_HIDE);
                        }
                    }
                }
                Err(err) => log::error!("Failed to get taskbars handles: {err:?}"),
            }
            // 任务完成，重置标志位
            IS_HIDING_TASKBAR.store(false, Ordering::SeqCst);
        }))
    }

    pub fn restore_taskbar() -> Result<()> {
        for hwnd in get_taskbars_handles()? {
            AppBarData::from_handle(hwnd).set_state(AppBarDataState::AlwaysOnTop);
            WindowsApi::show_window_async(hwnd, SW_SHOWNORMAL)?;
        }
        Ok(())
    }

    /// 检查所有 Taskbar 实例的窗口重叠状态
    /// 由 UIA visual_state_listener 在收到最大化/恢复事件时调用
    pub fn check_overlap_for_all_taskbars(window: &crate::windows_api::window::Window) {
        use crate::app::APP_MANAGER;

        // 死锁修复：短暂持 APP_MANAGER::read 仅克隆 taskbar Arc 列表后立即释放，
        // 避免在内层 instance.taskbar lock 期间持有外层 APP_MANAGER 锁造成嵌套死锁。
        let taskbars: Vec<
            std::sync::Arc<parking_lot::Mutex<Option<crate::widgets::taskbar::Taskbar>>>,
        > = {
            let Some(manager) = APP_MANAGER.try_read_for(std::time::Duration::from_millis(100))
            else {
                log::warn!(
                    "[UIA VisualState] skip taskbar overlap check: APP_MANAGER write lock busy, hwnd={:?}",
                    window.address()
                );
                crate::hook::request_foreground_reprocess("taskbar_overlap_app_manager_busy");
                return;
            };
            manager
                .instances
                .iter()
                .map(|i| i.taskbar.clone())
                .collect()
        };

        for tb_arc in &taskbars {
            let Some(mut taskbar) = tb_arc.try_lock_for(std::time::Duration::from_millis(100))
            else {
                log::warn!(
                    "[UIA VisualState] skip taskbar overlap check: taskbar lock busy, hwnd={:?}",
                    window.address()
                );
                continue;
            };

            if let Some(tb) = taskbar.as_mut() {
                if let Err(e) = tb.handle_overlaped_status(window) {
                    log::error!("Failed to check overlap status: {e:?}");
                }
            }
        }
    }

    pub fn ensure_taskbar_hidden_on_startup() -> JoinHandle<()> {
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let duration = std::time::Duration::from_secs(30);
            while start.elapsed() < duration {
                match get_taskbars_handles() {
                    Ok(handles) => {
                        for handle in &handles {
                            let app_bar = AppBarData::from_handle(*handle);
                            app_bar.set_state(AppBarDataState::AutoHide);
                            let _ = WindowsApi::show_window_async(*handle, SW_HIDE);
                        }
                    }
                    Err(err) => log::error!("Failed to get taskbars handles: {err:?}"),
                }
                sleep_millis(500);
            }
        })
    }
}
