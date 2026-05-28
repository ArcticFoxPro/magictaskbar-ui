use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::Arc,
};

use image::ImageFormat;
use libs_core::{
    state::{
        PinnedTaskbarItemData, RelaunchArguments, TaskbarAppGroupItem, TaskbarItem,
        TaskbarItemSubtype, TaskbarItems, TaskbarSide,
    },
    system_state::MonitorId,
};
use std::{thread, time::Duration};
use tauri::Emitter;

use super::Taskbar;
use crate::{
    app::get_app_handle,
    cli::ServicePipe,
    error::Result,
    modules::apps::application::{UserAppsManager, USER_APPS_MANAGER},
    state::application::FULL_STATE,
    trace_lock,
    utils::icon_whitelist,
    widgets::taskbar::taskbar_items_impl::{report_taskbar_state, TaskbarState, TASKBAR_STATE},
    windows_api::{window::Window, WindowsApi},
};
use slu_ipc::messages::SvcAction;

fn relaunch_args_to_string(args: &RelaunchArguments) -> String {
    match args {
        RelaunchArguments::String(value) => value.clone(),
        RelaunchArguments::Array(values) => values.join(" "),
    }
}

#[tauri::command(async)]
pub fn state_get_taskbar_items(monitor_id: Option<MonitorId>) -> TaskbarItems {
    // 先获取 items 的克隆，然后立即释放锁
    // 避免在持有锁期间调用可能耗时的 get_filtered_by_monitor
    let items = {
        let guard = trace_lock!(TASKBAR_STATE);
        guard.items.clone()
    };

    if let Some(id) = monitor_id {
        // 创建临时的 TaskbarState 来执行过滤，不需要持有全局锁
        let temp_state = TaskbarState { items };
        let filtered = temp_state
            .get_filtered_by_monitor()
            .unwrap_or_default()
            .get(&id)
            .cloned();
        return filtered.unwrap_or_else(|| temp_state.items.clone());
    }
    items
}

#[tauri::command(async)]
pub fn taskbar_request_update_previews(handles: Vec<isize>) -> Result<()> {
    let temp_dir = std::env::temp_dir();

    for addr in handles {
        let window = Window::from(addr);

        if !window.is_visible() || window.is_minimized() {
            continue;
        }

        let image = Taskbar::capture_window(window.hwnd());
        if let Some(image) = image {
            let rect = WindowsApi::get_inner_window_rect(window.hwnd())?;
            let shadow = WindowsApi::shadow_rect(window.hwnd())?;
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;

            let image = image.crop_imm(
                shadow.left.unsigned_abs(),
                shadow.top.unsigned_abs(),
                width as u32,
                height as u32,
            );

            image.save_with_format(temp_dir.join(format!("{addr}.png")), ImageFormat::Png)?;
            get_app_handle().emit(format!("taskbar-preview-update-{addr}").as_str(), ())?;
        }
    }
    Ok(())
}

fn schedule_close_reconcile(hwnd: isize, process_name: String, reason: &'static str) {
    thread::spawn(move || {
        let checkpoints = [300_u64, 1_000, 2_000];
        let mut elapsed = 0_u64;

        for checkpoint in checkpoints {
            thread::sleep(Duration::from_millis(checkpoint.saturating_sub(elapsed)));
            elapsed = checkpoint;

            let window = Window::from(hwnd);
            if !USER_APPS_MANAGER.contains_win(&window) {
                log::debug!(
                    "[taskbar_close_reconcile] already removed reason={} hwnd={:?}, process={}, after={}ms",
                    reason,
                    hwnd,
                    process_name,
                    checkpoint
                );
                break;
            }

            log::debug!(
                "[taskbar_close_reconcile] checking reason={} hwnd={:?}, process={}, after={}ms",
                reason,
                hwnd,
                process_name,
                checkpoint
            );
            if UserAppsManager::reconcile_closed_window(
                &window,
                &format!("{}:{}ms", reason, checkpoint),
            ) {
                break;
            }
        }
    });
}

#[tauri::command(async)]
pub fn taskbar_close_app(hwnd: isize) -> Result<()> {
    let window = Window::from(hwnd);
    let process_name = window.process().program_exe_name().unwrap_or_default();
    let service_result = ServicePipe::request(SvcAction::ExecuteBackendCommand {
        command: "taskbar_close_app".to_string(),
        args: serde_json::json!({ "hwnd": hwnd.to_string() }),
    });
    if service_result.is_ok() {
        schedule_close_reconcile(hwnd, process_name, "service_close");
    }
    service_result
}

#[tauri::command(async)]
pub fn taskbar_kill_app(hwnd: isize) -> Result<()> {
    let window = Window::from(hwnd);
    let process_name = window.process().program_exe_name().unwrap_or_default();
    ServicePipe::request(SvcAction::ExecuteBackendCommand {
        command: "taskbar_kill_app".to_string(),
        args: serde_json::json!({ "hwnd": hwnd.to_string() }),
    })?;
    schedule_close_reconcile(hwnd, process_name, "taskkill");
    Ok(())
}

#[tauri::command(async)]
pub fn taskbar_toggle_window_state(hwnd: isize, was_focused: bool) -> Result<()> {
    let window = Window::from(hwnd);
    let title = window.title();

    log::info!(
        "[taskbar_toggle_window_state] target hwnd={:?}, title=\"{}\", was_focused={}",
        window.address(),
        title,
        was_focused
    );
    ServicePipe::request(SvcAction::ExecuteBackendCommand {
        command: "taskbar_toggle_window_state".to_string(),
        args: serde_json::json!({ "hwnd": hwnd.to_string() }),
    })
}

#[tauri::command(async)]
pub fn set_foreground_window(hwnd: isize) -> Result<()> {
    ServicePipe::request(SvcAction::ExecuteBackendCommand {
        command: "taskbar_set_foreground_window".to_string(),
        args: serde_json::json!({ "hwnd": hwnd.to_string() }),
    })
}

