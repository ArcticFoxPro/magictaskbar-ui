// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(dead_code)]
#![allow(deprecated)]

mod app;
mod app_instance;
mod cli;
mod error;
mod exposed;
mod grpc_bridge;
mod hook;
mod modules;
mod resources;
mod restoration_and_migrations;
mod state;
mod system;
mod tauri_context;
mod tauri_plugins;
mod utils;
mod webview_recovery;
mod widgets;
mod windows_api;

#[macro_use]
extern crate rust_i18n;
i18n!("src/background/i18n", fallback = "en");

extern crate lazy_static;

use std::sync::{atomic::AtomicBool, OnceLock};

use app::{AppManager, APP_MANAGER};
use cli::{application::handle_console_client, SelfPipe};
use error::Result;
use exposed::register_invoke_handler;
use itertools::Itertools;
use libs_core::state::theme::builtin::BuiltinDefaultTheme;
use tauri_plugins::register_plugins;
use utils::{
    icon_whitelist::init_icon_whitelist,
    integrity::{
        print_initial_information, register_panic_hook, validate_webview_runtime_is_installed,
    },
};
use windows::Win32::System::Threading::{GetCurrentProcess, SetPriorityClass, HIGH_PRIORITY_CLASS};

use crate::app::get_app_handle;
use crate::windows_api::WindowsApi;

const DEFAULT_THEME_SHARED_CSS: &str = include_str!("../ui/taskbar/styles/shared.css");
const DEFAULT_THEME_TASKBAR_CSS: &str = include_str!("../ui/taskbar/styles/taskbar.css");
const UI_PROCESS_EXIT_REPORT_ID: &str = "669000049";

static APP_HANDLE: OnceLock<tauri::AppHandle<tauri::Wry>> = OnceLock::new();
static TOKIO_RUNTIME_HANDLE: OnceLock<tokio::runtime::Handle> = OnceLock::new();
static SILENT: AtomicBool = AtomicBool::new(false);
static STARTUP: AtomicBool = AtomicBool::new(false);
static VERBOSE: AtomicBool = AtomicBool::new(false);
// 防抖：确保解注册只触发一次
static UNREG_SENT: OnceLock<()> = OnceLock::new();
static UI_EXIT_REPORT_SENT: OnceLock<()> = OnceLock::new();

pub fn is_local_dev() -> bool {
    cfg!(dev)
}

pub fn get_tokio_handle() -> &'static tokio::runtime::Handle {
    TOKIO_RUNTIME_HANDLE
        .get()
        .expect("Tokio runtime was not initialized")
}

pub fn report_ui_process_lifecycle(reason: &str) {
    let content = serde_json::json!({
        "Reason": reason,
    })
    .to_string();

    log::info!(
        "[Report] data bridge disabled; skip ui process lifecycle report id={} content={}",
        UI_PROCESS_EXIT_REPORT_ID,
        content
    );
}

pub fn report_ui_process_exit(reason: &str) {
    if UI_EXIT_REPORT_SENT.set(()).is_err() {
        return;
    }

    report_ui_process_lifecycle(reason);
}

async fn setup(app_handle: &tauri::AppHandle<tauri::Wry>) -> Result<()> {
    print_initial_information();
    validate_webview_runtime_is_installed(app_handle)?;
    SelfPipe::start_listener()?;

    // Pre-initialize SID cache to avoid slow whoami command on first use
    let _ = crate::exposed::get_current_user_sid_for_cache();

    // Initialize icon whitelist
    init_icon_whitelist();

    WindowsApi::wait_for_native_shell();
    trace_write!(APP_MANAGER).start()?;
    Ok(())
}

