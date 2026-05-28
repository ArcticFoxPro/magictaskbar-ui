pub mod builtin;
pub mod config;

use std::{collections::HashMap, path::Path};

use config::ThemeSettingsDefinition;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    error::Result,
    resource::{ResourceKind, ResourceMetadata, SluResource, ThemeId, WidgetId},
    utils::search_resource_entrypoint,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[cfg_attr(feature = "gen-binds", ts(export))]
pub struct Theme {
    pub id: ThemeId,
    /// Metadata about the theme
    #[serde(alias = "info")] // for backwards compatibility before v2.0
    pub metadata: ResourceMetadata,
    pub settings: ThemeSettingsDefinition,
    /// Css Styles of the theme
    pub styles: HashMap<WidgetId, String>,
    /// Shared css styles for all widgets, commonly used to set styles
    /// for the components library
    pub shared_styles: String,
}

impl SluResource for Theme {
    const KIND: ResourceKind = ResourceKind::Theme;

    fn metadata(&self) -> &ResourceMetadata {
        &self.metadata
    }

    fn metadata_mut(&mut self) -> &mut ResourceMetadata {
        &mut self.metadata
    }

    fn load_from_folder(path: &Path) -> Result<Theme> {
        let mut theme = Self::load_metadata(path)?;

        // 默认主题直接使用嵌入的 CSS，不读取文件系统
        if builtin::is_default_theme_id(&theme.id) {
            if let Some(css) = builtin::get_builtin_shared_css() {
                theme.shared_styles = css.to_string();
            }
            if let Some(css) = builtin::get_builtin_taskbar_css() {
                // 前端使用 @effect/taskbar 作为 widgetId
                theme
                    .styles
                    .insert(WidgetId::from("@effect/taskbar"), css.to_string());
            }
            return Ok(theme);
        }

        Ok(theme)
    }
}

impl Theme {
    /// 加载主题的元数据文件
    fn load_metadata(path: &Path) -> Result<Theme> {
        let file = search_resource_entrypoint(path).unwrap_or_else(|| path.join("theme.yml"));
        Self::load_from_file(&file)
    }
}
