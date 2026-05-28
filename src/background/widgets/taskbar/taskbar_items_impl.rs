use libs_core::{
    handlers::FuncEvent,
    state::{
        IconBackplateStyle, PinnedTaskbarItemData, RelaunchArguments, TaskbarAppGroupItem,
        TaskbarItem, TaskbarItemSubtype, TaskbarItems, TaskbarPinnedItemsVisibility,
        TaskbarTemporalItemsVisibility,
    },
    system_state::{MonitorId, StartMenuItem},
};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    sync::{Arc, LazyLock},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tauri::Emitter;

/// 全局标志：标记是否跳过文件同步
/// 当用户进行固定/取消固定操作时，会设置这个标志为 true
/// 文件监听器检测到变化时，如果标志为 true 则跳过同步
pub static SKIP_FILE_SYNC: AtomicBool = AtomicBool::new(false);
/// 跳过同步的过期时间
pub static SKIP_FILE_SYNC_UNTIL: LazyLock<Mutex<Instant>> =
    LazyLock::new(|| Mutex::new(Instant::now()));

/// 设置跳过文件同步标志，并设置过期时间
pub fn set_skip_file_sync(duration_ms: u64) {
    SKIP_FILE_SYNC.store(true, Ordering::SeqCst);
    *SKIP_FILE_SYNC_UNTIL.lock() = Instant::now() + std::time::Duration::from_millis(duration_ms);
}

use crate::{
    app::get_app_handle,
    error::{Result, ResultLogExt},
    modules::{
        apps::application::{UserAppsEvent, UserAppsManager, USER_APPS_MANAGER},
        start::application::{StartMenuMatchKind, START_MENU_MANAGER},
    },
    state::application::FULL_STATE,
    trace_lock,
    utils::{
        constants::VAR_COMMON,
        icon_extractor::{
            extract_and_save_icon_from_file, extract_and_save_icon_from_window,
            extract_and_save_icon_umid,
        },
    },
    windows_api::{types::AppUserModelId, window::Window, MonitorEnumerator},
};

pub static TASKBAR_STATE: LazyLock<Arc<Mutex<TaskbarState>>> =
    LazyLock::new(|| Arc::new(Mutex::new(TaskbarState::new())));

static TASKBAR_LIVENESS_RECONCILE_RUNNING: AtomicBool = AtomicBool::new(false);

fn schedule_tracked_window_liveness_reconcile(reason: &'static str) {
    if TASKBAR_LIVENESS_RECONCILE_RUNNING.swap(true, Ordering::SeqCst) {
        log::debug!(
            "[TaskbarItems][Liveness] reconcile already scheduled, skip duplicate reason={}",
            reason
        );
        return;
    }

    crate::get_tokio_handle().spawn(async move {
        for delay_ms in [300_u64, 700, 1000] {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            let _guard = trace_lock!(TASKBAR_STATE);
        }
        TASKBAR_LIVENESS_RECONCILE_RUNNING.store(false, Ordering::SeqCst);
    });
}

/// 从 TASKBAR_STATE 获取所有窗口的列表
///
/// 辅助函数：获取任务栏项的显示名称（用于打点）
fn get_item_display_name(item: &TaskbarItem) -> String {
    match item {
        TaskbarItem::StartMenu { .. } => "StartMenu".to_string(),
        TaskbarItem::Pinned(data) => data.display_name.clone(),
        TaskbarItem::Temporal(data) => data.display_name.clone(),
        TaskbarItem::RecycleBin { .. } => "RecycleBin".to_string(),
        TaskbarItem::Separator { .. } => "Separator".to_string(),
        TaskbarItem::SystemTray { .. } => "SystemTray".to_string(),
    }
}

/// 辅助函数：上报任务栏状态（打点）
pub fn report_taskbar_state(action: &str, items: &TaskbarItems) {
    let left_names: Vec<String> = items.left.iter().map(get_item_display_name).collect();
    let center_names: Vec<String> = items.center.iter().map(get_item_display_name).collect();
    let right_names: Vec<String> = items.right.iter().map(get_item_display_name).collect();

    let content = format!(
        r#"{{"Action":"{}","Left":{{"counts":{},"process":"{}"}},"Center":{{"counts":{},"process":"{}"}},"Right":{{"counts":{},"process":"{}"}}}}"#,
        action,
        items.left.len(),
        left_names.join(","),
        items.center.len(),
        center_names.join(","),
        items.right.len(),
        right_names.join(",")
    );

    // 调用 exposed.rs 中的 Tauri Command
    let _ = crate::exposed::report_taskbar_state(content);
}

/// 这是一个统一的辅助函数，用于避免在多处重复相同的窗口列表获取逻辑。
/// 返回所有 taskbar items 中的窗口（包括 Pinned 和 Temporal 类型）。
///
/// # 性能
/// - 只读取内存，无系统调用
/// - 自动释放锁，不会长时间持有
/// - 窗口数量通常为 5-20 个（vs 全量枚举的 50-200 个）
pub fn get_taskbar_windows() -> Vec<Window> {
    let guard = trace_lock!(TASKBAR_STATE);
    let mut windows = Vec::new();

    for item in guard
        .items
        .center
        .iter()
        .chain(guard.items.left.iter())
        .chain(guard.items.right.iter())
    {
        match item {
            TaskbarItem::Pinned(data) | TaskbarItem::Temporal(data) => {
                for win in &data.windows {
                    windows.push(Window::from(win.handle));
                }
            }
            _ => {}
        }
    }

    windows
}

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub handle: isize,
    pub title: String,
    pub path: PathBuf,
    pub umid: Option<String>,
    pub display_name: String,
    pub relaunch_program: String,
    pub relaunch_args: Option<RelaunchArguments>,
    pub icon_png_base64: Option<String>,
    pub is_approximately_square: Option<bool>,
    pub is_from_local: Option<bool>,
    pub is_iconic: bool,
    pub is_zoomed: bool,
    pub is_real_file_explorer: bool,
    /// 窗口是否禁止固定到任务栏固定区。
    /// 用于对齐 Windows 默认任务栏行为：WPS 子应用 / 文档窗口不允许固定，
    /// 仅 WPS 主程序（主应用身份）可固定。
    pub pin_disabled: bool,
}

/// WPS 主程序标题判据：display_name/title 必须同时满足：
/// 1) 包含 "WPS Office" 子串；
/// 2) 不包含常见文档扩展名（.pptx/.docx/.xlsx/.ppt/.doc/.xls/.pdf）。
/// 用于在 UMID 相同（如主程序与 PPT 文档窗口共享动态 WpsOffice.<时间戳>）
/// 时通过标题区分主程序与文档窗口。
fn wps_display_name_is_main_program(display_name: &str) -> bool {
    let dn_lower = display_name.to_lowercase();
    if !dn_lower.contains("wps office") {
        return false;
    }
    let doc_markers = [".pptx", ".docx", ".xlsx", ".ppt", ".doc", ".xls", ".pdf"];
    !doc_markers.iter().any(|m| dn_lower.contains(m))
}

