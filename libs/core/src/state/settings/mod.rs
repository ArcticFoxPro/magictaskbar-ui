/* In this file we use #[serde_alias(SnakeCase)] as backward compatibility from versions below v1.9.8 */
pub mod by_monitor;
pub mod by_theme;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_alias::serde_alias;
use ts_rs::TS;

use crate::{
    error::Result,
    resource::{IconPackId, ThemeId},
    state::{by_monitor::MonitorConfiguration, by_theme::ThemeSettings, ByWidgetSettings},
};

// ============== Fancy Toolbar Settings ==============

// ============== Taskbar Settings ==============

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum TaskbarMode {
    #[serde(alias = "Full-Width")]
    FullWidth,
    #[serde(alias = "Min-Content")]
    MinContent,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum TaskbarTemporalItemsVisibility {
    All,
    OnMonitor,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum TaskbarPinnedItemsVisibility {
    Always,
    WhenPrimary,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum HideMode {
    /// never hide
    Never,
    /// auto-hide always on
    Always,
    /// auto-hide only if is overlaped by the focused window
    #[serde(alias = "On-Overlap")]
    OnOverlap,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum TaskbarSide {
    Left,
    Right,
    Top,
    Bottom,
}

#[serde_alias(SnakeCase)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
pub struct TaskbarSettings {
    /// enable or disable the taskbar
    pub enabled: bool,
    /// Dock/Taskbar mode
    pub mode: TaskbarMode,
    /// When to hide the dock
    pub hide_mode: HideMode,
    /// Which temporal items to show on the dock instance (this can be overridden per monitor)
    pub temporal_items_visibility: TaskbarTemporalItemsVisibility,
    /// Determines is the pinned item should be shown or not (this can be overridden per monitor).
    pub pinned_items_visibility: TaskbarPinnedItemsVisibility,
    /// Dock position
    pub position: TaskbarSide,
    /// enable or disable the instance counter visibility on taskbar instance
    pub show_instance_counter: bool,
    /// enable or disable the window title visibility for opened apps
    pub show_window_title: bool,
    /// enable or disable separators visibility
    pub visible_separators: bool,
    /// item size in px
    pub size: u32,
    /// zoomed item size in px
    pub zoom_size: u32,
    /// Dock/Taskbar margin in px
    pub margin: u32,
    /// Dock/Taskbar padding in px
    pub padding: u32,
    /// space between items in px
    pub space_between_items: u32,
    /// delay to show the toolbar on Mouse Hover in milliseconds
    pub delay_to_show: u32,
    /// delay to hide the toolbar on Mouse Leave in milliseconds
    pub delay_to_hide: u32,
    /// show end task button on context menu (needs developer mode enabled)
    pub show_end_task: bool,
    /// icon backplate style
    pub icon_backplate_style: IconBackplateStyle,
    /// enable or disable zoom effect on taskbar items
    pub enable_zoom_effect: bool,
    /// zoom effect type: wave or single icon
    pub zoom_effect_type: TaskbarZoomEffectType,
}

impl Default for TaskbarSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: TaskbarMode::MinContent,
            hide_mode: HideMode::OnOverlap,
            position: TaskbarSide::Bottom,
            visible_separators: true,
            show_instance_counter: true,
            show_window_title: false,
            temporal_items_visibility: TaskbarTemporalItemsVisibility::All,
            pinned_items_visibility: TaskbarPinnedItemsVisibility::Always,
            size: 52,
            zoom_size: 70,
            margin: 0,
            padding: 10,
            space_between_items: 11,
            delay_to_show: 100,
            delay_to_hide: 500,
            show_end_task: false,
            icon_backplate_style: IconBackplateStyle::Transparent,
            enable_zoom_effect: true,
            zoom_effect_type: TaskbarZoomEffectType::Wave,
        }
    }
}

impl TaskbarSettings {
    /// total height or width of the dock, depending on the Position
    pub fn total_size(&self) -> u32 {
        self.size + (self.padding * 2) + (self.margin * 2)
    }
}

// ================= Launcher ================

// ======================== Final Settings Struct ===============================
#[serde_alias(SnakeCase)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[cfg_attr(feature = "gen-binds", ts(export))]
pub struct Settings {
    /// list of monitors and their configurations
    pub monitors_v3: HashMap<String, MonitorConfiguration>,
    /// list of selected themes as filename as backguard compatibility for versions before v2.3.8, will be removed in v3
    #[serde(alias = "selectedThemes")]
    pub old_active_themes: Vec<String>,
    /// list of selected themes
    pub active_themes: Vec<ThemeId>,
    /// list of selected icon packs
    pub active_icon_packs: Vec<IconPackId>,
    /// enable or disable dev tools tab in settings
    pub dev_tools: bool,
    /// language to use, if null the system locale is used
    pub language: Option<String>,
    /// MomentJS date format
    pub date_format: String,
    /// Taskbar settings
    pub taskbar: TaskbarSettings, //zhang
    /// Widgets specific settings (e.g., Fancy Toolbar)
    pub by_widget: ByWidgetSettings,
    /// Custom variables for themes by theme id
    /// ### example
    /// ```json
    /// {
    ///     "@username/themeName": {
    ///         "--css-variable-name": "123px",
    ///         "--css-variable-name2": "#aabbccaa",
    ///     }
    /// }
    /// ```
    pub by_theme: HashMap<ThemeId, ThemeSettings>,
    /// Performance options
    pub performance_mode: PerformanceModeSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            performance_mode: PerformanceModeSettings::default(),

            old_active_themes: Vec::new(),
            active_themes: vec!["@default/theme".into()],
            active_icon_packs: vec!["@system/icon-pack".into()],
            monitors_v3: HashMap::new(),
            dev_tools: false,
            language: Some(Self::get_system_language()),
            date_format: "ddd D MMM, hh:mm A".to_owned(),
            taskbar: TaskbarSettings::default(), //zhang
            by_widget: ByWidgetSettings::default(),
            by_theme: HashMap::new(),
        }
    }
}