#[allow(deprecated)]
#[tauri::command(async)]
pub fn taskbar_pin_item(
    umid: Option<String>,
    relaunch_program: String,
    display_name: String,
    path: PathBuf,
    original_id: Option<String>,
    relaunch_args: Option<String>, // 新增参数：用于UWP应用的启动参数
    target_index: Option<usize>,   // 新增参数：指定固定项在左侧区域的目标位置
) -> Result<()> {
    // 关键修复：提前解析目标路径和 UMID，确保重复项检查能够识别出同一个应用
    let mut umid = umid;
    let mut relaunch_program = relaunch_program;
    let mut display_name = display_name;
    let mut path = path;
    let mut relaunch_args = relaunch_args;

    let incoming_umid_lower = umid.as_ref().map(|value| value.to_lowercase());
    let incoming_display_name_lower = display_name.to_lowercase();
    let runtime_item = {
        let state = trace_lock!(TASKBAR_STATE);
        state
            .items
            .center
            .iter()
            .chain(state.items.right.iter())
            .filter_map(|item| match item {
                TaskbarItem::Temporal(data) => Some(data),
                _ => None,
            })
            .find(|data| {
                original_id.as_ref().map_or(false, |id| data.id == *id)
                    || incoming_umid_lower.as_ref().map_or(false, |incoming_umid| {
                        data.umid.as_ref().map_or(false, |data_umid| {
                            data_umid.to_lowercase() == incoming_umid.as_str()
                        })
                    })
                    || (!incoming_display_name_lower.is_empty()
                        && data.display_name.to_lowercase() == incoming_display_name_lower)
            })
            .cloned()
    };

    if let Some(runtime_data) = runtime_item {
        let before_path = path.clone();
        let before_relaunch_program = relaunch_program.clone();
        let before_relaunch_args = relaunch_args.clone();
        let before_umid = umid.clone();

        if path.as_os_str().is_empty() && !runtime_data.path.as_os_str().is_empty() {
            path = runtime_data.path.clone();
        }
        if relaunch_program.is_empty() && !runtime_data.relaunch_program.is_empty() {
            relaunch_program = runtime_data.relaunch_program.clone();
        }
        if relaunch_args.is_none() {
            if let Some(args) = runtime_data.relaunch_args.as_ref() {
                relaunch_args = Some(relaunch_args_to_string(args));
            }
        }
        if umid.is_none() {
            umid = runtime_data.umid.clone();
        }
        if display_name.is_empty() && !runtime_data.display_name.is_empty() {
            display_name = runtime_data.display_name.clone();
        }

        log::info!(
            "[Taskbar][PinHydrate] original_id={:?}, runtime_id={}, display='{}', path='{}' -> '{}', relaunch='{}' -> '{}', args={:?} -> {:?}, umid={:?} -> {:?}",
            original_id,
            runtime_data.id,
            display_name,
            before_path.to_string_lossy(),
            path.to_string_lossy(),
            before_relaunch_program,
            relaunch_program,
            before_relaunch_args,
            relaunch_args,
            before_umid,
            umid
        );
    }

    let (target_program, resolved_arguments) = if path.extension() == Some(OsStr::new("lnk")) {
        WindowsApi::resolve_lnk_target(&path).unwrap_or((path.clone(), std::ffi::OsString::new()))
    } else {
        (path.clone(), std::ffi::OsString::new())
    };

    let resolved_umid = umid
        .clone()
        .or_else(|| {
            if path.extension() == Some(OsStr::new("lnk")) {
                WindowsApi::get_file_umid(&path).ok()
            } else {
                None
            }
        })
        .or_else(|| {
            // 多应用启动器（如 Androws）的 .lnk 可能没有 AppUserModel_ID 属性，
            // 但可以从启动器参数中提取子应用标识来推断 UMID
            let target_name = target_program
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if let Some(config) = crate::utils::constants::find_launcher_by_exe(target_name) {
                let args_str = resolved_arguments.to_string_lossy();
                if let Some(pkg) =
                    crate::utils::constants::extract_launcher_arg_value(&args_str, config.arg_key)
                {
                    let umid = format!("{}{}", config.umid_prefix, pkg);
                    log::debug!("[Taskbar] 从 .lnk 参数推断多应用启动器 UMID: {}", umid);
                    return Some(umid);
                }
            }
            None
        });

    let actual_relaunch_program = if path.extension() == Some(OsStr::new("lnk")) {
        target_program.to_string_lossy().to_string()
    } else {
        relaunch_program.clone()
    };

    // 检查是否已经存在相同的固定项，避免重复添加
    let current_state = FULL_STATE.load();
    let already_exists = current_state.taskbar_items.left.iter().any(|item| {
        if let TaskbarItem::Pinned(pinned_item) = item {
            // 1. 优先使用UMID匹配（最可靠）
            if let (Some(item_umid), Some(ref incoming_umid)) = (&pinned_item.umid, &resolved_umid)
            {
                if item_umid == incoming_umid {
                    log::debug!("[Taskbar] 重复项检查:UMID匹配,已存在固定项: {}", item_umid);
                    return true;
                }
            }

            let is_pinned_explorer = pinned_item
                .umid
                .as_ref()
                .map_or(false, |u| u == "Microsoft.Windows.Explorer")
                || pinned_item.display_name.to_lowercase().contains("explorer")
                || pinned_item
                    .display_name
                    .to_lowercase()
                    .contains("资源管理器");

            let is_incoming_explorer = resolved_umid
                .as_ref()
                .map_or(false, |u| u == "Microsoft.Windows.Explorer")
                || display_name.to_lowercase().contains("explorer")
                || display_name.to_lowercase().contains("资源管理器");

            if is_incoming_explorer && is_pinned_explorer {
                log::debug!("[Taskbar] 重复项检查:资源管理器匹配");
                return true;
            }

            // 3. 显示名称匹配（排除资源管理器后）
            let display_name_normalized = display_name.to_lowercase().replace(".lnk", "");
            let pinned_display_name_normalized =
                pinned_item.display_name.to_lowercase().replace(".lnk", "");

            if display_name_normalized == pinned_display_name_normalized {
                log::debug!(
                    "[Taskbar] 重复项检查:显示名称匹配,已存在固定项: {} vs {}",
                    pinned_item.display_name,
                    display_name
                );
                return true;
            }

            // 4. 最后使用relaunch_program匹配 (使用解析后的真实路径)
            // 关键修复：避免与资源管理器的relaunch_program误匹配
            let is_pinned_item_explorer = pinned_item
                .umid
                .as_ref()
                .map_or(false, |u| u == "Microsoft.Windows.Explorer")
                || pinned_item.display_name.to_lowercase().contains("explorer")
                || pinned_item
                    .display_name
                    .to_lowercase()
                    .contains("资源管理器");

            let is_incoming_item_explorer = resolved_umid
                .as_ref()
                .map_or(false, |u| u == "Microsoft.Windows.Explorer")
                || display_name.to_lowercase().contains("explorer")
                || display_name.to_lowercase().contains("资源管理器");

            // 检查是否为UWP应用（通过UMID或启动参数识别）
            let is_pinned_uwp = pinned_item.umid.is_some()
                && (pinned_item
                    .relaunch_program
                    .to_lowercase()
                    .contains("explorer.exe")
                    || pinned_item
                        .relaunch_args
                        .as_ref()
                        .map_or(false, |args| match args {
                            RelaunchArguments::String(s) => s.contains("shell:AppsFolder"),
                            _ => false,
                        }));

            let is_incoming_uwp = resolved_umid.is_some()
                && (actual_relaunch_program
                    .to_lowercase()
                    .contains("explorer.exe")
                    || relaunch_args
                        .as_ref()
                        .map_or(false, |args| args.contains("shell:AppsFolder")));

            // 只有当两者都不是资源管理器且都不是UWP应用时，才使用relaunch_program匹配
            // 对于UWP应用，relaunch_program通常都是explorer.exe，需要通过UMID或启动参数判断
            if !is_pinned_item_explorer
                && !is_incoming_item_explorer
                && !is_pinned_uwp
                && !is_incoming_uwp
            {
                if pinned_item.relaunch_program.to_lowercase()
                    == actual_relaunch_program.to_lowercase()
                {
                    // 当两者都有 UMID 时，必须 UMID 相同才算重复（如 Androws 子应用共用 AndrowsLauncher.exe）
                    if pinned_item.umid.is_some() && resolved_umid.is_some() {
                        if pinned_item.umid == resolved_umid {
                            log::debug!(
                                "[Taskbar] 重复项检查:relaunch_program+UMID匹配,已存在固定项: {}",
                                pinned_item.display_name
                            );
                            return true;
                        }
                    } else {
                        // 同一 exe 但无 UMID 时，还需比较启动参数
                        // 避免 Androws 子应用共用 AndrowsLauncher.exe 但参数不同被误判为重复
                        let pinned_args_str = pinned_item
                            .relaunch_args
                            .as_ref()
                            .map(|a| match a {
                                RelaunchArguments::String(s) => s.to_lowercase(),
                                RelaunchArguments::Array(arr) => arr.join(" ").to_lowercase(),
                            })
                            .unwrap_or_default();
                        let incoming_args_str = resolved_arguments.to_string_lossy().to_lowercase();
                        if pinned_args_str == incoming_args_str
                            || (pinned_args_str.is_empty() && incoming_args_str.is_empty())
                        {
                            log::debug!(
                                "[Taskbar] 重复项检查:relaunch_program+args匹配,已存在固定项: {}",
                                pinned_item.display_name
                            );
                            return true;
                        }
                    }
                }
            }
            // 对于UWP应用，需要检查启动参数或UMID是否匹配
            else if is_pinned_uwp && is_incoming_uwp {
                // 优先检查UMID匹配
                if let (Some(item_umid), Some(incoming_umid)) = (&pinned_item.umid, &resolved_umid)
                {
                    if item_umid == incoming_umid {
                        log::debug!(
                            "[Taskbar] 重复项检查:UWP应用UMID匹配,已存在固定项: {}",
                            item_umid
                        );
                        return true;
                    }
                }
                // 检查启动参数匹配
                else if let (Some(item_args), Some(incoming_args)) =
                    (&pinned_item.relaunch_args, &relaunch_args)
                {
                    let item_args_str = match item_args {
                        RelaunchArguments::String(s) => s.to_string(),
                        RelaunchArguments::Array(arr) => arr.join(" "),
                    };
                    if item_args_str == *incoming_args {
                        log::debug!(
                            "[Taskbar] 重复项检查:UWP应用启动参数匹配,已存在固定项: {}",
                            pinned_item.display_name
                        );
                        return true;
                    }
                }
            }
        }
        false
    });

    if already_exists {
        log::info!("[Taskbar] 固定项已存在，跳过添加: {}", display_name);
        return Ok(());
    }

    // 检查是否是UWP应用
    let is_uwp_app = if let Some(ref umid_str) = resolved_umid {
        WindowsApi::is_uwp_package_id(umid_str)
    } else {
        false
    };

    // 确定启动参数
    let mut arguments_os = resolved_arguments;
    if relaunch_args.is_some() {
        arguments_os = std::ffi::OsString::from(relaunch_args.clone().unwrap());
    } else if resolved_umid.is_some() {
        // 多应用启动器需要保留原始 .lnk 参数（如 --launch-pkg-name），
        // 而不是用 shell:AppsFolder\{UMID} 覆盖，否则启动器不知道打开哪个子应用
        let is_multi_app_launcher = target_program
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| crate::utils::constants::find_launcher_by_exe(n))
            .is_some();
        if !is_multi_app_launcher {
            if let Some(ref umid_str) = resolved_umid {
                arguments_os = std::ffi::OsString::from(format!("shell:AppsFolder\\{}", umid_str));
            }
        }
    }

    // 确定子类型
    let subtype = if target_program.is_dir() {
        TaskbarItemSubtype::Folder
    } else if target_program.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe")) ||
              std::path::Path::new(&relaunch_program).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe")) ||
              // 特殊处理：基于UMID识别真正的文件资源管理器应用
              resolved_umid.as_ref().map_or(false, |u| u == "Microsoft.Windows.Explorer")
    {
        TaskbarItemSubtype::App
    } else {
        TaskbarItemSubtype::File
    };

    // 特殊处理：统一File Explorer的显示名称
    let final_display_name = if subtype == TaskbarItemSubtype::App
        && (resolved_umid
            .as_ref()
            .map_or(false, |u| u == "Microsoft.Windows.Explorer"))
    {
        "app_menu.explorer".to_string()
    } else {
        display_name.clone()
    };

    // 仅通过yml文件管理任务栏固定，不操作Windows原生任务栏
    if is_uwp_app {
        if let Some(ref umid_str) = resolved_umid {
            log::debug!("固定UWP应用到MagicTaskbar任务栏: {}", umid_str);
        }
    } else {
        log::debug!("固定应用到MagicTaskbar任务栏: {}", final_display_name);
    }

    // 更新 FULL_STATE 中的 taskbar_items
    let current_state = FULL_STATE.load();
    let relaunch_args_val = if arguments_os.is_empty() {
        None
    } else {
        Some(RelaunchArguments::String(
            arguments_os.to_string_lossy().to_string(),
        ))
    };

    // 检查本地图标目录是否存在对应进程的图标，如果存在则标记为正方形（不显示背板）
    let process_name = Path::new(&actual_relaunch_program)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| final_display_name.clone());

    // 获取当前背板模式
    let settings = FULL_STATE.load();
    let backplate_style = settings.settings().taskbar.icon_backplate_style;

    // 本地图标不显示背板，非本地图标显示背板
    let has_local_icon = match backplate_style {
        libs_core::state::IconBackplateStyle::White => {
            icon_whitelist::has_local_process_icon_white(&process_name)
        }
        _ => icon_whitelist::has_local_process_icon(&process_name),
    };
    let is_approximately_square = Some(has_local_icon);

    let data = PinnedTaskbarItemData {
        id: original_id
            .as_ref()
            .map(|id| id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()), // 使用前端的原始ID，或生成新ID
        subtype,
        umid: resolved_umid.clone(),
        display_name: final_display_name.clone(),
        icon_hash: None,
        is_approximately_square,
        path: path.clone(),
        is_dir: target_program.is_dir(),
        relaunch_command: None,
        relaunch_program: actual_relaunch_program.clone(), // 使用实际的可执行程序路径
        relaunch_args: relaunch_args_val,
        relaunch_in: None,
        windows: vec![],
        pin_disabled: false,
    };

    // 克隆当前状态并修改
    let mut new_state = (**current_state).clone();
    let mut items = new_state.taskbar_items.clone();

    // 根据目标位置插入新的固定项
    // 如果提供了目标位置，则插入到特定位置；否则添加到末尾
    let pinned_data = TaskbarItem::Pinned(data);
    if let Some(idx) = target_index {
        // 确保索引有效
        let insert_pos = std::cmp::min(idx, items.left.len());
        items.left.insert(insert_pos, pinned_data);
    } else {
        // 没有目标位置信息，直接添加到末尾
        items.left.push(pinned_data);
    }

    // 🔧 关键修复：从 center 中删除被固定的项（如果存在）
    // 因为前端乐观更新时可能把项从 center 移到了 left，
    // 但后端的 center 中还保留着该项，导致前后端不一致
    // 注意：UWP 应用的 relaunch_program 是 explorer.exe，不能用于匹配
    let original_count = items.center.len();
    let new_relaunch_lower = actual_relaunch_program.to_lowercase();
    let is_uwp_app = resolved_umid.is_some();
    let new_relaunch_path = std::path::Path::new(&new_relaunch_lower);
    let new_relaunch_file_name = new_relaunch_path.file_name();
    items.center.retain(|item| {
            if let TaskbarItem::Temporal(temporal_item) = item {
            // 1. 优先使用 ID 匹配
            if let Some(original_id_value) = &original_id {
                if temporal_item.id == *original_id_value {
                    log::debug!(
                        "[Taskbar] 通过 ID 匹配移除中心项: id={}, display_name={}", temporal_item.id, temporal_item.display_name);
                    return false;
                }
            }

            // 2. 使用 UMID 匹配 (最适用于 UWP 应用)
            if let (Some(u1), Some(u2)) = (&temporal_item.umid, &resolved_umid) {
                if u1.to_lowercase() == u2.to_lowercase() {
                    log::debug!(
                        "[Taskbar] 通过 UMID 匹配移除中心项: umid={}, display_name={}", u1, temporal_item.display_name);
                    return false;
                }
            }

            // 3. 使用路径匹配（仅限非 UWP 应用，因为 UWP 的 relaunch_program 是 explorer.exe）
            if !is_uwp_app {
                let item_relaunch_lower = temporal_item.relaunch_program.to_lowercase();

                // 🔧 检查临时项本身是否是 UWP 应用
                // UWP 应用：有 UMID 或 relaunch_args 包含 shell:Appsfolder
                let item_is_uwp = temporal_item.umid.is_some()
                    || temporal_item.relaunch_args.as_ref().map_or(false, |args| {
                        match args {
                            libs_core::state::RelaunchArguments::String(s) => s.to_lowercase().contains("shell:appsfolder"),
                            libs_core::state::RelaunchArguments::Array(arr) => arr.iter().any(|s| s.to_lowercase().contains("shell:appsfolder")),
                        }
                    });

                if item_relaunch_lower == new_relaunch_lower {
                    // 如果两个都是 explorer.exe，需要额外检查：
                    // - 如果临时项是 UWP 应用，不应该被匹配（因为它的 explorer.exe 只是启动器）
                    // - 如果临时项不是 UWP 应用，才是真正的资源管理器窗口
                    if item_relaunch_lower.ends_with("explorer.exe") && item_is_uwp {
                        log::debug!(
                            "[Taskbar] 跳过 UWP 应用的 explorer.exe 匹配: display_name={}, umid={:?}",
                            temporal_item.display_name,
                            temporal_item.umid
                        );
                    } else {
                        log::debug!(
                            "[Taskbar] 通过路径匹配移除中心项: path={}, display_name={}", temporal_item.relaunch_program, temporal_item.display_name);
                        return false;
                    }
                }

                // 4. 使用文件名或目录关联匹配（处理托盘服务进程与主程序的关联）
                let item_path = std::path::Path::new(&item_relaunch_lower);
                if let Some(fname) = new_relaunch_file_name {
                    let fname_lower = fname.to_string_lossy().to_lowercase();
                    let is_setup_file = fname_lower == "setup.exe" || fname_lower == "install.exe";

                    // 4.1 文件名直接匹配（但对于 setup.exe 等通用安装程序，跳过此匹配）
                    if !is_setup_file && item_path.file_name() == Some(fname) {
                        // 🔧 对于 explorer.exe，同样需要排除 UWP 应用
                        if fname_lower == "explorer.exe" && item_is_uwp {
                            log::debug!(
                                "[Taskbar] 跳过 UWP 应用的 explorer 文件名匹配: display_name={}",
                                temporal_item.display_name
                            );
                        } else {
                            log::debug!(
                                "[Taskbar] 通过文件名匹配移除中心项: exe={}, display_name={}",
                                fname.to_string_lossy(),
                                temporal_item.display_name
                            );
                            return false;
                        }
                    }

                    // 4.2 🔧 智能目录关联匹配
                    if !is_setup_file {
                        if let (Some(p1), Some(p2)) = (item_path.parent(), new_relaunch_path.parent()) {
                            let p1_str = p1.to_string_lossy().to_lowercase();
                            let p2_str = p2.to_string_lossy().to_lowercase();
                            // 排除系统目录以防误杀
                            if !p1_str.ends_with("system32") && !p1_str.ends_with("syswow64") {
                                if p1_str == p2_str || p1_str.starts_with(&p2_str) || p2_str.starts_with(&p1_str) {
                                    log::debug!(
                                        "[Taskbar] 通过目录关联匹配移除中心项: tray_dir={}, fixed_dir={}, display_name={}",
                                        p1_str,
                                        p2_str,
                                        temporal_item.display_name
                                    );
                                    return false;
                                }
                            }
                        }
                    }
                }

                // 5. 使用显示名称匹配（对所有应用都适用，包括 UWP）
                // 已移到 !is_uwp_app 块外面

                // 6. 特殊处理：可执行文件名匹配
                let item_exe_name = std::path::Path::new(&temporal_item.relaunch_program)
                    .file_name()
                    .map(|name| name.to_string_lossy().to_lowercase().replace(".exe", ""))
                    .unwrap_or_default();
                let target_exe_name = std::path::Path::new(&actual_relaunch_program)
                    .file_name()
                    .map(|name| name.to_string_lossy().to_lowercase().replace(".exe", ""))
                    .unwrap_or_default();

                if !item_exe_name.is_empty() && !target_exe_name.is_empty() && item_exe_name == target_exe_name {
                    log::debug!(
                        "[Taskbar] 通过可执行文件名匹配移除中心项: exe={}, display_name={}",
                        item_exe_name,
                        temporal_item.display_name
                    );
                    return false;
                }
            }

            // 5. 使用显示名称匹配（对所有应用都适用，包括 UWP）
            let item_display_name_lower = temporal_item.display_name.to_lowercase();
            let target_display_name_lower = display_name.to_lowercase();
            if item_display_name_lower == target_display_name_lower ||
               item_display_name_lower.contains(&target_display_name_lower) ||
               target_display_name_lower.contains(&item_display_name_lower) {
                log::debug!(
                    "[Taskbar] 通过显示名称匹配移除中心项: display_name={}, relaunch_program={}",
                    temporal_item.display_name,
                    temporal_item.relaunch_program
                );
                return false;
            }
        }
        true
    });

    if items.center.len() < original_count {
        log::debug!(
            "[Taskbar] 从center区域移除了重复或被固定的项: original_id={:?}, relaunch={}",
            original_id,
            actual_relaunch_program
        );
    } else {
        log::debug!(
            "[Taskbar] 没有从center区域移除任何项: original_id={:?}, relaunch={}",
            original_id,
            actual_relaunch_program
        );
    }

    // 注意：不在这里调用 sanitize()，以避免破坏插入的位置顺序

    // 🔧 关键修复：设置跳过文件同步标志，避免文件监听器触发 on_stored_changed
    crate::widgets::taskbar::taskbar_items_impl::set_skip_file_sync(500);

    // 更新 FULL_STATE
    new_state.taskbar_items = items.clone();
    log::info!(
        "[任务栏变化] left={}, center={}, right={}",
        items.left.len(),
        items.center.len(),
        items.right.len()
    );

    // 打点：记录固定操作后的任务栏状态
    report_taskbar_state("Pin", &items);

    FULL_STATE.store(Arc::new(new_state.clone()));
    FULL_STATE
        .load()
        .write_taskbar_items(&new_state.taskbar_items)?;

    // 🔧 关键修复：在 TASKBAR_STATE 中更新，保留窗口信息
    // 正确做法：
    // 1. 从 center 找到被固定的临时项，获取其窗口信息
    // 2. 将窗口信息添加到新固定项中
    // 3. 从 center 移除临时项
    {
        let mut state = trace_lock!(TASKBAR_STATE);
        let relaunch_lower = actual_relaunch_program.to_lowercase();
        // 从 center 中找到被固定的临时项，获取其窗口信息
        // 注意：UWP 应用的 relaunch_program 是 explorer.exe，不能用于匹配，必须使用 ID 或 UMID
        let is_uwp = resolved_umid.is_some();
        let mut windows_to_transfer: Vec<TaskbarAppGroupItem> = vec![];
        for item in state.items.center.iter().chain(state.items.right.iter()) {
            // 🔧 Bug2修复：同时搜索 right 区域
            if let TaskbarItem::Temporal(data) = item {
                // 🔧 Bug1修复：检查候选临时项本身是否是 UWP/PropertyStore 应用
                // 因为 Qoder、豆包等 PropertyStore 应用的 relaunch_program 也是 explorer.exe
                let item_is_uwp = data.umid.is_some()
                    || data
                        .relaunch_args
                        .as_ref()
                        .map_or(false, |args| match args {
                            libs_core::state::RelaunchArguments::String(s) => {
                                s.to_lowercase().contains("shell:appsfolder")
                            }
                            libs_core::state::RelaunchArguments::Array(arr) => arr
                                .iter()
                                .any(|s| s.to_lowercase().contains("shell:appsfolder")),
                        });

                let matched = if is_uwp {
                    // UWP 应用：只使用 ID 或 UMID 匹配，不使用 relaunch_program
                    original_id.as_ref().map_or(false, |oid| data.id == *oid)
                        || resolved_umid.as_ref().map_or(false, |u| {
                            data.umid
                                .as_ref()
                                .map_or(false, |du| du.to_lowercase() == u.to_lowercase())
                        })
                } else {
                    // 普通应用：优先 ID/UMID，relaunch_program 只匹配非 UWP 临时项
                    original_id.as_ref().map_or(false, |oid| data.id == *oid) ||
                    resolved_umid.as_ref().map_or(false, |u|
                        data.umid.as_ref().map_or(false, |du| du.to_lowercase() == u.to_lowercase())
                    ) ||
                    // 🔧 关键：排除 UWP 应用（Qoder等PropertyStore应用也使用 explorer.exe 作为 relaunch_program）
                    (!item_is_uwp && data.relaunch_program.to_lowercase() == relaunch_lower)
                };

                if matched {
                    log::info!(
                        "[固定诊断] 在 center 中找到匹配项: id={}, windows={}",
                        data.id,
                        data.windows.len()
                    );
                    if !data.windows.is_empty() {
                        windows_to_transfer = data.windows.clone();
                        log::info!("[固定诊断] 将转移 {} 个窗口", windows_to_transfer.len());
                    }
                    break;
                }
            }
        }

        // 🔧 关键修复：保留现有固定项的窗口信息，只更新新固定项
        // 原因：items 来自 FULL_STATE，不包含窗口信息，直接覆盖会丢失其他固定项的窗口信息
        // 1. 先保存现有固定项的窗口信息
        let mut existing_windows: std::collections::HashMap<String, Vec<TaskbarAppGroupItem>> =
            std::collections::HashMap::new();
        for item in &state.items.left {
            if let TaskbarItem::Pinned(data) = item {
                if !data.windows.is_empty() {
                    existing_windows.insert(data.id.clone(), data.windows.clone());
                }
            }
        }
        // 2. 更新 left 区域
        log::info!("[固定诊断] items.left 数量: {}", items.left.len());
        state.items.left = items.left.clone();
        // 3. 恢复所有固定项的窗口信息
        for item in &mut state.items.left {
            if let TaskbarItem::Pinned(data) = item {
                // 优先恢复保存的窗口信息
                if let Some(windows) = existing_windows.remove(&data.id) {
                    data.windows = windows;
                }
            }
        }
        // 4. 为新固定项添加从 center 转移的窗口信息
        if !windows_to_transfer.is_empty() {
            for item in &mut state.items.left {
                if let TaskbarItem::Pinned(data) = item {
                    let matched = if is_uwp {
                        original_id.as_ref().map_or(false, |oid| data.id == *oid)
                            || resolved_umid.as_ref().map_or(false, |u| {
                                data.umid
                                    .as_ref()
                                    .map_or(false, |du| du.to_lowercase() == u.to_lowercase())
                            })
                    } else {
                        original_id.as_ref().map_or(false, |oid| data.id == *oid)
                            || data.relaunch_program.to_lowercase() == relaunch_lower
                            || resolved_umid.as_ref().map_or(false, |u| {
                                data.umid
                                    .as_ref()
                                    .map_or(false, |du| du.to_lowercase() == u.to_lowercase())
                            })
                    };
                    if matched {
                        log::info!(
                            "[固定诊断] 为固定项添加窗口: id={}, windows={}",
                            data.id,
                            windows_to_transfer.len()
                        );
                        data.windows = windows_to_transfer.clone();
                        break;
                    }
                }
            }
        }

        // 从 center 中移除被固定的项
        // 注意：UWP 应用不能使用 relaunch_program 匹配，否则会误删其他应用
        state.items.center.retain(|item| {
            if let TaskbarItem::Temporal(data) = item {
                // 通过 ID 匹配
                if let Some(ref oid) = original_id {
                    if data.id == *oid {
                        log::info!("[固定诊断] 从 center 移除(ID匹配): {}", data.id);
                        return false;
                    }
                }
                // 通过 UMID 匹配
                if let (Some(u1), Some(u2)) = (&data.umid, &resolved_umid) {
                    if u1.to_lowercase() == u2.to_lowercase() {
                        log::info!("[固定诊断] 从 center 移除(UMID匹配): {}", data.id);
                        return false;
                    }
                }
                // 通过 relaunch_program 匹配（仅限非 UWP 应用）
                // 🔧 关键修复：需要同时检查临时项是否是 UWP 应用
                // 因为很多 UWP 应用的 relaunch_program 也是 explorer.exe
                if !is_uwp {
                    // 检查临时项本身是否是 UWP 应用
                    let item_is_uwp = data.umid.is_some()
                        || data
                            .relaunch_args
                            .as_ref()
                            .map_or(false, |args| match args {
                                libs_core::state::RelaunchArguments::String(s) => {
                                    s.to_lowercase().contains("shell:appsfolder")
                                }
                                libs_core::state::RelaunchArguments::Array(arr) => arr
                                    .iter()
                                    .any(|s| s.to_lowercase().contains("shell:appsfolder")),
                            });

                    // 只有当临时项不是 UWP 应用时，才进行 path 匹配
                    if !item_is_uwp && data.relaunch_program.to_lowercase() == relaunch_lower {
                        log::info!("[固定诊断] 从 center 移除(path匹配): {}", data.id);
                        return false;
                    }
                }
            }
            true
        });

        // 🔧 Bug3修复：同样清理 right 区域中被固定的项（避免临时项在固定后残留）
        items.right.retain(|item| {
            if let TaskbarItem::Temporal(temporal_item) = item {
                if let Some(original_id_value) = &original_id {
                    if temporal_item.id == *original_id_value {
                        log::debug!(
                            "[Taskbar] 通过 ID 从 right 移除被固定项: id={}",
                            temporal_item.id
                        );
                        return false;
                    }
                }
                if let (Some(u1), Some(u2)) = (&temporal_item.umid, &resolved_umid) {
                    if u1.to_lowercase() == u2.to_lowercase() {
                        return false;
                    }
                }
            }
            true
        });

        state.emit_to_webview()?;
    }

    Ok(())
}

