use std::{collections::HashMap, path::PathBuf, sync::Arc};

use libs_core::state::{
    by_monitor::MonitorConfiguration, IconPack, PerformanceMode, TaskbarAppGroupItem, TaskbarItem,
    TaskbarItems, TaskbarPinnedItemsVisibility,
};

use crate::{
    error::Result,
    state::application::performance::PERFORMANCE_MODE,
    trace_lock,
    widgets::taskbar::taskbar_items_impl::{set_skip_file_sync, TASKBAR_STATE},
    windows_api::window::Window,
};

use super::{application::FULL_STATE, domain::Settings};

// 简单的注册表读写工具：HKCU\SOFTWARE\HONOR\Magicanimation -> FancyToolbarHideMode
fn read_toolbar_hide_mode_from_registry() -> Option<libs_core::state::HideMode> {
    use winreg::{
        enums::{HKEY_CURRENT_USER, KEY_READ},
        RegKey,
    };
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(r"SOFTWARE\HONOR\Magicanimation", KEY_READ) {
        if let Ok(val) = key.get_value::<String, _>("FancyToolbarHideMode") {
            match val.as_str() {
                "Never" => return Some(libs_core::state::HideMode::Never),
                "OnOverlap" => return Some(libs_core::state::HideMode::OnOverlap),
                "Always" => return Some(libs_core::state::HideMode::Always),
                _ => {}
            }
        }
    }
    None
}

fn write_toolbar_hide_mode_to_registry(mode: libs_core::state::HideMode) {
    use winreg::{
        enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS},
        RegKey,
    };
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let val = match mode {
        libs_core::state::HideMode::Never => "Never",
        libs_core::state::HideMode::OnOverlap => "OnOverlap",
        libs_core::state::HideMode::Always => "Always",
    };
    if let Ok((key, _disp)) =
        hkcu.create_subkey_with_flags(r"SOFTWARE\HONOR\Magicanimation", KEY_ALL_ACCESS)
    {
        let _ = key.set_value("FancyToolbarHideMode", &val);
    } else {
        log::warn!(
            r"Failed to open/create registry key SOFTWARE\HONOR\Magicanimation for hide mode"
        );
    }
}

// 快捷键注册表操作函数
fn read_shortcut_keys_from_registry() -> Vec<String> {
    use winreg::{
        enums::{HKEY_CURRENT_USER, KEY_READ},
        RegKey,
    };
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(r"SOFTWARE\HONOR\Magicanimation", KEY_READ) {
        if let Ok(val) = key.get_value::<String, _>("ShortcutKey") {
            if !val.is_empty() {
                return val
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }
    }
    Vec::new()
}

fn write_shortcut_keys_to_registry(shortcut_ids: &[String]) {
    use winreg::{
        enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS},
        RegKey,
    };
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let val = shortcut_ids.join(",");
    if let Ok((key, _disp)) =
        hkcu.create_subkey_with_flags(r"SOFTWARE\HONOR\Magicanimation", KEY_ALL_ACCESS)
    {
        let _ = key.set_value("ShortcutKey", &val);
    } else {
        log::warn!(
            r"Failed to open/create registry key SOFTWARE\HONOR\Magicanimation for shortcut keys"
        );
    }
}

#[tauri::command(async)]
pub fn state_get_icon_packs() -> Vec<IconPack> {
    let mutex = FULL_STATE.load().icon_packs().clone();
    let icon_packs = trace_lock!(mutex);
    icon_packs.owned_list()
}