impl Settings {
    pub fn get_locale() -> Option<String> {
        sys_locale::get_locale()
    }

    pub fn get_system_language() -> String {
        match sys_locale::get_locale() {
            Some(l) => l.split('-').next().unwrap_or("en").to_string(),
            None => "en".to_string(),
        }
    }

    pub fn dedup_themes(&mut self) {
        let mut seen = HashSet::new();
        self.active_themes.retain(|x| seen.insert(x.clone())); // dedup
    }

    pub fn dedup_icon_packs(&mut self) {
        let mut seen = HashSet::new();
        self.active_icon_packs.retain(|x| seen.insert(x.clone())); // dedup
    }

    pub fn sanitize(&mut self) -> Result<()> {
        if self.language.is_none() {
            self.language = Some(Self::get_system_language());
        }

        // ensure base is always selected
        self.active_themes.insert(0, "@default/theme".into());
        self.dedup_themes();
        // ensure base is always selected
        self.active_icon_packs.insert(0, "@system/icon-pack".into());
        self.dedup_icon_packs();

        Ok(())
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let default_settings = Self::default();

        if path.exists() {
            match std::fs::read_to_string(path) {
                Ok(content) => match serde_json::from_str::<Self>(&content) {
                    Ok(mut settings) => {
                        let default_size = default_settings.taskbar.size;
                        if settings.taskbar.size != default_size {
                            log::info!(
                                "Settings size ({}) differs from default ({}), updating to default",
                                settings.taskbar.size,
                                default_size
                            );
                            settings.taskbar.size = default_size;

                            if let Err(e) = settings.save(path) {
                                log::error!("Failed to save updated settings: {e}");
                            }
                        }

                        settings.sanitize()?;
                        Ok(settings)
                    }
                    Err(err) => {
                        log::error!("Failed to parse settings file: {err}");
                        let mut settings = Self::default();
                        settings.sanitize()?;
                        Ok(settings)
                    }
                },
                Err(err) => {
                    log::error!("Failed to read settings file: {err}");
                    let mut settings = Self::default();
                    settings.sanitize()?;
                    Ok(settings)
                }
            }
        } else {
            let mut settings = Self::default();
            settings.sanitize()?;
            Ok(settings)
        }
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
pub struct PerformanceModeSettings {
    pub default: PerformanceMode,
}

impl Default for PerformanceModeSettings {
    fn default() -> Self {
        Self {
            default: PerformanceMode::Disabled,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum PerformanceMode {
    /// Does nothing, all animations are enabled.
    Disabled,
    /// Disables windows animations and other heavy effects.
    Minimal,
    /// Disables all the animations.
    Extreme,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum IconBackplateStyle {
    /// Use transparent backplate for icons
    Transparent,
    /// Use white backplate for icons
    White,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum TaskbarZoomEffectType {
    /// Wave magnification effect (affects neighboring icons)
    Wave,
    /// Single icon magnification effect (only affects hovered icon)
    SingleIcon,
}
