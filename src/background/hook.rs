use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        LazyLock,
    },
    time::{Duration, Instant},
};

// 回收站状态缓存（原子变量，无锁访问）
pub static RECYCLE_BIN_EMPTY_CACHE: AtomicBool = AtomicBool::new(false); // 默认为非空
pub static RECYCLE_BIN_COUNT_CACHE: AtomicU32 = AtomicU32::new(1); // 回收站文件数量缓存，默认 1 个文件
pub static RECYCLE_BIN_STATUS_INITIALIZED: AtomicBool = AtomicBool::new(false);
use std::sync::atomic::AtomicIsize;
pub static RECYCLE_BIN_HWND_CACHE: AtomicIsize = AtomicIsize::new(0); // 已知的回收站窗口 hwnd（0 表示未知）

use libs_core::handlers::FuncEvent;
use libs_core::state::HideMode;
use parking_lot::{Mutex, RwLock};
use tauri::Emitter;
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{SWP_ASYNCWINDOWPOS, SWP_NOMOVE, SWP_NOSIZE},
};

use crate::{
    app::{get_app_handle, APP_MANAGER},
    error::{ErrorMap, Result, ResultLogExt},
    event_manager, log_error,
    state::application::FULL_STATE,
    utils::{constants::NATIVE_UI_POPUP_CLASSES, dump_active_locks, spawn_named_thread},
    widgets::taskbar::Taskbar,
    windows_api::{
        window::{event::WinEvent, Window},
        WindowsApi,
    },
};

pub static LOG_WIN_EVENTS: AtomicBool = AtomicBool::new(false);

// -----------------------------
// Desktop scene detection (Win+D)
// -----------------------------
static DESKTOP_SCENE_ACTIVE: AtomicBool = AtomicBool::new(false);

pub static GAME_FULLSCREEN_BLOCKED: AtomicBool = AtomicBool::new(false);

const WIN_EVENT_LOCK_WAIT: Duration = Duration::from_millis(100);
const WIN_EVENT_LOCK_DIAG_AFTER: Duration = Duration::from_secs(5);
const WIN_EVENT_LOCK_EXIT_AFTER: Duration = Duration::from_secs(15);

struct WinEventLockSkipState {
    first_skipped_at: Instant,
    skipped_count: u64,
    diag_logged: bool,
    last_event: WinEvent,
}

static WIN_EVENT_LOCK_SKIPS: LazyLock<Mutex<HashMap<&'static str, WinEventLockSkipState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static FOREGROUND_REPROCESS_PENDING: AtomicBool = AtomicBool::new(false);

pub fn request_foreground_reprocess(reason: &'static str) {
    if FOREGROUND_REPROCESS_PENDING.swap(true, Ordering::SeqCst) {
        log::debug!(
            "[WinEvent] foreground reprocess already pending, reason={} coalesced",
            reason
        );
        return;
    }

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        FOREGROUND_REPROCESS_PENDING.store(false, Ordering::SeqCst);

        let foreground = Window::get_foregrounded();
        log::warn!(
            "[WinEvent] reprocessing foreground after skipped event, reason={}, hwnd={:?}",
            reason,
            foreground.address()
        );
        let _ = HookManager::event_tx().send((WinEvent::SystemForeground, foreground));
    });
}

fn spawn_win_event_lock_watchdog(component: &'static str) {
    let thread_name = format!("win-event-lock-watchdog-{component}");
    if let Err(err) = spawn_named_thread(&thread_name, move || {
        std::thread::sleep(WIN_EVENT_LOCK_DIAG_AFTER);

        let diag_snapshot = {
            let mut skips = WIN_EVENT_LOCK_SKIPS.lock();
            if let Some(state) = skips.get_mut(component) {
                if state.diag_logged {
                    None
                } else {
                    state.diag_logged = true;
                    Some((
                        state.first_skipped_at.elapsed(),
                        state.skipped_count,
                        state.last_event,
                    ))
                }
            } else {
                return;
            }
        };

        if let Some((elapsed, skipped_count, last_event)) = diag_snapshot {
            let dump = dump_active_locks();
            log::error!(
                "[WinEvent] {} lock busy for {:.3}s (skipped {} events, last={:?})\n=== Active locks snapshot ===\n{}=============================",
                component,
                elapsed.as_secs_f64(),
                skipped_count,
                last_event,
                dump
            );
        }

        std::thread::sleep(WIN_EVENT_LOCK_EXIT_AFTER - WIN_EVENT_LOCK_DIAG_AFTER);

        let exit_snapshot = {
            let skips = WIN_EVENT_LOCK_SKIPS.lock();
            skips.get(component).map(|state| {
                (
                    state.first_skipped_at.elapsed(),
                    state.skipped_count,
                    state.last_event,
                )
            })
        };

        if let Some((elapsed, skipped_count, last_event)) = exit_snapshot {
            log::error!(
                "[WinEvent] {} lock remained busy for {:.3}s (skipped {} events, last={:?}); exiting UI process for service restart",
                component,
                elapsed.as_secs_f64(),
                skipped_count,
                last_event
            );
            crate::report_ui_process_exit("WinEventLockBusyWatchdog");
            std::process::exit(1);
        }
    }) {
        log::warn!(
            "[WinEvent] failed to spawn {} lock watchdog: {:?}",
            component,
            err
        );
    }
}

