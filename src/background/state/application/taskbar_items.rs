use std::{fs::OpenOptions, io::Write, path::PathBuf};

use libs_core::state::{PinnedTaskbarItemData, TaskbarItem, TaskbarItemSubtype, TaskbarItems};

use crate::{error::Result, modules::uwp::UwpManager, utils::constants::VAR_COMMON};

use super::FullState;

impl FullState {
    const YOYOCLAW_DEFAULT_PIN_MIGRATION: &'static str = "default_pin_yoyoclaw_v1.done";
    const YOYOCLAW_PATH: &'static str = r"C:\Program Files\HONOR\MagicClaw\HnMagicClawUI.exe";

    fn yoyo_claw_migration_path() -> PathBuf {
        VAR_COMMON
            .app_data_dir()
            .join("migrations")
            .join(Self::YOYOCLAW_DEFAULT_PIN_MIGRATION)
    }

    fn mark_yoyo_claw_migration_complete() -> Result<()> {
        let migration_path = Self::yoyo_claw_migration_path();
        if let Some(parent) = migration_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&migration_path, "done")?;
        Ok(())
    }

    fn is_yoyo_claw_item(item: &TaskbarItem) -> bool {
        let TaskbarItem::Pinned(data) = item else {
            return false;
        };

        let expected = Self::YOYOCLAW_PATH.to_ascii_lowercase();
        data.path.to_string_lossy().to_ascii_lowercase() == expected
            || data.relaunch_program.to_ascii_lowercase() == expected
            || data.display_name.eq_ignore_ascii_case("YOYOClaw")
    }

    fn ensure_yoyo_claw_pinned_once(&mut self) -> Result<bool> {
        let migration_path = Self::yoyo_claw_migration_path();
        if migration_path.exists() {
            return Ok(false);
        }

        let yoyo_claw_path = PathBuf::from(Self::YOYOCLAW_PATH);
        let already_pinned = self
            .taskbar_items
            .left
            .iter()
            .chain(self.taskbar_items.center.iter())
            .chain(self.taskbar_items.right.iter())
            .any(Self::is_yoyo_claw_item);

        if !already_pinned {
            if !yoyo_claw_path.exists() {
                log::info!(
                    "[TaskbarItems] Skipping one-time default pinned item YOYOClaw because executable does not exist: {}",
                    Self::YOYOCLAW_PATH
                );
                return Ok(false);
            }

            log::info!("[TaskbarItems] Injecting one-time default pinned item: YOYOClaw");
            self.taskbar_items
                .left
                .push(TaskbarItem::Pinned(PinnedTaskbarItemData {
                    id: "e6717ab2-ec8d-45e4-9288-f205f5d53162".to_string(),
                    subtype: TaskbarItemSubtype::App,
                    umid: None,
                    path: yoyo_claw_path,
                    relaunch_command: None,
                    relaunch_program: Self::YOYOCLAW_PATH.to_string(),
                    relaunch_args: None,
                    relaunch_in: None,
                    display_name: "YOYOClaw".to_string(),
                    icon_hash: None,
                    is_dir: false,
                    windows: Vec::new(),
                    is_approximately_square: Some(false),
                    pin_disabled: false,
                }));
        } else {
            log::info!("[TaskbarItems] YOYOClaw is already pinned; marking migration complete");
        }

        Ok(!already_pinned)
    }

    fn update_taskbar_items_paths(items: &mut [TaskbarItem]) {
        for item in items {
            match item {
                TaskbarItem::Pinned(data) | TaskbarItem::Temporal(data) => {
                    if let Some(umid) = &data.umid {
                        if let Ok(Some(app_path)) = UwpManager::get_app_path(umid) {
                            data.path = app_path;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn _read_taskbar_items(&mut self) -> Result<()> {
        if VAR_COMMON.taskbar_items_path().exists() {
            self.taskbar_items =
                serde_yaml::from_str(&std::fs::read_to_string(VAR_COMMON.taskbar_items_path())?)?;

            self.taskbar_items.sanitize();
            Self::update_taskbar_items_paths(&mut self.taskbar_items.left);
            Self::update_taskbar_items_paths(&mut self.taskbar_items.center);
            Self::update_taskbar_items_paths(&mut self.taskbar_items.right);

            // Fix: always write back to ensure StartMenu in left and RecycleBin in right
            let startmenu_in_left = self
                .taskbar_items
                .left
                .iter()
                .any(|item| matches!(item, TaskbarItem::StartMenu { .. }));
            let recycle_in_right = self
                .taskbar_items
                .right
                .iter()
                .any(|item| matches!(item, TaskbarItem::RecycleBin { .. }));
            log::info!(
                "[TaskbarItems] StartMenu in left: {}, RecycleBin in right: {}",
                startmenu_in_left,
                recycle_in_right
            );
            let yoyo_claw_inserted = self.ensure_yoyo_claw_pinned_once()?;
            // Always write on startup to ensure correct positions
            log::info!(
                "[TaskbarItems] Writing file on startup... yoyo_claw_inserted={}",
                yoyo_claw_inserted
            );
            self.write_taskbar_items(&self.taskbar_items)?;
            Self::mark_yoyo_claw_migration_complete()?;
        } else {
            self.taskbar_items.sanitize();
            Self::update_taskbar_items_paths(&mut self.taskbar_items.left);
            Self::update_taskbar_items_paths(&mut self.taskbar_items.center);
            Self::update_taskbar_items_paths(&mut self.taskbar_items.right);
            let yoyo_claw_inserted = self.ensure_yoyo_claw_pinned_once()?;
            log::info!(
                "[TaskbarItems] Writing default file on startup... yoyo_claw_inserted={}",
                yoyo_claw_inserted
            );
            self.write_taskbar_items(&self.taskbar_items)?;
            Self::mark_yoyo_claw_migration_complete()?;
        }
        Ok(())
    }

    pub(super) fn read_taskbar_items(&mut self) {
        if let Err(err) = self._read_taskbar_items() {
            log::error!("Failed to read taskbar items: {err}");
            Self::show_corrupted_state_to_user(VAR_COMMON.taskbar_items_path());
        }
    }

    pub fn write_taskbar_items(&self, items: &TaskbarItems) -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(VAR_COMMON.taskbar_items_path())?;
        file.write_all(serde_yaml::to_string(&items)?.as_bytes())?;
        file.flush()?;
        Ok(())
    }
}
