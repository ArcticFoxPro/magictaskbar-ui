use itertools::Itertools;
use tauri::webview_version;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_shell::ShellExt;

use crate::{error::Result, is_local_dev, windows_api::WindowsApi};

pub fn register_panic_hook() {
    let base_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let cause = info
            .payload()
            .downcast_ref::<String>()
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                info.payload()
                    .downcast_ref::<&str>()
                    .unwrap_or(&"<cause unknown>")
                    .to_string()
            });

        let mut string_location = String::from("<location unknown>");
        if let Some(location) = info.location() {
            string_location = format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            );
        }

        log::error!("A panic occurred:\n  Cause: {cause}\n  Location: {string_location}");

        // 死锁 panic 发生在 extern "system" FFI 回调中时无法 unwind，
        // 直接退出进程避免第二次 "cannot unwind" panic 导致状态不确定 退出后srv会重新拉起UI进程
        if cause.contains("deadlocked") {
            log::error!("Deadlock detected, exiting UI process");
            crate::report_ui_process_exit("PanicDeadlock");
            std::process::exit(1);
        }

        base_hook(info);
    }));
}

/// Prints information about the computer runtime context to help debugging.
pub fn print_initial_information() {
    let version = env!("CARGO_PKG_VERSION");
    let debug = if tauri::is_dev() { " (debug)" } else { "" };
    let local = if is_local_dev() { " (local)" } else { "" };
    log::info!("───────────────────── Starting  UI v{version}{local}{debug} ─────────────────────");
    let os = os_info::get();
    let sys_locale = libs_core::state::Settings::get_locale();
    log::info!("Arguments       : {:?}", std::env::args().collect_vec());
    log::info!("Operating System: {}", os.os_type());
    log::info!("  version       : {}", os.version());
    log::info!("  edition       : {}", os.edition().unwrap_or("None"));
    log::info!("  codename      : {}", os.codename().unwrap_or("None"));
    log::info!("  bitness       : {}", os.bitness());
    log::info!(
        "  architecture  : {}",
        os.architecture().unwrap_or("Unknown")
    );
    log::info!(
        "  locate        : {}",
        sys_locale.unwrap_or("Unknown".to_owned())
    );
    log::info!("WebView2 Runtime: {:?}", webview_version());
    log::info!("Elevated        : {:?}", WindowsApi::is_elevated());
}

pub fn validate_webview_runtime_is_installed(app: &tauri::AppHandle) -> Result<()> {
    let error = match webview_version() {
        Ok(version) => {
            let mut version = version.split('.');
            let major = version.next().unwrap_or("0").parse().unwrap_or(0);
            if major < 110 {
                Some((
                    t!("runtime.outdated"),
                    t!("runtime.outdated_description", min_version = "110"),
                ))
            } else {
                None
            }
        }
        Err(_) => Some((t!("runtime.not_found"), t!("runtime.not_found_description"))),
    };

    if let Some((title, message)) = error {
        let ok_pressed = app
            .dialog()
            .message(message)
            .title(title)
            .kind(MessageDialogKind::Error)
            .buttons(MessageDialogButtons::OkCustom(
                t!("runtime.download").to_string(),
            ))
            .blocking_show();
        if ok_pressed {
            let url = "https://developer.microsoft.com/en-us/microsoft-edge/webview2/?form=MA13LH#download";
            #[allow(deprecated)]
            app.shell().open(url, None)?;
        }
        return Err("Webview runtime not installed or outdated".into());
    }
    Ok(())
}