fn record_win_event_lock_skip(component: &'static str, event: WinEvent) {
    let now = Instant::now();
    let mut skips = WIN_EVENT_LOCK_SKIPS.lock();
    let should_spawn_watchdog = !skips.contains_key(component);
    let state = skips
        .entry(component)
        .or_insert_with(|| WinEventLockSkipState {
            first_skipped_at: now,
            skipped_count: 0,
            diag_logged: false,
            last_event: event,
        });
    state.skipped_count += 1;
    state.last_event = event;

    let elapsed = now.duration_since(state.first_skipped_at);
    if !state.diag_logged && elapsed >= WIN_EVENT_LOCK_DIAG_AFTER {
        state.diag_logged = true;
        let dump = dump_active_locks();
        log::error!(
            "[WinEvent] {} lock busy for {:.3}s (skipped {} events, last={:?})\n=== Active locks snapshot ===\n{}=============================",
            component,
            elapsed.as_secs_f64(),
            state.skipped_count,
            event,
            dump
        );
    } else {
        log::warn!(
            "[WinEvent] Skipping {} event {:?}: lock busy for {:.3}s (skipped {})",
            component,
            event,
            elapsed.as_secs_f64(),
            state.skipped_count
        );
    }

    drop(skips);

    if should_spawn_watchdog {
        spawn_win_event_lock_watchdog(component);
    }

    if elapsed >= WIN_EVENT_LOCK_EXIT_AFTER {
        log::error!(
            "[WinEvent] {} lock remained busy for {:.3}s; exiting UI process for service restart",
            component,
            elapsed.as_secs_f64()
        );
        crate::report_ui_process_exit("WinEventLockBusy");
        std::process::exit(1);
    }
}

fn clear_win_event_lock_skip(component: &'static str) {
    let removed = WIN_EVENT_LOCK_SKIPS.lock().remove(component);
    if let Some(state) = removed {
        let elapsed = state.first_skipped_at.elapsed();
        log::info!(
            "[WinEvent] {} lock recovered after {:.3}s (skipped {} events)",
            component,
            elapsed.as_secs_f64(),
            state.skipped_count
        );
    }
}

/// Check if current foreground window is a game in fullscreen mode
pub fn is_game_fullscreen_blocked() -> bool {
    GAME_FULLSCREEN_BLOCKED.load(Ordering::Acquire)
}

static LAST_REAL_FOREGROUND: LazyLock<RwLock<Option<Window>>> = LazyLock::new(|| {
    let current = Window::from(WindowsApi::get_foreground_window());
    let initial = if current.is_bar_overlay() || !current.is_window() {
        None
    } else {
        Some(current)
    };
    RwLock::new(initial)
});

pub struct HookManager;

event_manager!(HookManager, (WinEvent, Window));

impl HookManager {
    fn should_ignore_desktop_scene_origin(origin: &Window) -> bool {
        if origin.is_bar_overlay() {
            return true;
        }

        let class = origin.class();
        // HnAppStore and similar apps can briefly surface empty-title HwndWrapper shell windows
        // before switching to the real window. Treat them as transient so they don't refresh
        // desktop-scene state prematurely.
        if class.starts_with("HwndWrapper[") && origin.title().is_empty() {
            return true;
        }

        if NATIVE_UI_POPUP_CLASSES.contains(&class.as_str()) {
            return true;
        }

        if let Ok(exe_name) = origin.process().program_exe_name() {
            if exe_name.eq_ignore_ascii_case("magictaskbar-ui.exe") {
                return true;
            }
        }

        false
    }

