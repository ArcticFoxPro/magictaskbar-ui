use std::collections::{HashMap, HashSet};
use std::sync::{atomic::AtomicBool, Arc, LazyLock};
use std::time::{Duration, Instant};

use getset::{Getters, MutGetters};
use libs_core::system_state::MonitorId;
use parking_lot::{Mutex, RwLock};
use tauri::{AppHandle, Emitter, Wry};

use crate::{
    app_instance::SluMonitorInstance,
    error::Result,
    hook::register_win_hook,
    log_error,
    modules::{
        monitors::{MonitorManager, MonitorManagerEvent},
        system_settings::application::{SystemSettings, SystemSettingsEvent},
    },
    restoration_and_migrations::RestorationAndMigration,
    state::application::{FullState, FULL_STATE},
    system::{declare_system_events_handlers, release_system_events_handlers},
    trace_lock, trace_read, trace_write,
    widgets::taskbar::{taskbar_items_impl::TASKBAR_STATE, Taskbar},
    windows_api::{event_window::create_background_window, monitor::MonitorView, WindowsApi},
    APP_HANDLE,
};

const CONTROL_CENTER_AUX_WINDOW_CLASS: &str = "ControlCenterAuxBackgroundWindows";
const WM_USER_GET_ONLINE_DEVICES: u32 = 0x0400 + 300;

pub static APP_MANAGER: LazyLock<Arc<RwLock<AppManager>>> =
    LazyLock::new(|| Arc::new(RwLock::new(AppManager::default())));

static APP_IS_RUNNING: AtomicBool = AtomicBool::new(false);

/// Toolbar 模式上报点位 ID
const TOOLBAR_MODE_REPORT_ID: &str = "669000009";

fn report_string(report_id: &str, content: &str) -> bool {
    log::debug!(
        "[Report] data bridge disabled; skip report id={} content={}",
        report_id,
        content
    );
    false
}

/// 上一次 Toolbar 的 hide_mode，用于检测模式变化
static LAST_TOOLBAR_HIDE_MODE: LazyLock<Mutex<Option<libs_core::state::HideMode>>> =
    LazyLock::new(|| Mutex::new(None));

/// 防抖间隔：同一显示器的相同事件在此时间内重复出现则忽略
const MONITOR_EVENT_DEBOUNCE: Duration = Duration::from_millis(300);
const HWND_RESOLVE_TIMEOUT: Duration = Duration::from_secs(10);
const HWND_RESOLVE_MAX_NONFATAL_TIMEOUTS: u32 = 1;
const HWND_RESOLVE_RETRY_DELAY: Duration = Duration::from_secs(2);

