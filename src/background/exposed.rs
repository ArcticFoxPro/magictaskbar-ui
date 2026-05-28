use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use libs_core::state::{IconBackplateStyle, RelaunchArguments};
use libs_core::{command_handler_list, system_state::Color};

use tauri::{AppHandle, Builder, Emitter, Manager, WebviewWindow, Wry};
use translators::Translator;
use windows::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};

use crate::error::Result;
use crate::hook::HookManager;
use crate::modules::input::Keyboard;
use crate::state::application::FULL_STATE;

use crate::cli::ServicePipe;
use crate::utils::constants::VAR_COMMON;
use crate::utils::icon_extractor::{extract_and_save_icon_from_file, extract_and_save_icon_umid};
use crate::windows_api::hdc::DeviceContext;
use crate::windows_api::window::event::WinEvent;
use crate::windows_api::window::Window;
use crate::windows_api::{WindowEnumerator, WindowsApi};

use serde::Deserialize;
use slu_ipc::messages::SvcAction;
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};
use windows::Win32::UI::WindowsAndMessaging::WM_USER;

// ==================== SID Cache ====================

// Static cache for SID to avoid repeated queries
static CACHED_SID: OnceLock<Option<String>> = OnceLock::new();

fn get_current_user_sid() -> Option<String> {
    // If already cached, return directly
    if let Some(cached) = CACHED_SID.get() {
        log::debug!("[get_current_user_sid] Using cached SID");
        return cached.clone();
    }

    // First time getting SID
    let sid = get_current_user_sid_internal();

    // Cache result
    CACHED_SID.set(sid.clone()).ok();

    sid
}

// Public function to pre-initialize SID cache (called from main.rs setup)
pub fn get_current_user_sid_for_cache() {
    let _ = get_current_user_sid();
}

fn get_current_user_sid_internal() -> Option<String> {
    // Try HKLM registry first (faster, no HKEY_USERS needed)
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    match hklm.open_subkey("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Authentication\\LogonUI")
    {
        Ok(settings) => {
            if let Ok(sid) = settings.get_value::<String, _>("LastLoggedOnUserSID") {
                log::info!(
                    "[get_current_user_sid_internal] Successfully obtained SID from registry: {}",
                    sid
                );
                return Some(sid);
            }
        }
        Err(_) => {}
    }

    // Fallback: use whoami /user command
    let output = std::process::Command::new("whoami")
        .args(["/user", "/fo", "csv", "/nh"])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let output_str = String::from_utf8_lossy(&o.stdout);
            let sid_str = output_str.trim().to_string();
            if !sid_str.is_empty() {
                log::info!(
                    "[get_current_user_sid_internal] Successfully obtained SID via whoami: {}",
                    sid_str
                );
                return Some(sid_str);
            }
        }
        _ => {}
    }

    log::warn!("[get_current_user_sid_internal] Failed to obtain user SID");
    None
}

// ==================== Blur background (Plan A: screenshot + Gaussian blur) ====================

/// GDI: capture a rectangular region of the desktop into RGBA pixels.
unsafe fn capture_desktop_region(x: i32, y: i32, w: i32, h: i32) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, GetDC, GetDIBits, ReleaseDC,
        SelectObject, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HGDIOBJ, SRCCOPY,
    };

    let screen_dc = GetDC(None);
    let mem_dc = CreateCompatibleDC(Some(screen_dc));
    let bitmap = CreateCompatibleBitmap(screen_dc, w, h);
    let old = SelectObject(mem_dc, HGDIOBJ(bitmap.0));

    let _ = BitBlt(mem_dc, 0, 0, w, h, Some(screen_dc), x, y, SRCCOPY);

    let mut bmi: BITMAPINFO = std::mem::zeroed();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w;
    bmi.bmiHeader.biHeight = -h; // top-down
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = 0; // BI_RGB

    let mut buf = vec![0u8; (w * h * 4) as usize];
    GetDIBits(
        mem_dc,
        bitmap,
        0,
        h as u32,
        Some(buf.as_mut_ptr().cast()),
        &mut bmi,
        DIB_RGB_COLORS,
    );

    SelectObject(mem_dc, old);
    let _ = DeleteDC(mem_dc);
    ReleaseDC(None, screen_dc);
    // DeleteObject for HBITMAP via FFI (HGDIOBJ wrapper)
    {
        #[link(name = "gdi32")]
        extern "system" {
            fn DeleteObject(ho: *mut c_void) -> i32;
        }
        DeleteObject(bitmap.0);
    }

    // BGRA → RGBA
    for chunk in buf.chunks_exact_mut(4) {
        chunk.swap(0, 2);
    }

    Some(buf)
}

fn create_transparent_background(width: i32, height: i32) -> Result<String> {
    use base64::Engine;
    use image::{codecs::png::PngEncoder, ImageEncoder};

    let w = width as u32;
    let h = height as u32;

    // 创建完全透明的 RGBA 图片
    let pixels = vec![0u8; (w * h * 4) as usize];

    // 编码为 PNG（支持透明度）
    let mut png_buf: Vec<u8> = Vec::new();
    PngEncoder::new(&mut png_buf)
        .write_image(&pixels, w, h, image::ExtendedColorType::Rgba8)
        .map_err(|e| format!("PNG encode error: {e}"))?;

    // 转换为 base64
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png_buf);
    Ok(format!("data:image/png;base64,{}", b64))
}

fn load_screen_recorders() -> Option<Vec<String>> {
    use std::fs;

    // 使用相对于可执行文件的路径访问配置文件
    let path = crate::utils::get_app_dir()
        .join("static")
        .join("screen_recorders.yml");

    // 使用 ? 操作符简化错误处理链
    let content = fs::read_to_string(&path).ok()?;
    let yaml = serde_yaml::from_str::<serde_yaml::Value>(&content).ok()?;
    let recorders = yaml.get("recorders")?.as_sequence()?;

    // 使用迭代器链式操作，更简洁地提取名称
    let names: Vec<String> = recorders
        .iter()
        .filter_map(|v| v.as_str())
        .map(|s| s.to_string())
        .collect();

    // 如果列表不为空，返回它，否则返回 None
    if names.is_empty() {
        None
    } else {
        Some(names)
    }
}

#[tauri::command(async)]
pub async fn kill_process_by_name(image: String) -> Result<()> {
    service_backend_command(
        "kill_process_by_name",
        serde_json::json!({ "image": image }),
    )
    .await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn open_file(path: String) -> Result<()> {
    let working_dir = default_exe_working_dir(&path, None);
    service_backend_command(
        "open_file_path",
        serde_json::json!({
            "path": path,
            "workingDir": working_dir.map(|path| path.to_string_lossy().to_string()),
        }),
    )
    .await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn honor_calendar_widget_open() -> Result<bool> {
    use std::path::Path;

    const CALENDAR_EXE: &str = r"C:\Program Files\HONOR\HnAgentStudio\HnCalendarWidget.exe";
    const CALENDAR_EXE_NAME: &str = "HnCalendarWidget.exe";

    if Path::new(CALENDAR_EXE).exists() {
        let is_running = || {
            let mut sys = sysinfo::System::new();
            sys.refresh_processes();
            sys.processes().values().any(|p| {
                p.exe()
                    .is_some_and(|path| path.ends_with(CALENDAR_EXE_NAME))
            })
        };

        // 如果进程未运行：先无参拉起一次，再 sleep 200ms 让进程完成初始化
        // 拉起进程前判断进程是否存在
        if is_running() {
            // 点击图标时，通过 -start 触发日历显示/激活
            let _ = spawn_with_cmd_start(CALENDAR_EXE, "-start", None);
            return Ok(true);
        }
    }

    log::warn!(
        "[Calendar] exe not found: {}; falling back to Win+N",
        CALENDAR_EXE
    );
    send_keys("{win}n".to_string())?;
    Ok(false)
}

#[tauri::command(async)]
pub async fn honor_message_center_ui_open() -> Result<bool> {
    use std::path::Path;
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};

    const MESSAGE_CENTER_EXE: &str = r"C:\Program Files\HONOR\PCManager\MessageCenterUI.exe";
    const MIN_PC_MANAGER_VERSION: [u32; 4] = [20, 0, 0, 45];

    fn parse_4part_version(s: &str) -> Option<[u32; 4]> {
        let mut out = [0u32; 4];
        let parts: Vec<&str> = s.trim().split('.').collect();
        if parts.is_empty() {
            return None;
        }
        for (idx, p) in parts.iter().take(4).enumerate() {
            out[idx] = p.trim().parse::<u32>().ok()?;
        }
        Some(out)
    }

    fn version_ge(a: [u32; 4], b: [u32; 4]) -> bool {
        for i in 0..4 {
            if a[i] > b[i] {
                return true;
            }
            if a[i] < b[i] {
                return false;
            }
        }
        true
    }

    // Gate by PC Manager version. If version < 20.0.0.45, fall back to Win+N.
    let pc_manager_version_ok = (|| {
        let key = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PC Manager")
            .ok()?;
        let v = key.get_value::<String, _>("DisplayVersion").ok()?;
        let parsed = parse_4part_version(&v)?;
        Some(version_ge(parsed, MIN_PC_MANAGER_VERSION))
    })()
    .unwrap_or(false);

    if !pc_manager_version_ok {
        log::warn!(
            "[MessageCenter] PC Manager version < {}.{}.{}.{}; falling back to Win+N",
            MIN_PC_MANAGER_VERSION[0],
            MIN_PC_MANAGER_VERSION[1],
            MIN_PC_MANAGER_VERSION[2],
            MIN_PC_MANAGER_VERSION[3]
        );
        send_keys("{win}n".to_string())?;
        return Ok(false);
    }

    if Path::new(MESSAGE_CENTER_EXE).exists() {
        let ui_exe_name = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "magictaskbar-ui.exe".to_string());

        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis().to_string())
            .unwrap_or_else(|_| "0".to_string());

        let args = RelaunchArguments::Array(vec![ui_exe_name, ts_ms, "/r".to_string()]);
        run(MESSAGE_CENTER_EXE.to_string(), Some(args), None).await?;
        return Ok(true);
    }

    log::warn!(
        "[MessageCenter] exe not found: {}; falling back to Win+N",
        MESSAGE_CENTER_EXE
    );
    send_keys("{win}n".to_string())?;
    Ok(false)
}

#[tauri::command(async)]
async fn select_file_on_explorer(path: String) -> Result<()> {
    service_backend_command(
        "select_file_on_explorer",
        serde_json::json!({ "path": path }),
    )
    .await?;
    Ok(())
}

/// 使用 cmd start 方式启动程序
fn spawn_with_cmd_start(program: &str, args: &str, working_dir: Option<PathBuf>) -> Result<()> {
    log::debug!("[Run] 普通程序，使用 cmd start 方式启动");
    let mut cmd = std::process::Command::new("cmd");
    cmd.raw_arg("/c").raw_arg("start").raw_arg("\"\""); // 窗口标题（空）

    // 设置工作目录
    if let Some(dir) = working_dir {
        cmd.raw_arg("/D").raw_arg(format!("\"{}\"", dir.display()));
    }

    // 添加程序路径
    cmd.raw_arg(format!("\"{program}\""));

    // 添加参数
    if !args.is_empty() {
        cmd.raw_arg(args);
    }

    log::info!("[Run] cmd 方式启动: {:?}", cmd);

    cmd.creation_flags(CREATE_NO_WINDOW.0 | CREATE_NEW_PROCESS_GROUP.0)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    Ok(())
}

fn default_exe_working_dir(program: &str, working_dir: Option<PathBuf>) -> Option<PathBuf> {
    if working_dir.is_some() {
        return working_dir;
    }

    let program_path = Path::new(program);
    if !program_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return None;
    }

    program_path.parent().map(Path::to_path_buf)
}

