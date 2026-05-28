//! WebView2 Process Failed Recovery Module
//!
//! Monitors WebView2 ProcessFailed events.
//! When the browser process exits (Kind 0) or render process crashes (Kind 1-3),
//! UI proactively exits so srv can detect and restart it.

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::WebviewWindow;

/// Flag to track if recovery is already in progress (prevent multiple exit attempts)
static RECOVERY_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// Register ProcessFailed event handler for a WebView window.
///
/// When the render process crashes, it logs the error and notifies srv to restart UI.
///
/// # Arguments
/// * `window` - The WebviewWindow to monitor
/// * `window_name` - A descriptive name for logging (e.g., "Taskbar", "Toolbar")
pub fn register_process_failed_handler(window: &WebviewWindow, window_name: &'static str) {
    let window_label = window.label().to_string();

    if let Err(e) = window.with_webview(move |platform_webview| {
        // Get the ICoreWebView2Controller
        #[cfg(windows)]
        {
            use webview2_com::Microsoft::Web::WebView2::Win32::{
                ICoreWebView2, ICoreWebView2ProcessFailedEventArgs,
                COREWEBVIEW2_PROCESS_FAILED_KIND,
            };
            use webview2_com::ProcessFailedEventHandler;

            let controller = platform_webview.controller();

            // Get ICoreWebView2 from controller
            let webview: ICoreWebView2 = match unsafe { controller.CoreWebView2() } {
                Ok(wv) => wv,
                Err(e) => {
                    log::warn!(
                        "[WebViewRecovery] [{window_name}] Failed to get ICoreWebView2 from controller: {:?}",
                        e
                    );
                    return;
                }
            };

            // Create ProcessFailed event handler
            let window_label_clone = window_label.clone();
            let handler = ProcessFailedEventHandler::create(Box::new(
                move |_sender: Option<ICoreWebView2>,
                      args: Option<ICoreWebView2ProcessFailedEventArgs>|
                      -> windows_core::Result<()> {
                    if let Some(args) = args {
                        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                        let _ = unsafe { args.ProcessFailedKind(&mut kind) };

                        log::error!(
                            "[WebViewRecovery] [{window_name}] ProcessFailed detected! \
                             Window: {}, Kind: {:?}",
                            window_label_clone,
                            kind.0
                        );

                        // Kind 0: Browser process exited (all WebViews broken)
                        // Kind 1: Render process exited
                        // Kind 2: Render process unresponsive
                        // Kind 3: Frame render process exited
                        let is_fatal = kind.0 >= 0 && kind.0 <= 3;

                        if is_fatal {
                            let kind_desc = match kind.0 {
                                0 => "Browser process exited (fatal)",
                                1 => "Render process exited",
                                2 => "Render process unresponsive",
                                3 => "Frame render process exited",
                                _ => "Unknown issue",
                            };
                            log::error!(
                                "[WebViewRecovery] [{window_name}] {kind_desc} - Window: {}",
                                window_label_clone
                            );

                            // Prevent multiple simultaneous exit attempts
                            if !RECOVERY_IN_PROGRESS.swap(true, Ordering::SeqCst) {
                                log::error!(
                                    "[WebViewRecovery] [{window_name}] WebView2 unrecoverable, UI will exit. srv will restart us."
                                );

                                // Exit UI process proactively, srv's restart_gui_on_crash will detect and relaunch
                                let exit_reason = match kind.0 {
                                    0 => "WebViewBrowserProcessExited",
                                    1 => "WebViewRenderProcessExited",
                                    2 => "WebViewRenderProcessUnresponsive",
                                    3 => "WebViewFrameRenderProcessExited",
                                    _ => "WebViewProcessFailed",
                                };
                                std::thread::spawn(move || {
                                    // Brief delay to allow log flush
                                    std::thread::sleep(std::time::Duration::from_millis(500));
                                    log::error!("[WebViewRecovery] Exiting UI process now (exit code 1)");
                                    crate::report_ui_process_exit(exit_reason);
                                    std::process::exit(1);
                                });
                            } else {
                                log::warn!(
                                    "[WebViewRecovery] [{window_name}] Exit already in progress, skipping"
                                );
                            }
                        }
                    }
                    Ok(())
                },
            ));

            // Register the event handler
            let mut token: i64 = 0;
            let result = unsafe { webview.add_ProcessFailed(&handler, &mut token) };

            match result {
                Ok(()) => {
                    log::info!(
                        "[WebViewRecovery] [{window_name}] ProcessFailed handler registered successfully \
                         for window: {}",
                        window_label
                    );
                }
                Err(e) => {
                    log::error!(
                        "[WebViewRecovery] [{window_name}] Failed to register ProcessFailed handler: {:?}",
                        e
                    );
                }
            }
        }
    }) {
        log::error!(
            "[WebViewRecovery] [{window_name}] Failed to access webview: {:?}",
            e
        );
    }
}