/// 记录每个显示器每种事件最后一次处理的时间戳
/// Key: (MonitorId, 事件类型)
static LAST_MONITOR_EVENTS: LazyLock<Mutex<HashMap<(MonitorId, &'static str), Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static PENDING_HWND_RESOLVES: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

static HWND_RESOLVE_TIMEOUTS: LazyLock<Mutex<HashMap<String, u32>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static POSITION_REFRESH_RETRY_PENDING: AtomicBool = AtomicBool::new(false);
const POSITION_REFRESH_RETRY_DELAY: Duration = Duration::from_millis(800);

/// 检查是否应该处理该事件（防抖）
/// 返回 true 表示应该处理，false 表示应该跳过
fn should_process_monitor_event(id: &MonitorId, event_type: &'static str) -> bool {
    let key = (id.clone(), event_type);
    let now = Instant::now();

    let mut last_events = LAST_MONITOR_EVENTS.lock();
    if let Some(&last_time) = last_events.get(&key) {
        if now.duration_since(last_time) < MONITOR_EVENT_DEBOUNCE {
            log::debug!(
                "[AppManager] {}事件被限流, monitor={}, 距上次小于{}ms",
                event_type,
                id.0,
                MONITOR_EVENT_DEBOUNCE.as_millis()
            );
            return false;
        }
    }

    // 更新时间戳
    last_events.insert(key, now);

    // 清理过期的条目（超过 1 秒未更新的）
    last_events.retain(|_, &mut time| now.duration_since(time) < Duration::from_secs(1));

    true
}

fn try_begin_hwnd_resolve(window_label: &str) -> bool {
    PENDING_HWND_RESOLVES
        .lock()
        .insert(window_label.to_string())
}

fn finish_hwnd_resolve(window_label: &str) {
    PENDING_HWND_RESOLVES.lock().remove(window_label);
}

fn clear_hwnd_resolve_timeouts(window_label: &str) {
    HWND_RESOLVE_TIMEOUTS.lock().remove(window_label);
}

fn record_hwnd_resolve_timeout(window_label: &str) -> u32 {
    let mut timeouts = HWND_RESOLVE_TIMEOUTS.lock();
    let count = timeouts.entry(window_label.to_string()).or_insert(0);
    *count += 1;
    *count
}

fn start_hwnd_resolve_watchdog(
    widget_name: &'static str,
    window_label: String,
    completed: Arc<AtomicBool>,
    fatal_on_timeout: bool,
) {
    std::thread::spawn(move || {
        let started = Instant::now();

        loop {
            std::thread::sleep(Duration::from_secs(5));
            if completed.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }

            if started.elapsed() < HWND_RESOLVE_TIMEOUT {
                continue;
            }

            log::error!(
                "[HwndResolveWatchdog] {widget_name} hwnd resolve timed out after {:.1}s, fatal_on_timeout={}, label={}",
                started.elapsed().as_secs_f64(),
                fatal_on_timeout,
                window_label
            );
            finish_hwnd_resolve(&window_label);
            completed.store(true, std::sync::atomic::Ordering::SeqCst);

            let timeout_count = record_hwnd_resolve_timeout(&window_label);
            let should_restart =
                fatal_on_timeout || timeout_count > HWND_RESOLVE_MAX_NONFATAL_TIMEOUTS;

            if should_restart {
                log::error!(
                    "[HwndResolveWatchdog] {widget_name} hwnd resolve timed out {} consecutive times; exiting UI for srv restart. label={}",
                    timeout_count,
                    window_label
                );
                crate::report_ui_process_exit("WebViewHwndResolveTimeout");
                std::process::exit(1);
            }

            let retry_delay = HWND_RESOLVE_RETRY_DELAY * timeout_count;
            log::warn!(
                "[HwndResolveWatchdog] {widget_name} hwnd resolve timeout is treated as recoverable ({}/{}); scheduling retry after {:.1}s. label={}",
                timeout_count,
                HWND_RESOLVE_MAX_NONFATAL_TIMEOUTS,
                retry_delay.as_secs_f64(),
                window_label
            );
            schedule_delayed_async_refresh_positions(retry_delay);
            return;
        }
    });
}

fn notify_controlcenteraux_on_startup() {
    let hwnd = match WindowsApi::find_window(
        None,
        None,
        None,
        Some(CONTROL_CENTER_AUX_WINDOW_CLASS.to_string()),
    ) {
        Ok(hwnd) => hwnd,
        Err(error) => {
            log::warn!(
                "[Startup] ControlCenterAux hidden window not found (class={}): {:?}",
                CONTROL_CENTER_AUX_WINDOW_CLASS,
                error
            );
            return;
        }
    };

    if let Err(error) = WindowsApi::post_message(hwnd, WM_USER_GET_ONLINE_DEVICES, 0, 0) {
        log::warn!(
            "[Startup] Failed to post WM_USER+300 to ControlCenterAux hidden window: {:?}",
            error
        );
        return;
    }

    log::info!(
        "[Startup] Posted WM_USER+300 to ControlCenterAux hidden window (class={})",
        CONTROL_CENTER_AUX_WINDOW_CLASS
    );
}

fn schedule_delayed_async_refresh_positions(delay: Duration) {
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        log::warn!(
            "[sync_monitors] delayed async position refresh after {:.1}s",
            delay.as_secs_f64()
        );
        log_error!(trace_read!(APP_MANAGER).schedule_async_refresh_positions(false));
    });
}