#[tauri::command(async)]
pub fn state_write_taskbar_items(window: tauri::Window, mut items: TaskbarItems) -> Result<()> {
    let windows_by_id: HashMap<String, Vec<TaskbarAppGroupItem>> = {
        let runtime = trace_lock!(TASKBAR_STATE);
        runtime
            .items
            .left
            .iter()
            .chain(runtime.items.center.iter())
            .chain(runtime.items.right.iter())
            .filter_map(|item| {
                if let TaskbarItem::Pinned(data) | TaskbarItem::Temporal(data) = item {
                    if !data.windows.is_empty() {
                        return Some((data.id.clone(), data.windows.clone()));
                    }
                }
                None
            })
            .collect()
    };

    for item in items
        .left
        .iter_mut()
        .chain(items.center.iter_mut())
        .chain(items.right.iter_mut())
    {
        if let TaskbarItem::Pinned(data) | TaskbarItem::Temporal(data) = item {
            if let Some(windows) = windows_by_id.get(&data.id) {
                data.windows = windows.clone();
            }
        }
    }

    items.sanitize();

    // Fix: ensure StartMenu in left and RecycleBin in right
    let startmenu_in_left = items
        .left
        .iter()
        .any(|item| matches!(item, TaskbarItem::StartMenu { .. }));
    let recycle_in_right = items
        .right
        .iter()
        .any(|item| matches!(item, TaskbarItem::RecycleBin { .. }));
    log::info!(
        "[TaskbarItems] state_write: StartMenu in left: {}, RecycleBin in right: {}",
        startmenu_in_left,
        recycle_in_right
    );
    if !startmenu_in_left || !recycle_in_right {
        log::info!("[TaskbarItems] state_write: Fixing positions...");
    }

    let guard = FULL_STATE.load();

    let monitor = Window::from(window.hwnd()?.0 as isize).monitor();
    let device_id = monitor.stable_id2()?;
    let pinned_items_visibility = guard.get_taskbar_pinned_item_visibility(&device_id);
    let is_non_primary = !monitor.is_primary();
    let items_equal = items == guard.taskbar_items;
    let should_skip_write = pinned_items_visibility == TaskbarPinnedItemsVisibility::WhenPrimary
        && is_non_primary
        || (items_equal
            && items.left.len() == guard.taskbar_items.left.len()
            && items.center.len() == guard.taskbar_items.center.len());

    if should_skip_write {
        return Ok(());
    }

    let mut new_state = (**guard).clone();
    new_state.taskbar_items = items.clone();

    set_skip_file_sync(500);
    guard.write_taskbar_items(&items)?;
    FULL_STATE.store(Arc::new(new_state));

    {
        let mut runtime = trace_lock!(TASKBAR_STATE);
        runtime.items = items;
        runtime.emit_to_webview()?;
    }

    Ok(())
}

#[tauri::command(async)]
pub fn state_get_settings(path: Option<PathBuf>) -> Result<Settings> {
    // 尝试从文件加载设置，如果失败则使用默认设置
    let mut settings = match path {
        Some(p) => Settings::load(p)?,
        None => Settings::load(crate::utils::constants::VAR_COMMON.settings_path())?,
    };
    // 从注册表读取 FancyToolbarHideMode 并覆盖默认值
    if let Some(mode) = read_toolbar_hide_mode_from_registry() {
        log::info!(
            "[Toolbar] override fancy_toolbar.hide_mode from registry: {:?}",
            mode
        );
        settings.by_widget.fancy_toolbar.hide_mode = mode;
    } else {
        log::info!("[Toolbar] no FancyToolbarHideMode found in registry, keep default");
    }
    settings.sanitize()?;
    Ok(settings)
}

#[tauri::command(async)]
pub fn state_get_default_settings() -> Result<Settings> {
    let mut settings = Settings::default();
    settings.sanitize()?;
    Ok(settings)
}

#[tauri::command(async)]
pub fn state_get_default_monitor_settings() -> MonitorConfiguration {
    MonitorConfiguration::default()
}

#[tauri::command(async)]
pub fn state_write_settings(settings: Settings) -> Result<()> {
    // 避免闭包 move 导致后续使用 settings 报 E0382
    let settings_for_state = settings.clone();
    FULL_STATE.rcu(move |state| {
        let mut state = state.cloned();
        state.settings = settings_for_state.clone();
        state
    });
    log::info!(
        "[Toolbar] write fancy_toolbar.hide_mode to registry: {:?}",
        settings.by_widget.fancy_toolbar.hide_mode
    );
    // 写入注册表以便下次启动记忆模式
    write_toolbar_hide_mode_to_registry(settings.by_widget.fancy_toolbar.hide_mode);
    FULL_STATE.load().write_settings()
}

#[tauri::command(async)]
pub fn state_delete_cached_icons() -> Result<()> {
    let mutex = FULL_STATE.load().icon_packs().clone();
    let mut icon_manager = trace_lock!(mutex);
    icon_manager.clear_system_icons()?;
    icon_manager.sanitize_system_icon_pack(false)?;
    icon_manager.write_system_icon_pack()?;
    drop(icon_manager);
    FULL_STATE.load().emit_icon_packs()?;
    Ok(())
}

#[tauri::command(async)]
pub fn state_get_performance_mode() -> PerformanceMode {
    **PERFORMANCE_MODE.load()
}

#[tauri::command(async)]
pub fn shortcut_get_keys() -> Vec<String> {
    read_shortcut_keys_from_registry()
}

#[tauri::command(async)]
pub fn shortcut_save_keys(shortcut_ids: Vec<String>) {
    write_shortcut_keys_to_registry(&shortcut_ids);
}
