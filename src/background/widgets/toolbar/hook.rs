use crate::{error::Result, windows_api::window::event::WinEvent, windows_api::window::Window};
use libs_core::state::HideMode;

use super::FancyToolbar;
use std::time::{Duration, Instant};

impl FancyToolbar {
    pub fn update_toolbar_state(&mut self, origin: Option<&Window>) -> Result<()> {
        use crate::cli::ServicePipe;
        use crate::widgets::taskbar::taskbar_items_impl::get_taskbar_windows;

        let now = Instant::now();
        if let Some(last) = self.last_state_update_at {
            if now.duration_since(last) < Duration::from_millis(200) {
                return Ok(());
            }
        }
        self.last_state_update_at = Some(now);

        let toolbar_hwnd = self.hwnd()?;
        let candidate_hwnds: Vec<isize> = get_taskbar_windows()
            .iter()
            .map(|window| window.address())
            .collect();
        let args = serde_json::json!({
            "widget_hwnd": toolbar_hwnd.0 as isize,
            "origin_hwnd": origin.map(|window| window.address()),
            "candidate_hwnds": candidate_hwnds,
        });

        let data = ServicePipe::request_with_response_blocking(
            slu_ipc::messages::SvcAction::ExecuteBackendCommand {
                command: "check_toolbar_state".to_string(),
                args,
            },
            Duration::from_millis(200),
        )?
        .ok_or("check_toolbar_state returned no data")?;
        let value: serde_json::Value = serde_json::from_str(&data)?;
        let has_maximized = value
            .get("has_maximized")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

        if has_maximized {
            let hwnd = value.get("hwnd").and_then(|value| value.as_i64());
            let exe = value
                .get("exe")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            log::info!(
                target: "Toolbar",
                "Maximized window detected by srv: exe={}, hwnd={:?}",
                exe,
                hwnd
            );
        }

        if self.last_has_maximized_window != Some(has_maximized) {
            log::info!(
                target: "Toolbar",
                "Sending ToolbarHasMaximizedWindow event: has_maximized={}",
                has_maximized
            );
            self.emit(
                libs_core::handlers::FuncEvent::ToolbarHasMaximizedWindow,
                has_maximized,
            )?;
            self.last_has_maximized_window = Some(has_maximized);
        }

        Ok(())
    }

    pub fn process_win_event(&mut self, event: WinEvent, origin: &Window) -> Result<()> {
        if matches!(
            event,
            WinEvent::SystemCaptureEnd | WinEvent::SystemForeground
        ) {
            self.is_processing = true;
            let res: Result<()> = (|| {
                let state = crate::state::application::FULL_STATE.load();
                let hide_mode = state.settings.by_widget.fancy_toolbar.hide_mode;
                if matches!(hide_mode, HideMode::OnOverlap) {
                    self.handle_overlaped_status(origin)?;
                } else {
                    self.update_toolbar_state(Some(origin))?;
                }
                Ok(())
            })();
            self.is_processing = false;
            if res.is_err() {
                return res;
            }
        }

        if matches!(
            event,
            WinEvent::SystemMoveSizeEnd | WinEvent::SystemCaptureEnd
        ) {
            let _ = self.reposition_if_needed();
            let now = Instant::now();
            if let Some(last) = self.last_refresh_at {
                if now.duration_since(last) < Duration::from_millis(120) {
                    return Ok(());
                }
            }
            self.last_refresh_at = Some(Instant::now());
        }

        Ok(())
    }
}