/// 判断给定窗口信息是否属于“WPS 主应用身份”。
/// 所有不属于该身份的 WPS 窗口（子应用、文档窗口、WPS图片等）不允许固定，
/// 也不与主程序合并。
/// 判据优先级：
///   A) UMID = Kingsoft.Office.KPrometheus（登录/主应用）→ 主应用
///   B) UMID = Kingsoft.Office.WpsOffice.<纯数字> → 需 display_name 满足主程序标题条件
///   C) UMID = 其他 Kingsoft.Office.* → 子应用
///   D) UMID = None 且 exe 属于主程序启动路径 (wps.exe / ksolaunch.exe) → 需 display_name 满足主程序标题条件
fn is_wps_main_identity(umid: Option<&str>, exe_lower: &str, display_name: &str) -> bool {
    match umid {
        Some(u) => {
            let lower = u.to_lowercase();
            if lower == "kingsoft.office.kprometheus" {
                return true;
            }
            if lower.starts_with("kingsoft.office.wpsoffice.") {
                let suffix = &lower["kingsoft.office.wpsoffice.".len()..];
                let is_dynamic = !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit());
                return is_dynamic && wps_display_name_is_main_program(display_name);
            }
            false
        }
        None => {
            (exe_lower == "wps.exe" || exe_lower == "ksolaunch.exe")
                && wps_display_name_is_main_program(display_name)
        }
    }
}