#[tauri::command(async)]
pub fn taskbar_unpin_item(
    umid: Option<String>,
    relaunch_program: String,
    original_id: Option<String>,
    target_index: Option<usize>, // 新增参数：指定临时项在中间区域的目标位置
) -> Result<()> {
    // 保存umid的副本，用于后续比较
    let umid_copy = umid.clone();

    // 仅通过yml文件管理任务栏取消固定，不操作Windows原生任务栏
    log::debug!(
        "[Taskbar] 开始取消固定操作 - original_id: {:?}, relaunch_program: {:?}, umid: {:?}",
        original_id,
        relaunch_program,
        umid_copy
    );

    // 更新 FULL_STATE 中的 taskbar_items 字段
    // 1. 获取当前状态的 Arc 引用
    let current_state = FULL_STATE.load();
    // 2. 克隆 FullState 的值（不是 Arc）
    let mut new_state = (**current_state).clone();
    // 3. 修改 taskbar_items - 将固定项转为临时项

    // 查找要取消固定的项目 - 使用更精确的匹配
    if let Some(index) = new_state.taskbar_items.left.iter().position(|item| {
        if let TaskbarItem::Pinned(data) = item {
            // 优先使用ID匹配
            if let Some(ref original_id_value) = original_id {
                data.id == *original_id_value
            }
            // 其次使用relaunch_program和UMID的组合匹配
            else if data.relaunch_program.to_lowercase() == relaunch_program.to_lowercase() {
                // 如果有UMID，必须匹配UMID
                if let (Some(data_umid), Some(copy_umid)) = (&data.umid, &umid_copy) {
                    data_umid == copy_umid
                }
                // 如果没有UMID，则只匹配relaunch_program
                else if data.umid.is_none() && umid_copy.is_none() {
                    true
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        }
    }) {
        // 从 FULL_STATE 的 taskbar_items 中移除固定项（仅用于文件持久化，不含窗口信息）
        new_state.taskbar_items.left.remove(index);
        log::debug!("[Taskbar] 从左侧区域移除固定项: 索引 {}", index);

        // 🔧 修复：FULL_STATE 不包含窗口信息，不在这里判断是否有窗口
        // 窗口判断由 TASKBAR_STATE 的处理逻辑来完成
        // 这里只负责文件操作（移除固定项、保存文件）
    } else {
        log::debug!("[Taskbar] 未找到匹配的固定项");
    }

    // 清理状态
    new_state.taskbar_items.sanitize();

    // 🔧 关键修复：设置跳过文件同步标志，避免文件监听器触发 on_stored_changed
    crate::widgets::taskbar::taskbar_items_impl::set_skip_file_sync(500);

    // 打点：记录取消固定操作后的任务栏状态
    report_taskbar_state("Unpin", &new_state.taskbar_items);

    // 先保存到文件以保整一性
    current_state.write_taskbar_items(&new_state.taskbar_items)?;

    // 再更新 FULL_STATE
    FULL_STATE.store(Arc::new(new_state.clone()));

    // 🔧 关键修复：在 TASKBAR_STATE 中更新，保留窗口信息
    // 原因：new_state.taskbar_items 来自 FULL_STATE，不包含窗口信息
    // 正确做法：从 TASKBAR_STATE.left 中移除固定项，转为临时项并保留窗口信息

    {
        let mut state = trace_lock!(TASKBAR_STATE);

        // 在 TASKBAR_STATE.left 中查找要取消固定的项
        let relaunch_lower = relaunch_program.to_lowercase();
        if let Some(index) = state.items.left.iter().position(|item| {
            if let TaskbarItem::Pinned(data) = item {
                // 优先使用 ID 匹配
                if let Some(ref oid) = original_id {
                    if data.id == *oid {
                        return true;
                    }
                }
                // 其次使用 relaunch_program 和 UMID 匹配
                if data.relaunch_program.to_lowercase() == relaunch_lower {
                    if let (Some(u1), Some(u2)) = (&data.umid, &umid_copy) {
                        return u1 == u2;
                    }
                    return data.umid.is_none() && umid_copy.is_none();
                }
            }
            false
        }) {
            // 移除固定项（包含窗口信息）
            let pinned_item = state.items.left.remove(index);
            // 转换为临时项，保留窗口信息
            if let TaskbarItem::Pinned(mut data) = pinned_item {
                // 使用 original_id 作为临时项的 ID
                if let Some(ref oid) = original_id {
                    data.id = oid.clone();
                }
                data.set_pin_disabled(false);
                // 根据是否有窗口来决定处理方式
                // - 有窗口：保留窗口信息，添加到 center
                // - 没有窗口：不添加到 center（直接消失）
                if data.windows.is_empty() {
                    // 没有窗口，不添加到 center
                    log::debug!(
                        "[Taskbar] 取消固定的应用没有窗口，不显示: {}",
                        data.display_name
                    );
                } else {
                    // 有窗口的应用：正常添加到 center
                    let temporal_item = TaskbarItem::Temporal(data);

                    if let Some(idx) = target_index {
                        let insert_pos = std::cmp::min(idx, state.items.center.len());
                        state.items.center.insert(insert_pos, temporal_item);
                    } else {
                        state.items.center.push(temporal_item);
                    }
                }
            }
        }

        state.emit_to_webview()?;
    }

    Ok(())
}

#[tauri::command(async)]
pub fn taskbar_get_webview_hwnd(webview: tauri::WebviewWindow<tauri::Wry>) -> Result<isize> {
    Ok(webview.hwnd()?.0 as isize)
}

#[tauri::command(async)]
pub fn taskbar_save_window_coordinates(content: String, filename: String) -> Result<()> {
    use serde_json::{json, Value};
    use std::fs::{create_dir_all, read_to_string, write};

    let log_dir = std::path::PathBuf::from(r"C:\ProgramData\Comms\MagicAnimation");

    // 创建目录（如果不存在）
    if !log_dir.exists() {
        create_dir_all(&log_dir).map_err(|e| format!("创建日志目录失败: {}", e))?;
        log::info!("创建坐标目录: {}", log_dir.display());
    }

    let file_path = log_dir.join(&filename);

    // 解析新数据
    let new_data: Value =
        serde_json::from_str(&content).map_err(|e| format!("解析JSON失败: {}", e))?;

    // 获取新数据中的坐标数
    let new_coords_count = new_data["iconCount"].as_i64().unwrap_or(0);

    // 关键逻辑：当前端发来的坐标数为0时，清空文件
    // 否则执行智能合并：按 hwnd 替换而不是按显示器合并
    if new_coords_count == 0 {
        // 直接写入空坐标数据
        write(&file_path, &content).map_err(|e| format!("写入文件失败: {}", e))?;
        log::info!("清空坐标文件: {}", file_path.display());
    } else {
        // 有坐标数据时，执行合并：按 hwnd 替换
        let mut merged_data: Value = if file_path.exists() {
            match read_to_string(&file_path) {
                Ok(existing_content) => {
                    serde_json::from_str(&existing_content).unwrap_or_else(|_| {
                        json!({
                            "timestamp": "",
                            "iconCount": 0,
                            "coordinates": []
                        })
                    })
                }
                Err(_) => json!({
                    "timestamp": "",
                    "iconCount": 0,
                    "coordinates": []
                }),
            }
        } else {
            json!({
                "timestamp": "",
                "iconCount": 0,
                "coordinates": []
            })
        };

        // 执行合并逻辑：按 hwnd + monitorName 更新（而不是替换整个hwnd）
        if let (Some(existing_coords), Some(new_coords)) = (
            merged_data["coordinates"].as_array_mut(),
            new_data["coordinates"].as_array(),
        ) {
            for new_coord in new_coords {
                if let (Some(new_hwnd), Some(new_monitors)) =
                    (new_coord["hwnd"].as_i64(), new_coord["monitors"].as_array())
                {
                    // 找到相同 hwnd 的项目
                    if let Some(existing_coord) = existing_coords
                        .iter_mut()
                        .find(|c| c["hwnd"].as_i64() == Some(new_hwnd))
                    {
                        // 更新该 hwnd 的 monitors 数组
                        // 关键：只更新对应 monitorName 的数据，保留其他显示器的数据
                        if let Some(existing_monitors) = existing_coord["monitors"].as_array_mut() {
                            for new_monitor in new_monitors {
                                if let Some(new_monitor_name) = new_monitor["monitorName"].as_str()
                                {
                                    // 查找是否已有该显示器的数据
                                    if let Some(existing_monitor) =
                                        existing_monitors.iter_mut().find(|m| {
                                            m["monitorName"].as_str() == Some(new_monitor_name)
                                        })
                                    {
                                        // 更新该显示器的坐标
                                        *existing_monitor = new_monitor.clone();
                                    } else {
                                        // 新增该显示器的坐标
                                        existing_monitors.push(new_monitor.clone());
                                    }
                                }
                            }
                        }
                        // 更新title
                        if let Some(new_title) = new_coord["title"].as_str() {
                            existing_coord["title"] = json!(new_title);
                        }
                    } else {
                        // hwnd不存在，直接添加
                        existing_coords.push(new_coord.clone());
                    }
                }
            }

            // 关键：丹清理已关闭window的坐标
            // 因为前端已经告诉我们的activeHwnds，我们要删除不在这个列表中的hwnd
            if let Some(active_hwnds_array) = new_data.get("activeHwnds").and_then(|v| v.as_array())
            {
                let active_hwnds: Vec<i64> = active_hwnds_array
                    .iter()
                    .filter_map(|v| v.as_i64())
                    .collect();

                // 删除不在active_hwnds中的hwnd数据
                // 但保留回收站窗口的记录（因为回收站不是普通应用，不在activeHwnds中）
                existing_coords.retain(|c| {
                    if let Some(hwnd) = c["hwnd"].as_i64() {
                        // 检查是否为回收站窗口
                        let is_recycle_bin = if let Some(title) = c["title"].as_str() {
                            title == "回收站"
                                || title.starts_with("回收站")
                                || title == "Recycle Bin"
                        } else {
                            false
                        };
                        // 回收站窗口始终保留，其他窗口只在active_hwnds中才保留
                        is_recycle_bin || active_hwnds.contains(&hwnd)
                    } else {
                        false
                    }
                });
            }
        }

        // 特殊处理：将回收站窗口的坐标替换为回收站图标位置
        // 前端已通过 system_get_recycle_bin_hwnd 获取了实际窗口句柄并传递过来
        // 现在只需要根据 hwnd 找到对应的记录，并替换其坐标为图标位置
        if let Some(recycle_bin_icon) = new_data.get("recycleBinIcon") {
            if let (
                Some(monitor_name),
                Some(icon_x),
                Some(icon_y),
                Some(icon_width),
                Some(icon_x_rel),
                Some(icon_y_rel),
            ) = (
                recycle_bin_icon["monitorName"].as_str(),
                recycle_bin_icon["x"].as_i64(),
                recycle_bin_icon["y"].as_i64(),
                recycle_bin_icon["width"].as_i64(),
                recycle_bin_icon["x-relative"].as_f64(),
                recycle_bin_icon["y-relative"].as_f64(),
            ) {
                // 获取真实的回收站窗口句柄
                let recycle_bin_hwnd = crate::windows_api::WindowsApi::get_recycle_bin_hwnd()
                    .map(|h| h.0 as i64)
                    .unwrap_or(-1);

                if let Some(coords) = merged_data["coordinates"].as_array_mut() {
                    // 查找是否已存在回收站记录（通过标题）
                    let mut found = false;
                    for coord in coords.iter_mut() {
                        let is_recycle_bin = if let Some(title) = coord["title"].as_str() {
                            title == "回收站"
                                || title.starts_with("回收站")
                                || title == "Recycle Bin"
                        } else {
                            false
                        };

                        if is_recycle_bin {
                            // 只有当前端传递的是 -1 时，才用后端的结果修正
                            if coord["hwnd"].as_i64() == Some(-1) && recycle_bin_hwnd != -1 {
                                coord["hwnd"] = json!(recycle_bin_hwnd);
                            }
                            // 否则信任前端传递的值

                            // 更新坐标
                            if let Some(monitors) = coord["monitors"].as_array_mut() {
                                let mut monitor_found = false;
                                for monitor in monitors.iter_mut() {
                                    if monitor["monitorName"].as_str() == Some(monitor_name) {
                                        monitor["x"] = json!(icon_x);
                                        monitor["y"] = json!(icon_y);
                                        monitor["width"] = json!(icon_width);
                                        monitor["x-relative"] = json!(icon_x_rel);
                                        monitor["y-relative"] = json!(icon_y_rel);
                                        monitor_found = true;
                                        break;
                                    }
                                }
                                if !monitor_found {
                                    monitors.push(json!({
                                        "monitorName": monitor_name,
                                        "x": icon_x,
                                        "y": icon_y,
                                        "width": icon_width,
                                        "x-relative": icon_x_rel,
                                        "y-relative": icon_y_rel
                                    }));
                                }
                            }
                            found = true;
                            break;
                        }
                    }

                    // 如果没有找到现有记录，添加新记录
                    if !found && recycle_bin_hwnd != -1 {
                        coords.push(json!({
                            "hwnd": recycle_bin_hwnd,
                            "title": "回收站",
                            "monitors": [{
                                "monitorName": monitor_name,
                                "x": icon_x,
                                "y": icon_y,
                                "width": icon_width,
                                "x-relative": icon_x_rel,
                                "y-relative": icon_y_rel
                            }]
                        }));
                    }
                }
            }
        }

        // 更新时间戳和iconCount
        if let Some(new_timestamp) = new_data["timestamp"].as_str() {
            merged_data["timestamp"] = json!(new_timestamp);
        }
        if let Some(coords) = merged_data["coordinates"].as_array() {
            merged_data["iconCount"] = json!(coords.len());
        }

        // 写入文件
        let merged_content = serde_json::to_string_pretty(&merged_data)
            .map_err(|e| format!("序列化JSON失败: {}", e))?;
        write(&file_path, merged_content).map_err(|e| format!("写入文件失败: {}", e))?;
    }

    Ok(())
}

#[tauri::command(async)]
pub fn taskbar_bring_to_front(webview: tauri::WebviewWindow<tauri::Wry>) -> Result<()> {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            SetWindowPos, HWND_TOPMOST, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        },
    };

    let hwnd = HWND(webview.hwnd()?.0);

    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        );
    }

    Ok(())
}

