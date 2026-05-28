pub(crate) mod icons;
pub mod performance;
mod settings;
mod taskbar_items;

use arc_swap::ArcSwap;
use getset::Getters;
use icons::IconPacksManager;
use lazy_static::lazy_static;
use libs_core::{resource::ResourceKind, state::TaskbarItems};
use parking_lot::Mutex;
use std::{path::Path, sync::Arc};

use crate::{error::Result, resources::RESOURCES};

use super::domain::Settings;

lazy_static! {
    pub static ref FULL_STATE: Arc<ArcSwap<FullState>> = Arc::new(ArcSwap::from_pointee({
        log::trace!("Creating new State Manager");
        FullState::new().expect("Failed to create State Manager")
    }));
}

#[derive(Getters, Debug, Clone)]
#[getset(get = "pub")]
pub struct FullState {
    // ======== data ========
    pub settings: Settings,
    pub taskbar_items: TaskbarItems,
    // ====== resources ========
    pub icon_packs: Arc<Mutex<IconPacksManager>>,
}

unsafe impl Sync for FullState {}

impl FullState {
    fn new() -> Result<Self> {
        let mut manager = Self {
            // ======== data ========
            settings: Settings::default(),
            taskbar_items: TaskbarItems::default(),
            icon_packs: Arc::new(Mutex::new(IconPacksManager::default())),
        };
        manager.load_all()?;
        Ok(manager)
    }

    /// Shorthand of `FullState::clone` on Arc reference
    ///
    /// Intended to be used with `ArcSwap::rcu` to mofify the state
    pub fn cloned(&self) -> Self {
        self.clone()
    }

    /// We log each step on this cuz for some reason a deadlock is happening somewhere.
    fn load_all(&mut self) -> Result<()> {
        log::trace!("Initial load: settings");
        self.read_settings();

        log::trace!("Initial load: taskbar items");
        self.read_taskbar_items();

        log::trace!("Initial load: themes");
        RESOURCES.load_all_of_type(ResourceKind::Theme)?;

        log::trace!("Initial load: icons packs");
        self.load_icons_packs(true)?;
        Ok(())
    }

    fn show_corrupted_state_to_user(path: &Path) {
        // Popup UI has been removed, using log instead
        log::error!("Corrupted data file detected: {:?}", path);
        log::error!(
            "The file may be corrupted or in an invalid format. Please check or delete it."
        );
    }
}
