use libs_core::{handlers::FuncEvent, resource::ResourceKind};
use tauri::Emitter;

use crate::{app::get_app_handle, error::Result};

use super::ResourceManager;

impl ResourceManager {
    pub fn emit_themes(&self) -> Result<()> {
        let mut themes = Vec::new();
        self.themes.scan(|_, v| {
            themes.push(v.clone());
        });
        get_app_handle().emit(FuncEvent::StateThemesChanged, themes)?;
        Ok(())
    }

    pub fn emit_icon_packs(&self) -> Result<()> {
        let mut icon_packs = Vec::new();
        self.icon_packs.scan(|_, v| {
            icon_packs.push(v.clone());
        });
        get_app_handle().emit(FuncEvent::StateIconPacksChanged, icon_packs)?;
        Ok(())
    }

    pub fn emit_kind_changed(&self, kind: &ResourceKind) -> Result<()> {
        match kind {
            ResourceKind::Theme => self.emit_themes()?,
            ResourceKind::IconPack => self.emit_icon_packs()?,
            _ => {}
        }
        Ok(())
    }
}