    /// Handle desktop scene (Win+D): when foreground switches to desktop,
    /// set toolbar to topmost so it stays visible; when switching back to
    /// normal app, restore toolbar to normal Z-order.
    /// Only affects Never mode toolbars (OnOverlap/Always are always topmost).
    fn handle_desktop_scene(origin: &Window) {
        if Self::should_ignore_desktop_scene_origin(origin) {
            log::debug!(
                target: "DesktopScene",
                "Skip transient foreground: class={}, hwnd=0x{:X}",
                origin.class(),
                origin.address()
            );
            return;
        }

        // Check if foreground is desktop-like (Progman, WorkerW, Shell_TrayWnd, etc.)
        let is_desktop_scene = origin.is_desktop() || {
            let class = origin.class();
            class == "Shell_TrayWnd" // Taskbar
        };

        let was_desktop_scene = DESKTOP_SCENE_ACTIVE.swap(is_desktop_scene, Ordering::SeqCst);

        // Only act on transitions
        if was_desktop_scene == is_desktop_scene {
            return;
        }

        log::info!(
            target: "DesktopScene",
            "Desktop scene transition: {} -> {} (class={}, hwnd=0x{:X})",
            was_desktop_scene,
            is_desktop_scene,
            origin.class(),
            origin.address()
        );

        // Only toggle topmost for Never mode toolbars
        let state = FULL_STATE.load();
        let hide_mode = state.settings.by_widget.fancy_toolbar.hide_mode;

        // Only Never mode needs desktop scene handling
        if hide_mode != HideMode::Never {
            log::debug!(target: "DesktopScene", "Skip: hide_mode={:?} (not Never)", hide_mode);
            return;
        }

        let Some(manager) = APP_MANAGER.try_read_for(std::time::Duration::from_millis(100)) else {
            log::warn!(
                target: "DesktopScene",
                "Skip toolbar topmost update: APP_MANAGER write lock busy"
            );
            request_foreground_reprocess("desktop_scene_app_manager_busy");
            return;
        };
        let toolbars: Vec<_> = manager
            .instances
            .iter()
            .map(|instance| instance.toolbar.clone())
            .collect();
        drop(manager);

        for toolbar_arc in toolbars {
            let Some(window) = toolbar_arc
                .try_lock_for(WIN_EVENT_LOCK_WAIT)
                .and_then(|toolbar| toolbar.as_ref().map(|tl| tl.window()))
            else {
                log::warn!(
                    target: "DesktopScene",
                    "Skip toolbar topmost update: toolbar lock busy"
                );
                request_foreground_reprocess("desktop_scene_toolbar_busy");
                continue;
            };

            if is_desktop_scene {
                // Desktop scene: set toolbar to topmost so it stays visible above desktop
                let _ = window.set_always_on_top(true);
                log::debug!(target: "DesktopScene", "Set toolbar to topmost for desktop scene");
            } else {
                // Normal app: restore toolbar to normal Z-order
                let _ = window.set_always_on_top(false);
                // Place toolbar right behind the foreground window to avoid covering it.
                // This is more deterministic than relying on NOTOPMOST alone.
                if origin.is_window() && !origin.is_desktop() && !origin.is_bar_overlay() {
                    if let Ok(raw) = window.hwnd() {
                        let toolbar_hwnd = HWND(raw.0);
                        if let Ok(toolbar_rect) = WindowsApi::get_outer_window_rect(toolbar_hwnd) {
                            if let Err(e) = WindowsApi::set_position(
                                toolbar_hwnd,
                                Some(origin.hwnd()),
                                &toolbar_rect,
                                SWP_NOMOVE | SWP_NOSIZE | SWP_ASYNCWINDOWPOS,
                            ) {
                                log::debug!(
                                    target: "DesktopScene",
                                    "Failed to place toolbar below foreground hwnd=0x{:X}: {:?}",
                                    origin.address(),
                                    e
                                );
                            } else {
                                log::debug!(
                                    target: "DesktopScene",
                                    "Placed toolbar below foreground hwnd=0x{:X}",
                                    origin.address()
                                );
                            }
                        }
                    }
                }
                log::debug!(target: "DesktopScene", "Restored toolbar to normal Z-order");
            }
        }
    }

