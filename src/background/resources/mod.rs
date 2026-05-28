pub mod cli;
pub mod commands;
mod emitters;

use std::{
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};

use libs_core::{
    resource::{ResourceKind, SluResource},
    state::{IconPack, Theme},
};

use crate::{error::Result, utils::constants::VAR_COMMON};

pub static RESOURCES: LazyLock<Arc<ResourceManager>> =
    LazyLock::new(|| Arc::new(ResourceManager::default()));

#[derive(Default)]
pub struct ResourceManager {
    pub themes: scc::HashMap<PathBuf, Arc<Theme>>,
    pub icon_packs: scc::HashMap<PathBuf, Arc<IconPack>>,
    /// list of manual loaded resources
    pub manual: scc::HashSet<PathBuf>,
}

impl ResourceManager {
    pub fn load(&self, kind: &ResourceKind, path: &Path) -> Result<()> {
        match kind {
            ResourceKind::Theme => {
                let mut theme = Theme::load(path)?;
                if theme.id.starts_with("@deprecated") {
                    return Ok(());
                }
                theme.metadata.internal.bundled =
                    path.starts_with(VAR_COMMON.bundled_themes_path());
                self.themes.upsert(path.to_path_buf(), Arc::new(theme));
            }
            ResourceKind::IconPack => {
                let mut icon_pack = IconPack::load(path)?;
                icon_pack.metadata.internal.bundled =
                    path == VAR_COMMON.user_icons_path().join("system");
                self.icon_packs
                    .upsert(path.to_path_buf(), Arc::new(icon_pack));
            }
            _ => {}
        }
        Ok(())
    }

    pub fn unload(&self, kind: &ResourceKind, path: &Path) {
        match kind {
            ResourceKind::Theme => {
                self.themes.remove(path);
            }
            ResourceKind::IconPack => {
                self.icon_packs.remove(path);
            }
            _ => {}
        }
    }

    pub fn unload_all(&self, kind: &ResourceKind) {
        match kind {
            ResourceKind::Theme => self.themes.retain(|k, _| !self.manual.contains(k)),
            ResourceKind::IconPack => self.icon_packs.retain(|k, _| !self.manual.contains(k)),
            _ => {}
        }
    }

    /// returns a list of dirs to be read by this kind
    fn get_entries_for_type(kind: &ResourceKind) -> Result<Vec<std::fs::ReadDir>> {
        let list = match kind {
            ResourceKind::Theme => {
                let user_path = VAR_COMMON.user_themes_path();
                let bundled_path = VAR_COMMON.bundled_themes_path();
                // Ensure directories exist before reading
                std::fs::create_dir_all(user_path)?;
                vec![
                    std::fs::read_dir(bundled_path)?,
                    std::fs::read_dir(user_path)?,
                ]
            }
            ResourceKind::IconPack => {
                let user_path = VAR_COMMON.user_icons_path();
                // Ensure directory exists before reading
                std::fs::create_dir_all(user_path)?;
                vec![std::fs::read_dir(user_path)?]
            }
            _ => vec![],
        };
        Ok(list)
    }

    pub fn load_all_of_type(&self, kind: ResourceKind) -> Result<()> {
        let entries = Self::get_entries_for_type(&kind)?;
        self.unload_all(&kind);
        for entry in entries.into_iter().flatten().flatten() {
            match self.load(&kind, &entry.path()) {
                Ok(_) => {}
                Err(e) => {
                    log::error!("Failed to load {kind:?}, error: {e}");
                }
            }
        }
        Ok(())
    }
}

unsafe impl Send for ResourceManager {}
unsafe impl Sync for ResourceManager {}
