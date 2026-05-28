use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use itertools::Itertools;
use libs_core::{
    handlers::FuncEvent,
    resource::{ResourceText, SluResource},
    state::{
        CustomIconPackEntry, Icon, IconPack, IconPackEntry, SharedIconPackEntry,
        UniqueIconPackEntry,
    },
};
use tauri::Emitter;

use crate::{app::get_app_handle, error::Result, trace_lock, utils::constants::VAR_COMMON};

use super::FullState;

pub(crate) static SYSTEM_ICONS: LazyLock<PathBuf> =
    LazyLock::new(|| VAR_COMMON.user_icons_path().join("system"));

#[derive(Debug, Clone, Default)]
pub struct IconPacksManager(HashMap<PathBuf, IconPack>);

impl IconPacksManager {
    pub fn list(&self) -> Vec<&IconPack> {
        self.0.values().collect_vec()
    }

    pub fn owned_list(&self) -> Vec<IconPack> {
        self.0.values().cloned().collect_vec()
    }

    pub fn get_system(&self) -> &IconPack {
        self.0.get(SYSTEM_ICONS.as_path()).unwrap()
    }

    pub fn get_system_mut(&mut self) -> &mut IconPack {
        self.0.get_mut(SYSTEM_ICONS.as_path()).unwrap()
    }

    pub fn add_system_app_icon(&mut self, umid: Option<&str>, path: Option<&Path>, icon: Icon) {
        if umid.is_none() && path.is_none() {
            return;
        }
        let system_pack = self.get_system_mut();
        system_pack.add_entry(IconPackEntry::Unique(UniqueIconPackEntry {
            umid: umid.map(|s| s.to_string()),
            path: path.map(|p| p.to_path_buf()),
            redirect: None,
            icon: Some(icon),
        }));
    }

    pub fn add_system_icon_redirect(
        &mut self,
        umid: Option<String>,
        origin: &Path,
        redirect: &Path,
    ) {
        let system_pack = self.get_system_mut();
        system_pack.add_entry(IconPackEntry::Unique(UniqueIconPackEntry {
            umid,
            path: Some(origin.to_path_buf()),
            redirect: Some(redirect.to_path_buf()),
            icon: None,
        }));
    }

    pub fn add_system_file_icon(&mut self, origin_extension: &str, icon: Icon) {
        let system_pack = self.get_system_mut();
        system_pack.add_entry(IconPackEntry::Shared(SharedIconPackEntry {
            extension: origin_extension.to_string(),
            icon,
        }));
    }

    fn icon_exists(&self, icon: &Icon) -> bool {
        icon.base
            .as_ref()
            .is_some_and(|p| SYSTEM_ICONS.join(p).exists())
            || (icon
                .light
                .as_ref()
                .is_some_and(|p| SYSTEM_ICONS.join(p).exists())
                && icon
                    .dark
                    .as_ref()
                    .is_some_and(|p| SYSTEM_ICONS.join(p).exists()))
    }

    /// Get icon pack by app user model id, filename or path
    pub fn has_app_icon(&self, umid: Option<&str>, path: Option<&Path>) -> bool {
        let icon_pack = self.get_system();
        let lower_path = path.map(|p| p.to_string_lossy().to_lowercase());

        for entry in &icon_pack.entries {
            let IconPackEntry::Unique(entry) = entry else {
                continue;
            };

            let mut found = None;
            if let (Some(entry_umid), Some(umid)) = (&entry.umid, umid) {
                if entry_umid == umid {
                    found = Some(entry);
                }
            }

            // 仅当搜索路径不为 None 时才按路径匹配，避免 None == None 误匹配
            if found.is_none()
                && lower_path.is_some()
                && entry
                    .path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_lowercase())
                    == lower_path
            {
                found = Some(entry);
            }

            if let Some(entry) = found {
                if let Some(redirect_path) = &entry.redirect {
                    let redirect_lower = redirect_path.to_string_lossy().to_lowercase();
                    let target_valid = icon_pack.entries.iter().any(|e| {
                        if let IconPackEntry::Unique(target) = e {
                            target.path.as_ref().is_some_and(|p| {
                                p.to_string_lossy().to_lowercase() == redirect_lower
                            }) && target
                                .icon
                                .as_ref()
                                .is_some_and(|icon| self.icon_exists(icon))
                        } else {
                            false
                        }
                    });
                    if target_valid {
                        return true;
                    }
                    // redirect 目标的图标文件不存在，视为缓存未命中
                    continue;
                }

                if entry
                    .icon
                    .as_ref()
                    .is_some_and(|icon| self.icon_exists(icon))
                {
                    return true;
                }
            };
        }

