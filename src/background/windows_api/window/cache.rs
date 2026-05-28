//! Windows have a lot of api to get information about the window.
//! Also provides events for theses to be listened, but some events like fullscreen, maximize, etc.
//! are not standard windows events, so we handle cached windows data to check for these changes, and emit
//! synthetic events for them.

use libs_core::system_state::{FocusedApp, UserAppWindow};

use super::Window;

impl Window {
    pub fn to_serializable(self: &Window) -> UserAppWindow {
        UserAppWindow {
            hwnd: self.address(),
            monitor: self.monitor().stable_id().unwrap_or_default().into(),
            title: self.title(),
            is_iconic: self.is_minimized(),
            is_zoomed: self.is_maximized(),
            is_fullscreen: self.is_fullscreen(),
            icon_png_base64: None,         // No icon for basic serialization
            is_approximately_square: None, // No shape information for basic serialization
            is_from_local: None,           // No local icon information for basic serialization
        }
    }

    pub fn as_focused_app_information(&self) -> FocusedApp {
        let process = self.process();

        FocusedApp {
            hwnd: self.address(),
            monitor: self.monitor().stable_id().unwrap_or_default().into(),
            title: self.title(),
            name: self
                .app_display_name()
                .unwrap_or(String::from("Error on App Name")),
            exe: process.program_path().ok(),
            umid: self.app_user_model_id().map(|umid| umid.to_string()),
            is_maximized: self.is_maximized(),
            is_fullscreened: self.is_fullscreen(),
            is_bar_overlay: self.is_bar_overlay(),
        }
    }
}