fn normalized_path_text(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn paths_equivalent(left: &Path, right: &Path) -> bool {
    normalized_path_text(left) == normalized_path_text(right)
}

fn paths_share_app_directory(left: &Path, right: &Path) -> bool {
    let Some(left_parent) = left.parent() else {
        return false;
    };
    let Some(right_parent) = right.parent() else {
        return false;
    };

    if paths_equivalent(left_parent, right_parent) {
        return true;
    }

    let left_parts: Vec<_> = normalized_path_text(left_parent)
        .split('\\')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect();
    let right_parts: Vec<_> = normalized_path_text(right_parent)
        .split('\\')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect();

    let common_parts = left_parts
        .iter()
        .zip(right_parts.iter())
        .take_while(|(left, right)| left == right)
        .count();

    let last_common_part = common_parts
        .checked_sub(1)
        .and_then(|index| left_parts.get(index))
        .map(String::as_str);
    let generic_roots = [
        "c:",
        "users",
        "appdata",
        "local",
        "roaming",
        "programs",
        "program files",
        "program files (x86)",
        "windows",
        "system32",
        "microsoft",
        "start menu",
    ];

    // Require a real app-level shared directory, not just "C:\Program Files"
    // or "AppData\Local\Programs".
    common_parts >= 3 && last_common_part.is_some_and(|part| !generic_roots.contains(&part))
}

fn paths_related(left: &Path, right: &Path) -> bool {
    paths_equivalent(left, right) || paths_share_app_directory(left, right)
}

fn shortcut_path_related_to_window(
    shortcut: &StartMenuItem,
    process_path: &Path,
    relaunch_program: Option<&str>,
) -> bool {
    let Some(target) = shortcut.target.as_deref() else {
        return false;
    };

    if paths_related(target, process_path) {
        return true;
    }

    relaunch_program
        .map(Path::new)
        .is_some_and(|relaunch_path| paths_related(target, relaunch_path))
}

fn score_start_menu_shortcut_candidate(
    shortcut: &StartMenuItem,
    kind: StartMenuMatchKind,
    process_path: &Path,
    relaunch_program: Option<&str>,
) -> Option<i32> {
    let has_path_relation =
        shortcut_path_related_to_window(shortcut, process_path, relaunch_program);

    if kind != StartMenuMatchKind::ExactUmid && !has_path_relation {
        return None;
    }

    let mut score = match kind {
        StartMenuMatchKind::ExactUmid => 100,
        StartMenuMatchKind::TargetSuffix => 50,
        StartMenuMatchKind::FuzzyIdentity => 25,
    };

    if has_path_relation {
        score += 100;
    }

    if let Some(target) = shortcut.target.as_deref() {
        if paths_equivalent(target, process_path) {
            score += 50;
        }
        if let Some(relaunch_program) = relaunch_program {
            if paths_equivalent(target, Path::new(relaunch_program)) {
                score += 50;
            }
        }
    }

    Some(score)
}

fn select_trusted_start_menu_shortcut<'a>(
    candidates: Vec<(&'a StartMenuItem, StartMenuMatchKind)>,
    umid: &str,
    process_path: &Path,
    relaunch_program: Option<&str>,
) -> Option<(&'a StartMenuItem, StartMenuMatchKind)> {
    let mut rejected = 0;
    let best = candidates
        .into_iter()
        .filter_map(|(shortcut, kind)| {
            score_start_menu_shortcut_candidate(shortcut, kind, process_path, relaunch_program)
                .map(|score| (shortcut, kind, score))
                .or_else(|| {
                    rejected += 1;
                    log::warn!(
                        "[extract_window_info] Rejecting weak Start Menu shortcut for UMID {}: kind={:?}, shortcut={:?}, target={:?}, process_path={:?}, relaunch={:?}",
                        umid,
                        kind,
                        shortcut.path,
                        shortcut.target,
                        process_path,
                        relaunch_program
                    );
                    None
                })
        })
        .max_by_key(|(_, _, score)| *score)
        .map(|(shortcut, kind, _)| (shortcut, kind));

    if best.is_none() && rejected > 0 {
        log::warn!(
            "[extract_window_info] No trusted Start Menu shortcut remained for UMID {}, falling back to window/process icon",
            umid
        );
    }

    best
}

fn extract_property_store_fallback_icon(
    handle: isize,
    process_name: &str,
    use_local_icon: bool,
    process_icon_path: &Path,
    relaunch_program: Option<&str>,
) {
    if extract_and_save_icon_from_window(
        handle,
        process_name,
        use_local_icon,
        Some(process_icon_path),
    ) {
        return;
    }

    if let Some(relaunch_program) = relaunch_program {
        let relaunch_path = Path::new(relaunch_program);
        if relaunch_path.exists() {
            extract_and_save_icon_from_file(relaunch_path, use_local_icon);
            return;
        }
    }

    extract_and_save_icon_from_file(process_icon_path, use_local_icon);
}

fn extract_window_info(window: &Window) -> Result<WindowInfo> {
    let handle = window.address();
    let title = window.title();
    let process_name = window.process().program_exe_name().unwrap_or_default();

    // 1. 获取路径 (可能涉及 IPC)
    let mut path = match window.get_frame_creator() {
        Ok(None) => get_process_path_with_fallback(window, "frame without creator")?,
        Ok(Some(creator)) => get_process_path_with_fallback(&creator, "frame creator")?,
        Err(_) => get_process_path_with_fallback(window, "non-frame window")?,
    };

    // 2. 获取 UMID 和显示名称
    let umid_raw = window.app_user_model_id();
    let mut display_name = window.app_display_name().unwrap_or_default();

    // WPS 特殊处理：直接使用窗口标题作为显示名称
    if process_name.to_lowercase().contains("wps") {
        display_name = window.title();
    } else if path.to_string_lossy().to_lowercase().contains("wegame")
        && process_name.to_lowercase() == "browser.exe"
    {
        // WeGame 特殊处理：wegame目录下的browser.exe，直接使用窗口标题作为显示名称
        let title = window.title();
        if !title.is_empty() {
            display_name = title;
        }
    } else if display_name.is_empty() || display_name.to_lowercase() == "unknown" {
        // For War3, prefer window title over process name
        let title = window.title();
        if title.to_lowercase().contains("warcraft") {
            display_name = title;
        } else {
            display_name = get_process_name_with_fallback(window);
        }
    }

    // 判定是否为真实资源管理器
    let is_real_file_explorer = (display_name.to_lowercase().contains("资源管理器")
        || display_name.to_lowercase().contains("explorer"))
        && umid_raw
            .as_ref()
            .map_or(true, |u| u.to_string() == "Microsoft.Windows.Explorer");

    if let Some(stripped) = display_name.strip_suffix(".exe") {
        display_name = stripped.to_string();
    }
    if let Some(stripped) = display_name.strip_suffix(".EXE") {
        display_name = stripped.to_string();
    }

    // 豆包浏览器特殊处理：根据窗口标题判断，如果是浏览器则从 proxy 获取图标
    let mut icon_path = path.clone();
    if let Some(ref umid) = umid_raw {
        let umid_str = umid.to_string();
        if umid_str.contains("Doubao.BrowserApp") || umid_str.contains("Doubao.ChatApp") {
            // 根据窗口标题判断是否是浏览器
            let is_browser =
                title.to_lowercase().contains("浏览器") || title.to_lowercase().contains("browser");
            log::info!(
                "[豆包] umid: {}, title: {}, is_browser: {}",
                umid_str,
                title,
                is_browser
            );
            if is_browser {
                // 浏览器：从 Doubao.exe 同目录向上搜索 Doubao_browser_proxy.exe
                if let Some(parent_dir) = path.parent() {
                    if let Some(doubao_dir) = parent_dir.parent() {
                        // 递归搜索 browser_proxy.exe - 搜索所有子目录
                        fn find_browser_proxy(dir: &std::path::Path) -> Option<std::path::PathBuf> {
                            if let Ok(entries) = std::fs::read_dir(dir) {
                                for entry in entries.flatten() {
                                    let path = entry.path();
                                    if path.is_dir() {
                                        // 递归搜索子目录
                                        if let Some(found) = find_browser_proxy(&path) {
                                            return Some(found);
                                        }
                                    } else if let Some(name) = path.file_name() {
                                        if name
                                            .to_string_lossy()
                                            .to_lowercase()
                                            .contains("browser_proxy")
                                        {
                                            return Some(path);
                                        }
                                    }
                                }
                            }
                            None
                        }
                        if let Some(proxy_path) = find_browser_proxy(doubao_dir) {
                            log::info!("[豆包] 找到 proxy: {:?}", proxy_path);
                            icon_path = proxy_path;
                        } else {
                            log::info!("[豆包] 未找到 proxy");
                        }
                    }
                }
            }
        }
    }

    // 3. 获取当前背板模式设置
    let settings = FULL_STATE.load();
    let use_local_icon =
        settings.settings().taskbar.icon_backplate_style == IconBackplateStyle::Transparent;

    // 4. 计算 Relaunch 信息
    let (relaunch_program, relaunch_args) = if let Some(umid) = &umid_raw {
        match umid {
            AppUserModelId::Appx(umid_str) => {
                // 豆包浏览器特殊处理：使用 Doubao_browser_proxy.exe 获取图标
                if umid_str.contains("Doubao.BrowserApp") && icon_path.exists() {
                    extract_and_save_icon_from_file(&icon_path, use_local_icon);
                } else {
                    extract_and_save_icon_umid(
                        &AppUserModelId::Appx(umid_str.clone()),
                        use_local_icon,
                    );
                }
                (
                    VAR_COMMON
                        .system_dir()
                        .join("explorer.exe")
                        .to_string_lossy()
                        .to_string(),
                    Some(RelaunchArguments::String(format!(
                        "shell:AppsFolder\\{umid_str}"
                    ))),
                )
            }
            AppUserModelId::PropertyStore(umid_str) => {
                let relaunch_command = window.relaunch_command();
                let relaunch_display_name = window.relaunch_display_name();
                let relaunch_parts = relaunch_command
                    .as_ref()
                    .map(|command| get_parts_of_inline_command(command));
                let relaunch_program_for_match =
                    relaunch_parts.as_ref().map(|(program, _)| program.as_str());
                let start_menu_manager = START_MENU_MANAGER.load();
                let shortcut = select_trusted_start_menu_shortcut(
                    start_menu_manager.get_by_file_umid_candidates(umid_str),
                    umid_str,
                    &icon_path,
                    relaunch_program_for_match,
                );
                if let Some((shortcut, match_kind)) = &shortcut {
                    log::info!(
                        "[extract_window_info] Trusted Start Menu shortcut for UMID {}: kind={:?}, shortcut={:?}, target={:?}, process_path={:?}, relaunch={:?}",
                        umid_str,
                        match_kind,
                        shortcut.path,
                        shortcut.target,
                        icon_path,
                        relaunch_program_for_match
                    );
                    extract_and_save_icon_from_file(&shortcut.path, use_local_icon);
                    path = shortcut.path.clone();
                    if !process_name.eq_ignore_ascii_case("wps.exe") {
                        display_name = path
                            .file_stem()
                            .unwrap_or_else(|| OsStr::new("Unknown"))
                            .to_string_lossy()
                            .to_string();
                    }
                } else {
                    extract_property_store_fallback_icon(
                        handle,
                        &process_name,
                        use_local_icon,
                        &icon_path,
                        relaunch_program_for_match,
                    );
                }

                if let (Some((prog, args)), Some(relaunch_display_name)) =
                    (relaunch_parts, relaunch_display_name)
                {
                    // 仅对非 wps.exe 进程应用 relaunch_display_name。
                    if !process_name.eq_ignore_ascii_case("wps.exe") {
                        display_name = relaunch_display_name;
                    }
                    (prog, args.map(RelaunchArguments::String))
                } else if let Some(config) =
                    crate::utils::constants::find_launcher_by_umid(umid_str)
                {
                    // 多应用启动器子应用：从 UMID 反向生成启动参数
                    let pkg_name = &umid_str[config.umid_prefix.len()..];
                    (
                        icon_path.to_string_lossy().to_string(),
                        Some(RelaunchArguments::String(format!(
                            "{} {}",
                            config.arg_key, pkg_name
                        ))),
                    )
                } else if shortcut.is_some() {
                    (
                        VAR_COMMON
                            .system_dir()
                            .join("explorer.exe")
                            .to_string_lossy()
                            .to_string(),
                        Some(RelaunchArguments::String(format!(
                            "shell:AppsFolder\\{umid_str}"
                        ))),
                    )
                } else {
                    (path.to_string_lossy().to_string(), None)
                }
            }
        }
    } else {
        // 检查路径是否是可执行文件
        let path_ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());

        let is_executable = path_ext
            .as_ref()
            .map_or(false, |e| e == "exe" || e == "lnk" || e == "url");
        if is_executable {
            extract_and_save_icon_from_file(&icon_path, use_local_icon);
        } else {
            // 非 .exe 文件（如 .pak、.tmp 等），尝试从窗口句柄提取图标
            log::info!("[extract_window_info] Non-executable path detected: {:?}, trying window icon extraction", icon_path);
            let process_name = process_name.clone();
            if !extract_and_save_icon_from_window(
                handle,
                &process_name,
                use_local_icon,
                Some(&icon_path),
            ) {
                // 窗口图标提取失败，仍然尝试从文件提取（可能失败）
                log::warn!("[extract_window_info] Window icon extraction failed, falling back to file extraction");
                extract_and_save_icon_from_file(&icon_path, use_local_icon);
            }
        }
        (path.to_string_lossy().to_string(), None)
    };

    // 5. 智能序列化：只对白名单应用提取图标
    let user_app_window = window.to_smart_serializable(Some(&display_name));

    // 6. 白名单应用图标直接保存到 icon pack（与其他图标保存方式一致）
    if let (Some(ref umid), Some(ref icon_base64)) = (&umid_raw, &user_app_window.icon_png_base64) {
        if user_app_window.is_from_local != Some(true) {
            let umid_str = umid.as_str();
            if let Ok(png_data) =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, icon_base64)
            {
                if let Ok(img) = image::load_from_memory(&png_data) {
                    let root = crate::utils::constants::VAR_COMMON
                        .user_icons_path()
                        .join("system");
                    let name = crate::utils::umid_based_hash_id(umid_str);
                    let filename = format!("{}.png", name);
                    let save_path = root.join(&filename);
                    if img.save(&save_path).is_ok() {
                        log::info!(
                            "[extract_window_info] Saved whitelist icon for UMID {} as {}",
                            umid_str,
                            filename
                        );
                        let mut gen_icon: libs_core::state::Icon = Default::default();
                        gen_icon.base = Some(filename);
                        gen_icon.is_aproximately_square = false;
                        let icon_manager_mutex = FULL_STATE.load().icon_packs().clone();
                        let mut icon_manager = trace_lock!(icon_manager_mutex);
                        // 先清理同 UMID 的旧条目，避免 path 不同导致 matches 失败而产生重复
                        let target_umid = umid_str.to_string();
                        icon_manager.get_system_mut().entries.retain(|e| {
                            if let libs_core::state::IconPackEntry::Unique(u) = e {
                                u.umid.as_deref() != Some(&target_umid)
                            } else {
                                true
                            }
                        });
                        icon_manager.add_system_app_icon(Some(umid_str), None, gen_icon);
                        let _ = icon_manager.write_system_icon_pack();
                        drop(icon_manager);
                        let _ = FULL_STATE.load().emit_icon_packs();
                    }
                }
            }
        }
    }

    // 豆包浏览器特殊处理：返回的 path 也使用 icon_path
    let final_path = if icon_path.to_string_lossy().contains("Doubao_browser_proxy") {
        icon_path.clone()
    } else {
        path.clone()
    };

    // 窗口固定准入：优先依据 Windows 窗口属性 PKEY_AppUserModel_PreventPinning
    // （与资源管理器右键菜单是否显示“固定到任务栏”一致）；再叠加 WPS 文档窗口兑底
    // （WPS 未给 PPT / Word / Excel 文档窗口设置 PreventPinning属性）。
    let pin_disabled = {
        if window.prevent_pinning() {
            true
        } else {
            let proc_is_wps = process_name.eq_ignore_ascii_case("wps.exe");
            let title_lower = title.to_lowercase();
            let doc_markers = [".pptx", ".docx", ".xlsx", ".ppt", ".doc", ".xls", ".pdf"];
            let title_has_doc_ext = doc_markers.iter().any(|m| title_lower.contains(m));
            proc_is_wps && title_has_doc_ext
        }
    };

    Ok(WindowInfo {
        handle,
        title,
        path: final_path,
        umid: umid_raw.map(|u| u.to_string()),
        display_name,
        relaunch_program,
        relaunch_args,
        icon_png_base64: user_app_window.icon_png_base64,
        is_approximately_square: user_app_window.is_approximately_square,
        is_from_local: user_app_window.is_from_local,
        is_iconic: window.is_minimized(),
        is_zoomed: window.is_maximized(),
        is_real_file_explorer,
        pin_disabled,
    })
}