    fn log_event(event: WinEvent, origin: Window) {
        if event == WinEvent::ObjectLocationChange || !LOG_WIN_EVENTS.load(Ordering::Acquire) {
            return;
        }
        let event_value = {
            #[cfg(dev)]
            {
                use owo_colors::OwoColorize;
                event.green()
            }
            #[cfg(not(dev))]
            {
                &event
            }
        };
        if event == WinEvent::ObjectDestroy {
            return log::debug!("{:?}({:0x})", event_value, origin.address());
        }
        log::debug!("{event_value:?} | {origin:?}");
    }

    fn process_event(event: WinEvent, origin: Window) {
        Self::log_event(event, origin);

        // 回收站检测 - 异步执行避免阻塞事件分发线程
        if event == WinEvent::ObjectCreate
            || event == WinEvent::ObjectDestroy
            || event == WinEvent::ObjectHide
        {
            let origin = origin.clone();
            let event_clone = event.clone();
            std::thread::spawn(move || {
                let class = origin.class();

                let is_recycle_bin = if event_clone == WinEvent::ObjectCreate {
                    // Create 时：class 必须是 CabinetWClass 且标题匹配
                    if class != "CabinetWClass" {
                        return;
                    }
                    let title = origin.title();
                    let matched = title.starts_with("回收站")
                        || title.starts_with("Recycle Bin")
                        || title.starts_with("資源回收筒");
                    if matched {
                        // 记录 hwnd 到缓存
                        RECYCLE_BIN_HWND_CACHE.store(origin.address(), Ordering::Relaxed);
                    }
                    matched
                } else if event_clone == WinEvent::ObjectDestroy {
                    // Destroy 时：窗口已销毁，class/title 均不可用
                    // 用缓存的 hwnd 精确比对
                    let cached = RECYCLE_BIN_HWND_CACHE.load(Ordering::Relaxed);
                    let matched = cached != 0 && cached == origin.address();
                    if matched {
                        // 清空缓存
                        RECYCLE_BIN_HWND_CACHE.store(0, Ordering::Relaxed);
                    }
                    matched
                } else {
                    // ObjectHide 时：窗口还存在，可以读 class/标题/hwnd
                    if class != "CabinetWClass" {
                        return;
                    }
                    let title = origin.title();
                    let title_match = title.starts_with("回收站")
                        || title.starts_with("Recycle Bin")
                        || title.starts_with("資源回收筒");
                    let hwnd_match = !title_match
                        && crate::windows_api::WindowsApi::get_recycle_bin_hwnd()
                            .map(|h| h == origin.hwnd())
                            .unwrap_or(false);
                    title_match || hwnd_match
                };

                if is_recycle_bin {
                    let is_open = if event_clone == WinEvent::ObjectCreate {
                        true
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(200));
                        crate::windows_api::WindowsApi::get_recycle_bin_hwnd().is_some()
                    };
                    let _ = get_app_handle().emit("recycle-bin-state-changed", is_open);
                }
            });
        }

