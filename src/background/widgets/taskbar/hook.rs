use std::{
    collections::HashMap,
    sync::LazyLock,
    time::{Duration, Instant},
};

use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{EVENT_OBJECT_CREATE, EVENT_OBJECT_SHOW},
};

use crate::{
    error::Result, hook::update_last_real_foreground, modules::apps::application::UserAppsManager,
    windows_api::window::event::WinEvent, windows_api::window::Window,
};

use super::{Taskbar, TASKBAR_CLASS};

const MAGICANIMATION_REG_PATH: &str = r"Software\HONOR\Magicanimation";
const PRELAUNCH_APP_KEY: &str = "PrelaunchApp";
const EVENT_OBJECT_UNCLOAKED: u32 = 0x8018;
const DEBOUNCE_INTERVAL_MS: u64 = 400;

fn is_in_prelaunch_registry(process_name: &str) -> bool {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey(MAGICANIMATION_REG_PATH) {
        if let Ok(app_names) = key.get_value::<String, _>(PRELAUNCH_APP_KEY) {
            log::info!(
                "[PrelaunchApp] Reading registry for '{}': value='{}'",
                process_name,
                app_names
            );

            for name in app_names.split(',') {
                let trimmed = name.trim();
                if trimmed.eq_ignore_ascii_case(process_name) {
                    log::info!(
                        "[PrelaunchApp] Matched process '{}' in registry value",
                        process_name
                    );
                    return true;
                }
            }
            log::info!(
                "[PrelaunchApp] Process '{}' not found in registry value",
                process_name
            );
        } else {
            log::info!(
                "[PrelaunchApp] Key '{}' not found in registry",
                PRELAUNCH_APP_KEY
            );
        }
    } else {
        log::info!(
            "[PrelaunchApp] Failed to open registry key: {}",
            MAGICANIMATION_REG_PATH
        );
    }
    false
}

fn is_prelaunch_app_minimized(process_name: &str) -> bool {
    log::info!(
        "[PrelaunchApp] Checking if '{}' is in registry",
        process_name
    );

    if is_in_prelaunch_registry(process_name) {
        log::info!(
            "[PrelaunchApp] Found in registry, will hide indicator for '{}'",
            process_name
        );
        return true;
    }

    log::info!(
        "[PrelaunchApp] Process '{}' not in registry, normal behavior",
        process_name
    );
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowVisualState {
    Normal = 0,
    Maximized = 1,
    Minimized = 2,
}

impl From<i32> for WindowVisualState {
    fn from(value: i32) -> Self {
        match value {
            0 => Self::Normal,
            1 => Self::Maximized,
            2 => Self::Minimized,
            _ => Self::Normal,
        }
    }
}

static LAST_OVERLAP_CHECK: LazyLock<
    parking_lot::Mutex<HashMap<(isize, WindowVisualState), Instant>>,
> = LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));

fn schedule_observed_window_reconcile(window: Window, reason: &'static str) {
    crate::get_tokio_handle().spawn(async move {
        for delay_ms in [0_u64, 200, 800] {
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            UserAppsManager::reconcile_observed_window(&window, reason);
        }
    });
}

fn should_process_overlap_check(window: &Window, state: WindowVisualState) -> bool {
    let key = (window.address(), state);
    let now = Instant::now();

    let mut last_check = LAST_OVERLAP_CHECK.lock();
    if let Some(&last_time) = last_check.get(&key) {
        if now.duration_since(last_time).as_millis() < DEBOUNCE_INTERVAL_MS as u128 {
            return false;
        }
    }

    last_check.insert(key, now);
    last_check.retain(|_, &mut time| now.duration_since(time).as_millis() < 1000);

    true
}

fn check_toolbar_overlap_status(window: &Window) {
    let state = crate::state::application::FULL_STATE.load();
    let hide_mode = state.settings.by_widget.fancy_toolbar.hide_mode;

    let toolbars: Vec<_> = {
        let manager = crate::trace_read!(crate::app::APP_MANAGER);
        manager
            .instances
            .iter()
            .map(|instance| instance.toolbar.clone())
            .collect()
    };

    for toolbar_arc in toolbars {
        let Some(mut toolbar) = toolbar_arc.try_lock_for(Duration::from_millis(100)) else {
            log::warn!(
                "[UIA VisualState] skip toolbar overlap check: toolbar lock busy, hwnd={:?}",
                window.address()
            );
            continue;
        };

        if let Some(tl) = toolbar.as_mut() {
            if libs_core::state::HideMode::OnOverlap == hide_mode {
                if let Err(e) = tl.handle_overlaped_status(window) {
                    log::error!(
                        "[UIA VisualState] Toolbar handle_overlaped_status failed: {:?}",
                        e
                    );
                }
            } else if let Err(e) = tl.update_toolbar_state(Some(window)) {
                log::error!(
                    "[UIA VisualState] Toolbar update_toolbar_state failed: {:?}",
                    e
                );
            }
        }
    }
}