#[derive(Debug, Clone)]
pub struct TaskbarState {
    pub items: TaskbarItems,
}

impl TaskbarState {
    pub fn new() -> Self {
        let mut items = FULL_STATE.load().taskbar_items.clone();
        items.sanitize();

        // 打点：记录初始化时的任务栏状态
        report_taskbar_state("Init", &items);

        let state = TaskbarState { items };

        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));

            UserAppsManager::subscribe(|e| match e {
                UserAppsEvent::WinAdded(addr) => {
                    let window = Window::from(addr);
                    log::info!(
                        "[TaskbarItems][Event] WinAdded hwnd={:?}, title='{}', class='{}'",
                        addr,
                        window.title(),
                        window.class()
                    );
                    match extract_window_info(&window) {
                        Ok(info) => {
                            log::info!(
                                "[TaskbarItems][Event] WinAdded extracted hwnd={}, title='{}', display='{}', path='{}', relaunch='{}', umid={:?}",
                                info.handle,
                                info.title,
                                info.display_name,
                                info.path.display(),
                                info.relaunch_program,
                                info.umid
                            );
                            let mut guard = trace_lock!(TASKBAR_STATE);
                            if guard.contains_hwnd(info.handle) {
                                log::info!(
                                    "[TaskbarItems][Event] WinAdded ignored because hwnd already exists: {}",
                                    info.handle
                                );
                            } else {
                                match guard.add_item(info) {
                                    Ok(()) => {
                                        guard.items.sanitize();
                                        match guard.emit_to_webview() {
                                            Ok(()) => log::info!("[TaskbarItems][Event] WinAdded emitted StateTaskbarItemsChanged"),
                                            Err(e) => log::error!("[TaskbarItems][Event] WinAdded emit failed: {:?}", e),
                                        }
                                    }
                                    Err(e) => log::error!(
                                        "[TaskbarItems][Event] WinAdded add_item failed: {:?}",
                                        e
                                    ),
                                }
                            }
                        }
                        Err(e) => {
                            log::error!(
                                "[TaskbarItems][Event] WinAdded extract_window_info failed hwnd={:?}: {:?}",
                                addr,
                                e
                            );
                        }
                    }
                }
                UserAppsEvent::WinUpdated(addr) => {
                    let window = Window::from(addr);
                    log::info!(
                        "[TaskbarItems][Event] WinUpdated hwnd={:?}, title='{}', class='{}'",
                        addr,
                        window.title(),
                        window.class()
                    );
                    {
                        let mut guard = trace_lock!(TASKBAR_STATE);
                        guard.update_window_info(&window);
                        match guard.emit_to_webview() {
                            Ok(()) => log::info!(
                                "[TaskbarItems][Event] WinUpdated emitted StateTaskbarItemsChanged"
                            ),
                            Err(e) => {
                                log::error!("[TaskbarItems][Event] WinUpdated emit failed: {:?}", e)
                            }
                        }
                    }
                    schedule_tracked_window_liveness_reconcile("win_updated");
                }
                UserAppsEvent::WinRemoved(addr) => {
                    let window = Window::from(addr);
                    log::info!(
                        "[TaskbarItems][Event] WinRemoved hwnd={:?}, title='{}', class='{}'",
                        addr,
                        window.title(),
                        window.class()
                    );
                    {
                        let mut guard = trace_lock!(TASKBAR_STATE);
                        guard.remove(&window);
                    }
                    // 在释放锁后执行耗时操作，避免死锁
                    crate::get_tokio_handle().spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        // 执行 sanitize 和 emit_to_webview
                        {
                            let mut guard = trace_lock!(TASKBAR_STATE);
                            guard.items.sanitize();
                            // 打点：记录窗口实时移除后的任务栏状态
                            report_taskbar_state("WindowRemove", &guard.items);
                            match guard.emit_to_webview() {
                                Ok(()) => log::info!("[TaskbarItems][Event] WinRemoved emitted StateTaskbarItemsChanged"),
                                Err(e) => log::error!("[TaskbarItems][Event] WinRemoved emit failed: {:?}", e),
                            }
                        }

                        crate::widgets::taskbar::Taskbar::check_overlap_for_all_taskbars(&window);
                    });
                }
                _ => {}
            });

            log::info!("[Background] Starting initial window processing...");
            USER_APPS_MANAGER.interactable_windows.for_each(|w| {
                let window = Window::from(w.hwnd);
                if let Ok(info) = extract_window_info(&window) {
                    let mut guard = trace_lock!(TASKBAR_STATE);
                    if !guard.contains_hwnd(info.handle) {
                        guard.add_item(info).ok();
                    }
                }
            });

            {
                let mut guard = trace_lock!(TASKBAR_STATE);
                guard.items.sanitize();
                guard.emit_to_webview().log_error();
                log::debug!("[Background] Initial window processing complete");
            }
        });

        state
    }

    pub fn contains_hwnd(&self, handle: isize) -> bool {
        self.iter_all()
            .any(|item| item_contains_window(item, handle))
    }

    pub fn add_item(&mut self, mut info: WindowInfo) -> Result<()> {
        if info.title.is_empty() {
            if let Some(file_name) = info.path.file_name() {
                if file_name
                    .to_string_lossy()
                    .eq_ignore_ascii_case("AIAssistantMain.exe")
                {
                    info.title = "YOYO".to_string();
                    log::debug!(
                        "[Taskbar] AIAssistantMain.exe detected with empty title, set to 'YOYO'"
                    );
                }
            }
        }

        // 2. 匹配固定项
        for item in self.items.left.iter_mut() {
            if let TaskbarItem::Pinned(current) = item {
                if Self::is_window_matching_item(&info, current) {
                    current.windows.push(TaskbarAppGroupItem {
                        title: info.title.clone(),
                        handle: info.handle,
                        is_iconic: info.is_iconic,
                        is_zoomed: info.is_zoomed,
                        last_active: 0,
                        icon_png_base64: info.icon_png_base64.clone(),
                        is_approximately_square: info.is_approximately_square,
                        is_from_local: info.is_from_local,
                    });
                    return Ok(());
                }
            }
        }

        // 3. 匹配现有临时项
        let mut matched = false;
        for item in self
            .items
            .center
            .iter_mut()
            .chain(self.items.right.iter_mut())
        {
            if let TaskbarItem::Temporal(current) = item {
                if Self::is_window_matching_item(&info, current) {
                    log::info!("[TaskBar_AddItem] Matched existing temporal: current_display={}, new_display={}, current_path={}, new_path={}", 
                        current.display_name, info.display_name, current.relaunch_program, info.relaunch_program);
                    current.windows.push(TaskbarAppGroupItem {
                        title: info.title.clone(),
                        handle: info.handle,
                        is_iconic: info.is_iconic,
                        is_zoomed: info.is_zoomed,
                        last_active: 0,
                        icon_png_base64: info.icon_png_base64.clone(),
                        is_approximately_square: info.is_approximately_square,
                        is_from_local: info.is_from_local,
                    });
                    matched = true;
                    break;
                }
            }
        }

        if matched {
            return Ok(());
        }

        // 4. 作为新临时项推入末尾
        // 直接使用 extract_window_info 计算好的 relaunch 信息（已区分 Appx/PropertyStore/Androws 等场景）
        let (final_relaunch_program, final_relaunch_args) =
            (info.relaunch_program.clone(), info.relaunch_args.clone());

        let data = PinnedTaskbarItemData {
            id: uuid::Uuid::new_v4().to_string(),
            subtype: TaskbarItemSubtype::App,
            umid: info.umid.clone(),
            path: info.path.clone(),
            relaunch_command: None,
            relaunch_program: final_relaunch_program,
            relaunch_args: final_relaunch_args,
            relaunch_in: None,
            display_name: info.display_name.clone(),
            icon_hash: None,
            is_approximately_square: info.is_approximately_square,
            is_dir: false,
            windows: vec![TaskbarAppGroupItem {
                title: info.title,
                handle: info.handle,
                is_iconic: info.is_iconic,
                is_zoomed: info.is_zoomed,
                last_active: 0,
                icon_png_base64: info.icon_png_base64,
                is_approximately_square: info.is_approximately_square,
                is_from_local: info.is_from_local,
            }],
            pin_disabled: info.pin_disabled,
        };

        self.items.center.push(TaskbarItem::Temporal(data));

        // 打点：记录窗口添加后的任务栏状态
        report_taskbar_state("WindowAdd", &self.items);
        log::info!(
            "[TaskbarItems][AddItem] created temporal hwnd={}",
            info.handle
        );

        Ok(())
    }

    fn is_window_matching_item(info: &WindowInfo, item: &PinnedTaskbarItemData) -> bool {
        // -1. WPS 主应用身份早返回：双方都满足主应用身份时，直接视为同一项。
        // 场景：用户固定 WPS 主程序后，固定项 relaunch 可能为 ksolaunch.exe（.lnk 目标）且 UMID=None,
        // 新启动的 wps.exe 主程序窗口 UMID=WpsOffice.<数字>，两者因 exe 名不同且被下方
        // “ksolaunch × wps” 硬拒规则隔断。通过身份早返回让主程序窗口归并回固定项。
        // 注意：is_wps_main_identity 已要求 display_name 不含文档扩展名，可有效过滤 PPT/XLS
        // 文档窗口；对 WPS图片 等子应用 UMID 也会因不匹配而排除。
        let current_exe_lower = PathBuf::from(&item.relaunch_program)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();
        let info_exe_lower = PathBuf::from(&info.relaunch_program)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();
        let item_is_main =
            is_wps_main_identity(item.umid.as_deref(), &current_exe_lower, &item.display_name);
        let info_is_main =
            is_wps_main_identity(info.umid.as_deref(), &info_exe_lower, &info.display_name);
        if item_is_main && info_is_main {
            log::debug!(
                "[WindowMatch] WPS 主应用身份合并: item(exe={}, umid={:?}, dn={}), info(exe={}, umid={:?}, dn={})",
                current_exe_lower, item.umid, item.display_name,
                info_exe_lower, info.umid, info.display_name);
            return true;
        }

        // 0. WPS 特殊处理：禁止 ksolaunch (启动器) 与 wps.exe (实际进程) 的任何匹配
        let item_is_wps_launcher = item.relaunch_program.to_lowercase().contains("ksolaunch");
        let info_is_wps_app = info.relaunch_program.to_lowercase().contains("wps");
        if item_is_wps_launcher && info_is_wps_app {
            return false;
        }

        // 1. 严格 UMID 匹配
        let umid_matches = match (&item.umid, &info.umid) {
            (Some(c), Some(w)) => c.to_lowercase() == w.to_lowercase(),
            _ => false,
        };

        let current_exe = PathBuf::from(&item.relaunch_program)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();
        let info_exe = PathBuf::from(&info.relaunch_program)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();

        // 检查是否为通用安装程序
        let is_generic_installer = crate::utils::icon_whitelist::is_generic_installer(&info_exe);
        let current_is_generic = crate::utils::icon_whitelist::is_generic_installer(&current_exe);

        // 2. 资源管理器特殊处理逻辑
        let pinned_is_explorer = current_exe == "explorer.exe"
            && item
                .umid
                .as_deref()
                .map_or(false, |u| u == "Microsoft.Windows.Explorer");
        let mut exe_matches = !current_exe.is_empty() && current_exe == info_exe;

        // 对于通用安装程序（setup.exe, install.exe等），必须路径完全相同
        if is_generic_installer && current_is_generic {
            exe_matches =
                item.relaunch_program.to_lowercase() == info.relaunch_program.to_lowercase();
            log::debug!(
                "[WindowMatch] Generic installer check: exe1={}, exe2={}, path_matches={}",
                current_exe,
                info_exe,
                exe_matches
            );
            // 对于通用安装程序，路径不符就立即拒绝，不继续检查其他条件
            if !exe_matches {
                return false;
            }
        } else if exe_matches {
            // 如果是资源管理器，必须满足 is_real_file_explorer 且 UMID 匹配
            if current_exe == "explorer.exe" {
                if info.umid.is_some() {
                    exe_matches = umid_matches;
                } else {
                    exe_matches = pinned_is_explorer && info.is_real_file_explorer;
                }
            }
            // Androws 启动器承载多个子应用，必须通过 UMID 区分
            if current_exe == "androwslauncher.exe" {
                exe_matches = umid_matches;
            }
        }

        // 3. UWP / ApplicationFrameHost 降级匹配 (还原被删逻辑)
        if !exe_matches && info_exe == "applicationframehost.exe" {
            exe_matches = umid_matches;
        }

        let display_name_matches =
            item.display_name.to_lowercase() == info.display_name.to_lowercase();

        // WPS 特殊处理：禁用 display_name 匹配，让不同文档根据不同的窗口标题创建独立项目
        let display_name_matches = if item.display_name.to_lowercase().contains("wps")
            || info.display_name.to_lowercase().contains("wps")
        {
            // WPS 文档全部禁用 display_name 匹配，碼国为会根据窗口标题不同而创建独立项目
            false
        } else {
            display_name_matches
        };

        // --- 特殊应用处理逻辑 ---

        // 1. WPS Office: 已禁用特殊合并逻辑，各文档独立显示
        // 让不同标题的 WPS 文档窗口创建各自独立的任务栏项，而不是合并到启动器项
        let wps_matches = false;

        // 2. 芒果TV: 芒果TV.exe -> mgtv.exe
        let is_mgtv_launcher = item.relaunch_program.to_lowercase().contains("芒果tv")
            && item.relaunch_program.to_lowercase().ends_with("芒果tv.exe");
        let is_mgtv_app = info.relaunch_program.to_lowercase().contains("mgtv.exe");
        let mgtv_matches = is_mgtv_launcher && is_mgtv_app;

        // 3. 优酷: 播放器进程 (YoukuPlayer/ykplayer) 匹配主程序
        let is_youku_main = item.relaunch_program.to_lowercase().contains("youku")
            || item.relaunch_program.to_lowercase().contains("ykplayer")
            || item.display_name.contains("优酷");
        let is_youku_player = info.relaunch_program.to_lowercase().contains("youku")
            || info.relaunch_program.to_lowercase().contains("ykplayer")
            || info.display_name.contains("优酷");
        let youku_matches = is_youku_main && is_youku_player;

        // 4. Edge 浏览器
        let edge_matches = current_exe == "msedge.exe" && info_exe == "msedge.exe";

        // 5. 通用安装程序处理（browser.exe, launcher.exe 等必须路径完全匹配）
        // 这已经能处理 WeGame browser.exe 的情况
        // 注意：此逻辑与上面的通用安装程序检查配合使用

        // 6. 🔧 关键修复：限制 display_name 匹配
        // 如果窗口有 UMID（UWP 应用），但临时项的 relaunch_program 是 explorer.exe（启动器），
        // 不应该通过 display_name 匹配，因为不同的 UWP 应用都通过 explorer.exe 启动
        let should_use_display_name_match = if info.umid.is_some() {
            let item_is_uwp_launcher = current_exe == "explorer.exe" && item.umid.is_none();
            if item_is_uwp_launcher {
                false
            } else {
                display_name_matches
            }
        } else {
            display_name_matches
        };

        umid_matches
            || exe_matches
            || should_use_display_name_match
            || wps_matches
            || mgtv_matches
            || youku_matches
            || edge_matches
    }

    pub fn emit_to_webview(&mut self) -> Result<()> {
        let handle = get_app_handle();
        handle.emit(FuncEvent::StateTaskbarItemsChanged, ())?;
        Ok(())
    }

    pub fn iter_all(&self) -> impl Iterator<Item = &TaskbarItem> {
        self.items
            .left
            .iter()
            .chain(self.items.center.iter())
            .chain(self.items.right.iter())
    }

    pub fn iter_all_mut(&mut self) -> impl Iterator<Item = &mut TaskbarItem> {
        self.items
            .left
            .iter_mut()
            .chain(self.items.center.iter_mut())
            .chain(self.items.right.iter_mut())
    }

    pub fn contains(&self, window: &Window) -> bool {
        let searching = window.address();
        self.iter_all()
            .any(|item| item_contains_window(item, searching))
    }

    pub fn remove(&mut self, window: &Window) {
        let searching = window.address();
        self.iter_all_mut().for_each(|item| match item {
            TaskbarItem::Pinned(data) | TaskbarItem::Temporal(data) => {
                data.windows.retain(|w| w.handle != searching);
            }
            _ => {}
        });

        // 清理空的临时项
        self.items.center.retain(|item| {
            if let TaskbarItem::Temporal(data) = item {
                // 普通应用项如果没有窗口了，则清理
                !data.windows.is_empty()
            } else {
                true
            }
        });

        self.items.sanitize();
    }

    pub fn get_window_mut(&mut self, window: &Window) -> Option<&mut TaskbarAppGroupItem> {
        let searching = window.address();
        self.iter_all_mut().find_map(|item| match item {
            TaskbarItem::Pinned(data) | TaskbarItem::Temporal(data) => {
                data.windows.iter_mut().find(|w| w.handle == searching)
            }
            _ => None,
        })
    }

    pub fn update_window_info(&mut self, window: &Window) {
        let searching = window.address();
        let mut updated = false;

        // Find the window and its associated display_name
        for item in self.iter_all_mut() {
            match item {
                TaskbarItem::Pinned(data) | TaskbarItem::Temporal(data) => {
                    if let Some(app_window) =
                        data.windows.iter_mut().find(|w| w.handle == searching)
                    {
                        // 获取标题并修正 AIAssistantMain.exe 的空标题
                        let mut title = window.title();

                        // 注意：display_name 可能是 UMID（如 app_menu.ai_assistant），所以通过路径判断
                        if title.is_empty() {
                            if let Ok(process_path) = window.process().program_path() {
                                if let Some(file_name) = process_path.file_name() {
                                    if file_name
                                        .to_string_lossy()
                                        .eq_ignore_ascii_case("AIAssistantMain.exe")
                                    {
                                        title = "YOYO".to_string();
                                        log::debug!("[Taskbar] AIAssistantMain.exe with empty title, set to 'YOYO'");
                                    }
                                }
                            }
                        }
                        app_window.title = title;
                        app_window.is_iconic = window.is_minimized();
                        app_window.is_zoomed = window.is_maximized();

                        // Check if this is a Control Panel window based on display_name OR window class
                        let _is_control_panel = data.display_name.contains("控制面板");

                        // 使用智能序列化，自动区分白名单和非白名单应用
                        let user_app_window =
                            window.to_smart_serializable(Some(&data.display_name));
                        app_window.icon_png_base64 = user_app_window.icon_png_base64;
                        app_window.is_approximately_square =
                            user_app_window.is_approximately_square;
                        updated = true;
                        break;
                    }
                }
                _ => {}
            }
        }
        if updated {
            log::info!(
                "[TaskbarItems][Update] updated hwnd={}, title='{}'",
                searching,
                window.title()
            );
        } else {
            log::warn!(
                "[TaskbarItems][Update] skipped because hwnd not found: hwnd={}, title='{}'",
                searching,
                window.title()
            );
        }
    }

    pub fn update_window_activation(&mut self, window: &Window) {
        if let Some(app_window) = self.get_window_mut(window) {
            app_window.last_active = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards")
                .as_millis() as u64;
        }
    }

    /// 刷新所有窗口的图标信息（用于背板切换时更新白名单窗口的图标）
    pub fn refresh_all_window_icons(&mut self) {
        for item in self.iter_all_mut() {
            match item {
                TaskbarItem::Pinned(data) | TaskbarItem::Temporal(data) => {
                    for app_window in &mut data.windows {
                        let window = Window::from(app_window.handle);
                        let user_app_window =
                            window.to_smart_serializable(Some(&data.display_name));
                        app_window.icon_png_base64 = user_app_window.icon_png_base64;
                        app_window.is_approximately_square =
                            user_app_window.is_approximately_square;
                        app_window.is_from_local = user_app_window.is_from_local;
                    }
                }
                _ => {}
            }
        }
    }

    fn filter_by_monitor(&mut self, monitor_id: &MonitorId) {
        for item in self.iter_all_mut() {
            match item {
                TaskbarItem::Pinned(data) | TaskbarItem::Temporal(data) => {
                    data.windows.retain(|w| {
                        let window = Window::from(w.handle);
                        &window.monitor_id() == monitor_id
                    });
                }
                _ => {}
            }
        }
    }

    pub fn get_filtered_by_monitor(&self) -> Result<HashMap<MonitorId, TaskbarItems>> {
        let mut result = HashMap::new();
        let state = FULL_STATE.load();
        // 从缓存获取回收站状态（无锁原子读取，性能极高）
        let is_empty = crate::hook::RECYCLE_BIN_EMPTY_CACHE.load(Ordering::Relaxed);
        let mut items = self.items.clone();
        for item in items
            .left
            .iter_mut()
            .chain(items.center.iter_mut())
            .chain(items.right.iter_mut())
        {
            if let TaskbarItem::RecycleBin {
                is_empty: empty_ref,
                ..
            } = item
            {
                *empty_ref = is_empty;
            }
        }
        for monitor in MonitorEnumerator::get_all_v2()? {
            let monitor_id = monitor.stable_id()?.into();
            if !state.is_taskbar_enabled_on_monitor(&monitor_id) {
                continue;
            }

            let temporal_mode = state.get_taskbar_temporal_item_visibility(&monitor_id);
            let pinned_mode = state.get_taskbar_pinned_item_visibility(&monitor_id);
            let pinned_visible = match pinned_mode {
                TaskbarPinnedItemsVisibility::Always => true,
                TaskbarPinnedItemsVisibility::WhenPrimary => monitor.is_primary(),
            };

            match temporal_mode {
                TaskbarTemporalItemsVisibility::All => {
                    let mut items = items.clone();
                    if !pinned_visible {
                        temporalise(&mut items);
                    }
                    items.sanitize();
                    // sanitize() 后重新更新 RecycleBin 的状态
                    for item in items
                        .left
                        .iter_mut()
                        .chain(items.center.iter_mut())
                        .chain(items.right.iter_mut())
                    {
                        if let TaskbarItem::RecycleBin {
                            is_empty: empty_ref,
                            ..
                        } = item
                        {
                            *empty_ref = is_empty;
                        }
                    }
                    result.insert(monitor_id, items);
                }
                TaskbarTemporalItemsVisibility::OnMonitor => {
                    let mut taskbar_items = TaskbarState {
                        items: items.clone(),
                    };
                    taskbar_items.filter_by_monitor(&monitor_id);
                    if !pinned_visible {
                        temporalise(&mut taskbar_items.items);
                    }
                    taskbar_items.items.sanitize();
                    // sanitize() 后重新更新 RecycleBin 的状态
                    for item in taskbar_items
                        .items
                        .left
                        .iter_mut()
                        .chain(taskbar_items.items.center.iter_mut())
                        .chain(taskbar_items.items.right.iter_mut())
                    {
                        if let TaskbarItem::RecycleBin {
                            is_empty: empty_ref,
                            ..
                        } = item
                        {
                            *empty_ref = is_empty;
                        }
                    }
                    result.insert(monitor_id, taskbar_items.items);
                }
            }
        }

        Ok(result)
    }
}

