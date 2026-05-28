use libs_core::{handlers::FuncEvent, state::Settings};
use tauri::Emitter;

use crate::{
    app::{get_app_handle, APP_MANAGER},
    error::Result,
    trace_lock, trace_read,
    widgets::taskbar::taskbar_items_impl::TASKBAR_STATE,
};

use super::FullState;

impl FullState {
    pub(super) fn emit_settings(&self) -> Result<()> {
        get_app_handle().emit(FuncEvent::StateSettingsChanged, self.settings())?;
        trace_read!(APP_MANAGER).on_settings_change(self)?;
        trace_lock!(TASKBAR_STATE).emit_to_webview()?;
        Ok(())
    }

    fn _read_settings(&mut self) -> Result<()> {
        // 从文件加载设置，如果失败则使用默认值
        self.settings = Settings::load(crate::utils::constants::VAR_COMMON.settings_path())?;
        // 从注册表读取 FancyToolbarHideMode 并覆盖默认值
        use winreg::{
            enums::{HKEY_CURRENT_USER, KEY_READ},
            RegKey,
        };
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey_with_flags(r"SOFTWARE\HONOR\Magicanimation", KEY_READ) {
            if let Ok(val) = key.get_value::<String, _>("FancyToolbarHideMode") {
                use libs_core::state::HideMode;
                self.settings.by_widget.fancy_toolbar.hide_mode = match val.as_str() {
                    "Never" => HideMode::Never,
                    "OnOverlap" => HideMode::OnOverlap,
                    "Always" => HideMode::Always,
                    _ => self.settings.by_widget.fancy_toolbar.hide_mode,
                };
            }
        }
        self.settings.sanitize()?;
        Ok(())
    }

    pub(super) fn read_settings(&mut self) {
        if let Err(err) = self._read_settings() {
            log::error!("Failed to initialize default settings: {err}");
        }
    }

    pub fn write_settings(&self) -> Result<()> {
        // 保存设置到文件
        self.settings
            .save(crate::utils::constants::VAR_COMMON.settings_path())?;
        // 向前端和管理器广播变更
        self.emit_settings()
    }
}