pub fn process_visual_state_event(hwnd: isize, state: i32) {
    let state = WindowVisualState::from(state);

    let window = Window::from(hwnd);
    if !matches!(
        state,
        WindowVisualState::Maximized | WindowVisualState::Normal | WindowVisualState::Minimized
    ) {
        return;
    }

    update_last_real_foreground(window);

    if matches!(
        state,
        WindowVisualState::Maximized | WindowVisualState::Normal
    ) {
        schedule_observed_window_reconcile(window, "uia_visual_state");
    }

    if Taskbar::contains_app(&window) {
        if let Err(e) = Taskbar::update_app(&window) {
            log::error!("[UIA VisualState] Failed to update app: {:?}", e);
        }
    }

    if !should_process_overlap_check(&window, state) {
        return;
    }

    crate::get_tokio_handle().spawn(async move {
        Taskbar::check_overlap_for_all_taskbars(&window);
        check_toolbar_overlap_status(&window);

        if matches!(state, WindowVisualState::Minimized) {
            let glass_hwnds: Vec<isize> = {
                let manager = crate::trace_read!(crate::app::APP_MANAGER);
                manager
                    .instances
                    .iter()
                    .filter_map(|inst| {
                        let taskbar_guard = inst
                            .taskbar
                            .try_lock_for(std::time::Duration::from_secs(1))?;
                        let tb = taskbar_guard.as_ref()?;
                        let glass = tb.glass_effect.as_ref()?;
                        let h = glass.hwnd().0 as isize;
                        if h != 0 {
                            Some(h)
                        } else {
                            None
                        }
                    })
                    .collect()
            };
            if !glass_hwnds.is_empty() {
                let refresh_msg = super::glass_effect::WM_GLASS_REFRESH_MSG;
                #[allow(non_snake_case)]
                extern "system" {
                    fn PostMessageW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> i32;
                }
                for _ in 0..3 {
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    for &hwnd_raw in &glass_hwnds {
                        unsafe {
                            PostMessageW(hwnd_raw, refresh_msg, 0, 0);
                        }
                    }
                }
            }
        }
    });
}

impl Taskbar {
    pub fn process_global_win_event(event: WinEvent, window: &Window) -> Result<()> {
        match event {
            WinEvent::SystemMoveSizeEnd => {
                if Self::contains_app(window) {
                    Self::update_app(window)?;
                }
            }
            WinEvent::SystemForeground => {
                if Self::contains_app(window) {
                    Self::foregrounded_app(window)?;
                }
            }
            WinEvent::ObjectNameChange => {
                if Self::contains_app(window) {
                    Self::update_app(window)?;
                }
            }
            WinEvent::SystemMinimizeStart => {
                if Self::contains_app(window) {
                    if let Ok(process_name) = window.process().program_exe_name() {
                        if is_prelaunch_app_minimized(&process_name) {
                            Self::emit_hide_indicator(&process_name);
                        }
                    }
                    Self::update_app(window)?;
                }
            }
            WinEvent::SystemMinimizeEnd => {
                if Self::contains_app(window) {
                    Self::update_app(window)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn process_individual_win_event(&mut self, event: WinEvent, origin: &Window) -> Result<()> {
        // 注：最大化/恢复事件由 UIA visual_state_listener 处理
        if matches!(
            event,
            WinEvent::SystemCaptureEnd | WinEvent::SystemForeground
        ) {
            // 注意：此时已持有 instance.taskbar 组件锁，不能 sleep，否则会阻塞其他线程操作该 taskbar
            self.handle_overlaped_status(origin)?;
        }
        if matches!(
            event,
            WinEvent::SystemMoveSizeEnd | WinEvent::SystemCaptureEnd
        ) {
            self.reposition_if_needed()?;
        }
        Ok(())
    }

    // move this to independent function as this should work independently if dock is enabled or not
    pub fn process_raw_win_event(event: u32, origin_hwnd: HWND) -> Result<()> {
        let origin = Window::from(origin_hwnd);
        match event {
            EVENT_OBJECT_SHOW | EVENT_OBJECT_CREATE | EVENT_OBJECT_UNCLOAKED => {
                let class = origin.class();
                let parent_class = origin.parent().map(|p| p.class()).unwrap_or_default();
                if TASKBAR_CLASS
                    .iter()
                    .any(|t| t == &class || t == &parent_class)
                {
                    Self::hide_taskbar();
                    log::info!("[process_raw_win_event] Hiding window taskbar");
                    return Ok(());
                }
            }
            _ => {}
        }
        Ok(())
    }
}