fn item_contains_window(item: &TaskbarItem, searching: isize) -> bool {
    match item {
        TaskbarItem::Pinned(data) | TaskbarItem::Temporal(data) => {
            data.windows.iter().any(|w| w.handle == searching)
        }
        _ => false,
    }
}

fn temporalise_collection(source: &Vec<TaskbarItem>) -> Vec<TaskbarItem> {
    let mut items = vec![];
    for item in source {
        match item {
            TaskbarItem::Temporal(pinned_taskbar_item_data) => {
                let mut cloned = pinned_taskbar_item_data.clone();
                cloned.set_pin_disabled(true);
                items.push(TaskbarItem::Temporal(cloned))
            }
            TaskbarItem::Pinned(pinned_taskbar_item_data) => {
                let mut cloned = pinned_taskbar_item_data.clone();
                cloned.set_pin_disabled(true);
                items.push(TaskbarItem::Temporal(cloned))
            }
            TaskbarItem::Separator { id } => items.push(TaskbarItem::Separator { id: id.clone() }),
            TaskbarItem::StartMenu { id } => items.push(TaskbarItem::StartMenu { id: id.clone() }),
            TaskbarItem::RecycleBin { id, is_empty } => items.push(TaskbarItem::RecycleBin {
                id: id.clone(),
                is_empty: *is_empty,
            }),
            TaskbarItem::SystemTray { id } => {
                items.push(TaskbarItem::SystemTray { id: id.clone() })
            }
        }
    }
    items
}