const TRAY_APP_ACTIVATION_WAIT_MS: u64 = 1200;
const TRAY_APP_ACTIVATION_POLL_MS: u64 = 100;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrayActivationHotkey {
    #[serde(default)]
    modifiers: Vec<String>,
    key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrayActivationRule {
    #[serde(default)]
    name: Option<String>,
    process_names: Vec<String>,
    hotkey: TrayActivationHotkey,
    #[serde(default)]
    suppress_launch_when_process_exists: bool,
    #[serde(default)]
    wait_for_window_ms: Option<u64>,
}

static TRAY_ACTIVATION_RULES: OnceLock<Vec<TrayActivationRule>> = OnceLock::new();

fn tray_activation_rules() -> &'static [TrayActivationRule] {
    TRAY_ACTIVATION_RULES
        .get_or_init(|| {
            serde_json::from_str(include_str!("config/tray_activation_rules.json"))
                .expect("tray activation rules config must be valid json")
        })
        .as_slice()
}

fn resolve_launch_target_path(program: &str) -> PathBuf {
    let program_path = Path::new(program);

    if program.to_lowercase().ends_with(".lnk") && program_path.exists() {
        if let Ok((target_path, _)) = WindowsApi::resolve_lnk_target(program_path) {
            return target_path;
        }
    }

    PathBuf::from(program)
}

fn normalized_process_match_key(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(name)
        .trim()
        .to_ascii_lowercase()
}

fn matches_process_name_owned(process_name: &str, expected_names: &[String]) -> bool {
    let process_name = normalized_process_match_key(process_name);
    expected_names
        .iter()
        .any(|name| process_name == normalized_process_match_key(name))
}

fn tray_activation_rule(program: &str) -> Option<&'static TrayActivationRule> {
    let target_path = resolve_launch_target_path(program);
    let exe_stem = normalized_process_match_key(&target_path.to_string_lossy());

    tray_activation_rules().iter().find(|rule| {
        rule.process_names
            .iter()
            .any(|name| normalized_process_match_key(name) == exe_stem)
    })
}

fn tray_activation_window_process_name(window: &Window) -> Option<String> {
    if let Ok(name) = window.process().program_exe_name() {
        return Some(name);
    }

    if let Ok(name) = window.process().exe_name_by_snapshot() {
        return Some(name);
    }

    None
}

fn find_visible_window_for_processes(process_names: &[String]) -> Option<Window> {
    let mut matched_window = None;

    let _ = WindowEnumerator::new().for_each(|window| {
        if matched_window.is_some() || !window.is_window() || !window.is_visible() {
            return;
        }

        let Some(process_name) = tray_activation_window_process_name(&window) else {
            return;
        };

        if matches_process_name_owned(&process_name, process_names) {
            matched_window = Some(window);
        }
    });

    matched_window
}

fn is_any_target_process_running(process_names: &[String]) -> bool {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes();

    sys.processes().values().any(|process| {
        process
            .exe()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .is_some_and(|name| matches_process_name_owned(name, process_names))
    })
}

fn hotkey_modifier_token(modifier: &str) -> Option<&'static str> {
    match modifier.to_ascii_lowercase().as_str() {
        "control" | "ctrl" => Some("{ctrl}"),
        "alt" => Some("{alt}"),
        "shift" => Some("{shift}"),
        "windows" | "win" | "meta" => Some("{win}"),
        _ => None,
    }
}

fn format_hotkey_for_log(hotkey: &TrayActivationHotkey) -> String {
    let mut parts: Vec<String> = hotkey
        .modifiers
        .iter()
        .map(|modifier| modifier.trim().to_string())
        .filter(|modifier| !modifier.is_empty())
        .collect();

    let key = hotkey.key.trim();
    if !key.is_empty() {
        parts.push(key.to_uppercase());
    }

    if parts.is_empty() {
        String::from("<empty hotkey>")
    } else {
        parts.join("+")
    }
}

fn send_tray_activation_hotkey(hotkey: &TrayActivationHotkey) -> Result<()> {
    let mut keyboard = Keyboard::new();

    let modifiers = hotkey
        .modifiers
        .iter()
        .filter_map(|modifier| hotkey_modifier_token(modifier))
        .collect::<String>();

    if !modifiers.is_empty() {
        keyboard.begin_hold_keys(&modifiers)?;
    }

    keyboard.send_keys(&hotkey.key.to_ascii_lowercase())?;

    if !modifiers.is_empty() {
        keyboard.end_hold_keys()?;
    }

    Ok(())
}

fn try_activate_existing_tray_aware_instance(program: &str) -> Result<bool> {
    let Some(rule) = tray_activation_rule(program) else {
        return Ok(false);
    };

    if let Some(window) = find_visible_window_for_processes(&rule.process_names) {
        log::info!(
            "[Run] tray-aware app already has a visible window, focusing it: program={program:?}, hwnd={}",
            window.address()
        );
        window.focus()?;
        return Ok(true);
    }

    if !is_any_target_process_running(&rule.process_names) {
        return Ok(false);
    }

    log::info!(
        "[Run] tray-aware app is already running without a visible window, trying {}: program={program:?}, rule={:?}",
        format_hotkey_for_log(&rule.hotkey),
        rule.name.as_deref().unwrap_or("unnamed")
    );
    send_tray_activation_hotkey(&rule.hotkey)?;

    let wait_ms = rule
        .wait_for_window_ms
        .unwrap_or(TRAY_APP_ACTIVATION_WAIT_MS);
    let poll_attempts = (wait_ms / TRAY_APP_ACTIVATION_POLL_MS).max(1);
    for _ in 0..poll_attempts {
        std::thread::sleep(Duration::from_millis(TRAY_APP_ACTIVATION_POLL_MS));

        if let Some(window) = find_visible_window_for_processes(&rule.process_names) {
            log::info!(
                "[Run] tray-aware app restored a visible window after hotkey, focusing it: program={program:?}, hwnd={}",
                window.address()
            );
            window.focus()?;
            return Ok(true);
        }
    }

    if !rule.suppress_launch_when_process_exists {
        return Ok(false);
    }

    log::info!(
        "[Run] tray-aware app is already running but no visible window appeared after hotkey, suppressing normal launch: program={program:?}"
    );
    Ok(true)
}

#[tauri::command(async)]
async fn run(
    program: String,
    args: Option<RelaunchArguments>,
    working_dir: Option<PathBuf>,
) -> Result<()> {
    let args = match args {
        Some(args) => match args {
            RelaunchArguments::String(args) => args,
            RelaunchArguments::Array(args) => args.join(" ").trim().to_owned(),
        },
        None => String::new(),
    };

    let working_dir = default_exe_working_dir(&program, working_dir);

    log::info!("[Run] program={program:?}, args={args:?}, working_dir={working_dir:?}");

    if try_activate_existing_tray_aware_instance(&program)? {
        return Ok(());
    }

    service_backend_command(
        "run_program",
        serde_json::json!({
            "program": program,
            "args": args,
            "workingDir": working_dir.map(|path| path.to_string_lossy().to_string()),
        }),
    )
    .await?;

    Ok(())
}

#[tauri::command(async)]
async fn run_as_admin(program: PathBuf, args: Option<RelaunchArguments>) -> Result<()> {
    let args = match args {
        Some(args) => match args {
            RelaunchArguments::String(args) => args,
            RelaunchArguments::Array(args) => args.join(" ").trim().to_owned(),
        },
        None => String::new(),
    };
    log::trace!("Running as admin: {program:?} {args}");

    service_backend_command(
        "system_run_as_admin",
        serde_json::json!({
            "program": program.to_string_lossy(),
            "args": args,
        }),
    )
    .await?;
    Ok(())
}

#[tauri::command(async)]
fn is_dev_mode() -> bool {
    tauri::is_dev()
}

#[tauri::command(async)]
async fn is_appx_package() -> bool {
    service_backend_bool_command("is_appx_package", false)
        .await
        .unwrap_or(false)
}

#[tauri::command(async)]
pub fn check_taskbar_overlap_status(webview: WebviewWindow<tauri::Wry>) -> bool {
    use crate::app::APP_MANAGER;

    let caller_label = webview.label().to_string();
    let taskbars: Vec<_> = {
        let manager = crate::trace_read!(APP_MANAGER);
        manager
            .instances
            .iter()
            .map(|instance| instance.taskbar.clone())
            .collect()
    };

    for taskbar_arc in taskbars {
        let taskbar = taskbar_arc.lock();
        if let Some(tb) = taskbar.as_ref() {
            if tb.window.label() == caller_label {
                let overlapped = tb.overlaped_by.is_some();
                log::info!(
                    "[check_taskbar_overlap_status] queried overlap state: overlapped={}, label={}",
                    overlapped,
                    caller_label
                );
                return overlapped;
            }
        }
    }

    log::warn!(
        "[check_taskbar_overlap_status] no matching taskbar for caller label={}",
        caller_label
    );
    false
}

#[tauri::command(async)]
pub fn get_user_envs() -> HashMap<String, String> {
    std::env::vars().collect::<HashMap<String, String>>()
}

#[tauri::command(async)]
fn send_keys(keys: String) -> Result<()> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};

    // 保存当前焦点，以便在发送完按键后恢复
    let prev_focus = unsafe { GetForegroundWindow() };
    log::debug!(
        "send_keys: saving focus before sending keys, prev_focus={:?}",
        prev_focus
    );

    let result = Keyboard::new().send_keys(&keys);
    log::debug!("send_keys: keys sent, result={:?}", result);

    // 恢复焦点
    if !prev_focus.is_invalid() {
        // 等待一段时间让应用体验充分引起的事件
        std::thread::sleep(std::time::Duration::from_millis(50));
        log::debug!("send_keys: restoring focus to prev_focus={:?}", prev_focus);
        unsafe {
            let success = SetForegroundWindow(prev_focus);
            log::debug!(
                "send_keys: SetForegroundWindow result={}",
                success.as_bool()
            );
        }
    } else {
        log::debug!("send_keys: prev_focus is invalid, skipping restore");
    }

    result
}

// used to request icon extraction
#[tauri::command(async)]
fn get_icon(path: Option<PathBuf>, umid: Option<String>) -> Result<()> {
    let current_settings = FULL_STATE.load();
    let use_local =
        current_settings.settings.taskbar.icon_backplate_style == IconBackplateStyle::Transparent;
    if let Some(umid) = umid {
        extract_and_save_icon_umid(&umid.into(), use_local);
    }
    if let Some(path) = path {
        if !path.as_os_str().is_empty() {
            extract_and_save_icon_from_file(&path, use_local);
        }
    }
    Ok(())
}

#[tauri::command(async)]
async fn get_local_icon(process_name: String) -> Result<Option<String>> {
    local_icon_from_service(process_name, false).await
}

#[tauri::command(async)]
async fn get_local_icon_white(process_name: String) -> Result<Option<String>> {
    local_icon_from_service(process_name, true).await
}

async fn local_icon_from_service(process_name: String, white: bool) -> Result<Option<String>> {
    let data = service_backend_command(
        "icon_get_local_icon",
        serde_json::json!({
            "processName": process_name,
            "white": white,
        }),
    )
    .await?;

    Ok(data.and_then(|value| {
        value
            .get("value")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned)
    }))
}