        false
    }

    pub fn get_file_icon(&self, path: &Path) -> Option<&Icon> {
        let extension = path.extension()?.to_string_lossy().to_lowercase();
        let icon_pack = self.get_system();
        for entry in &icon_pack.entries {
            match entry {
                IconPackEntry::Shared(entry) if entry.extension.to_lowercase() == extension => {
                    if self.icon_exists(&entry.icon) {
                        return Some(&entry.icon);
                    }
                }
                _ => {}
            }
        }
        None
    }

    pub fn clear_system_icons(&mut self) -> Result<()> {
        let system_pack = self.get_system_mut();
        system_pack.entries.clear();
        let meta = std::ffi::OsStr::new("metadata.yml");
        for entry in std::fs::read_dir(SYSTEM_ICONS.as_path())?.flatten() {
            if entry.file_type()?.is_dir() {
                std::fs::remove_dir_all(entry.path())?;
            } else if entry.file_name() != meta {
                std::fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }

    pub fn sanitize_system_icon_pack(&mut self, is_first_load: bool) -> Result<()> {
        // add default icon pack if not exists
        if !self.0.contains_key(SYSTEM_ICONS.as_path()) {
            let mut icon_pack = IconPack {
                id: "@system/icon-pack".into(),
                ..Default::default()
            };
            icon_pack.metadata.display_name = ResourceText::En("System".to_string());
            icon_pack.metadata.description =
                ResourceText::En("Icons from Windows and Program Files".to_string());
            icon_pack.metadata.internal.path = SYSTEM_ICONS.to_path_buf();

            self.0
                .insert(icon_pack.metadata.internal.path.clone(), icon_pack);
            self.write_system_icon_pack()?;
        }

        let system_pack = self.get_system_mut();
        let missing_path = SYSTEM_ICONS.join("missing-icon.png");
        let start_path = SYSTEM_ICONS.join("start-menu-icon.svg");

        if is_first_load || !missing_path.exists() {
            std::fs::copy(
                VAR_COMMON
                    .app_resource_dir()
                    .join("static/icons/missing.png"),
                missing_path,
            )?;
        }

        if is_first_load || !start_path.exists() {
            std::fs::copy(
                VAR_COMMON
                    .app_resource_dir()
                    .join("static/icons/start-menu.svg"),
                start_path,
            )?;
        }

        system_pack.missing = Some(Icon {
            base: Some("missing-icon.png".to_owned()),
            ..Default::default()
        });

        let mut add_custom = |key: &str, value: &str| {
            system_pack.add_entry(IconPackEntry::Custom(CustomIconPackEntry {
                key: key.to_owned(),
                icon: Icon {
                    base: Some(value.to_owned()),
                    mask: Some(value.to_owned()),
                    ..Default::default()
                },
            }));
        };

        add_custom("@effect/taskbar::start-menu", "start-menu-icon.svg");
        Ok(())
    }

    pub fn write_system_icon_pack(&self) -> Result<()> {
        self.get_system().save()?;
        Ok(())
    }
}

impl FullState {
    pub fn emit_icon_packs(&self) -> Result<()> {
        get_app_handle().emit(
            FuncEvent::StateIconPacksChanged,
            trace_lock!(self.icon_packs()).list(),
        )?;
        Ok(())
    }

    pub(super) fn load_icons_packs(&mut self, is_first_load: bool) -> Result<()> {
        // Ensure system icon pack folder exists
        std::fs::create_dir_all(SYSTEM_ICONS.as_path())?;

        let entries = std::fs::read_dir(VAR_COMMON.user_icons_path())?;
        let mut icon_packs_manager = trace_lock!(self.icon_packs);
        icon_packs_manager.0.clear();

        for entry in entries.flatten() {
            let path = entry.path();
            let mut icon_pack = match IconPack::load(&path) {
                Ok(icon_pack) => icon_pack,
                Err(err) => {
                    // Skip system folder if metadata doesn't exist yet, will be created below
                    if entry.file_name() == "system" {
                        log::debug!("System icon pack metadata not found, will be created");
                    } else {
                        log::error!("Failed to load icon pack ({path:?}): {err:?}");
                    }
                    continue;
                }
            };

            icon_pack.metadata.internal.bundled = entry.file_name() == "system";
            icon_packs_manager
                .0
                .insert(icon_pack.metadata.internal.path.clone(), icon_pack);
        }

        icon_packs_manager.sanitize_system_icon_pack(is_first_load)?;
        Ok(())
    }
}