fn temporalise(items: &mut TaskbarItems) {
    items.left = temporalise_collection(&items.left);
    items.center = temporalise_collection(&items.center);
    items.right = temporalise_collection(&items.right);
}

fn get_parts_of_inline_command(cmd: &str) -> (String, Option<String>) {
    let start_double_quoted = cmd.starts_with("\"");
    if start_double_quoted || cmd.starts_with("'") {
        let delimiter = if start_double_quoted { '"' } else { '\'' };
        let mut parts = cmd.split(['"', '\'']).filter(|s| !s.is_empty());

        let program = parts.next().unwrap_or_default().trim().to_owned();
        let args = cmd
            .trim_start_matches(&format!("{delimiter}{program}{delimiter}"))
            .trim()
            .to_owned();
        return (program, if args.is_empty() { None } else { Some(args) });
    }

    let cmd_as_path = PathBuf::from(cmd);
    if cmd_as_path.exists() {
        let program = cmd_as_path.to_string_lossy().to_string();
        return (program, None);
    }

    let mut parts = cmd.split(" ").filter(|s| !s.is_empty());
    let program = parts.next().unwrap_or_default().trim().to_owned();
    let args = cmd.trim_start_matches(&program).trim().to_owned();
    (program, if args.is_empty() { None } else { Some(args) })
}

