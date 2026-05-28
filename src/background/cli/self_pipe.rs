use clap::Parser;
use slu_ipc::{
    messages::{AppMessage, IpcResponse},
    AppIpc,
};
use tauri::Emitter;

use crate::{cli::application::AppCli, error::Result};

pub struct SelfPipe;
impl SelfPipe {
    fn _handle_message(mut argv: Vec<String>) -> Result<()> {
        if argv.is_empty() {
            return Ok(());
        }

        let first = argv.first().unwrap();
        if !first.contains("magictaskbar-ui") {
            argv.insert(0, "magictaskbar-ui.exe".to_string());
        }

        if let Ok(cli) = AppCli::try_parse_from(argv) {
            if let Err(err) = cli.process() {
                log::error!("Failed to process command: {err}");
                return Err(err);
            }
        }
        Ok(())
    }

    fn handle_message(message: AppMessage) -> IpcResponse {
        match message {
            AppMessage::Cli(argv) => match Self::_handle_message(argv) {
                Ok(()) => IpcResponse::Success,
                Err(err) => IpcResponse::Err(err.to_string()),
            },
            AppMessage::WinEvent { event, hwnd } => {
                let window = crate::windows_api::window::Window::from(hwnd);
                let win_event = crate::windows_api::window::event::WinEvent::from(event);
                if let Err(err) = crate::hook::HookManager::event_tx().send((win_event, window)) {
                    log::error!("[SelfPipe] failed to enqueue forwarded WinEvent: {err}");
                    return IpcResponse::Err(err.to_string());
                }
                if let Err(err) =
                    crate::widgets::taskbar::Taskbar::process_raw_win_event(event, window.hwnd())
                {
                    log::warn!("[SelfPipe] process_raw_win_event failed: {err:?}");
                }
                IpcResponse::Success
            }
            AppMessage::KeyboardWinKeyDown { pressed } => {
                if !pressed {
                    return IpcResponse::Success;
                }
                crate::widgets::taskbar::Taskbar::hide_taskbar();
                log::debug!("[SelfPipe] hidden taskbar on forwarded Win key press");
                IpcResponse::Success
            }
            AppMessage::GlobalMouseMove { x, y, emitted_at } => {
                let payload = serde_json::json!([x, y, emitted_at]);
                match crate::app::get_app_handle()
                    .emit(libs_core::handlers::FuncEvent::GlobalMouseMove, payload)
                {
                    Ok(_) => IpcResponse::Success,
                    Err(err) => {
                        log::error!("[SelfPipe] failed to emit forwarded GlobalMouseMove: {err:?}");
                        IpcResponse::Err(err.to_string())
                    }
                }
            }
            AppMessage::RecycleBinContentChanged {
                recycle_bin_empty,
                recycle_bin_count,
            } => {
                crate::hook::RECYCLE_BIN_EMPTY_CACHE
                    .store(recycle_bin_empty, std::sync::atomic::Ordering::Relaxed);
                crate::hook::RECYCLE_BIN_COUNT_CACHE
                    .store(recycle_bin_count, std::sync::atomic::Ordering::Relaxed);
                crate::hook::RECYCLE_BIN_STATUS_INITIALIZED
                    .store(true, std::sync::atomic::Ordering::Relaxed);

                match crate::app::get_app_handle()
                    .emit("recycle-bin-content-changed", recycle_bin_empty)
                {
                    Ok(_) => IpcResponse::Success,
                    Err(err) => {
                        log::error!(
                            "[SelfPipe] failed to emit forwarded recycle bin state: {err:?}"
                        );
                        IpcResponse::Err(err.to_string())
                    }
                }
            }
            AppMessage::GameFullscreenChanged { blocked } => {
                crate::hook::GAME_FULLSCREEN_BLOCKED
                    .store(blocked, std::sync::atomic::Ordering::SeqCst);

                match crate::app::get_app_handle().emit(
                    libs_core::handlers::FuncEvent::GameFullscreenChanged,
                    blocked,
                ) {
                    Ok(_) => IpcResponse::Success,
                    Err(err) => {
                        log::error!(
                            "[SelfPipe] failed to emit forwarded game fullscreen state: {err:?}"
                        );
                        IpcResponse::Err(err.to_string())
                    }
                }
            }
            AppMessage::SystemVolumeChanged { volume, muted } => {
                let payload = libs_core::system_state::VolumeState { volume, muted };
                match crate::app::get_app_handle().emit(
                    libs_core::handlers::FuncEvent::SystemVolumeChanged,
                    &payload,
                ) {
                    Ok(_) => IpcResponse::Success,
                    Err(err) => {
                        log::error!("[SelfPipe] failed to emit forwarded volume state: {err:?}");
                        IpcResponse::Err(err.to_string())
                    }
                }
            }
            AppMessage::SystemBluetoothStateChanged { enabled } => {
                crate::modules::bluetooth::set_cached_bluetooth_enabled(enabled);
                let manager = crate::modules::bluetooth::BluetoothManager::instance();
                let scan_result = if enabled {
                    manager.start_scanning()
                } else {
                    manager.stop_scanning()
                };
                if let Err(err) = scan_result {
                    log::warn!(
                        "[SelfPipe] failed to update bluetooth scanning after radio state: {err:?}"
                    );
                }

                match crate::app::get_app_handle().emit(
                    libs_core::handlers::FuncEvent::SystemBluetoothStateChanged,
                    enabled,
                ) {
                    Ok(_) => IpcResponse::Success,
                    Err(err) => {
                        log::error!("[SelfPipe] failed to emit forwarded bluetooth state: {err:?}");
                        IpcResponse::Err(err.to_string())
                    }
                }
            }
            AppMessage::SystemNetworksChanged { networks } => {
                match crate::app::get_app_handle().emit(
                    libs_core::handlers::FuncEvent::SystemNetworksChanged,
                    &networks,
                ) {
                    Ok(_) => IpcResponse::Success,
                    Err(err) => {
                        log::error!("[SelfPipe] failed to emit forwarded networks: {err:?}");
                        IpcResponse::Err(err.to_string())
                    }
                }
            }
            AppMessage::WindowVisualStateChanged { hwnd, state } => {
                crate::widgets::taskbar::hook::process_visual_state_event(hwnd, state);
                IpcResponse::Success
            }
            _ => IpcResponse::Success,
        }
    }

    pub fn start_listener() -> Result<()> {
        AppIpc::start(Self::handle_message)?;
        Ok(())
    }

    pub async fn request_open_settings() -> Result<()> {
        AppIpc::send(AppMessage::Cli(vec!["settings".to_owned()])).await?;
        Ok(())
    }
}