/// 更新 亚克力玻璃窗口尺寸
#[tauri::command(async)]
pub fn taskbar_update_window_size(
    webview: tauri::WebviewWindow<tauri::Wry>,
    width: i32,
    container_left: i32,
    container_top: i32,
    container_height: i32,
) -> Result<()> {
    use crate::app::APP_MANAGER;

    let hwnd = windows::Win32::Foundation::HWND(webview.hwnd()?.0 as _);
    let monitor = crate::windows_api::WindowsApi::monitor_from_window(hwnd);
    let monitor_dpi = crate::windows_api::WindowsApi::get_monitor_scale_factor(monitor)?;

    // 玻璃效果是 taskbar WebView 的 child window，resize 时始终锚定在父窗口 (0, 0)。
    // 因此 child window 应保持父窗口大小，真正随容器变化的是 blur region。
    let current_rect = crate::windows_api::WindowsApi::get_outer_window_rect(hwnd)?;
    let window_width = current_rect.right - current_rect.left;
    let window_height = current_rect.bottom - current_rect.top;

    // 先更新 APP_MANAGER 中保存的 webview_rect
    let taskbars = {
        let manager = crate::trace_read!(APP_MANAGER);
        manager
            .instances
            .iter()
            .map(|instance| instance.taskbar.clone())
            .collect::<Vec<_>>()
    };

    for taskbar_arc in taskbars {
        let Some(mut taskbar) = taskbar_arc.try_lock_for(std::time::Duration::from_millis(50))
        else {
            log::warn!("[TaskbarGlass] skip update blur: taskbar lock busy");
            continue;
        };
        if let Some(tb) = taskbar.as_mut() {
            if tb.window.label() == webview.label() {
                // 同步更新玻璃效果的模糊区域
                // 直接使用前端传来的容器位置和尺寸（物理像素），
                // 避免后端按假设二次计算导致错位
                let blur_x = container_left as f32;
                let mut blur_y = container_top as f32;
                let blur_w = width as f32;
                // 直接使用前端传来的容器高度（zoom 缩放后的实际视觉高度）
                let blur_h = container_height as f32;
                let win_w = window_width as f32;
                let win_h = window_height as f32;

                // 前端未就绪时值异常，可能为 0 或负
                if blur_w > 0.0 && blur_h > 0.0 {
                    let state = FULL_STATE.load();
                    if state.settings.taskbar.position == TaskbarSide::Bottom {
                        let bottom_padding =
                            (Taskbar::CONTAINER_BOTTOM_MARGIN_CSS as f64 * monitor_dpi) as f32;
                        let expected_blur_y = (win_h - blur_h - bottom_padding).max(0.0);
                        if (blur_y - expected_blur_y).abs() > 4.0 {
                            log::warn!(
                                    "[TaskbarGlass] correcting unstable bottom blur y: frontend_y={}, expected_y={}, h={}, win_h={}, dpi={}",
                                    blur_y,
                                    expected_blur_y,
                                    blur_h,
                                    win_h,
                                    monitor_dpi
                                );
                        }
                        blur_y = expected_blur_y;
                    }
                    if let Some(glass) = &tb.glass_effect {
                        glass.resize(window_width, window_height);
                        glass.update_blur_region(blur_x, blur_y, blur_w, blur_h, win_w, win_h);
                    }
                }

                break;
            }
        }
    }

    Ok(())
}