fn schedule_position_refresh_retry(reason: &'static str) {
    if POSITION_REFRESH_RETRY_PENDING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        log::debug!(
            "[refresh_positions] retry already pending, reason={} coalesced",
            reason
        );
        return;
    }

    std::thread::spawn(move || {
        std::thread::sleep(POSITION_REFRESH_RETRY_DELAY);
        POSITION_REFRESH_RETRY_PENDING.store(false, std::sync::atomic::Ordering::SeqCst);

        let Some(manager) = APP_MANAGER.try_read_for(Duration::from_millis(500)) else {
            log::warn!(
                "[refresh_positions] retry delayed: APP_MANAGER read lock busy after {}",
                reason
            );
            schedule_position_refresh_retry(reason);
            return;
        };

        log::warn!(
            "[refresh_positions] retrying skipped async position refresh after {}",
            reason
        );
        log_error!(manager.schedule_async_refresh_positions(false));
    });
}

/// Tauri app handle
pub fn get_app_handle<'a>() -> &'a AppHandle<Wry> {
    APP_HANDLE
        .get()
        .expect("get_app_handle called but app is still not initialized")
}

/// Emit event to all webviews
pub fn emit_to_webviews<T: serde::Serialize + Clone>(event: &str, payload: T) -> Result<()> {
    let handle = get_app_handle();
    handle.emit(event, payload)?;
    Ok(())
}

/** Struct should be initialized first before calling any other methods */
#[derive(Getters, MutGetters, Default)]
pub struct AppManager {
    pub instances: Vec<SluMonitorInstance>,
}

/* ============== Getters ============== */
impl AppManager {
    pub fn instances_mut(&mut self) -> &mut Vec<SluMonitorInstance> {
        &mut self.instances
    }

    pub fn is_running() -> bool {
        APP_IS_RUNNING.load(std::sync::atomic::Ordering::Acquire)
    }
}

/* ============== Methods ============== */
impl AppManager {
    fn monitor_id_for_view(view: &MonitorView) -> Result<MonitorId> {
        match view.primary_target().and_then(|target| target.stable_id2()) {
            Ok(id) if !id.0.is_empty() => Ok(id),
            _ => {
                let win32_monitor = view.as_win32_monitor()?;
                let name = win32_monitor.name()?;
                log::warn!(
                    "[AppManager] stable_id为空或获取失败, 使用win32_name作为monitor id: {}",
                    name
                );
                Ok(name.into())
            }
        }
    }

    fn monitor_layout_signature(view: &MonitorView) -> Result<(i32, i32, i32, i32, i32, i32)> {
        let monitor = view.as_win32_monitor()?;
        let info = WindowsApi::monitor_info(monitor.handle())?;
        let rect = info.monitorInfo.rcMonitor;
        let monitor_dpi =
            (WindowsApi::get_monitor_scale_factor(monitor.handle())? * 1000.0).round() as i32;
        let text_scale = (WindowsApi::get_text_scale_factor()? * 1000.0).round() as i32;

        Ok((
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            monitor_dpi,
            text_scale,
        ))
    }