fn app_callback(app_handle: &tauri::AppHandle<tauri::Wry>, event: tauri::RunEvent) {
    match event {
        tauri::RunEvent::Ready => {
            log::info!("Tauri Application is ready.");
            // 启动 UI 进程内的 gRPC 服务器（统一端口 127.0.0.1:50051）
            // 动态选择可用端口（范围 56400..58000），若失败回退到 50051
            let pick_port = || -> u16 {
                for p in 56400u16..58000u16 {
                    let addr = format!("127.0.0.1:{}", p);
                    if std::net::TcpListener::bind(&addr).is_ok() {
                        // 释放探测占用，交由 gRPC server 绑定
                        return p;
                    }
                }
                50051
            };
            let ui_port = pick_port();
            // 将端口写入注册表 HKCU\SOFTWARE\HONOR\MagicAI -> AIBarGrpcServicePort
            {
                use winreg::{
                    enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS},
                    RegKey,
                };
                let hkcu = RegKey::predef(HKEY_CURRENT_USER);
                match hkcu
                    .create_subkey_with_flags("SOFTWARE\\HONOR\\Magicanimation", KEY_ALL_ACCESS)
                {
                    Ok((key, _disp)) => {
                        // write port as string so consumers reading REG_SZ can parse it
                        let port_str = ui_port.to_string();
                        if let Err(e) = key.set_value("AIBarGrpcServicePort", &port_str) {
                            log::warn!("Failed to set registry value AIBarGrpcServicePort: {}", e);
                        } else {
                            log::info!("[gRPC] UI server port selected: {} (written to HKCU\\SOFTWARE\\HONOR\\Magicanimation)", ui_port);
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to open/create registry key SOFTWARE\\HONOR\\Magicanimation: {}", e);
                    }
                }
            }
            // 不再写入静态文件，grpc_test 将从注册表读取端口
            if let Err(err) = crate::grpc_bridge::start_server(ui_port, app_handle.clone()) {
                log::warn!("[gRPC] Failed to start grpc_bridge.dll server: {err:?}");
            }
            // 再启动后台事件注册与转发
            crate::exposed::register_events_after_ready();
        }
        tauri::RunEvent::Resumed => {
            log::info!("Tauri Event Loop was resumed.");
        }
        tauri::RunEvent::ExitRequested { api, code, .. } => {
            // 在首次收到退出请求时，先尝试解注册
            if UNREG_SENT.set(()).is_ok() {
                crate::exposed::unregister_scene_on_exit();
            }
            match code {
                Some(code) => {
                    let reason = if code == 0 {
                        "NormalExitRequested"
                    } else {
                        "AbnormalExitRequested"
                    };
                    report_ui_process_exit(reason);
                    // if exit code is 0 it means that the app was closed by the user
                }
                // prevent close background on webview windows closing
                None => api.prevent_exit(),
            }
        }
        tauri::RunEvent::Exit => {
            log::info!("───────────────────── Exiting UI ─────────────────────");
            report_ui_process_exit("TauriExit");
            if AppManager::is_running() {
                trace_read!(APP_MANAGER).stop();
            }
        }
        _ => {}
    }
}

fn is_already_runnning() -> bool {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes();
    sys.processes()
        .values()
        .filter(|p| {
            p.exe()
                .is_some_and(|path| path.ends_with("magictaskbar-ui.exe"))
        })
        .collect_vec()
        .len()
        > 1
}

fn try_set_current_process_high_priority() {
    unsafe {
        match SetPriorityClass(GetCurrentProcess(), HIGH_PRIORITY_CLASS) {
            Ok(()) => {
                log::info!("Set current process priority to HIGH_PRIORITY_CLASS");
            }
            Err(err) => {
                log::warn!(
                    "Failed to set process priority to HIGH_PRIORITY_CLASS: {}",
                    err
                );
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // 首先初始化内置默认主题，必须在 FULL_STATE 初始化之前
    libs_core::state::theme::builtin::set_builtin_default_theme(BuiltinDefaultTheme {
        shared_css: DEFAULT_THEME_SHARED_CSS,
        taskbar_css: DEFAULT_THEME_TASKBAR_CSS,
    });

    register_panic_hook();
    try_set_current_process_high_priority();
    handle_console_client().await?;

    if is_already_runnning() {
        match SelfPipe::request_open_settings().await {
            Ok(()) => {
                println!("MagicTaskbar UI is already running");
                return Ok(());
            }
            Err(err) => {
                log::warn!(
                    "Detected another magictaskbar-ui.exe, but self IPC request failed; continuing startup: {err}"
                );
            }
        }
    }

    TOKIO_RUNTIME_HANDLE
        .set(tokio::runtime::Handle::current())
        .expect("Failed to set runtime handle");

    rust_i18n::set_locale(&libs_core::state::Settings::get_system_language());

    let mut app_builder = tauri::Builder::default();
    app_builder = register_plugins(app_builder);
    app_builder = register_invoke_handler(app_builder);

    let app = app_builder
        .setup(|app| {
            APP_HANDLE.set(app.handle().to_owned()).unwrap();

            tokio::spawn(async move {
                let handle = get_app_handle();
                if let Err(err) = setup(handle).await {
                    log::error!("Error while setting up: {err:?}");
                    report_ui_process_exit("SetupFailed");
                    handle.exit(1);
                } else {
                    report_ui_process_lifecycle("StartupSuccess");
                }
            });
            Ok(())
        })
        .build(tauri_context::get_context())
        .expect("Error while building tauri application");

    // share the current runtime with Tauri
    tauri::async_runtime::set(tokio::runtime::Handle::current());
    app.run(app_callback);
    Ok(())
}