/// 使用备用方法提取图标（当常规方法失败时）
/// 使用 ExtractIconW 直接从可执行文件提取图标
#[tauri::command(async)]
pub fn icon_extract_with_fallback(
    path: Option<String>,
    umid: Option<String>,
) -> crate::error::Result<()> {
    use crate::trace_lock;
    use std::path::PathBuf;
    use windows::Win32::UI::Shell::ExtractIconW;
    use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;

    log::info!(
        "[IconFallback] Starting fallback extraction for path: {:?}, umid: {:?}",
        path,
        umid
    );

    // 确定要提取图标的路径
    let target_path = if let Some(p) = &path {
        PathBuf::from(p)
    } else if let Some(u) = &umid {
        // 尝试从 UMID 获取应用路径
        let app_umid = crate::windows_api::types::AppUserModelId::from(u.clone());
        match &app_umid {
            crate::windows_api::types::AppUserModelId::Appx(appx_umid) => {
                crate::modules::uwp::UwpManager::get_app_path(appx_umid)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| PathBuf::from("C:\\Windows\\explorer.exe"))
            }
            crate::windows_api::types::AppUserModelId::PropertyStore(ps_umid) => {
                let start_menu = crate::modules::start::application::START_MENU_MANAGER.load();
                start_menu
                    .get_by_file_umid(ps_umid)
                    .map(|lnk| lnk.path.clone())
                    .unwrap_or_else(|| PathBuf::from("C:\\Windows\\explorer.exe"))
            }
        }
    } else {
        return Err("No path or umid provided for icon extraction".into());
    };

    // 使用 ExtractIconW 提取图标
    let path_wide: Vec<u16> = target_path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let hicon = ExtractIconW(
            None, // 不需要窗口句柄
            windows::core::PCWSTR(path_wide.as_ptr()),
            0, // 提取第一个图标
        );

        if hicon.is_invalid() || hicon.0.is_null() {
            log::error!("[IconFallback] ExtractIconW failed for {:?}", target_path);
            return Err(format!("ExtractIconW failed for {:?}", target_path).into());
        }

        // 转换图标为 PNG
        let result = crate::utils::icon_extractor::convert_hicon_to_rgba_image(&hicon);
        let _ = DestroyIcon(hicon);

        match result {
            Ok(rgba_image) => {
                // 保存图标
                let root = VAR_COMMON.user_icons_path().join("system");
                let icon_manager_mutex = FULL_STATE.load().icon_packs().clone();

                // 生成文件名
                let filename = if let Some(u) = &umid {
                    format!("{}.png", crate::utils::umid_based_hash_id(u))
                } else if let Some(p) = &path {
                    format!(
                        "{}.png",
                        crate::utils::path_based_hash_id(std::path::Path::new(p))
                    )
                } else {
                    "fallback_icon.png".to_string()
                };

                rgba_image.save(root.join(&filename))?;

                // 更新图标管理器
                let mut gen_icon = libs_core::state::Icon::default();
                gen_icon.base = Some(filename);
                gen_icon.is_aproximately_square = false;

                let mut icon_manager = trace_lock!(icon_manager_mutex);
                icon_manager.add_system_app_icon(umid.as_deref(), Some(&target_path), gen_icon);
                icon_manager.write_system_icon_pack()?;
                drop(icon_manager);

                // 通知前端更新
                let _ = FULL_STATE.load().emit_icon_packs();

                log::info!(
                    "[IconFallback] Successfully extracted icon using fallback method for {:?}",
                    target_path
                );
            }
            Err(e) => {
                log::error!("[IconFallback] Failed to convert icon: {:?}", e);
                return Err(format!("Failed to convert icon: {:?}", e).into());
            }
        }
    }

    Ok(())
}

#[tauri::command(async)]
fn simulate_fullscreen(webview: WebviewWindow<tauri::Wry>, value: bool) -> Result<()> {
    let window = Window::from(webview.hwnd()?.0 as isize);
    let event = match value {
        true => WinEvent::SyntheticFullscreenStart,
        false => WinEvent::SyntheticFullscreenEnd,
    };
    HookManager::event_tx().send((event, window))?;
    Ok(())
}

#[tauri::command(async)]
fn get_foreground_window_color(webview: WebviewWindow<tauri::Wry>) -> Result<Color> {
    let webview = Window::from(webview.hwnd()?.0 as isize);
    let foreground = Window::get_foregrounded();

    if webview.monitor() != foreground.monitor() {
        return Ok(Color::default());
    }

    if !foreground.is_visible() || foreground.is_desktop() {
        return Ok(Color::default());
    }

    let hdc = DeviceContext::create(None);
    let rect = foreground.inner_rect()?;
    let x = rect.left + (rect.right - rect.left) / 2;
    Ok(hdc.get_pixel(x, rect.top + 2))
}

#[tauri::command(async)]
async fn translate_text(
    source: String,
    source_lang: String,
    mut target_lang: String,
) -> Result<String> {
    use translators::GoogleTranslator;
    let translator = GoogleTranslator::default();

    if target_lang == "zh" {
        target_lang = "zh-CN".to_string();
    }

    let translated = translator
        .translate_async(&source, &source_lang, &target_lang)
        .await?;
    Ok(translated)
}

pub fn register_invoke_handler(app_builder: Builder<Wry>) -> Builder<Wry> {
    use crate::state::infrastructure::*;
    use crate::widgets::taskbar::handler::*;

    use crate::modules::apps::infrastructure::*;
    use crate::modules::bluetooth::*;
    use crate::modules::language::*;
    use crate::modules::monitors::infrastructure::*;
    use crate::modules::network::infrastructure::*;
    use crate::modules::system_settings::infrastructure::*;
    use crate::modules::system_tray::infrastructure::*;
    use crate::resources::commands::*;

    app_builder.invoke_handler(command_handler_list!())
}

// Bridge commands not captured by macro generation in some builds
#[tauri::command(async)]
pub async fn get_text_scale_factor() -> crate::error::Result<f64> {
    let data = service_backend_command("get_text_scale_factor", serde_json::json!({})).await?;
    Ok(data
        .and_then(|value| value.get("value").and_then(|value| value.as_f64()))
        .unwrap_or(1.0))
}

async fn service_backend_command(
    command: &str,
    args: serde_json::Value,
) -> crate::error::Result<Option<serde_json::Value>> {
    let data = ServicePipe::request_with_response(SvcAction::ExecuteBackendCommand {
        command: command.to_string(),
        args,
    })
    .await?;

    match data {
        Some(data) if !data.trim().is_empty() => Ok(Some(serde_json::from_str(&data)?)),
        _ => Ok(None),
    }
}

async fn service_backend_bool_command(command: &str, default: bool) -> crate::error::Result<bool> {
    let data = service_backend_command(command, serde_json::json!({})).await?;
    Ok(data
        .and_then(|value| value.get("value").and_then(|value| value.as_bool()))
        .unwrap_or(default))
}

async fn service_backend_u32_command(command: &str, default: u32) -> crate::error::Result<u32> {
    let data = service_backend_command(command, serde_json::json!({})).await?;
    Ok(data
        .and_then(|value| value.get("value").and_then(|value| value.as_u64()))
        .map(|value| value.min(u32::MAX as u64) as u32)
        .unwrap_or(default))
}