        // Handle desktop scene (Win+D): toggle toolbar topmost
        if event == WinEvent::SystemForeground && origin.is_focused() {
            // Foreground can be transient right after Win+D / gesture desktop transitions.
            // Re-evaluate once with the stabilized foreground to avoid wrong early decisions.
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(180));
                let stabilized = Window::get_foregrounded();
                HookManager::handle_desktop_scene(&stabilized);
            });

            // Check game fullscreen state when foreground window changes
            // 只有句柄变化时才检查，避免重复检查同一窗口
        }

        let shoup_update_focused = matches!(
            event,
            WinEvent::SystemForeground
                | WinEvent::ObjectNameChange
                | WinEvent::SystemMoveSizeStart
                | WinEvent::SystemMoveSizeEnd
                | WinEvent::SyntheticFullscreenStart
                | WinEvent::SyntheticFullscreenEnd
        );

        if shoup_update_focused && origin.is_focused() {
            if !origin.is_bar_overlay() {
                *LAST_REAL_FOREGROUND.write() = Some(origin);
            }
            get_app_handle()
                .emit(
                    FuncEvent::GlobalFocusChanged,
                    origin.as_focused_app_information(),
                )
                .wrap_error()
                .log_error();
        }

        // 只对 process_global_win_event 实际处理的事件才 spawn 线程，避免为每个 WinEvent 创建无用线程
        if matches!(
            event,
            WinEvent::SystemMoveSizeEnd
                | WinEvent::SystemForeground
                | WinEvent::ObjectNameChange
                | WinEvent::SystemMinimizeStart
                | WinEvent::SystemMinimizeEnd
        ) {
            std::thread::spawn(move || {
                log_error!(Taskbar::process_global_win_event(event, &origin), event);
            });
        }

        // 对需要延迟的事件，在获取锁之前等待窗口状态稳定
        if matches!(
            event,
            WinEvent::SystemCaptureEnd | WinEvent::SystemForeground
        ) {
            std::thread::sleep(std::time::Duration::from_millis(180));
        }

        // 只对 taskbar/toolbar 实际处理的事件才获取实例锁，避免高频无关事件频繁加锁
        // 注意：如果 taskbar/toolbar 新增其他事件处理，需同步更新此白名单
        if !matches!(
            event,
            WinEvent::SystemCaptureEnd | WinEvent::SystemForeground | WinEvent::SystemMoveSizeEnd
        ) {
            return;
        }

        // 死锁修复：短暂持 APP_MANAGER::read 仅克隆 taskbar/toolbar Arc 列表后立即释放，
        // 避免在内层 instance.taskbar / instance.toolbar lock 期间持有外层 APP_MANAGER 锁。
        let widgets: Vec<(
            std::sync::Arc<parking_lot::Mutex<Option<crate::widgets::taskbar::Taskbar>>>,
            std::sync::Arc<parking_lot::Mutex<Option<crate::widgets::toolbar::FancyToolbar>>>,
        )> = {
            let Some(app_manager) = APP_MANAGER.try_read_for(std::time::Duration::from_millis(100))
            else {
                log::warn!(
                    "[WinEvent] skip taskbar/toolbar event: APP_MANAGER write lock busy, event={:?}, hwnd={:?}",
                    event,
                    origin.address()
                );
                request_foreground_reprocess("win_event_app_manager_busy");
                return;
            };
            app_manager
                .instances
                .iter()
                .map(|i| (i.taskbar.clone(), i.toolbar.clone()))
                .collect()
        };

        for (tb_arc, tl_arc) in &widgets {
            // 对齐时序：先处理 Taskbar，再处理 Toolbar，避免 Toolbar 过早发出事件导致 webview 未完成监听
            {
                if let Some(mut taskbar) = tb_arc.try_lock_for(WIN_EVENT_LOCK_WAIT) {
                    clear_win_event_lock_skip("taskbar");
                    if let Some(tb) = taskbar.as_mut() {
                        if let Err(err) = tb.process_individual_win_event(event, &origin) {
                            log::error!("Context: {:?} Err: {:?}", event, err);
                        }
                    }
                } else {
                    record_win_event_lock_skip("taskbar", event);
                }
            }
            {
                if let Some(mut toolbar) = tl_arc.try_lock_for(WIN_EVENT_LOCK_WAIT) {
                    clear_win_event_lock_skip("toolbar");
                    if let Some(tl) = toolbar.as_mut() {
                        log_error!(tl.process_win_event(event, &origin), event);
                    }
                } else {
                    record_win_event_lock_skip("toolbar", event);
                }
            }
        }
    }
}

/// 更新上一次非任务栏前台窗口（供 UIA 视觉状态监听器调用）
pub fn update_last_real_foreground(window: Window) {
    // 只有当窗口确实是前台窗口时才更新（与 process_event 逻辑一致）
    if !window.is_focused() {
        return;
    }
    if !window.is_bar_overlay() {
        *LAST_REAL_FOREGROUND.write() = Some(window);
    }
    // 无论是否是 bar_overlay，都发送焦点变化事件到前端（与 process_event 逻辑一致）
    get_app_handle()
        .emit(
            FuncEvent::GlobalFocusChanged,
            window.as_focused_app_information(),
        )
        .wrap_error()
        .log_error();
}

pub fn register_win_hook() -> Result<()> {
    log::trace!("Registering Windows and Virtual Desktop Hooks");

    // WinEvent hooks are registered in the service process. The UI keeps only the
    // event processing pipeline so forwarded events can update WebView state.
    HookManager::subscribe(|(event, mut origin)| {
        if event == WinEvent::SystemForeground {
            origin = Window::get_foregrounded(); // sometimes event is emitted with wrong origin
        }

        let synthetics = event.get_synthetics(&origin);
        HookManager::process_event(event, origin);
        if let Ok(synthetics) = synthetics {
            for synthetic_event in synthetics {
                HookManager::process_event(synthetic_event, origin)
            }
        }
    });

    Ok(())
}
