use libs_core::{handlers::FuncEvent, system_state::PhysicalMonitor};
use tauri::Emitter;

use crate::{
    app::get_app_handle,
    error::Result,
    log_error,
    modules::monitors::MONITOR_MANAGER,
    windows_api::{MonitorEnumerator, WindowsApi},
};

use super::MonitorManager;

pub fn register_monitor_webview_events() -> Result<()> {
    MONITOR_MANAGER.lock().init()?;
    MonitorManager::subscribe(|_event| {
        if let Ok(monitors) = get_connected_monitors() {
            log_error!(get_app_handle().emit(FuncEvent::SystemMonitorsChanged, monitors));
        }
    });
    Ok(())
}

#[tauri::command(async)]
pub fn get_connected_monitors() -> Result<Vec<PhysicalMonitor>> {
    let win32_monitors = MonitorEnumerator::get_all_v2()?;
    let mut monitors = Vec::new();

    for win32_mon in win32_monitors.iter() {
        let name = win32_mon.name()?; // 设备名如\\.\\ DISPLAY1
        let rect = win32_mon.rect()?;
        let dpi = WindowsApi::get_monitor_scale_factor(win32_mon.handle())?;

        // 使用stable_id作为id,为空时fallback到name
        let id = match win32_mon.stable_id2() {
            Ok(stable_id) if !stable_id.0.is_empty() => stable_id,
            _ => name.clone().into(), // fallback到name
        };

        let physical_monitor = PhysicalMonitor {
            id,
            name,
            rect,
            dpi,
        };

        log::info!("[get_connected_monitors] Monitor: id={}, name={}, rect={{left:{}, top:{}, right:{}, bottom:{}}}, dpi={}",
            physical_monitor.id, physical_monitor.name,
            physical_monitor.rect.left, physical_monitor.rect.top,
            physical_monitor.rect.right, physical_monitor.rect.bottom,
            physical_monitor.dpi);

        monitors.push(physical_monitor);
    }
    Ok(monitors)
}