async fn service_backend_string_command(
    command: &str,
    default: &str,
) -> crate::error::Result<String> {
    let data = service_backend_command(command, serde_json::json!({})).await?;
    Ok(data
        .and_then(|value| {
            value
                .get("value")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| default.to_string()))
}

#[tauri::command(async)]
pub async fn system_open_wifi_settings() -> crate::error::Result<()> {
    service_backend_command("system_open_wifi_settings", serde_json::json!({})).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn system_open_location_settings() -> crate::error::Result<()> {
    service_backend_command("system_open_location_settings", serde_json::json!({})).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn system_open_wlan_flyout() -> crate::error::Result<()> {
    service_backend_command("system_open_wlan_flyout", serde_json::json!({})).await?;
    Ok(())
}

// Removed explicit bridge for power settings to avoid duplicate command names.

// Audio bridges
#[tauri::command(async)]
pub async fn system_get_master_volume() -> crate::error::Result<u8> {
    let data = service_backend_command("system_get_master_volume", serde_json::json!({})).await?;
    Ok(data
        .and_then(|value| {
            value
                .get("value")
                .and_then(|value| value.as_u64())
                .map(|v| v as u8)
        })
        .unwrap_or(50))
}

#[tauri::command(async)]
pub async fn system_set_master_volume(volume: u8) -> crate::error::Result<()> {
    service_backend_command(
        "system_set_master_volume",
        serde_json::json!({ "volume": volume }),
    )
    .await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn system_get_master_muted() -> crate::error::Result<bool> {
    let data = service_backend_command("system_get_master_muted", serde_json::json!({})).await?;
    Ok(data
        .and_then(|value| value.get("value").and_then(|value| value.as_bool()))
        .unwrap_or(false))
}

#[tauri::command(async)]
pub async fn system_set_master_muted(muted: bool) -> crate::error::Result<()> {
    service_backend_command(
        "system_set_master_muted",
        serde_json::json!({ "muted": muted }),
    )
    .await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn system_open_volume_mixer() -> crate::error::Result<()> {
    service_backend_command("system_open_volume_mixer", serde_json::json!({})).await?;
    Ok(())
}

pub fn register_events_after_ready() {
    // Called on startup to begin emitting volume change events
    crate::modules::audio::infrastructure::register_volume_events();
    // 使用本地总线转发（取消跨进程长连接订阅）
    start_local_bus_forwarder();
    // On UI init, send per-scene registration messages (enter/exit) once
    start_initial_scene_registration();
}

// --- 在进程退出时向感知发送解注册 ---
pub fn unregister_scene_on_exit() {
    let regs = get_or_load_scene_regs();
    if regs.is_empty() {
        log::info!("[Exit-SceneUnreg] No scenes to unregister");
        return;
    }
    tauri::async_runtime::spawn(async move {
        let notifications: Vec<_> = regs
            .iter()
            .flat_map(|r| [r.enter_message.as_ref(), r.exit_message.as_ref()])
            .flatten()
            .map(|msg| {
                serde_json::json!({
                    "port_key_name": "AIBarGrpcServicePort",
                    "process_name": "magictaskbar-ui.exe",
                    "notify": msg,
                    "filter": "",
                })
            })
            .collect();
        let count = notifications.len();
        let payload = serde_json::json!({ "notifications": notifications });
        match crate::grpc_bridge::unregister_scenes(payload).await {
            Ok(_) => log::info!("[Exit-SceneUnreg] Sent {} UnregisterMsg calls", count),
            Err(e) => log::warn!("[Exit-SceneUnreg] grpc_bridge unregister failed: {}", e),
        }
    });
}

#[tauri::command(async)]
#[allow(non_snake_case)]
pub async fn ai_recommend_icon_clicked(btnId: String, windowTitle: Option<String>) -> Result<()> {
    log::info!(
        "[UI->YOYO] ai_recommend_icon_clicked invoked with btnId={}, windowTitle={:?}",
        btnId,
        windowTitle
    );

    let window_info = get_focused_window_hwnd();
    let mut extra_data = String::new();
    let need_awareness = matches!(
        btnId.as_str(),
        "SongSimilarRecommend"
            | "SongInterpretation"
            | "VideoSimilarRecommend"
            | "VideoInterpretation"
    );
    if need_awareness {
        if let Some(scene_data) = get_music_data().await {
            extra_data = scene_data;
            log::info!("[AIRecommend] scene data from awareness: {}", extra_data);
        }
    }

    if extra_data.is_empty() && matches!(get_current_scene_type(), Some(1)) {
        if let Some(pd) = get_last_process_data() {
            extra_data = pd;
        }
    }

    let norm_key = btnId.trim().to_lowercase();
    let mapped_btn_id = btnId.clone();
    let mapped_window_title = windowTitle.clone().unwrap_or_else(|| btnId.clone());
    let mapping_hit = windowTitle.is_some();
    let payload = serde_json::json!({
        "btn_id": mapped_btn_id,
        "window_title": mapped_window_title,
        "window_pin_mode": false,
        "window_pos_x": 14,
        "window_pos_y": 40,
        "screenshot_data": "",
        "data": extra_data,
        "focus_window_hwnd": window_info,
    });
    log::info!(
        "[UI->YOYO] sending through grpc_bridge: btn_id='{}', window_title='{}', scene_type_current={:?}, process_data='{}', window_pin_mode=false, window_pos=(14, 40), screenshot_len=0, mapping_hit={} (key='{}')",
        payload.get("btn_id").and_then(|v| v.as_str()).unwrap_or_default(),
        payload.get("window_title").and_then(|v| v.as_str()).unwrap_or_default(),
        get_current_scene_type(),
        payload.get("data").and_then(|v| v.as_str()).unwrap_or_default(),
        mapping_hit,
        norm_key
    );
    match crate::grpc_bridge::send_yoyo_scene(payload).await {
        Ok(resp) => log::info!("[UI->YOYO] StartYOYOSceneTask ok: {}", resp),
        Err(e) => log::warn!("[UI->YOYO] grpc_bridge send_yoyo_scene failed: {}", e),
    }
    Ok(())
}

pub async fn get_music_data() -> Option<String> {
    match crate::grpc_bridge::get_music_data(serde_json::json!({})).await {
        Ok(value) => value
            .get("scene_data")
            .or_else(|| value.get("sceneData"))
            .and_then(|v| v.as_str())
            .map(|v| v.to_string()),
        Err(e) => {
            log::warn!("grpc_bridge get_music_data failed: {}", e);
            None
        }
    }
}

// --- AIRecommend registration on startup ---
#[derive(Debug, Clone)]
struct SceneRegistration {
    scene_type: String,
    enter_message: Option<String>,
    exit_message: Option<String>,
}

// Cache parsed registrations for matching incoming scene_notify events
static SCENE_REGS: std::sync::OnceLock<Vec<SceneRegistration>> = std::sync::OnceLock::new();

fn get_or_load_scene_regs() -> Vec<SceneRegistration> {
    if let Some(v) = SCENE_REGS.get() {
        return v.clone();
    }
    if let Some(xml) = read_ai_recommend_xml() {
        let regs = parse_scene_registrations(&xml);
        let _ = SCENE_REGS.set(regs.clone());
        regs
    } else {
        Vec::new()
    }
}

fn read_ai_recommend_xml() -> Option<String> {
    use std::fs;
    use std::path::PathBuf;
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("static/AIRecommend.xml"));
            candidates.push(dir.join("AIRecommend.xml"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("static/AIRecommend.xml"));
    }
    candidates.push(PathBuf::from("static/AIRecommend.xml"));
    candidates.push(PathBuf::from("src/static/AIRecommend.xml"));
    for p in candidates.iter() {
        if let Ok(txt) = fs::read_to_string(p) {
            log::info!("[AIRecommend.xml] Loaded from {}", p.display());
            return Some(txt);
        }
    }
    let searched = candidates
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(" ; ");
    log::warn!("[AIRecommend.xml] Not found. Searched paths: {}", searched);
    None
}

// --- Cache for last process_data and current scene type ---
static LAST_PROCESS_DATA: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static CURRENT_SCENE_TYPE: OnceLock<Mutex<Option<i32>>> = OnceLock::new();

// --- Cache for ContextMenu state ---
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextMenuState {
    item_id: Option<String>,
    display_name: Option<String>,
}

static CONTEXT_MENU_STATE: OnceLock<Mutex<ContextMenuState>> = OnceLock::new();

fn set_context_menu_state(item_id: Option<String>, display_name: Option<String>) {
    let cell = CONTEXT_MENU_STATE.get_or_init(|| {
        Mutex::new(ContextMenuState {
            item_id: None,
            display_name: None,
        })
    });
    if let Ok(mut guard) = cell.lock() {
        guard.item_id = item_id;
        guard.display_name = display_name;
    }
}

fn get_context_menu_state() -> ContextMenuState {
    let cell = CONTEXT_MENU_STATE.get_or_init(|| {
        Mutex::new(ContextMenuState {
            item_id: None,
            display_name: None,
        })
    });
    cell.lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or(ContextMenuState {
            item_id: None,
            display_name: None,
        })
}
static FOCUSED_WINDOW_HWND: OnceLock<Mutex<Option<String>>> = OnceLock::new(); // 现在存储窗口位置 "x,y,w,h"
static SCENE_PROCESS_NAME: OnceLock<Mutex<Option<String>>> = OnceLock::new(); // 存储场景进程名
static SCENE_NOTIFY: OnceLock<Mutex<Option<String>>> = OnceLock::new(); // 存储场景通知
static AI_FUNCTION_NAMES: OnceLock<Mutex<Option<String>>> = OnceLock::new(); // 存储当前展示的aiFunction names

fn set_last_process_data(data: &str) {
    let cell = LAST_PROCESS_DATA.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = cell.lock() {
        *guard = Some(data.to_string());
    }
}

fn get_last_process_data() -> Option<String> {
    let cell = LAST_PROCESS_DATA.get_or_init(|| Mutex::new(None));
    cell.lock().ok().and_then(|g| g.clone())
}

fn set_current_scene_type(scene: i32) {
    let cell = CURRENT_SCENE_TYPE.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = cell.lock() {
        *guard = Some(scene);
    }
}

fn clear_current_scene_type() {
    let cell = CURRENT_SCENE_TYPE.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = cell.lock() {
        *guard = None;
    }
}

fn get_current_scene_type() -> Option<i32> {
    let cell = CURRENT_SCENE_TYPE.get_or_init(|| Mutex::new(None));
    cell.lock().ok().and_then(|g| *g)
}

/// 保存前台窗口句柄（供 gRPC server 调用）
pub fn set_focused_window_hwnd(hwnd_str: String) {
    let cell = FOCUSED_WINDOW_HWND.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = cell.lock() {
        *guard = Some(hwnd_str);
    }
}

/// 获取前台窗口句柄
fn get_focused_window_hwnd() -> String {
    let cell = FOCUSED_WINDOW_HWND.get_or_init(|| Mutex::new(None));
    // 默认值格式："0,0,0,0,0"
    cell.lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(|| "0,0,0,0,0".to_string())
}

/// 保存场景进程名（供 gRPC server 调用）
pub fn set_scene_process_name(name: String) {
    let cell = SCENE_PROCESS_NAME.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = cell.lock() {
        *guard = Some(name);
    }
}

/// 获取场景进程名
fn get_scene_process_name() -> String {
    let cell = SCENE_PROCESS_NAME.get_or_init(|| Mutex::new(None));
    cell.lock().ok().and_then(|g| g.clone()).unwrap_or_default()
}

/// 保存场景通知
pub fn set_scene_notify(notify: String) {
    let cell = SCENE_NOTIFY.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = cell.lock() {
        *guard = Some(notify);
    }
}

/// 获取场景通知
fn get_scene_notify() -> String {
    let cell = SCENE_NOTIFY.get_or_init(|| Mutex::new(None));
    cell.lock().ok().and_then(|g| g.clone()).unwrap_or_default()
}

/// 保存当前展示的aiFunction names
pub fn set_ai_function_names(names: String) {
    let cell = AI_FUNCTION_NAMES.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = cell.lock() {
        *guard = Some(names);
    }
}

/// 获取当前展示的aiFunction names
fn get_ai_function_names() -> String {
    let cell = AI_FUNCTION_NAMES.get_or_init(|| Mutex::new(None));
    cell.lock().ok().and_then(|g| g.clone()).unwrap_or_default()
}

fn normalize_msg(s: &str) -> String {
    s.trim().to_lowercase()
}

fn update_scene_state_from_notify(notify: &str) {
    let n = normalize_msg(notify);
    let regs = get_or_load_scene_regs();
    for r in regs.iter() {
        if let Some(em) = &r.enter_message {
            if normalize_msg(em) == n {
                if let Ok(st) = r.scene_type.trim().parse::<i32>() {
                    set_current_scene_type(st);
                }
                return;
            }
        }
        if let Some(xm) = &r.exit_message {
            if normalize_msg(xm) == n {
                clear_current_scene_type();
                return;
            }
        }
    }
}

pub fn handle_scene_notify_value(v: &serde_json::Value) {
    let evt = v.get("event").and_then(|x| x.as_str()).unwrap_or("");
    if evt == "scene_set" || evt == "scene_next" {
        if let Some(scene) = v.get("scene_type").and_then(|x| x.as_i64()) {
            let st = scene as i32;
            set_current_scene_type(st);
            let _ = crate::app::get_app_handle().emit("ai-recommend:scene", st);
        }
        return;
    }

    if evt != "scene_notify" {
        log::debug!("[SceneNotify] ignored event: {}", evt);
        return;
    }

    let Some(notify) = v.get("notify").and_then(|x| x.as_str()) else {
        return;
    };

    update_scene_state_from_notify(notify);
    if let Some(pd) = v.get("ProcessData").and_then(|x| x.as_str()) {
        set_last_process_data(pd);
        log::debug!("[SceneNotify] cached ProcessData ({} bytes)", pd.len());
    }

    {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect};

        let focused_hwnd = unsafe { GetForegroundWindow() };
        if focused_hwnd.0 != std::ptr::null_mut() {
            let mut rect = RECT::default();
            unsafe {
                if GetWindowRect(focused_hwnd, &mut rect).is_ok() {
                    let width = rect.right - rect.left;
                    let height = rect.bottom - rect.top;
                    let hwnd_value = focused_hwnd.0 as usize;
                    set_focused_window_hwnd(format!(
                        "{},{},{},{},{}",
                        hwnd_value, rect.left, rect.top, width, height
                    ));
                }
            }
        }
    }

    if let Some(scene_proc) = v.get("SceneProcessName").and_then(|x| x.as_str()) {
        set_scene_process_name(scene_proc.to_string());
    }
    set_scene_notify(notify.to_string());
}

fn parse_scene_registrations(xml: &str) -> Vec<SceneRegistration> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);

    let mut buf = Vec::new();
    let mut regs: Vec<SceneRegistration> = Vec::new();

    // Minimal state machine to capture <Scene ...> and its child enterMessage/exitMessage
    let mut in_scene = false;
    let mut current_scene_type: Option<String> = None;
    let mut current_enter: Option<String> = None;
    let mut current_exit: Option<String> = None;
    let mut current_child: Option<String> = None; // "enterMessage" or "exitMessage"

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = e.name().as_ref().to_vec();
                if name.as_slice() == b"Scene" {
                    in_scene = true;
                    current_scene_type = None;
                    current_enter = None;
                    current_exit = None;
                    // Read attributes
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"sceneType" {
                            if let Ok(v) = attr.unescape_value() {
                                current_scene_type = Some(v.to_string());
                            }
                        } else if attr.key.as_ref() == b"enterMessage" {
                            if let Ok(v) = attr.unescape_value() {
                                let t = v.to_string();
                                let t = t.trim().to_string();
                                if !t.is_empty() {
                                    current_enter = Some(t);
                                }
                            }
                        } else if attr.key.as_ref() == b"exitMessage" {
                            if let Ok(v) = attr.unescape_value() {
                                let t = v.to_string();
                                let t = t.trim().to_string();
                                if !t.is_empty() {
                                    current_exit = Some(t);
                                }
                            }
                        }
                    }
                } else if in_scene
                    && (name.as_slice() == b"enterMessage" || name.as_slice() == b"exitMessage")
                {
                    current_child = Some(String::from_utf8_lossy(&name).to_string());
                }
            }
            Ok(Event::Text(e)) => {
                if in_scene {
                    if let Some(child) = &current_child {
                        let t = e.unescape().unwrap_or_default().to_string();
                        let t = t.trim().to_string();
                        if !t.is_empty() {
                            if child == "enterMessage" {
                                current_enter = Some(t);
                            } else if child == "exitMessage" {
                                current_exit = Some(t);
                            }
                        }
                    }
                }
            }
            Ok(Event::End(e)) => {
                let name = e.name().as_ref().to_vec();
                if in_scene
                    && (name.as_slice() == b"enterMessage" || name.as_slice() == b"exitMessage")
                {
                    current_child = None;
                }
                if name.as_slice() == b"Scene" {
                    // finalize
                    if let Some(st) = current_scene_type.clone() {
                        regs.push(SceneRegistration {
                            scene_type: st,
                            enter_message: current_enter.clone(),
                            exit_message: current_exit.clone(),
                        });
                    }
                    in_scene = false;
                    current_scene_type = None;
                    current_enter = None;
                    current_exit = None;
                    current_child = None;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    regs
}

fn start_initial_scene_registration() {
    use std::sync::OnceLock;
    static INIT_SENT: OnceLock<()> = OnceLock::new();
    if INIT_SENT.set(()).is_err() {
        return;
    }

    if let Some(xml) = read_ai_recommend_xml() {
        let regs = parse_scene_registrations(&xml);
        let _ = SCENE_REGS.set(regs.clone());
        if regs.is_empty() {
            log::info!("[Init-SceneReg] No scenes found for registration");
            return;
        }
        tokio::spawn(async move {
            let registrations: Vec<_> = regs
                .iter()
                .flat_map(|r| {
                    [
                        r.enter_message.as_ref().map(|msg| {
                            serde_json::json!({
                                "scene_type": r.scene_type,
                                "port_key_name": "AIBarGrpcServicePort",
                                "process_name": "magictaskbar-ui.exe",
                                "notify": msg,
                                "filter": "",
                            })
                        }),
                        r.exit_message.as_ref().map(|msg| {
                            serde_json::json!({
                                "scene_type": r.scene_type,
                                "port_key_name": "AIBarGrpcServicePort",
                                "process_name": "magictaskbar-ui.exe",
                                "notify": msg,
                                "filter": "",
                            })
                        }),
                    ]
                })
                .flatten()
                .collect();
            let count = registrations.len();
            let payload = serde_json::json!({ "registrations": registrations });
            match crate::grpc_bridge::register_scenes(payload).await {
                Ok(_) => log::info!("[Init-SceneReg] Sent {} scene RegisterMsg calls", count),
                Err(e) => log::warn!("[Init-SceneReg] grpc_bridge register failed: {}", e),
            }
        });
    } else {
        log::warn!("[Init-SceneReg] AIRecommend.xml not found in candidate paths");
    }
}

#[tauri::command(async)]
pub async fn control_center_post_tray_click() -> Result<()> {
    service_backend_command("control_center_post_tray_click", serde_json::json!({})).await?;
    Ok(())
}

// 检查控制中心窗口是否可见
#[tauri::command(async)]
pub async fn control_center_is_visible() -> bool {
    service_backend_bool_command("control_center_is_visible", false)
        .await
        .unwrap_or(false)
}

// 检查日历窗口是否可见
#[tauri::command(async)]
pub async fn calendar_is_visible() -> bool {
    service_backend_bool_command("calendar_is_visible", false)
        .await
        .unwrap_or(false)
}

// 检查消息中心窗口是否可见
#[tauri::command(async)]
pub async fn message_center_is_visible() -> bool {
    service_backend_bool_command("message_center_is_visible", false)
        .await
        .unwrap_or(false)
}
// 根据窗口标题聚焦控制中心窗口（标题写死为“控制中心”）
#[tauri::command(async)]
pub async fn control_center_focus_by_title() -> Result<()> {
    service_backend_command("control_center_focus_by_title", serde_json::json!({})).await?;
    Ok(())
}

// 电源管理命令
#[tauri::command(async)]
pub async fn system_lock_screen() -> Result<()> {
    service_backend_command("system_lock_screen", serde_json::json!({})).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn yoyo_launch_assistant() -> Result<()> {
    service_backend_command("yoyo_launch_assistant", serde_json::json!({})).await?;
    Ok(())
}

#[tauri::command(async)]
#[allow(non_snake_case)]
pub async fn screenshot_launch(cmdLine: Option<String>) -> Result<()> {
    service_backend_command(
        "screenshot_launch",
        serde_json::json!({ "cmdLine": cmdLine.unwrap_or_else(|| "\\p".to_string()) }),
    )
    .await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn system_sleep() -> Result<()> {
    service_backend_command("system_sleep", serde_json::json!({})).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn system_hibernate() -> Result<()> {
    service_backend_command("system_hibernate", serde_json::json!({})).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn system_shutdown() -> Result<()> {
    service_backend_command("system_shutdown", serde_json::json!({})).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn system_restart() -> Result<()> {
    service_backend_command("system_restart", serde_json::json!({})).await?;
    Ok(())
}

// gRPC events are forwarded to Webview by grpc_bridge.dll callback.
fn start_local_bus_forwarder() {
    log::info!("[LocalBus] grpc events are forwarded by grpc_bridge.dll callback");
}

/// Wake the discrete GPU asynchronously.
/// Called from the UI when the HONOR power-menu button is clicked,
/// so the GPU is ready before any subsequent operation (exit, sleep, etc.).
#[tauri::command(async)]
pub fn gpu_wake_async() -> Result<()> {
    let _ = ServicePipe::request(SvcAction::ExecuteBackendCommand {
        command: "gpu_wake".to_string(),
        args: serde_json::json!({}),
    });
    Ok(())
}

#[tauri::command(async)]
pub async fn system_exit_to_desktop() -> Result<()> {
    service_backend_command("system_exit_to_desktop", serde_json::json!({})).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn system_check_update() -> Result<String> {
    service_backend_string_command("system_check_update", "").await
}

#[tauri::command(async)]
pub async fn system_send_check_update_to_magicvisuals() -> Result<()> {
    service_backend_command(
        "system_send_check_update_to_magicvisuals",
        serde_json::json!({}),
    )
    .await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn system_send_download_update_to_magicvisuals() -> Result<()> {
    service_backend_command(
        "system_send_download_update_to_magicvisuals",
        serde_json::json!({}),
    )
    .await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn system_send_start_install_to_magicvisuals() -> Result<()> {
    service_backend_command(
        "system_send_start_install_to_magicvisuals",
        serde_json::json!({}),
    )
    .await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn system_send_login_account_to_magicvisuals() -> Result<()> {
    service_backend_command(
        "system_send_login_account_to_magicvisuals",
        serde_json::json!({}),
    )
    .await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn system_open_check_update_window(app_handle: AppHandle) -> Result<()> {
    let label = "check-update";

    // 使用 Manager trait 中的方法，确保 trait 在作用域内
    if let Some(window) = app_handle.get_webview_window(label) {
        let _ = window.set_focus();
        return Ok(());
    }

    let args = crate::widgets::WebviewArgs::new().disable_gpu();
    let _window = tauri::WebviewWindowBuilder::new(
        &app_handle,
        label,
        tauri::WebviewUrl::App("check-update/index.html".into()),
    )
    .title("检查更新")
    .inner_size(400.0, 420.0)
    .resizable(false)
    .minimizable(false)
    .maximizable(false)
    .decorations(false)
    .transparent(true)
    .visible(false) // 初始不可见，防止白屏闪烁
    .always_on_top(false)
    .center()
    .skip_taskbar(false)
    .devtools(false)
    .data_directory(args.data_directory())
    .additional_browser_args(&args.to_string())
    .build()?;

    Ok(())
}

#[tauri::command(async)]
pub async fn system_open_about_window(app_handle: AppHandle) -> Result<()> {
    let label = "about";

    log::info!("[About] Opening about window");

    // 使用 Manager trait 中的方法，确保 trait 在作用域内
    if let Some(window) = app_handle.get_webview_window(label) {
        log::info!("[About] Window already exists, setting focus");
        let _ = window.set_focus();
        return Ok(());
    }

    log::info!("[About] Creating new about window");
    let args = crate::widgets::WebviewArgs::new().disable_gpu();
    let _window = tauri::WebviewWindowBuilder::new(
        &app_handle,
        label,
        tauri::WebviewUrl::App("about/index.html".into()),
    )
    .title("关于")
    .inner_size(360.0, 340.0)
    .resizable(false)
    .minimizable(false)
    .maximizable(false)
    .decorations(false)
    .transparent(true)
    .visible(false) // 初始不可见，防止白屏闪烁
    .always_on_top(false)
    .center()
    .skip_taskbar(false)
    .devtools(false)
    .data_directory(args.data_directory())
    .additional_browser_args(&args.to_string())
    .build()?;

    log::info!("[About] About window created successfully");
    Ok(())
}

#[tauri::command(async)]
pub async fn system_open_feedback_window(app_handle: AppHandle) -> Result<()> {
    let label = "feedback";

    log::info!("[Feedback] Opening feedback window");

    // 使用 Manager trait 中的方法，确保 trait 在作用域内
    if let Some(window) = app_handle.get_webview_window(label) {
        log::info!("[Feedback] Window already exists, setting focus");
        let _ = window.set_focus();
        return Ok(());
    }

    log::info!("[Feedback] Creating new feedback window");
    let args = crate::widgets::WebviewArgs::new().disable_gpu();
    let _window = tauri::WebviewWindowBuilder::new(
        &app_handle,
        label,
        tauri::WebviewUrl::App("feedback/index.html".into()),
    )
    .title("意见反馈")
    .inner_size(700.0, 610.0)
    .resizable(false)
    .minimizable(false)
    .maximizable(false)
    .decorations(false)
    .transparent(true)
    .visible(false) // 初始不可见，防止白屏闪烁
    .always_on_top(false)
    .center()
    .skip_taskbar(false)
    .devtools(false)
    .data_directory(args.data_directory())
    .additional_browser_args(&args.to_string())
    .build()?;

    log::info!("[Feedback] Feedback window created successfully");
    Ok(())
}

#[tauri::command(async)]
pub async fn system_get_app_version() -> Result<String> {
    service_backend_string_command("system_get_app_version", "未知版本").await
}

#[tauri::command]
pub fn system_is_game_fullscreen_blocked() -> bool {
    crate::hook::is_game_fullscreen_blocked()
}

#[tauri::command(async)]
pub fn get_toolbar_window_handles() -> Result<Vec<isize>> {
    // 返回toolbar窗口的另一种方法是使用窗口类名查找（不太可靠）
    // 更稳定的方法是直接使用背景窗口的HWND
    use crate::windows_api::event_window::BACKGROUND_HWND;

    let hwnd = BACKGROUND_HWND.load(std::sync::atomic::Ordering::Relaxed);
    log::info!("[ToolbarWindowHandles] Background window HWND: {:?}", hwnd);

    Ok(vec![hwnd])
}

#[tauri::command(async)]
pub async fn system_recycle_files(paths: Vec<PathBuf>) -> Result<()> {
    service_backend_command(
        "system_recycle_files",
        serde_json::json!({ "paths": paths }),
    )
    .await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn system_empty_recycle_bin() -> Result<()> {
    service_backend_command("system_empty_recycle_bin", serde_json::json!({})).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn system_open_recycle_bin() -> Result<()> {
    service_backend_command("system_open_recycle_bin", serde_json::json!({})).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn system_is_recycle_bin_open() -> bool {
    service_backend_bool_command("system_is_recycle_bin_open", false)
        .await
        .unwrap_or(false)
}

#[tauri::command(async)]
pub fn system_is_recycle_bin_empty() -> bool {
    crate::hook::RECYCLE_BIN_EMPTY_CACHE.load(std::sync::atomic::Ordering::Relaxed)
}

#[tauri::command(async)]
pub async fn system_close_recycle_bin() -> Result<()> {
    service_backend_command("system_close_recycle_bin", serde_json::json!({})).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn system_get_recycle_bin_hwnd() -> i64 {
    service_backend_command("system_get_recycle_bin_hwnd", serde_json::json!({}))
        .await
        .ok()
        .flatten()
        .and_then(|value| value.get("value").and_then(|value| value.as_i64()))
        .unwrap_or(-1)
}

#[tauri::command]
pub fn open_url(url: String) {
    let _ = ServicePipe::request(SvcAction::ExecuteBackendCommand {
        command: "open_url".to_string(),
        args: serde_json::json!({ "url": url }),
    });
}

const REPORT_ID: &str = "669000008";
const TOOLBAR_MODE_REPORT_ID: &str = "669000009";
const TASKBAR_STATE_REPORT_ID: &str = "669000012";
const SETTINGS_REPORT_ID: &str = "669000022";
const FEEDBACK_REPORT_ID: &str = "954690001";
const SHORTCUT_OPERATION_REPORT_ID: &str = "669000036";
const AI_RECOMMEND_FUNCTION_REPORT_ID: &str = "669000034";

fn report_string(report_id: &str, content: &str) -> bool {
    log::debug!(
        "[Report] data bridge disabled; skip report id={} content={}",
        report_id,
        content
    );
    false
}

#[tauri::command(async)]
pub fn report_click_component(content: String) -> Result<()> {
    let json = serde_json::json!({ "ClickComponent": content }).to_string();
    report_string(REPORT_ID, &json);
    Ok(())
}

#[tauri::command(async)]
pub fn report_shortcut_operation(operation: String, tool_name: String) -> Result<()> {
    log::info!(
        "[Report] report_shortcut_operation called with operation: {}, tool_name: {}",
        operation,
        tool_name
    );
    let json = serde_json::json!({ "Operation": operation, "ToolName": tool_name }).to_string();
    let result = report_string(SHORTCUT_OPERATION_REPORT_ID, &json);
    log::info!("[Report] report_shortcut_operation result: {}", result);
    Ok(())
}

#[tauri::command(async)]
pub fn report_ai_recommend_function(content: String) -> Result<()> {
    log::info!(
        "[Report] report_ai_recommend_function called with content: {}",
        content
    );
    let json = serde_json::json!({ "Function": content }).to_string();
    let result = report_string(AI_RECOMMEND_FUNCTION_REPORT_ID, &json);
    log::info!("[Report] report_ai_recommend_function result: {}", result);
    Ok(())
}

#[tauri::command(async)]
pub fn report_settings_click(content: String) -> Result<()> {
    report_string(SETTINGS_REPORT_ID, &content);
    Ok(())
}

#[tauri::command(async)]
pub fn report_toolbar_mode(content: String) -> Result<()> {
    let json = serde_json::json!({ "ToolbarMode": content }).to_string();
    report_string(&TOOLBAR_MODE_REPORT_ID, &json);
    Ok(())
}

#[tauri::command(async)]
pub fn report_taskbar_state(content: String) -> Result<()> {
    report_string(TASKBAR_STATE_REPORT_ID, &content);
    Ok(())
}

/// 反馈上报接口（支持上传日志文件）
/// 参数说明：
/// - opinionType: 意见类型（problem/suggestion，可为空）
/// - feedbackTypes: 反馈类型（多个类型用逗号分隔）
/// - description: 详细描述
/// - contactInfo: 联系方式（格式：类型:值，如 "微信:abc123"）
/// - uploadLogs: 是否上传日志
#[tauri::command(async)]
pub fn report_feedback(
    opinion_type: String,
    feedback_types: String,
    description: String,
    contact_info: String,
    upload_logs: bool,
) -> Result<()> {
    log::info!(
        "[Feedback] report_feedback: opinion_type={}, types={}, description={}, contact={}, upload_logs={}",
        opinion_type,
        feedback_types,
        description,
        contact_info,
        upload_logs
    );

    // 在后台线程执行打点，立即返回
    std::thread::spawn(move || {
        // 检查今日提交次数是否已达上限
        if !crate::modules::feedback::check_and_increment_daily_count() {
            log::info!("[Feedback] 今日提交次数已达上限，跳过打点");
            return;
        }

        // 如果需要上传日志，先收集并打包日志
        let delete_path = if upload_logs {
            match crate::modules::feedback::collect_and_zip_logs() {
                Ok(zip_path) => {
                    log::info!("[Feedback] 日志打包成功: {}", zip_path);
                    Some(zip_path)
                }
                Err(e) => {
                    log::error!("[Feedback] 日志打包失败: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // 构建字符串数组：[意见类型, 反馈类型, 详细描述, 联系方式, 是否上传日志]
        let upload_logs_str = if upload_logs { "true" } else { "false" };
        let string_array: Vec<String> = vec![
            opinion_type,
            feedback_types,
            description,
            contact_info,
            upload_logs_str.to_string(),
        ];
        let string_array_ref: Vec<&str> = string_array.iter().map(|s| s.as_str()).collect();

        let path = delete_path.as_deref();
        let success = false;
        log::info!(
            "[Feedback] data bridge disabled; skip report id={} upload_path={:?} fields={}",
            FEEDBACK_REPORT_ID,
            path,
            string_array_ref.len()
        );

        if success {
            log::info!("[Feedback] report_feedback success");
        } else {
            log::warn!("[Feedback] report_feedback failed");
        }
    });

    Ok(())
}

// 发送快捷键消息到 HonorControlCenterWndTray 窗口
#[tauri::command(async)]
pub async fn send_shortcut_message(shortcut_id: String) -> Result<()> {
    service_backend_command(
        "send_shortcut_message",
        serde_json::json!({ "shortcutId": shortcut_id }),
    )
    .await?;
    Ok(())
}

// === Screen Recognition gRPC ===

#[tauri::command(async)]
pub fn ai_recommend_send_screen_recognition(
    window: tauri::Window,
    btn_id: String,
    ai_function_names: String,
) -> Result<()> {
    set_ai_function_names(ai_function_names.clone());

    let window_info = get_focused_window_hwnd();
    let payload = serde_json::json!({
        "notify": get_scene_notify(),
        "scene_process_name": get_scene_process_name(),
        "process_data": get_last_process_data().unwrap_or_default(),
        "red_area_func": get_ai_function_names(),
        "focus_window_hwnd": window_info,
    });

    log::info!(
        "[ScreenRecognition] Sending request through grpc_bridge, btnId={}",
        btn_id
    );
    tauri::async_runtime::spawn(async move {
        match crate::grpc_bridge::send_screen_recognition(payload).await {
            Ok(resp) => {
                let current_status = resp
                    .get("status")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let current_area = resp
                    .get("area_type")
                    .or_else(|| resp.get("areaType"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let current_sugg = resp
                    .get("green_area_func_sugg")
                    .or_else(|| resp.get("greenAreaFuncSugg"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();

                log::info!(
                    "[gRPC] Received response: status={}, areaType={}",
                    current_status,
                    current_area
                );

                if !current_sugg.is_empty() {
                    if let Err(e) = window.emit("ai-recommend:recognition-data", current_sugg) {
                        log::error!("[Tauri] Emit event failed: {}", e);
                    }
                    if current_status && current_area == "green" {
                        log::info!("[gRPC] AI recommendation success and sent.");
                    } else {
                        log::info!(
                            "[gRPC] Fallback recommendation sent (status={}, areaType={}).",
                            current_status,
                            current_area
                        );
                    }
                } else {
                    log::warn!(
                        "[GPRC] No recommendation available: status={}, areaType={}",
                        current_status,
                        current_area
                    );
                }
            }
            Err(e) => log::error!("[gRPC] Request failed: {:?}", e),
        }
    });

    Ok(())
}
// ==================== Preview 窗口命令 ====================
#[tauri::command(async)]
pub fn preview_trigger_show(payload: serde_json::Value, monitor_id: Option<String>) -> Result<()> {
    use crate::{app::APP_MANAGER, trace_lock, trace_read};

    let Some(monitor_id) = monitor_id else {
        log::warn!("[Preview] preview_trigger_show ignored: missing monitor_id");
        return Ok(());
    };

    let manager = trace_read!(APP_MANAGER);
    for instance in &manager.instances {
        if monitor_id.as_str() != instance.main_target_id.0.as_str() {
            continue;
        }
        let preview = trace_lock!(instance.preview);
        if let Some(pv) = preview.as_ref() {
            let label = pv.window.label();
            crate::widgets::preview::preview_manager_show(label, payload.clone())?;
        }
    }
    Ok(())
}

#[tauri::command(async)]
pub fn preview_set_position(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    monitor_id: Option<String>,
) -> Result<()> {
    use crate::{app::APP_MANAGER, trace_lock, trace_read};

    let Some(monitor_id) = monitor_id else {
        log::warn!("[Preview] preview_set_position ignored: missing monitor_id");
        return Ok(());
    };

    let manager = trace_read!(APP_MANAGER);
    for instance in &manager.instances {
        if monitor_id.as_str() != instance.main_target_id.0.as_str() {
            continue;
        }
        let mut preview = trace_lock!(instance.preview);
        if let Some(pv) = preview.as_mut() {
            pv.set_position(x, y, width, height)?;
        }
    }
    Ok(())
}

#[tauri::command(async)]
pub fn preview_show(monitor_id: Option<String>) -> Result<()> {
    use crate::{app::APP_MANAGER, trace_lock, trace_read};

    let Some(monitor_id) = monitor_id else {
        log::warn!("[Preview] preview_show ignored: missing monitor_id");
        return Ok(());
    };

    let manager = trace_read!(APP_MANAGER);
    for instance in &manager.instances {
        if monitor_id.as_str() != instance.main_target_id.0.as_str() {
            continue;
        }
        let mut preview = trace_lock!(instance.preview);
        if let Some(pv) = preview.as_mut() {
            pv.show()?;
        }
    }
    Ok(())
}

#[tauri::command(async)]
pub fn preview_hide(monitor_id: Option<String>) -> Result<()> {
    use crate::{app::APP_MANAGER, trace_lock, trace_read};

    let manager = trace_read!(APP_MANAGER);
    for instance in &manager.instances {
        if monitor_id
            .as_ref()
            .is_some_and(|id| id != &instance.main_target_id.0)
        {
            continue;
        }
        let mut preview = trace_lock!(instance.preview);
        if let Some(pv) = preview.as_mut() {
            pv.hide()?;
        }
    }
    Ok(())
}

#[tauri::command(async)]
pub fn preview_ready(monitor_id: Option<String>) -> Result<()> {
    use crate::{app::APP_MANAGER, trace_lock, trace_read};

    let manager = trace_read!(APP_MANAGER);
    for instance in &manager.instances {
        if monitor_id
            .as_ref()
            .is_some_and(|id| id != &instance.main_target_id.0)
        {
            continue;
        }
        let preview = trace_lock!(instance.preview);
        if let Some(pv) = preview.as_ref() {
            let label = pv.window.label();
            crate::widgets::preview::preview_manager_ready(label)?;
        }
    }
    Ok(())
}

// ==================== ContextMenu 窗口命令（懒创建 + 延迟销毁） ====================

#[tauri::command(async)]
pub fn contextmenu_trigger(payload: serde_json::Value) -> Result<()> {
    crate::widgets::contextmenu::contextmenu_manager_trigger(payload)
}

#[tauri::command(async)]
pub fn contextmenu_ready() -> Result<()> {
    crate::widgets::contextmenu::contextmenu_manager_ready()
}

#[tauri::command(async)]
pub fn contextmenu_destroy() -> Result<()> {
    crate::widgets::contextmenu::contextmenu_manager_destroy()
}

#[tauri::command(async)]
pub fn contextmenu_set_position(x: i32, y: i32, width: i32, height: i32) -> Result<()> {
    crate::widgets::contextmenu::contextmenu_manager_set_position(x, y, width, height)
}

#[tauri::command(async)]
pub fn contextmenu_show() -> Result<()> {
    crate::widgets::contextmenu::contextmenu_manager_show()
}

#[tauri::command(async)]
pub fn contextmenu_hide() -> Result<()> {
    crate::widgets::contextmenu::contextmenu_manager_hide()
}

#[tauri::command(async)]
pub async fn system_open_bluetooth_settings() -> std::result::Result<(), String> {
    service_backend_command("system_open_bluetooth_settings", serde_json::json!({}))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 控制最小化窗口动画效果
/// enabled=true 表示开启，enabled=false 表示关闭
#[tauri::command(async)]
pub async fn system_toggle_minimize_animation(enabled: bool) -> Result<()> {
    service_backend_command(
        "system_toggle_minimize_animation",
        serde_json::json!({ "enabled": enabled }),
    )
    .await?;
    Ok(())
}

/// 切换纯净模式
/// enabled=true 表示开启，enabled=false 表示关闭
#[tauri::command(async)]
pub async fn system_toggle_clean_mode(enabled: bool) -> Result<()> {
    service_backend_command(
        "system_toggle_clean_mode",
        serde_json::json!({ "enabled": enabled }),
    )
    .await?;
    Ok(())
}
/// 通知同步进程
#[tauri::command(async)]
pub async fn system_toggle_calendar_sync(enabled: bool) -> Result<()> {
    service_backend_command(
        "system_toggle_calendar_sync",
        serde_json::json!({ "enabled": enabled }),
    )
    .await?;
    Ok(())
}

/// 读取注册表中的纯净模式状态
#[tauri::command(async)]
pub async fn get_clean_mode_from_registry() -> Result<bool> {
    service_backend_bool_command("get_clean_mode_from_registry", false).await
}

/// 读取注册表中的 Defender 状态
#[tauri::command(async)]
pub async fn get_defender_disabled_from_registry() -> Result<bool> {
    service_backend_bool_command("get_defender_disabled_from_registry", false).await
}

/// 切换 Defender 状态
#[tauri::command(async)]
pub async fn system_toggle_defender(disabled: bool) -> Result<()> {
    service_backend_command(
        "system_toggle_defender",
        serde_json::json!({ "disabled": disabled }),
    )
    .await?;
    Ok(())
}

/// 从注册表读取服务体验优化状态（StopWU）
#[tauri::command(async)]
pub async fn get_stop_wu_from_registry() -> Result<bool> {
    service_backend_bool_command("get_stop_wu_from_registry", false).await
}

/// 从注册表读取闲时更新状态（IdleUpgrade）
#[tauri::command(async)]
pub async fn get_upgrade_mode_from_registry() -> Result<bool> {
    service_backend_bool_command("get_upgrade_mode_from_registry", false).await
}

/// 切换服务体验优化状态（StopWU）
#[tauri::command(async)]
pub async fn system_toggle_stop_wu(enabled: bool) -> Result<()> {
    service_backend_command(
        "system_toggle_stop_wu",
        serde_json::json!({ "enabled": enabled }),
    )
    .await?;
    Ok(())
}

/// 从注册表读取浏览器体验增强状态（StopEdgeAds）
#[tauri::command(async)]
pub async fn get_browser_enhance_from_registry() -> Result<bool> {
    service_backend_bool_command("get_browser_enhance_from_registry", false).await
}

/// 切换浏览器体验增强状态
/// enabled=true 表示开启，enabled=false 表示关闭
#[tauri::command(async)]
pub async fn system_toggle_browser_enhance(enabled: bool) -> Result<()> {
    service_backend_command(
        "system_toggle_browser_enhance",
        serde_json::json!({ "enabled": enabled }),
    )
    .await?;
    Ok(())
}

/// 切换升级管理模式
/// enabled=true 表示开启，enabled=false 表示关闭
#[tauri::command(async)]
pub async fn system_toggle_upgrade_mode(enabled: bool) -> Result<()> {
    service_backend_command(
        "system_toggle_upgrade_mode",
        serde_json::json!({ "enabled": enabled }),
    )
    .await?;
    Ok(())
}

/// 检查 PCManager 是否存在
#[tauri::command(async)]
pub async fn check_pc_manager_exists() -> Result<bool> {
    service_backend_bool_command("check_pc_manager_exists", false).await
}

/// 打开指定文件（支持相对路径，相对于应用程序所在目录）
#[tauri::command(async)]
pub async fn system_open_file(file_path: String) -> Result<()> {
    service_backend_command(
        "system_open_file",
        serde_json::json!({ "filePath": file_path }),
    )
    .await?;
    Ok(())
}

/// 读取注册表中的最小化动画状态
#[tauri::command(async)]
pub async fn get_minimize_animation_from_registry() -> Result<bool> {
    service_backend_bool_command("get_minimize_animation_from_registry", true).await
}

/// 读取注册表中的图标主题状态
#[tauri::command(async)]
pub async fn get_icon_theme_from_registry() -> Result<u32> {
    service_backend_u32_command("get_icon_theme_from_registry", 0).await
}

/// 读取注册表中的用户体验计划状态
#[tauri::command(async)]
pub async fn get_user_experience_plan_from_registry() -> Result<bool> {
    service_backend_bool_command("get_user_experience_plan_from_registry", true).await
}

/// 控制用户体验计划
/// enabled=true 表示同意，enabled=false 表示取消
#[tauri::command(async)]
pub async fn system_toggle_user_experience_plan(enabled: bool) -> Result<()> {
    service_backend_command(
        "system_toggle_user_experience_plan",
        serde_json::json!({ "enabled": enabled }),
    )
    .await?;
    Ok(())
}

/// 停止服务
#[tauri::command(async)]
pub async fn system_stop_service() -> Result<()> {
    service_backend_command("system_stop_service", serde_json::json!({})).await?;
    Ok(())
}

/// 打开设置窗口
#[tauri::command(async)]
pub async fn system_open_settings_window(app_handle: AppHandle) -> Result<()> {
    let label = "settings";

    if let Some(window) = app_handle.get_webview_window(label) {
        // 窗口已存在，恢复并获焦
        let _ = window.unminimize();
        let _ = window.set_focus();
        return Ok(());
    }

    let args = crate::widgets::WebviewArgs::new().disable_gpu();
    let _window = tauri::WebviewWindowBuilder::new(
        &app_handle,
        label,
        tauri::WebviewUrl::App("settings/index.html".into()),
    )
    .title("Settings")
    .inner_size(800.0, 500.0)
    .resizable(false)
    .minimizable(true)
    .maximizable(false)
    .decorations(false)
    .transparent(true)
    .visible(false)
    .always_on_top(false)
    .center()
    .skip_taskbar(false)
    .devtools(false)
    .data_directory(args.data_directory())
    .additional_browser_args(&args.to_string())
    .build()?;

    Ok(())
}

pub async fn system_open_settings_window_with_tab(
    app_handle: AppHandle,
    tab: String,
) -> Result<()> {
    let label = "settings";

    if let Some(window) = app_handle.get_webview_window(label) {
        // 窗口已存在，恢复并获焦
        let _ = window.unminimize();
        let _ = window.set_focus();
        return Ok(());
    }

    let url = if tab.is_empty() {
        "settings/index.html".to_string()
    } else {
        format!("settings/index.html?tab={}", tab)
    };

    let args = crate::widgets::WebviewArgs::new().disable_gpu();
    let _window =
        tauri::WebviewWindowBuilder::new(&app_handle, label, tauri::WebviewUrl::App(url.into()))
            .title("Settings")
            .inner_size(800.0, 500.0)
            .resizable(true)
            .minimizable(true)
            .maximizable(true)
            .decorations(false)
            .transparent(true)
            .visible(false)
            .always_on_top(false)
            .center()
            .skip_taskbar(false)
            .devtools(false)
            .data_directory(args.data_directory())
            .additional_browser_args(&args.to_string())
            .build()?;

    Ok(())
}

/// 打开设置窗口并导航到纯净模式
#[tauri::command(async)]
pub async fn system_open_puremode_settings(app_handle: AppHandle) -> Result<()> {
    log::info!("[OpenPureModeSettings] >>> 打开设置窗口到纯净模式界面");

    let label = "settings";
    let is_existing = app_handle.get_webview_window(label).is_some();

    // 使用 URL 参数直接打开到纯净模式标签页
    system_open_settings_window_with_tab(app_handle.clone(), "puremode".to_string()).await?;

    // 显示窗口并恢复（如果最小化）
    if let Some(window) = app_handle.get_webview_window("settings") {
        let _ = window.unminimize();
        let _ = window.set_focus();

        // 如果窗口已经存在，需要发送事件通知前端切换标签页
        // 因为已存在的窗口不会重新加载 URL，无法读取 tab=puremode 参数
        if is_existing {
            let _ = window.emit("navigate-to-pure-mode", ());
        }
    } else {
        log::warn!("[OpenPureModeSettings] Window not found after creation");
    }

    log::info!("[OpenPureModeSettings] ✓ 完成");
    Ok(())
}

/// 切换图标主题
#[tauri::command(async)]
pub async fn system_change_theme(theme_type: u32) -> Result<()> {
    service_backend_command(
        "system_change_theme",
        serde_json::json!({ "theme_type": theme_type }),
    )
    .await?;
    Ok(())
}

#[tauri::command]
pub fn system_switch_icon_backplate_style(app_handle: AppHandle, style: String) -> Result<()> {
    log::info!("[BackplateStyle] >>> 开始切换背板样式，style={}", style);

    use crate::trace_lock;
    use crate::widgets::taskbar::taskbar_items_impl::TASKBAR_STATE;

    // 刷新所有窗口的图标信息（解决白名单窗口切换背板后图标不更新的问题）
    {
        let mut taskbar_state = trace_lock!(TASKBAR_STATE);
        taskbar_state.refresh_all_window_icons();
    }

    // 发送更新后的窗口信息到前端
    if let Err(e) = trace_lock!(TASKBAR_STATE).emit_to_webview() {
        log::error!("[BackplateStyle] Failed to emit taskbar items: {}", e);
    }

    // 发送事件给所有窗口
    let event_payload = serde_json::json!({
        "style": style
    });

    // 遇到未定义的窗口标签，使用 Manager trait 遵歷所有窗口
    use tauri::Manager;
    let windows = app_handle.webview_windows();
    let mut sent_count = 0;

    for (_label, window) in windows {
        if let Err(e) = window.emit("backplate-style-changed", event_payload.clone()) {
            log::warn!("[BackplateStyle] ✗ 发送给窗口 {} 失败: {:?}", _label, e);
        } else {
            log::info!("[BackplateStyle] ✓ 越庆事件已发送至窗口 {}", _label);
            sent_count += 1;
        }
    }

    if sent_count > 0 {
        log::info!(
            "[BackplateStyle] ✓ 背板样式切换事件已发送面所有 {} 个窗口，style={}",
            sent_count,
            style
        );
    } else {
        log::warn!("[BackplateStyle] ✗ 找不到任何窗口");
    }

    log::info!("[BackplateStyle] >>> 完成背板样式切换");
    Ok(())
}
/// 获取当前焦点窗口的类名和标题
#[tauri::command(async)]
async fn get_foreground_window_info() -> Result<Option<(String, String)>> {
    let data = service_backend_command("get_foreground_window_info", serde_json::json!({})).await?;
    Ok(data.and_then(|value| serde_json::from_value(value).ok()))
}

/// 设置 ContextMenu 状态
#[tauri::command(async)]
fn contextmenu_set_state(item_id: Option<String>, display_name: Option<String>) -> Result<()> {
    set_context_menu_state(item_id, display_name);
    Ok(())
}

/// 获取 ContextMenu 状态
#[tauri::command(async)]
fn contextmenu_get_state() -> Result<ContextMenuState> {
    Ok(get_context_menu_state())
}

/// 发送 WM_USER+17 消息通知进程发送开机自启列表
/// wParam: 我们自己后台窗口的句柄，对方通过 WM_COPYDATA 回传数据时使用
#[tauri::command(async)]
pub fn send_message_to_app_startup() -> Result<()> {
    use crate::windows_api::event_window::BACKGROUND_HWND;
    use std::sync::atomic::Ordering;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};

    const WM_APPSTARTUP_GET_LIST: u32 = WM_USER + 17;

    log::info!("[AppStartup] >>> 开始发送 WM_USER+17 消息");

    let hwnd = match unsafe { FindWindowW(windows_core::w!("MagicSpaceTurbo"), None) } {
        Ok(h) if !h.is_invalid() => h,
        _ => {
            let code = unsafe { windows::Win32::Foundation::GetLastError().0 };
            log::warn!("[AppStartup] FindWindowW failed, code={}", code);
            return Ok(());
        }
    };

    log::info!("[AppStartup] Window found, HWND: {:?}", hwnd);

    let our_hwnd = BACKGROUND_HWND.load(Ordering::SeqCst);
    log::info!("[AppStartup] Our HWND: {}", our_hwnd);

    unsafe {
        if PostMessageW(
            Some(hwnd),
            WM_APPSTARTUP_GET_LIST,
            WPARAM(our_hwnd as usize),
            LPARAM(0),
        )
        .is_ok()
        {
            log::info!("[AppStartup] PostMessageW sent, wParam=0x{:X}", our_hwnd);
        } else {
            let code = windows::Win32::Foundation::GetLastError().0;
            log::warn!("[AppStartup] PostMessageW failed, code={}", code);
        }
    }

    Ok(())
}

/// 发送消息到 MagicSpaceTurbo 窗口（用于三方软件管控初始化）
/// wParam: 我们自己后台窗口的句柄，对方通过 WM_COPYDATA 回传数据时使用
/// lParam: 附加参数
#[tauri::command(async)]
pub fn send_message_to_magic_space_turbo() -> Result<()> {
    use crate::windows_api::event_window::BACKGROUND_HWND;
    use std::sync::atomic::Ordering;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};

    const WM_USER_GET_LIST: u32 = WM_USER + 16;

    log::info!("[MagicSpaceTurbo] >>> 开始发送 WM_USER + 16 消息");

    // 查找 MagicSpaceTurbo 窗口
    let hwnd = match unsafe { FindWindowW(windows_core::w!("MagicSpaceTurbo"), None) } {
        Ok(h) if !h.is_invalid() => h,
        _ => {
            let code = unsafe { windows::Win32::Foundation::GetLastError().0 };
            log::warn!("[MagicSpaceTurbo] FindWindowW failed, code={}", code);
            return Ok(());
        }
    };

    log::info!("[MagicSpaceTurbo] Window found, HWND: {:?}", hwnd);
    let our_hwnd = BACKGROUND_HWND.load(Ordering::SeqCst);
    log::info!("[MagicSpaceTurbo] Our HWND: {}", our_hwnd);

    // 发送消息
    unsafe {
        if PostMessageW(
            Some(hwnd),
            WM_USER_GET_LIST,
            WPARAM(our_hwnd as usize),
            LPARAM(0),
        )
        .is_ok()
        {
            log::info!(
                "[MagicSpaceTurbo] PostMessageW sent, wParam=0x{:X}",
                our_hwnd
            );
        } else {
            let code = windows::Win32::Foundation::GetLastError().0;
            log::warn!("[MagicSpaceTurbo] PostMessageW failed, code={}", code);
        }
    }

    Ok(())
}

// ==================== Popup 玻璃模糊效果 ====================

/// 显示 popup 玻璃模糊效果
///
/// 参数：
/// - id: 模糊窗口的唯一标识符（用于支持多个同时存在的模糊窗口）
/// - x, y: popup 相对于屏幕的位置（CSS 像素，会乘以 DPI）
/// - width, height: popup 的尺寸（CSS 像素，会乘以 DPI）
/// - corner_radius: 模糊区域圆角半径（CSS 像素，会乘以 DPI）
#[tauri::command(async)]
pub async fn popup_glass_show(
    webview: WebviewWindow<Wry>,
    id: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    corner_radius: f32,
) -> Result<()> {
    use crate::{app::APP_MANAGER, trace_lock, windows_api::WindowsApi};
    use windows::Win32::Graphics::Gdi::MonitorFromWindow;
    use windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST;

    log::info!(
        "[PopupGlass] show called id='{}': x={}, y={}, width={}, height={}, corner_radius={}",
        id,
        x,
        y,
        width,
        height,
        corner_radius
    );

    let caller_label = webview.label().to_string();
    let mut matched_toolbar = false;
    let manager = crate::trace_read!(APP_MANAGER);
    for instance in &manager.instances {
        let mut toolbar = trace_lock!(instance.toolbar);
        if let Some(tl) = toolbar.as_mut() {
            if tl.window_label() != caller_label {
                continue;
            }
            matched_toolbar = true;

            // 获取 DPI 缩放比
            let dpi = if let Ok(hwnd) = tl.hwnd() {
                let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
                WindowsApi::get_monitor_scale_factor(monitor).unwrap_or(1.0)
            } else {
                1.0
            };

            // 将 CSS 像素转换为物理像素
            let phys_x = (x as f64 * dpi) as i32;
            let phys_y = (y as f64 * dpi) as i32;
            let phys_width = (width as f64 * dpi) as i32;
            let phys_height = (height as f64 * dpi) as i32;
            let phys_corner = corner_radius * dpi as f32;

            // 获取 toolbar 窗口位置，将屏幕坐标转换为相对于 toolbar 的坐标
            let (rel_x, rel_y) = if let Ok(hwnd) = tl.hwnd() {
                if let Ok(toolbar_rect) = WindowsApi::get_outer_window_rect(hwnd) {
                    let rel_x = phys_x;
                    let rel_y = phys_y;
                    log::info!("[PopupGlass] DPI={}, CSS({}, {}) -> 物理({}, {}), toolbar_rect={:?}, 相对坐标({}, {})", 
                        dpi, x, y, phys_x, phys_y, toolbar_rect, rel_x, rel_y);
                    (rel_x, rel_y)
                } else {
                    (phys_x, phys_y)
                }
            } else {
                (phys_x, phys_y)
            };

            // 如果该 ID 的模糊效果不存在，则创建
            if !tl.popup_glasses.contains_key(&id) {
                if let Ok(hwnd) = tl.hwnd() {
                    match crate::widgets::popup_glass_effect::PopupGlassEffect::new(
                        hwnd,
                        phys_corner,
                    ) {
                        Ok(glass) => {
                            log::info!("[PopupGlass] 创建成功 id='{}'", id);
                            tl.popup_glasses.insert(id.clone(), glass);
                        }
                        Err(e) => {
                            log::warn!("[PopupGlass] 创建失败 id='{}': {:?}", id, e);
                            continue;
                        }
                    }
                }
            }

            // 显示模糊效果（使用物理像素坐标）
            if let Some(glass) = tl.popup_glasses.get(&id) {
                glass.show_at(rel_x, rel_y, phys_width, phys_height, phys_corner);
                log::info!(
                    "[PopupGlass] 已显示 id='{}': 相对坐标({}, {}) 物理尺寸 {}x{}",
                    id,
                    rel_x,
                    rel_y,
                    phys_width,
                    phys_height
                );
            }
            break;
        }
    }
    if !matched_toolbar {
        log::warn!(
            "[PopupGlass] show skipped, no toolbar matches caller label='{}' id='{}'",
            caller_label,
            id
        );
    }

    Ok(())
}

/// 隐藏并销毁 popup 玻璃模糊效果
///
/// 参数：
/// - id: 要销毁的模糊窗口 ID
///
/// 优化：先隐藏窗口（立即响应），再销毁（异步清理）
#[tauri::command(async)]
pub async fn popup_glass_hide(webview: WebviewWindow<Wry>, id: String) -> Result<()> {
    use crate::{app::APP_MANAGER, trace_lock};

    log::info!("[PopupGlass] hide/destroy called id='{}'", id);

    let caller_label = webview.label().to_string();
    let mut matched_toolbar = false;
    let manager = crate::trace_read!(APP_MANAGER);
    for instance in &manager.instances {
        let mut toolbar = trace_lock!(instance.toolbar);
        if let Some(tl) = toolbar.as_mut() {
            if tl.window_label() != caller_label {
                continue;
            }
            matched_toolbar = true;

            // 先隐藏窗口（立即生效，避免视觉延迟）
            if let Some(glass) = tl.popup_glasses.get(&id) {
                glass.hide();
                log::info!("[PopupGlass] 已隐藏 id='{}'", id);
            }

            // 从 HashMap 中移除并销毁（异步清理）
            if let Some(glass) = tl.popup_glasses.remove(&id) {
                drop(glass);
                log::info!("[PopupGlass] 已销毁 id='{}'", id);
            }
            break;
        }
    }
    if !matched_toolbar {
        log::warn!(
            "[PopupGlass] hide skipped, no toolbar matches caller label='{}' id='{}'",
            caller_label,
            id
        );
    }

    Ok(())
}

/// 查询 Toolbar 当前的 overlap 状态
/// 供前端在 webview 加载完成后主动同步 overlap 状态，
/// 避免因事件在 webview 就绪前发射而丢失导致前端状态不同步
#[tauri::command(async)]
pub fn toolbar_get_overlap_state(webview: WebviewWindow<tauri::Wry>) -> bool {
    use crate::{app::APP_MANAGER, trace_lock};

    let caller_label = webview.label().to_string();
    let toolbars: Vec<_> = {
        let manager = crate::trace_read!(APP_MANAGER);
        manager
            .instances
            .iter()
            .map(|instance| instance.toolbar.clone())
            .collect()
    };
    for toolbar_arc in &toolbars {
        let toolbar = trace_lock!(toolbar_arc);
        if let Some(tl) = toolbar.as_ref() {
            if tl.window_label() != caller_label {
                continue;
            }
            let overlapped = tl.is_overlaped();
            log::info!(
                target: "toolbar",
                "[toolbar_get_overlap_state] queried overlap state: overlapped={}, label={}",
                overlapped,
                tl.window_label()
            );
            return overlapped;
        }
    }
    log::warn!(
        target: "toolbar",
        "[toolbar_get_overlap_state] no matching toolbar for caller label={}",
        caller_label
    );
    false
}

/// 查询 Toolbar 当前的最大化窗口状态
/// 供前端在 webview 加载完成后主动同步最大化状态，
/// 避免因 webview 重载或事件丢失导致前端状态不同步
#[tauri::command(async)]
pub fn toolbar_get_maximized_state(webview: WebviewWindow<tauri::Wry>) -> bool {
    use crate::{app::APP_MANAGER, trace_lock};

    let caller_label = webview.label().to_string();
    let toolbars: Vec<_> = {
        let manager = crate::trace_read!(APP_MANAGER);
        manager
            .instances
            .iter()
            .map(|instance| instance.toolbar.clone())
            .collect()
    };
    for toolbar_arc in &toolbars {
        let toolbar = trace_lock!(toolbar_arc);
        if let Some(tl) = toolbar.as_ref() {
            if tl.window_label() != caller_label {
                continue;
            }
            let has_maximized = tl.last_has_maximized_window.unwrap_or(false);
            log::info!(
                target: "toolbar",
                "[toolbar_get_maximized_state] queried maximized state: has_maximized={}, label={}",
                has_maximized,
                tl.window_label()
            );
            return has_maximized;
        }
    }
    log::warn!(
        target: "toolbar",
        "[toolbar_get_maximized_state] no matching toolbar for caller label={}",
        caller_label
    );
    false
}