    fn schedule_async_refresh_positions(&self, fatal_on_timeout: bool) -> Result<()> {
        // 异步设置窗口位置，避免 hwnd() 阻塞主线程
        for instance in &self.instances {
            let tb_ptr = Arc::into_raw(instance.taskbar.clone()) as usize;
            let tl_ptr = Arc::into_raw(instance.toolbar.clone()) as usize;
            let pv_ptr = Arc::into_raw(instance.preview.clone()) as usize;
            let monitor_val = instance.view.as_win32_monitor()?.handle().0 as isize;

            std::thread::spawn(move || {
                use windows::Win32::{Foundation::HWND, Graphics::Gdi::HMONITOR};

                // This worker is started while callers may still hold APP_MANAGER::write.
                // Let the outer sync return before taking per-widget locks, otherwise
                // startup WinEvent/UIA callbacks can observe an inverted lock order.
                // Give newly-created WebViews a short window to finish native initialization.
                // Calling hwnd() too early can occasionally block for a long time on startup.
                std::thread::sleep(std::time::Duration::from_millis(500));

                let taskbar: Arc<parking_lot::Mutex<Option<crate::widgets::taskbar::Taskbar>>> =
                    unsafe { Arc::from_raw(tb_ptr as *const _) };
                let toolbar: Arc<
                    parking_lot::Mutex<Option<crate::widgets::toolbar::FancyToolbar>>,
                > = unsafe { Arc::from_raw(tl_ptr as *const _) };
                let preview: Arc<parking_lot::Mutex<Option<crate::widgets::preview::Preview>>> =
                    unsafe { Arc::from_raw(pv_ptr as *const _) };
                let monitor = HMONITOR(monitor_val as _);
                let taskbar_for_position = taskbar.clone();
                let taskbar_monitor_val = monitor_val;

                std::thread::spawn(move || {
                    let monitor = HMONITOR(taskbar_monitor_val as _);
                    let taskbar_window = {
                        match taskbar_for_position
                            .try_lock_for(std::time::Duration::from_millis(200))
                        {
                            Some(guard) => guard.as_ref().map(|tb| tb.window.clone()),
                            None => {
                                log::warn!(
                                    "[refresh_positions] taskbar window clone skipped: taskbar lock busy"
                                );
                                schedule_position_refresh_retry("taskbar_window_clone_busy");
                                None
                            }
                        }
                    };

                    if let Some(window) = taskbar_window {
                        let window_label = window.label().to_string();
                        if !try_begin_hwnd_resolve(&window_label) {
                            log::warn!(
                                "[refresh_positions] taskbar hwnd resolve skipped because another resolve is pending. label={}",
                                window_label
                            );
                            return;
                        }

                        log::info!(
                            "[refresh_positions] taskbar hwnd resolve started outside lock. label={}",
                            window_label
                        );
                        let hwnd_resolved = Arc::new(AtomicBool::new(false));
                        start_hwnd_resolve_watchdog(
                            "Taskbar",
                            window_label.clone(),
                            hwnd_resolved.clone(),
                            fatal_on_timeout,
                        );
                        let step = std::time::Instant::now();
                        match window.hwnd() {
                            Ok(raw) => {
                                hwnd_resolved.store(true, std::sync::atomic::Ordering::SeqCst);
                                finish_hwnd_resolve(&window_label);
                                clear_hwnd_resolve_timeouts(&window_label);
                                let hwnd = HWND(raw.0);
                                log::info!(
                                    "[refresh_positions] taskbar hwnd resolved outside lock in {:.3}s, hwnd={:?}",
                                    step.elapsed().as_secs_f64(),
                                    hwnd
                                );

                                match taskbar_for_position
                                    .try_lock_for(std::time::Duration::from_millis(200))
                                {
                                    Some(mut guard) => {
                                        if let Some(tb) = guard.as_mut() {
                                            if let Err(e) = tb.set_position_with_hwnd(monitor, hwnd)
                                            {
                                                log::error!(
                                                    "[refresh_positions] taskbar set_position failed: {e:?}"
                                                );
                                            }
                                        }
                                    }
                                    None => {
                                        log::warn!(
                                            "[refresh_positions] taskbar set_position skipped: taskbar lock busy"
                                        );
                                        schedule_position_refresh_retry(
                                            "taskbar_set_position_busy",
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                hwnd_resolved.store(true, std::sync::atomic::Ordering::SeqCst);
                                finish_hwnd_resolve(&window_label);
                                clear_hwnd_resolve_timeouts(&window_label);
                                log::error!(
                                    "[refresh_positions] taskbar hwnd resolve failed: {e:?}"
                                );
                            }
                        }
                    }
                });
                {
                    let schedule_initial_reposition = {
                        let toolbar_window = {
                            match toolbar.try_lock_for(std::time::Duration::from_millis(200)) {
                                Some(mut guard) => guard.as_mut().map(|tl| {
                                    if let Err(e) = tl.reserve_appbar_position(monitor) {
                                        log::error!(
                                            "[refresh_positions] toolbar reserve_appbar_position failed: {e:?}"
                                        );
                                    }
                                    (tl.window(), !tl.is_positioned())
                                }),
                                None => {
                                    log::warn!(
                                        "[refresh_positions] toolbar window clone skipped: toolbar lock busy"
                                    );
                                    schedule_position_refresh_retry("toolbar_window_clone_busy");
                                    None
                                }
                            }
                        };

                        if let Some((window, first_position)) = toolbar_window {
                            log::info!(
                                "[refresh_positions] toolbar hwnd resolve started outside lock"
                            );
                            let step = std::time::Instant::now();
                            match window.hwnd() {
                                Ok(raw) => {
                                    let hwnd = HWND(raw.0);
                                    log::info!(
                                        "[refresh_positions] toolbar hwnd resolved outside lock in {:.3}s, hwnd={:?}",
                                        step.elapsed().as_secs_f64(),
                                        hwnd
                                    );

                                    match toolbar
                                        .try_lock_for(std::time::Duration::from_millis(200))
                                    {
                                        Some(mut guard) => {
                                            if let Some(tl) = guard.as_mut() {
                                                if let Err(e) =
                                                    tl.set_position_with_hwnd(monitor, hwnd)
                                                {
                                                    log::error!(
                                                        "[refresh_positions] toolbar set_position failed: {e:?}"
                                                    );
                                                    false
                                                } else {
                                                    if let Some(delay) = tl
                                                        .take_deferred_fixed_appbar_restore_delay(
                                                            monitor,
                                                        )
                                                    {
                                                        crate::widgets::toolbar::schedule_deferred_fixed_appbar_restore(
                                                            toolbar.clone(),
                                                            delay,
                                                        );
                                                    }
                                                    first_position
                                                }
                                            } else {
                                                false
                                            }
                                        }
                                        None => {
                                            log::warn!(
                                                "[refresh_positions] toolbar set_position skipped: toolbar lock busy"
                                            );
                                            schedule_position_refresh_retry(
                                                "toolbar_set_position_busy",
                                            );
                                            false
                                        }
                                    }
                                }
                                Err(e) => {
                                    log::error!(
                                        "[refresh_positions] toolbar hwnd resolve failed: {e:?}"
                                    );
                                    false
                                }
                            }
                        } else {
                            false
                        }
                    };
                    if schedule_initial_reposition {
                        crate::widgets::toolbar::schedule_initial_reposition(toolbar.clone());
                    }
                }
                {
                    let mut guard = preview.lock();
                    if let Some(pv) = guard.as_mut() {
                        if let Err(e) = pv.set_initial_position(monitor) {
                            log::error!(
                                "[refresh_positions] preview set_initial_position failed: {e:?}"
                            );
                        }
                    }
                }
            });
        }

        Ok(())
    }

    fn sync_monitor_instances(&mut self) -> Result<()> {
        log::warn!("[sync_monitors] enumerating monitor views");
        let mut current_views = Vec::new();
        for view in MonitorManager::get_all_views()? {
            let id = match Self::monitor_id_for_view(&view) {
                Ok(id) => id,
                Err(e) => {
                    log::warn!("[sync_monitors] skipping invalid monitor view id: {e:?}");
                    continue;
                }
            };
            let layout = match Self::monitor_layout_signature(&view) {
                Ok(layout) => layout,
                Err(e) => {
                    log::warn!(
                        "[sync_monitors] skipping invalid monitor view layout, id={}: {e:?}",
                        id
                    );
                    continue;
                }
            };
            current_views.push((id, view, layout));
        }

        if current_views.is_empty() {
            log::warn!("[AppManager] 未找到显示器，保留现有实例");
            return Ok(());
        }

        log::warn!(
            "[sync_monitors] found {} monitor view(s), current instances={}",
            current_views.len(),
            self.instances.len()
        );
        let initial_count = self.instances.len();
        self.instances.retain(|instance| {
            let still_connected = current_views
                .iter()
                .any(|(id, _, _)| id == &instance.main_target_id);
            if !still_connected {
                return false;
            }

            if instance.has_broken_taskbar_webview() {
                log::error!(
                    "[sync_monitors] removing unhealthy monitor instance so it can be recreated, id={}",
                    instance.main_target_id
                );
                return false;
            }

            true
        });
        let mut instances_changed = self.instances.len() != initial_count;
        let mut positions_need_refresh = instances_changed;

        let state = FULL_STATE.load();
        for (id, view, new_layout) in current_views {
            if let Some(instance) = self
                .instances
                .iter_mut()
                .find(|instance| instance.main_target_id == id)
            {
                log::warn!(
                    "[sync_monitors] existing instance found, updating view: {}",
                    id
                );
                let old_layout = instance
                    .last_layout_signature
                    .unwrap_or(Self::monitor_layout_signature(&instance.view)?);
                if old_layout != new_layout {
                    log::warn!(
                        "[sync_monitors] monitor layout changed, id={}, old_snapshot={:?}, new={:?}",
                        id,
                        old_layout,
                        new_layout
                    );
                    positions_need_refresh = true;
                }
                instance.view = view;
                instance.last_layout_signature = Some(new_layout);
                log::warn!("[sync_monitors] view updated: {}", id);
            } else {
                log::warn!(
                    "[sync_monitors] no existing instance, creating SluMonitorInstance::new: {}",
                    id
                );
                match SluMonitorInstance::new(view, &state) {
                    Ok(mut instance) => {
                        instance.last_layout_signature = Some(new_layout);
                        self.instances.push(instance);
                        log::warn!("[sync_monitors] SluMonitorInstance::new done: {}", id);
                        instances_changed = true;
                        positions_need_refresh = true;
                    }
                    Err(e) => {
                        log::warn!(
                            "[sync_monitors] skipping monitor instance creation, id={}: {e:?}",
                            id
                        );
                        positions_need_refresh = true;
                        continue;
                    }
                }
            }
        }

        if !positions_need_refresh {
            log::warn!("[sync_monitors] position refresh skipped");
            log::warn!("[sync_monitors] completed");
            return Ok(());
        }

        for instance in &self.instances {
            let monitor = instance.view.as_win32_monitor()?.handle();
            {
                let mut toolbar = trace_lock!(instance.toolbar);
                if let Some(tl) = toolbar.as_mut() {
                    tl.set_current_monitor_hint(monitor);
                }
            }
            let mut taskbar = trace_lock!(instance.taskbar);
            if let Some(tb) = taskbar.as_mut() {
                if let Err(e) = tb.emit_container_refresh_for_monitor(monitor) {
                    log::warn!("[sync_monitors] immediate taskbar container refresh failed: {e:?}");
                }
            }
        }

        self.schedule_async_refresh_positions(false)?;

        if instances_changed {
            schedule_delayed_async_refresh_positions(Duration::from_secs(2));
            schedule_delayed_async_refresh_positions(Duration::from_secs(6));
            log::warn!("[sync_monitors] instances changed, emitting to webview");
            trace_lock!(TASKBAR_STATE).emit_to_webview()?;
            log::warn!("[sync_monitors] emit_to_webview done");
        }

        log::warn!("[sync_monitors] completed");
        Ok(())
    }

    fn refresh_windows_positions(&self) -> Result<()> {
        for instance in &self.instances {
            instance.ensure_positions()?;
        }
        Ok(())
    }

    pub fn on_settings_change(&self, state: &FullState) -> Result<()> {
        rust_i18n::set_locale(state.locale());

        // 检测 Toolbar hide_mode 是否发生变化
        let current_hide_mode = state.settings.by_widget.fancy_toolbar.hide_mode;
        let mut last_hide_mode = LAST_TOOLBAR_HIDE_MODE.lock();

        if last_hide_mode.map_or(true, |last| last != current_hide_mode) {
            // 模式发生变化，上报新模式
            let mode_str = match current_hide_mode {
                libs_core::state::HideMode::Never => "Never",
                libs_core::state::HideMode::OnOverlap => "OnOverlap",
                libs_core::state::HideMode::Always => "Always",
            };
            let json = serde_json::json!({ "ToolbarMode": mode_str }).to_string();
            report_string(TOOLBAR_MODE_REPORT_ID, &json);
            log::info!("[AppManager] Toolbar mode changed to: {}", mode_str);

            // 更新上一次的 hide_mode
            *last_hide_mode = Some(current_hide_mode);
        }
        drop(last_hide_mode);

        if state.is_taskbar_enabled() {
            Taskbar::hide_taskbar();
        } else {
            Taskbar::restore_taskbar()?;
        }

        for monitor in &self.instances {
            monitor.load_settings(state)?;
        }

        self.refresh_windows_positions()?;
        Ok(())
    }

    fn on_monitor_event(event: MonitorManagerEvent) {
        match event {
            MonitorManagerEvent::ViewAdded(view) => {
                // 获取 monitor id 用于防抖检查
                let id = match Self::monitor_id_for_view(&view) {
                    Ok(id) => id,
                    _ => {
                        // 无法获取 id，跳过防抖直接处理
                        log::warn!("[AppManager] ViewAdded: 无法获取 monitor id，跳过防抖");
                        let mut guard = trace_write!(APP_MANAGER);
                        log_error!(guard.sync_monitor_instances());
                        return;
                    }
                };

                // 防抖检查
                if !should_process_monitor_event(&id, "ViewAdded") {
                    return;
                }

                let mut guard = trace_write!(APP_MANAGER);
                log_error!(guard.sync_monitor_instances());
            }
            MonitorManagerEvent::ViewRemoved(id) => {
                // 防抖检查
                if !should_process_monitor_event(&id, "ViewRemoved") {
                    return;
                }

                let mut guard = trace_write!(APP_MANAGER);
                log_error!(guard.sync_monitor_instances());
            }
            MonitorManagerEvent::ViewUpdated(id) => {
                // 防抖检查
                if !should_process_monitor_event(&id, "ViewUpdated") {
                    return;
                }

                let mut guard = trace_write!(APP_MANAGER);
                log_error!(guard.sync_monitor_instances());
            }
        }
    }

    fn on_system_settings_change(event: SystemSettingsEvent) {
        if event == SystemSettingsEvent::TextScaleChanged {
            log_error!(trace_read!(APP_MANAGER).refresh_windows_positions());
        }
    }

    pub fn start(&mut self) -> Result<()> {
        log::warn!("AppManager start");
        RestorationAndMigration::run_full()?;

        let state = FULL_STATE.load();
        rust_i18n::set_locale(state.locale());

        // On dock startup, force all tray icons to overflow by setting IsPromoted=0.
        // This follows the user's requirement to push all notify icons into the overflow area.
        // order is important
        create_background_window()?;
        declare_system_events_handlers()?;
        notify_controlcenteraux_on_startup();

        Taskbar::hide_taskbar();

        log::warn!("[Startup] 3. Triggering TASKBAR_STATE initialization (deferred)");
        // 只触发初始化，不获取锁
        let _ = &*TASKBAR_STATE;

        log::warn!("Synchronizing monitor instances");
        self.sync_monitor_instances()?;

        MonitorManager::subscribe(Self::on_monitor_event);
        SystemSettings::subscribe(Self::on_system_settings_change);

        register_win_hook()?;
        log::warn!("AppManager started successfully");
        APP_IS_RUNNING.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    /// Stop and release all resources
    pub fn stop(&self) {
        APP_IS_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
        release_system_events_handlers();

        log::info!("AppManager stopped successfully");
    }

    fn add_monitor(&mut self, view: MonitorView) -> Result<()> {
        let state = FULL_STATE.load();
        self.instances.push(SluMonitorInstance::new(view, &state)?);
        self.refresh_windows_positions()?;

        Ok(())
    }

    fn remove_monitor(&mut self, id: &MonitorId) -> Result<()> {
        let initial_count = self.instances.len();
        self.instances.retain(|m| &m.main_target_id != id);
        let removed_count = initial_count - self.instances.len();

        if removed_count > 0 {
            log::info!(
                "[AppManager] 移除显示器: {}, 剩余 {} 个实例",
                id,
                self.instances.len()
            );
        }

        self.refresh_windows_positions()?;
        trace_lock!(TASKBAR_STATE).emit_to_webview()?;
        Ok(())
    }
}
