use tauri::Manager;

use crate::{app::get_app_handle, error::Result, utils::constants::VAR_COMMON};

pub struct RestorationAndMigration;

impl RestorationAndMigration {
    pub fn recreate_user_folders() -> Result<()> {
        let path = get_app_handle().path();
        let data_path = path.app_data_dir()?;

        // temporal folder to group artifacts
        std::fs::create_dir_all(VAR_COMMON.app_temp_dir())?;

        let create_if_needed = move |folder: &str| -> Result<()> {
            let path = data_path.join(folder);
            if !path.exists() {
                std::fs::create_dir_all(path)?;
            }
            Ok(())
        };
        create_if_needed("themes")?;
        create_if_needed("iconpacks/system")?;
        create_if_needed("plugins")?;
        create_if_needed("widgets")?;

        Ok(())
    }

    pub fn run_full() -> Result<()> {
        Self::recreate_user_folders()?;
        Ok(())
    }
}
