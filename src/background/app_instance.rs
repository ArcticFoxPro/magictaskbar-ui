use libs_core::system_state::MonitorId;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;
use tauri::{WebviewWindow, Wry};

use crate::{
    error::Result, state::application::FullState, trace_lock, widgets::preview::Preview,
    widgets::taskbar::Taskbar, widgets::toolbar::FancyToolbar, windows_api::monitor::MonitorView,
};

/// This struct stores the widgets of a monitor.
/// Each widget is independently locked to allow fine-grained concurrent access.
pub struct SluMonitorInstance {
    pub view: MonitorView,
    pub main_target_id: MonitorId,
    pub last_layout_signature: Option<(i32, i32, i32, i32, i32, i32)>,
    pub taskbar: Arc<Mutex<Option<Taskbar>>>,
    pub toolbar: Arc<Mutex<Option<FancyToolbar>>>,
    pub preview: Arc<Mutex<Option<Preview>>>,
}

impl SluMonitorInstance {
    fn webview_failed(
        window: &WebviewWindow<Wry>,
        widget_name: &str,
        monitor_id: &MonitorId,
    ) -> bool {
        match window.is_visible() {
            Ok(_) => false,
            Err(e) => {
                log::error!(
                    "[SluMonitorInstance] {widget_name} webview is not responsive, monitor_id={}, label={}, err={e:?}",
                    monitor_id,
                    window.label()
                );
                true
            }
        }
    }

    pub fn has_broken_taskbar_webview(&self) -> bool {
        let Some(taskbar) = self.taskbar.try_lock() else {
            log::warn!(
                "[SluMonitorInstance] skip taskbar health check because lock is busy, monitor_id={}",
                self.main_target_id
            );
            return false;
        };

        taskbar
            .as_ref()
            .map(|tb| Self::webview_failed(&tb.window, "taskbar", &self.main_target_id))
            .unwrap_or(false)
    }

    pub fn new(view: MonitorView, settings: &FullState) -> Result<Self> {
        // 优先使用stable_id,为空时fallback到win32 name
        let main_target_id = match view.primary_target().and_then(|t| t.stable_id2()) {
            Ok(id) if !id.0.is_empty() => id,
            _ => {
                let win32_mon = view.as_win32_monitor()?;
                let name = win32_mon.name()?;
                log::warn!(
                    "[SluMonitorInstance] stable_id为空或获取失败,使用win32_name: {}",
                    name
                );
                name.into()
            }
        };

        log::info!(
            "[SluMonitorInstance] Creating instance with monitor ID: {}",
            main_target_id
        );

        let instance = Self {
            view,
            main_target_id,
            last_layout_signature: None,
            taskbar: Arc::new(Mutex::new(None)),
            toolbar: Arc::new(Mutex::new(None)),
            preview: Arc::new(Mutex::new(None)),
        };
        log::info!("[SluMonitorInstance] Calling load_settings");
        instance.load_settings(settings)?;
        log::info!("[SluMonitorInstance] load_settings done");
        Ok(instance)
    }

    pub fn ensure_positions(&self) -> Result<()> {
        let win32_monitor = self.view.as_win32_monitor()?;

        {
            log::info!("[SluMonitorInstance] ensure_positions: setting taskbar position");
            let mut taskbar = trace_lock!(self.taskbar);
            if let Some(tb) = taskbar.as_mut() {
                tb.set_position(win32_monitor.handle())?;
            }
            log::info!("[SluMonitorInstance] ensure_positions: taskbar position done");
        }
        let mut schedule_initial_toolbar_reposition = false;
        {
            log::info!("[SluMonitorInstance] ensure_positions: setting toolbar position");
            let mut toolbar = trace_lock!(self.toolbar);
            if let Some(tl) = toolbar.as_mut() {
                let first_position = !tl.is_positioned();
                tl.set_position(win32_monitor.handle())?;
                schedule_initial_toolbar_reposition = first_position;
                if let Some(delay) =
                    tl.take_deferred_fixed_appbar_restore_delay(win32_monitor.handle())
                {
                    crate::widgets::toolbar::schedule_deferred_fixed_appbar_restore(
                        self.toolbar.clone(),
                        delay,
                    );
                }
            }
            log::info!("[SluMonitorInstance] ensure_positions: toolbar position done");
        }
        if schedule_initial_toolbar_reposition {
            crate::widgets::toolbar::schedule_initial_reposition(self.toolbar.clone());
        }
        {
            log::info!("[SluMonitorInstance] ensure_positions: setting preview position");
            let mut preview = trace_lock!(self.preview);
            if let Some(pv) = preview.as_mut() {
                pv.set_initial_position(win32_monitor.handle())?;
            }
            log::info!("[SluMonitorInstance] ensure_positions: preview position done");
        }
        Ok(())
    }

