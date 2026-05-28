use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use itertools::Itertools;
use lazy_static::lazy_static;
use tauri::Manager;
use windows::Win32::UI::Shell::FOLDERID_Windows;

use crate::{app::get_app_handle, windows_api::WindowsApi};

lazy_static! {
    pub static ref VAR_COMMON: Arc<VarCommon> = Arc::new(VarCommon::new());

    /**
     * Some UWP apps like WhatsApp are resized after be opened,
     * this list will be used to resize them back after a delay.
     */
    pub static ref FORCE_RETILING_AFTER_ADD: Vec<String> = ["WhatsApp"]
    .iter()
    .map(|x| x.to_string())
    .collect_vec();
}

pub static NATIVE_UI_POPUP_CLASSES: [&str; 3] = [
    "ForegroundStaging",            // Task Switching and Task View
    "XamlExplorerHostIslandWindow", // Task Switching, Task View and other popups
    "ControlCenterWindow",          // Windows 11 right panel with quick settings
];

/// 多应用启动器配置：一个 exe 通过特定 CLI 参数承载多个子应用。
/// 用于 UMID 推断、启动参数保留、relaunch 参数生成等场景。
pub struct MultiAppLauncherConfig {
    /// 启动器进程名（小写），如 "androwslauncher.exe"
    pub exe_name: &'static str,
    /// 子应用标识参数名，如 "--launch-pkg-name"
    pub arg_key: &'static str,
    /// 构造 UMID 时的前缀，如 "Tencent.Androws.Androws."
    pub umid_prefix: &'static str,
}

pub static MULTI_APP_LAUNCHERS: &[MultiAppLauncherConfig] = &[
    MultiAppLauncherConfig {
        exe_name: "androwslauncher.exe",
        arg_key: "--launch-pkg-name",
        umid_prefix: "Tencent.Androws.Androws.",
    },
    // 未来新增启动器只需在这里添加配置
];

/// 根据进程名查找启动器配置
pub fn find_launcher_by_exe(exe_name: &str) -> Option<&'static MultiAppLauncherConfig> {
    let lower = exe_name.to_lowercase();
    MULTI_APP_LAUNCHERS.iter().find(|c| c.exe_name == lower)
}

/// 根据 UMID 前缀查找启动器配置
pub fn find_launcher_by_umid(umid: &str) -> Option<&'static MultiAppLauncherConfig> {
    MULTI_APP_LAUNCHERS
        .iter()
        .find(|c| umid.starts_with(c.umid_prefix))
}

/// 从启动器参数中提取子应用标识值（支持带引号和不带引号）
pub fn extract_launcher_arg_value(args: &str, arg_key: &str) -> Option<String> {
    let pos = args.find(arg_key)?;
    let after_flag = args[pos + arg_key.len()..].trim_start();
    let value = if after_flag.starts_with('"') {
        after_flag[1..].split('"').next().unwrap_or("")
    } else {
        after_flag.split_whitespace().next().unwrap_or("")
    };
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

pub static OVERLAP_BLACK_LIST_BY_EXE: [&str; 4] = [
    "SnippingTool.exe", // Windows Snipping Tool
    "HnAppStore.exe",
    "hexin.exe",
    "FlashCenter.exe",
];

/// Never 模式下用于 MoveWindowByOverlap 的窗口标题白名单（大小写不敏感，子串匹配）。
/// 只有标题命中该列表的窗口才会进入移动逻辑。
///
/// 默认沿用旧的 exe 黑名单条目作为占位，实际使用时建议改成对应窗口标题关键词。
pub static OVERLAP_MOVE_WHITE_LIST_BY_WINDOW_TITLE: [&str; 5] = [
    "Flash Center",       // placeholder
    "登录到全部行情主站", // placeholder
    "应用推荐",           // placeholder
    "截图工具",           // placeholder
    "照片",
];

pub struct VarCommon {
    // general
    resource_dir: PathBuf,
    data_dir: PathBuf,
    cache_dir: PathBuf,
    temp_dir: PathBuf,
    // specifits
    settings: PathBuf,
    taskbar_items: PathBuf,
    icons: PathBuf,
    user_themes: PathBuf,
    bundled_themes: PathBuf,
    widgets: PathBuf,
    bundled_widgets: PathBuf,
    // system
    system_dir: PathBuf,
}

#[allow(dead_code)]
impl VarCommon {
    pub fn new() -> Self {
        let resolver = get_app_handle().path();

        let resource_dir = resolver.resource_dir().expect("Failed to get resource dir");
        let data_dir = resolver.app_data_dir().expect("Failed to get app data dir");
        let cache_dir = resolver.app_cache_dir().expect("Failed to get cache dir");
        let temp_dir = resolver
            .temp_dir()
            .expect("Failed to get temp dir")
            .join("taskbarAndAIbar");

        let system_dir =
            WindowsApi::known_folder(FOLDERID_Windows).expect("Failed to get system dir");

        Self {
            settings: data_dir.join("settings.json"),
            taskbar_items: data_dir.join("taskbar_items_v4.yml"),
            icons: data_dir.join("iconpacks"),
            user_themes: data_dir.join("themes"),
            bundled_themes: resource_dir.join("static/themes"),
            widgets: data_dir.join("widgets"),
            bundled_widgets: resource_dir.join("static/widgets"),
            // general
            data_dir,
            resource_dir,
            cache_dir,
            temp_dir,
            system_dir,
        }
    }

    pub fn app_resource_dir(&self) -> &Path {
        &self.resource_dir
    }

    pub fn app_data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn app_cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn app_temp_dir(&self) -> &Path {
        &self.temp_dir
    }

    /// Windows: `X:\Windows`
    pub fn system_dir(&self) -> &Path {
        &self.system_dir
    }

    pub fn settings_path(&self) -> &Path {
        &self.settings
    }

    pub fn taskbar_items_path(&self) -> &Path {
        &self.taskbar_items
    }

    pub fn user_icons_path(&self) -> &Path {
        &self.icons
    }

    pub fn user_themes_path(&self) -> &Path {
        &self.user_themes
    }

    pub fn bundled_themes_path(&self) -> &Path {
        &self.bundled_themes
    }

    pub fn user_widgets_path(&self) -> &Path {
        &self.widgets
    }

    pub fn bundled_widgets_path(&self) -> &Path {
        &self.bundled_widgets
    }
}
