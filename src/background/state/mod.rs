pub mod application;
pub mod domain;
pub mod infrastructure;

use application::FullState;
use libs_core::{
    state::{TaskbarPinnedItemsVisibility, TaskbarTemporalItemsVisibility},
    system_state::MonitorId,
};

impl FullState {
    pub fn is_taskbar_enabled(&self) -> bool {
        self.settings.taskbar.enabled
    }

    pub fn is_taskbar_enabled_on_monitor(&self, _monitor_id: &MonitorId) -> bool {
        self.is_taskbar_enabled()
    }

    pub fn get_taskbar_temporal_item_visibility(
        &self,
        _monitor_id: &MonitorId,
    ) -> TaskbarTemporalItemsVisibility {
        self.settings.taskbar.temporal_items_visibility
    }

    pub fn get_taskbar_pinned_item_visibility(
        &self,
        _monitor_id: &MonitorId,
    ) -> TaskbarPinnedItemsVisibility {
        self.settings.taskbar.pinned_items_visibility
    }

    pub fn locale(&self) -> &String {
        // always should be filled
        self.settings().language.as_ref().unwrap()
    }
}