    fn add_taskbar(&self) -> Result<()> {
        {
            let taskbar = trace_lock!(self.taskbar);
            if taskbar.is_some() {
                return Ok(());
            }
        }

        let taskbar = match Taskbar::new(&self.main_target_id) {
            Ok(tb) => tb,
            Err(e) => {
                log::error!(
                    "[SluMonitorInstance] Failed to create Taskbar for monitor_id={}: {e:?}",
                    self.main_target_id
                );
                return Err(e);
            }
        };

        let mut slot = trace_lock!(self.taskbar);
        if slot.is_none() {
            *slot = Some(taskbar);
        }
        Ok(())
    }

    fn add_toolbar(&self) -> Result<()> {
        {
            let toolbar = trace_lock!(self.toolbar);
            if toolbar.is_some() {
                return Ok(());
            }
        }

        let toolbar = match FancyToolbar::new(&self.main_target_id) {
            Ok(tl) => tl,
            Err(e) => {
                log::error!(
                    "[SluMonitorInstance] Failed to create FancyToolbar for monitor_id={}: {e:?}",
                    self.main_target_id
                );
                return Err(e);
            }
        };

        let mut slot = trace_lock!(self.toolbar);
        if slot.is_none() {
            *slot = Some(toolbar);
        }
        Ok(())
    }

    fn add_preview(&self) -> Result<()> {
        {
            let preview = trace_lock!(self.preview);
            if preview.is_some() {
                return Ok(());
            }
        }

        let preview = match Preview::new(&self.main_target_id) {
            Ok(pv) => pv,
            Err(e) => {
                log::error!(
                    "[SluMonitorInstance] Failed to create Preview for monitor_id={}: {e:?}",
                    self.main_target_id
                );
                // Preview 失败不影响主流程，只记录日志
                return Ok(());
            }
        };

        let mut slot = trace_lock!(self.preview);
        if slot.is_none() {
            *slot = Some(preview);
        }
        Ok(())
    }

    pub fn load_settings(&self, _state: &FullState) -> Result<()> {
        log::info!("[SluMonitorInstance] load_settings: adding taskbar");
        if let Err(e) = self.add_taskbar() {
            log::error!(
                "[SluMonitorInstance] add_taskbar failed for monitor_id={}, exiting UI: {e:?}",
                self.main_target_id
            );
            std::thread::sleep(Duration::from_secs(1));
            crate::report_ui_process_exit("TaskbarWindowCreateFailed");
            crate::app::get_app_handle().exit(1);
            return Err(e);
        }
        log::info!("[SluMonitorInstance] load_settings: taskbar added, adding toolbar");
        if let Err(e) = self.add_toolbar() {
            log::error!(
                "[SluMonitorInstance] add_toolbar failed for monitor_id={}, exiting UI: {e:?}",
                self.main_target_id
            );
            std::thread::sleep(Duration::from_secs(1));
            crate::report_ui_process_exit("ToolbarWindowCreateFailed");
            crate::app::get_app_handle().exit(1);
            return Err(e);
        }
        log::info!("[SluMonitorInstance] load_settings: toolbar added, adding preview");
        // Preview 窗口创建失败不影响主流程
        let _ = self.add_preview();
        log::info!("[SluMonitorInstance] load_settings: all widgets added");
        // ContextMenu 窗口改为懒创建，不再在启动时创建
        Ok(())
    }
}

unsafe impl Send for SluMonitorInstance {}
unsafe impl Sync for SluMonitorInstance {}