/// 隐藏亚克力玻璃效果子窗口（容器隐藏动画前调用）
#[tauri::command(async)]
pub fn taskbar_hide_glass_effect(webview: tauri::WebviewWindow<tauri::Wry>) -> Result<()> {
    use crate::app::APP_MANAGER;

    let taskbars = {
        let manager = crate::trace_read!(APP_MANAGER);
        manager
            .instances
            .iter()
            .map(|instance| instance.taskbar.clone())
            .collect::<Vec<_>>()
    };

    for taskbar_arc in taskbars {
        let Some(taskbar) = taskbar_arc.try_lock_for(std::time::Duration::from_millis(50)) else {
            log::warn!("[TaskbarGlass] skip hide: taskbar lock busy");
            continue;
        };
        if let Some(tb) = taskbar.as_ref() {
            if tb.window.label() == webview.label() {
                if let Some(glass) = &tb.glass_effect {
                    glass.hide();
                }
                break;
            }
        }
    }
    Ok(())
}

/// 显示亚克力玻璃效果子窗口（容器显示后调用）
#[tauri::command(async)]
pub fn taskbar_show_glass_effect(webview: tauri::WebviewWindow<tauri::Wry>) -> Result<()> {
    use crate::app::APP_MANAGER;

    let taskbars = {
        let manager = crate::trace_read!(APP_MANAGER);
        manager
            .instances
            .iter()
            .map(|instance| instance.taskbar.clone())
            .collect::<Vec<_>>()
    };

    for taskbar_arc in taskbars {
        let Some(taskbar) = taskbar_arc.try_lock_for(std::time::Duration::from_millis(50)) else {
            log::warn!("[TaskbarGlass] skip show: taskbar lock busy");
            continue;
        };
        if let Some(tb) = taskbar.as_ref() {
            if tb.window.label() == webview.label() {
                if let Some(glass) = &tb.glass_effect {
                    glass.show();
                }
                break;
            }
        }
    }
    Ok(())
}
