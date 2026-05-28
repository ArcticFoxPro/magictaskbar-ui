mod windows;

pub use windows::*;

use std::sync::LazyLock;

use libs_core::system_state::UserAppWindow;

use crate::{event_manager, utils::lock_free::SyncVec, windows_api::window::Window};

pub static USER_APPS_MANAGER: LazyLock<UserAppsManager> = LazyLock::new(UserAppsManager::init);

pub struct UserAppsManager {
    pub interactable_windows: SyncVec<UserAppWindow>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserAppsEvent {
    WinAdded(isize),
    WinUpdated(isize),
    WinRemoved(isize),
    AppAdded,
    AppUpdated,
    AppRemoved,
}

event_manager!(UserAppsManager, UserAppsEvent);

impl UserAppsManager {
    fn init() -> Self {
        Self {
            interactable_windows: SyncVec::from(Self::init_listing_app_windows()),
        }
    }

    pub fn contains_win(&self, window: &Window) -> bool {
        let hwnd = window.address();
        self.interactable_windows.any(|w| w.hwnd == hwnd)
    }

    fn add_win(&self, window: &Window) {
        log::trace!("Adding: {window}");
        self.interactable_windows.push(window.to_serializable());
    }

    pub fn add_win_if_missing(&self, window: &Window) -> bool {
        let hwnd = window.address();
        self.interactable_windows
            .push_if_missing(window.to_serializable(), |w| w.hwnd == hwnd)
    }

    pub fn remove_win(&self, window: &Window) {
        log::trace!("Removing: {window}");
        let hwnd = window.address();
        self.interactable_windows.retain(|w| w.hwnd != hwnd);
    }
}