fn get_process_name_with_fallback(window: &Window) -> String {
    if let Ok(name) = window.process().program_exe_name() {
        return name;
    }

    use crate::cli::ServicePipe;
    use slu_ipc::messages::SvcAction;

    let result = ServicePipe::request_with_response_blocking(
        SvcAction::GetProcessName {
            hwnd: window.address(),
        },
        std::time::Duration::from_millis(800),
    );

    if let Ok(Some(name)) = result {
        return name;
    }

    let title = window.title();
    if !title.is_empty() {
        title
    } else {
        let class = window.class();
        if !class.is_empty() {
            class
        } else {
            "unknown".to_string()
        }
    }
}

fn get_process_path_with_fallback(window: &Window, context: &str) -> Result<PathBuf> {
    match window.process().program_path() {
        Ok(path) => Ok(path),
        Err(_) => {
            use crate::cli::ServicePipe;
            use slu_ipc::messages::SvcAction;

            let result = ServicePipe::request_with_response_blocking(
                SvcAction::GetProcessPath {
                    hwnd: window.address(),
                },
                std::time::Duration::from_millis(800),
            );

            if let Ok(Some(path_str)) = result {
                return Ok(PathBuf::from(path_str));
            }

            let process_name = get_process_name_with_fallback(window);

            if process_name == "consent.exe" {
                Ok(VAR_COMMON.system_dir().join("consent.exe"))
            } else {
                Err(format!(
                    "Failed to get process path for {} ({})",
                    process_name, context
                )
                .into())
            }
        }
    }
}
