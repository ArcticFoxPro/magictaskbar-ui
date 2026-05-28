use std::{collections::HashSet, path::PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
pub struct TaskbarAppGroupItem {
    pub handle: isize,
    pub title: String,
    pub is_iconic: bool,
    pub is_zoomed: bool,
    /// last time the app was active, timestamp in milliseconds,
    /// could be 0 if we don't know when the app was actived
    pub last_active: u64,
    /// Window icon as base64-encoded PNG (ManagedShell-style)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_png_base64: Option<String>,
    /// Whether the icon is approximately square, used to decide if backplate is needed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_approximately_square: Option<bool>,
    /// Whether the icon is from local icon directory
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_from_local: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum TaskbarItemSubtype {
    File,
    Folder,
    App,
    /// this is used for backward compatibility, will be removed in v3
    #[default]
    UnknownV2_1_6,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(untagged)]
pub enum RelaunchArguments {
    Array(Vec<String>),
    String(String),
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
pub struct PinnedTaskbarItemData {
    /// internal UUID to differentiate items
    pub id: String,
    /// Subtype of the item (mandatory, but is optional for backward compatibility)
    pub subtype: TaskbarItemSubtype,
    /// Application user model id.
    pub umid: Option<String>,
    /// path to file, forder or program.
    pub path: PathBuf,
    /// @deprecaed will be removed in v3, use relaunch_program instead.
    #[ts(skip)]
    #[deprecated]
    #[serde(skip_serializing)]
    pub relaunch_command: Option<String>,
    /// program to be executed
    pub relaunch_program: String,
    /// arguments to be passed to the relaunch program
    pub relaunch_args: Option<RelaunchArguments>,
    /// path where ejecute the relaunch command
    pub relaunch_in: Option<PathBuf>,
    /// display name of the item
    pub display_name: String,
    /// Hash of the icon image (used by tray icons to force frontend refresh)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_hash: Option<String>,
    ///@deprecaed will be removed in v3, use subtype `Folder` instead.
    #[ts(skip)]
    #[serde(skip_serializing)]
    #[deprecated]
    pub is_dir: bool,
    /// Window handles in the app group, in case of pinned file/dir always will be empty
    #[serde(skip_deserializing)]
    pub windows: Vec<TaskbarAppGroupItem>,
    /// 判断图标是否近似正方形（由后端计算并传递，解决托盘图标白框显示问题）
    pub is_approximately_square: Option<bool>,
    /// This intention is to prevent pinned state change, when this is neccesary
    pub pin_disabled: bool,
}

impl PinnedTaskbarItemData {
    pub fn set_pin_disabled(&mut self, pin_disabled: bool) {
        self.pin_disabled = pin_disabled;
    }

    /// Some apps changes of place on update, commonly this contains an App User Model Id
    /// the path should be updated to the new location on these cases.
    pub fn should_ensure_path(&self) -> bool {
        let path_str = self.path.to_string_lossy();
        if path_str.starts_with("::") {
            return false;
        }
        // 对于通用安装程序（setup.exe, install.exe 等），不检查路径存在性
        // 因为它们通常从临时目录运行，路径可能在安装过程中失效
        if let Some(exe_name) = self.path.file_name().and_then(|n| n.to_str()) {
            let lower = exe_name.to_lowercase();
            const GENERIC_INSTALLERS: &[&str] = &["setup.exe"];
            if GENERIC_INSTALLERS.iter().any(|name| lower == *name) {
                return false;
            }
        }
        self.umid.is_none() || self.path.extension().is_some_and(|ext| ext == "lnk")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[cfg_attr(feature = "gen-binds", ts(export))]
#[serde(tag = "type")]
pub enum TaskbarItem {
    #[serde(alias = "PinnedApp")]
    Pinned(PinnedTaskbarItemData),
    Temporal(PinnedTaskbarItemData),
    Separator {
        id: String,
    },
    StartMenu {
        id: String,
    },
    RecycleBin {
        id: String,
        #[serde(default)]
        is_empty: bool,
    },
    SystemTray {
        id: String,
    },
}

impl TaskbarItem {
    pub fn id(&self) -> &String {
        match self {
            TaskbarItem::Pinned(data) => &data.id,
            TaskbarItem::Temporal(data) => &data.id,
            TaskbarItem::Separator { id }
            | TaskbarItem::StartMenu { id }
            | TaskbarItem::SystemTray { id } => id,
            TaskbarItem::RecycleBin { id, .. } => id,
        }
    }

    fn set_id(&mut self, identifier: String) {
        match self {
            TaskbarItem::Pinned(data) => data.id = identifier,
            TaskbarItem::Temporal(data) => data.id = identifier,
            TaskbarItem::Separator { id }
            | TaskbarItem::StartMenu { id }
            | TaskbarItem::SystemTray { id } => *id = identifier,
            TaskbarItem::RecycleBin { id, .. } => *id = identifier,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(default, rename_all = "camelCase")]
#[cfg_attr(feature = "gen-binds", ts(export))]
pub struct TaskbarItems {
    /// Whether the reordering possible on the taskbar
    pub is_reorder_disabled: bool,
    pub left: Vec<TaskbarItem>,
    pub center: Vec<TaskbarItem>,
    pub right: Vec<TaskbarItem>,
}

#[allow(deprecated)]
impl Default for TaskbarItems {
    fn default() -> Self {
        Self {
            is_reorder_disabled: false,
            left: vec![
                TaskbarItem::StartMenu { id: String::new() },
                // 设置
                TaskbarItem::Pinned(PinnedTaskbarItemData {
                    id: String::new(),
                    umid: Some("windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel".to_string()),
                    subtype: TaskbarItemSubtype::App,
                    path: PathBuf::new(),
                    display_name: "设置".into(),
                    relaunch_command: None,
                    relaunch_program: String::new(),
                    relaunch_args: Some(RelaunchArguments::String("shell:AppsFolder\\windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel".to_string())),
                    relaunch_in: None,
                    icon_hash: None,
                    is_approximately_square: Some(false),
                    is_dir: false,
                    windows: vec![],
                    pin_disabled: false,
                }),
                // 文件资源管理器
                TaskbarItem::Pinned(PinnedTaskbarItemData {
                    id: String::new(),
                    umid: Some("Microsoft.Windows.Explorer".to_string()),
                    subtype: TaskbarItemSubtype::App,
                    path: "C:\\Windows\\explorer.exe".into(),
                    display_name: "app_menu.explorer".into(),
                    relaunch_command: None,
                    relaunch_program: "C:\\Windows\\explorer.exe".into(),
                    relaunch_args: None,
                    relaunch_in: None,
                    icon_hash: None,
                    is_approximately_square: None,
                    is_dir: false,
                    windows: vec![],
                    pin_disabled: false,
                }),
                // Microsoft Edge
                TaskbarItem::Pinned(PinnedTaskbarItemData {
                    id: String::new(),
                    umid: Some("MSEdge".to_string()),
                    subtype: TaskbarItemSubtype::App,
                    path: PathBuf::new(),
                    display_name: "Microsoft Edge".into(),
                    relaunch_command: None,
                    relaunch_program: "msedge.exe".to_string(),
                    relaunch_args: Some(RelaunchArguments::String(
                        "shell:AppsFolder\\MSEdge".to_string(),
                    )),
                    relaunch_in: None,
                    icon_hash: None,
                    is_approximately_square: Some(true),
                    is_dir: false,
                    windows: vec![],
                    pin_disabled: false,
                }),
                // Microsoft Word
                TaskbarItem::Pinned(PinnedTaskbarItemData {
                    id: String::new(),
                    umid: None,
                    subtype: TaskbarItemSubtype::App,
                    path: "C:\\Program Files\\Microsoft Office\\root\\Office16\\WINWORD.EXE".into(),
                    display_name: "Microsoft Word".into(),
                    relaunch_command: None,
                    relaunch_program:
                        "C:\\Program Files\\Microsoft Office\\root\\Office16\\WINWORD.EXE".into(),
                    relaunch_args: None,
                    relaunch_in: None,
                    icon_hash: None,
                    is_approximately_square: Some(true),
                    is_dir: false,
                    windows: vec![],
                    pin_disabled: false,
                }),
                // Microsoft Excel
                TaskbarItem::Pinned(PinnedTaskbarItemData {
                    id: String::new(),
                    umid: None,
                    subtype: TaskbarItemSubtype::App,
                    path: "C:\\Program Files\\Microsoft Office\\root\\Office16\\EXCEL.EXE".into(),
                    display_name: "Microsoft Excel".into(),
                    relaunch_command: None,
                    relaunch_program:
                        "C:\\Program Files\\Microsoft Office\\root\\Office16\\EXCEL.EXE".into(),
                    relaunch_args: None,
                    relaunch_in: None,
                    icon_hash: None,
                    is_approximately_square: Some(true),
                    is_dir: false,
                    windows: vec![],
                    pin_disabled: false,
                }),
                // Microsoft PowerPoint
                TaskbarItem::Pinned(PinnedTaskbarItemData {
                    id: String::new(),
                    umid: None,
                    subtype: TaskbarItemSubtype::App,
                    path: "C:\\Program Files\\Microsoft Office\\root\\Office16\\POWERPNT.EXE"
                        .into(),
                    display_name: "Microsoft PowerPoint".into(),
                    relaunch_command: None,
                    relaunch_program:
                        "C:\\Program Files\\Microsoft Office\\root\\Office16\\POWERPNT.EXE".into(),
                    relaunch_args: None,
                    relaunch_in: None,
                    icon_hash: None,
                    is_approximately_square: Some(true),
                    is_dir: false,
                    windows: vec![],
                    pin_disabled: false,
                }),
                // AI 助手
                TaskbarItem::Pinned(PinnedTaskbarItemData {
                    id: String::new(),
                    umid: None,
                    subtype: TaskbarItemSubtype::App,
                    path: "C:\\Program Files\\HONOR\\HNMagicAI\\AIAssistantMain.exe".into(),
                    display_name: "app_menu.ai_assistant".into(),
                    relaunch_command: None,
                    relaunch_program: "C:\\Program Files\\HONOR\\HNMagicAI\\AIAssistantMain.exe"
                        .into(),
                    relaunch_args: None,
                    relaunch_in: None,
                    icon_hash: None,
                    is_approximately_square: Some(true),
                    is_dir: false,
                    windows: vec![],
                    pin_disabled: false,
                }),
                // AI 搜索
                TaskbarItem::Pinned(PinnedTaskbarItemData {
                    id: String::new(),
                    umid: None,
                    subtype: TaskbarItemSubtype::App,
                    path: "C:\\Program Files\\HONOR\\HNMagicAI\\AISearchUI.exe".into(),
                    display_name: "app_menu.ai_search".into(),
                    relaunch_command: None,
                    relaunch_program: "C:\\Program Files\\HONOR\\HNMagicAI\\AISearchUI.exe".into(),
                    relaunch_args: None,
                    relaunch_in: None,
                    icon_hash: None,
                    is_approximately_square: None,
                    is_dir: false,
                    windows: vec![],
                    pin_disabled: false,
                }),
                // 电脑管家
                TaskbarItem::Pinned(PinnedTaskbarItemData {
                    id: String::new(),
                    umid: None,
                    subtype: TaskbarItemSubtype::App,
                    path: "C:\\Program Files\\HONOR\\PCManager\\PCManager.exe".into(),
                    display_name: "app_menu.pc_manager".into(),
                    relaunch_command: None,
                    relaunch_program: "C:\\Program Files\\HONOR\\PCManager\\PCManager.exe".into(),
                    relaunch_args: None,
                    relaunch_in: None,
                    icon_hash: None,
                    is_approximately_square: Some(false),
                    is_dir: false,
                    windows: vec![],
                    pin_disabled: false,
                }),
                // 荣耀超级工作台
                TaskbarItem::Pinned(PinnedTaskbarItemData {
                    id: String::new(),
                    umid: None,
                    subtype: TaskbarItemSubtype::App,
                    path: "C:\\Program Files\\Hihonornote\\Hihonornote.exe".into(),
                    display_name: "app_menu.hn_note".into(),
                    relaunch_command: None,
                    relaunch_program: "C:\\Program Files\\Hihonornote\\Hihonornote.exe".into(),
                    relaunch_args: None,
                    relaunch_in: None,
                    icon_hash: None,
                    is_approximately_square: Some(false),
                    is_dir: false,
                    windows: vec![],
                    pin_disabled: false,
                }),
                // 荣耀应用商店
                TaskbarItem::Pinned(PinnedTaskbarItemData {
                    id: String::new(),
                    umid: None,
                    subtype: TaskbarItemSubtype::App,
                    path: "C:\\Program Files (x86)\\HnAppStore\\HnAppStore.exe".into(),
                    display_name: "app_menu.hn_app_store".into(),
                    relaunch_command: None,
                    relaunch_program: "C:\\Program Files (x86)\\HnAppStore\\HnAppStore.exe".into(),
                    relaunch_args: None,
                    relaunch_in: None,
                    icon_hash: None,
                    is_approximately_square: Some(false),
                    is_dir: false,
                    windows: vec![],
                    pin_disabled: false,
                }),
                // RetailLauncher
                TaskbarItem::Pinned(PinnedTaskbarItemData {
                    id: "RetailLauncher".to_string(),
                    umid: None,
                    subtype: TaskbarItemSubtype::App,
                    path: "C:\\Users\\Public\\Desktop\\RetailLauncher.lnk".into(),
                    display_name: "RetailLauncher".into(),
                    relaunch_command: None,
                    relaunch_program: "C:\\Program Files\\HONOR\\HonorRetail\\RetailLauncher.exe".into(),
                    relaunch_args: None,
                    relaunch_in: None,
                    icon_hash: None,
                    is_approximately_square: Some(false),
                    is_dir: false,
                    windows: vec![],
                    pin_disabled: false,
                }),
            ],
            center: Vec::new(),
            right: vec![TaskbarItem::RecycleBin {
                id: "recycle-bin".to_string(),
                is_empty: true,
            }],
        }
    }
}

#[allow(deprecated)]
impl TaskbarItems {
    fn get_parts_of_deprecated_inline_command(cmd: &str) -> (String, String) {
        let start_double_quoted = cmd.starts_with("\"");
        if start_double_quoted || cmd.starts_with("'") {
            let delimiter = if start_double_quoted { '"' } else { '\'' };
            let mut parts = cmd.split(['"', '\'']).filter(|s| !s.is_empty());

            let program = parts.next().unwrap_or_default().trim().to_owned();
            let args = cmd
                .trim_start_matches(&format!("{delimiter}{program}{delimiter}"))
                .trim()
                .to_owned();
            (program, args)
        } else {
            (cmd.trim().to_string(), String::new())
        }
    }

    fn sanitize_items(dict: &mut HashSet<String>, items: Vec<TaskbarItem>) -> Vec<TaskbarItem> {
        let mut result = Vec::new();
        for mut item in items {
            match &mut item {
                TaskbarItem::Pinned(data) => {
                    // UWP 应用有 umid，不需要检查物理路径是否存在
                    // Win32 应用需要检查路径
                    let has_umid = data.umid.is_some();
                    let path_exists = !data.path.as_os_str().is_empty()
                        && (!data.should_ensure_path() || data.path.exists());
                    let relaunch_exists = !data.relaunch_program.is_empty()
                        && std::path::Path::new(&data.relaunch_program).exists();
                    if !has_umid && !path_exists && !relaunch_exists {
                        continue;
                    }
                    // 如果原始路径（如 .lnk）已删除但 relaunch_program 有效，
                    // 更新 path 为 relaunch_program，避免后续 sanitize 反复检查已删除的路径
                    if !path_exists && relaunch_exists {
                        data.path = std::path::PathBuf::from(&data.relaunch_program);
                    }

                    // migration step for items before v2.1.6
                    if data.subtype == TaskbarItemSubtype::UnknownV2_1_6 {
                        data.subtype = if data.is_dir {
                            TaskbarItemSubtype::Folder
                        } else if data
                            .path
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
                            || std::path::Path::new(&data.relaunch_program)
                                .extension()
                                .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
                            || data
                                .relaunch_command
                                .as_ref()
                                .is_some_and(|r| r.to_lowercase().contains(".exe"))
                        {
                            TaskbarItemSubtype::App
                        } else {
                            TaskbarItemSubtype::File
                        };
                    }

                    // migration step for items before v2.2.6
                    if let Some(old_command) = data.relaunch_command.take() {
                        if data.relaunch_program.is_empty() {
                            let (program, args) =
                                Self::get_parts_of_deprecated_inline_command(&old_command);
                            data.relaunch_program = program;
                            if !args.is_empty() {
                                data.relaunch_args = Some(RelaunchArguments::String(args));
                            }
                        }
                    }

                    // 只有非托盘图标才执行路径检查
                    if data.relaunch_program.is_empty() {
                        if data
                            .umid
                            .as_ref()
                            .map_or(false, |umid| umid.eq_ignore_ascii_case("MSEdge"))
                        {
                            data.relaunch_program = [
                                r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
                                r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
                            ]
                            .iter()
                            .find(|path| PathBuf::from(path).exists())
                            .map(|path| path.to_string())
                            .unwrap_or_else(|| "msedge.exe".to_string());
                        } else {
                            data.relaunch_program = data.path.to_string_lossy().to_string();
                        }
                    }
                }
                TaskbarItem::Temporal(data) => {
                    // Remove temporal items without any open windows
                    if data.windows.is_empty() {
                        continue;
                    }
                    // Check path validity
                    // 🔧 修复：有 UMID 且有窗口的临时项（如 Edge），不检查路径
                    // 因为这类项的 path 可能为空，但窗口存在说明程序在运行
                    let skip_path_check = data.umid.is_some() && !data.windows.is_empty();
                    if !skip_path_check
                        && (data.path.as_os_str().is_empty()
                            || (data.should_ensure_path() && !data.path.exists()))
                    {
                        continue;
                    }
                    if data.relaunch_program.is_empty() {
                        data.relaunch_program = data.path.to_string_lossy().to_string();
                    }
                }
                _ => {}
            }

            if item.id().is_empty() {
                item.set_id(uuid::Uuid::new_v4().to_string());
            }

            if !dict.contains(item.id()) {
                dict.insert(item.id().clone());
                result.push(item);
            }
        }
        result
    }

    pub fn sanitize(&mut self) {
        let mut dict = HashSet::new();

        // Step 1: Process pinned items first and collect their identifying info
        // This builds the dict with all pinned item IDs
        self.left = Self::sanitize_items(&mut dict, std::mem::take(&mut self.left));

        // Step 2: Build a map of pinned items by their UMID and relaunch_program
        // For matching: prioritize UMID, then full path, then exe name (only if no UMID)
        let mut pinned_identifiers = HashSet::new();
        let mut pinned_with_umid = HashSet::new();
        for item in &self.left {
            if let TaskbarItem::Pinned(data) = item {
                // Track which pinned items have UMID
                if let Some(umid) = &data.umid {
                    pinned_identifiers.insert(format!("umid:{}", umid));
                    pinned_with_umid.insert(format!("program:{}", data.relaunch_program));
                }
                // Add relaunch_program as identifier
                // Special case: for explorer.exe, only add as program identifier if it's the real File Explorer
                // or if it doesn't have a UMID (to avoid UWP apps which use explorer.exe matching everything)
                let exe_name = std::path::Path::new(&data.relaunch_program)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                let is_explorer = exe_name == "explorer.exe";
                let is_real_explorer = is_explorer
                    && data
                        .umid
                        .as_ref()
                        .map_or(false, |u| u == "Microsoft.Windows.Explorer");

                if !is_explorer || is_real_explorer || data.umid.is_none() {
                    pinned_identifiers.insert(format!("program:{}", data.relaunch_program));
                }

                // Add exe name only if no UMID (to avoid false matches with explorer.exe)
                if data.umid.is_none() {
                    if let Some(exe_name) = std::path::Path::new(&data.relaunch_program)
                        .file_name()
                        .and_then(|n| n.to_str())
                    {
                        pinned_identifiers.insert(format!("exe:{}", exe_name.to_lowercase()));
                    }
                }
            }
        }

        // Step 3: Process center (temporal) items
        // If a temporal item matches a pinned item, merge its windows into the pinned item
        let center_items = std::mem::take(&mut self.center);
        let mut filtered_center = Vec::new();

        for mut item in center_items {
            if let TaskbarItem::Temporal(data) = &mut item {
                // Check if this temporal item corresponds to a pinned item
                // Priority: UMID match > program path match > exe name match (only if no UMID)
                let umid_match = if let Some(umid) = &data.umid {
                    pinned_identifiers.contains(&format!("umid:{}", umid))
                } else {
                    false
                };

                // Only use program_match if both the temporal and pinned items have no UMID
                // This prevents UWP apps (which share explorer.exe as relaunch_program) from incorrectly matching
                let program_match = if data.umid.is_none() {
                    pinned_identifiers.contains(&format!("program:{}", data.relaunch_program))
                } else {
                    false // Don't use program matching for UWP apps
                };

                // Only use exe name matching if temporal item has no UMID
                // This prevents UWP apps (with explorer.exe as relaunch_program) from matching folder pinned items
                let exe_match = if data.umid.is_none() {
                    if let Some(exe_name) = std::path::Path::new(&data.relaunch_program)
                        .file_name()
                        .and_then(|n| n.to_str())
                    {
                        pinned_identifiers.contains(&format!("exe:{}", exe_name.to_lowercase()))
                    } else {
                        false
                    }
                } else {
                    false
                };

                // Check if display_name matches any pinned item (fallback for non-UWP apps)
                let display_name_match = if data.umid.is_none() {
                    // Check if there's a pinned item with the same display_name
                    self.left.iter().any(|item| {
                        if let TaskbarItem::Pinned(pinned_data) = item {
                            pinned_data.umid.is_none()
                                && pinned_data.display_name.to_lowercase()
                                    == data.display_name.to_lowercase()
                        } else {
                            false
                        }
                    })
                } else {
                    false
                };

                let matches_pinned = umid_match || program_match || exe_match || display_name_match;

                if matches_pinned {
                    // This temporal item matches a pinned item
                    // Transfer its windows to the corresponding pinned item
                    if !data.windows.is_empty() {
                        // Find the matching pinned item and transfer windows
                        for pinned_item in &mut self.left {
                            if let TaskbarItem::Pinned(pinned_data) = pinned_item {
                                // Match by UMID first
                                let umid_matches = if let Some(umid) = &data.umid {
                                    pinned_data.umid.as_ref() == Some(umid)
                                } else {
                                    false
                                };

                                // Match by full program path
                                let exe_name = std::path::Path::new(&pinned_data.relaunch_program)
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("")
                                    .to_lowercase();
                                let is_explorer = exe_name == "explorer.exe";
                                let is_real_explorer = is_explorer
                                    && pinned_data
                                        .umid
                                        .as_ref()
                                        .map_or(false, |u| u == "Microsoft.Windows.Explorer");

                                let program_matches = if is_explorer
                                    && !is_real_explorer
                                    && pinned_data.umid.is_some()
                                {
                                    // If pinned is a UWP app using explorer.exe, it must match by UMID or arguments
                                    false
                                } else {
                                    pinned_data.relaunch_program.to_lowercase()
                                        == data.relaunch_program.to_lowercase()
                                        && pinned_data.relaunch_args == data.relaunch_args
                                };

                                // Match by exe file name (only if both have no UMID)
                                let exe_name_matches =
                                    if pinned_data.umid.is_none() && data.umid.is_none() {
                                        let pinned_exe_name =
                                            std::path::Path::new(&pinned_data.relaunch_program)
                                                .file_name()
                                                .and_then(|n| n.to_str())
                                                .unwrap_or("")
                                                .to_lowercase();
                                        let temporal_exe_name =
                                            std::path::Path::new(&data.relaunch_program)
                                                .file_name()
                                                .and_then(|n| n.to_str())
                                                .unwrap_or("")
                                                .to_lowercase();
                                        !pinned_exe_name.is_empty()
                                            && !temporal_exe_name.is_empty()
                                            && pinned_exe_name == temporal_exe_name
                                    } else {
                                        false
                                    };
                                // Match by display_name (last resort for non-UWP apps)
                                let display_name_matches =
                                    if pinned_data.umid.is_none() && data.umid.is_none() {
                                        pinned_data.display_name.to_lowercase()
                                            == data.display_name.to_lowercase()
                                    } else {
                                        false
                                    };

                                if umid_matches
                                    || program_matches
                                    || exe_name_matches
                                    || display_name_matches
                                {
                                    // Transfer all windows from temporal item to pinned item
                                    pinned_data.windows.append(&mut data.windows);

                                    // If temporal item has UMID but pinned item doesn't, update the pinned item
                                    if data.umid.is_some() && pinned_data.umid.is_none() {
                                        pinned_data.umid = data.umid.clone();
                                        // Also update relaunch_program and relaunch_args for UWP apps
                                        if let Some(ref umid_str) = data.umid {
                                            pinned_data.relaunch_program =
                                                "explorer.exe".to_string();
                                            pinned_data.relaunch_args =
                                                Some(RelaunchArguments::String(format!(
                                                    "shell:AppsFolder\\{}",
                                                    umid_str
                                                )));
                                        }
                                    }

                                    break;
                                }
                            }
                        }
                    }
                    // Skip this temporal item (it's now merged with pinned item)
                    continue;
                }

                // Try to merge with the temporary items that have already been processed ---
                let mut merged = false;
                for existing in &mut filtered_center {
                    if let TaskbarItem::Temporal(existing_data) = existing {
                        // 1. UMID matching (highest priority)
                        let umid_matches = match (&data.umid, &existing_data.umid) {
                            (Some(u1), Some(u2)) => u1 == u2,
                            _ => false,
                        };

                        // 2. Program path and parameter matching
                        let exe_name = std::path::Path::new(&data.relaunch_program)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .to_lowercase();
                        let is_explorer = exe_name == "explorer.exe";
                        let is_real_explorer = is_explorer
                            && data
                                .umid
                                .as_ref()
                                .map_or(false, |u| u == "Microsoft.Windows.Explorer");
                        let is_uwp_explorer =
                            is_explorer && data.umid.is_some() && !is_real_explorer;

                        let program_matches = if is_uwp_explorer {
                            false // UWP explorer must match through UMID
                        } else {
                            data.relaunch_program.to_lowercase()
                                == existing_data.relaunch_program.to_lowercase()
                                && data.relaunch_args == existing_data.relaunch_args
                        };

                        // 3. Display name matching without UMID (fallback solution)
                        let display_name_matches = data.umid.is_none()
                            && existing_data.umid.is_none()
                            && data.display_name.to_lowercase()
                                == existing_data.display_name.to_lowercase();

                        // 4. 特别处理优酷: 优酷播放器窗口（YoukuPlayer/YoukuNPlayer/ykplayer）应该匹配优酷主窗口
                        let is_youku_existing = existing_data
                            .relaunch_program
                            .to_lowercase()
                            .contains("youku")
                            || existing_data
                                .relaunch_program
                                .to_lowercase()
                                .contains("ykplayer")
                            || existing_data.display_name.to_lowercase().contains("优酷")
                            || existing_data.display_name.to_lowercase().contains("youku")
                            || existing_data
                                .display_name
                                .to_lowercase()
                                .contains("ykplayer");
                        let is_youku_current =
                            data.relaunch_program.to_lowercase().contains("youku")
                                || data.relaunch_program.to_lowercase().contains("ykplayer")
                                || data.display_name.to_lowercase().contains("youku")
                                || data.display_name.to_lowercase().contains("ykplayer");
                        let youku_matches = is_youku_existing && is_youku_current;

                        if umid_matches || program_matches || display_name_matches || youku_matches
                        {
                            existing_data.windows.append(&mut data.windows);
                            merged = true;
                            break;
                        }
                    }
                }
                if merged {
                    continue;
                }

                // This is a standalone temporal item (not pinned)
                // Remove if no windows
                if data.windows.is_empty() {
                    continue;
                }

                // Check path validity
                let skip_path_check = data.umid.is_some() && !data.windows.is_empty();
                if !skip_path_check
                    && (data.path.as_os_str().is_empty()
                        || (data.should_ensure_path() && !data.path.exists()))
                {
                    continue;
                }

                // Ensure relaunch_program is set
                if data.relaunch_program.is_empty() {
                    data.relaunch_program = data.path.to_string_lossy().to_string();
                }

                // Ensure ID is set
                if item.id().is_empty() {
                    item.set_id(uuid::Uuid::new_v4().to_string());
                }

                // Check for duplicate ID
                if !dict.contains(item.id()) {
                    dict.insert(item.id().clone());
                    filtered_center.push(item);
                }
            } else {
                // Not a temporal item, keep it
                if item.id().is_empty() {
                    item.set_id(uuid::Uuid::new_v4().to_string());
                }
                if !dict.contains(item.id()) {
                    dict.insert(item.id().clone());
                    filtered_center.push(item);
                }
            }
        }

        self.center = filtered_center;

        // Step 4: Process right items
        self.right = Self::sanitize_items(&mut dict, std::mem::take(&mut self.right));

        // Step 5: Ensure StartMenu is in left area
        // 先从所有区域移除 StartMenu，然后在 left 添加唯一的一个
        self.left
            .retain(|item| !matches!(item, TaskbarItem::StartMenu { .. }));
        self.center
            .retain(|item| !matches!(item, TaskbarItem::StartMenu { .. }));
        self.right
            .retain(|item| !matches!(item, TaskbarItem::StartMenu { .. }));

        // 然后在 left 区域添加唯一的 StartMenu
        self.left
            .insert(0, TaskbarItem::StartMenu { id: String::new() });

        // Step 6: Ensure Recycle Bin exists in right area
        // 需要移除所有多余的 RecycleBin，只保留一个
        self.left
            .retain(|item| !matches!(item, TaskbarItem::RecycleBin { .. }));
        self.center
            .retain(|item| !matches!(item, TaskbarItem::RecycleBin { .. }));
        self.right
            .retain(|item| !matches!(item, TaskbarItem::RecycleBin { .. }));

        // 然后添加唯一的 RecycleBin（状态由调用者负责更新）
        self.right.push(TaskbarItem::RecycleBin {
            id: "recycle-bin".to_string(),
            is_empty: true, // 默认为 true，由后端在调用后更新
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::state::TaskbarItems;

    #[test]
    fn should_return_empty_response_for_empty_command() {
        let (program, args) = TaskbarItems::get_parts_of_deprecated_inline_command("");
        assert_eq!(program, "");
        assert_eq!(args, "");
    }

    #[test]
    fn should_parse_a_simple_command_without_arguments() {
        let (program, args) = TaskbarItems::get_parts_of_deprecated_inline_command("node");
        assert_eq!(program, "node");
        assert_eq!(args, "");
    }

    #[test]
    fn should_parse_a_quoted_program_path_without_splitting_args() {
        let (program, args) = TaskbarItems::get_parts_of_deprecated_inline_command(
            "\"C:\\Program Files\\node.exe\" script.js",
        );
        assert_eq!(program, "C:\\Program Files\\node.exe");
        assert_eq!(args, "script.js");
    }

    #[test]
    fn should_parse_a_single_quoted_program_path_without_splitting_args() {
        let (program, args) =
            TaskbarItems::get_parts_of_deprecated_inline_command("'/usr/local/bin/node' script.js");
        assert_eq!(program, "/usr/local/bin/node");
        assert_eq!(args, "script.js");
    }

    #[test]
    fn should_handle_program_path_with_spaces_without_quotes() {
        let (program, args) = TaskbarItems::get_parts_of_deprecated_inline_command(
            "C:\\Program Files\\node.exe script.js",
        );
        assert_eq!(program, "C:\\Program Files\\node.exe script.js");
        assert_eq!(args, "");
    }

    #[test]
    fn should_handle_command_with_only_quoted_program_and_no_args() {
        let (program, args) =
            TaskbarItems::get_parts_of_deprecated_inline_command("\"C:\\Program Files\\node.exe\"");
        assert_eq!(program, "C:\\Program Files\\node.exe");
        assert_eq!(args, "");
    }

    #[test]
    fn should_preserve_all_spaces_between_arguments() {
        let (program, args) =
            TaskbarItems::get_parts_of_deprecated_inline_command("node  script.js   arg1   arg2");
        assert_eq!(program, "node  script.js   arg1   arg2");
        assert_eq!(args, "");
    }

    #[test]
    fn should_trim_spaces_from_program() {
        let (program, args) = TaskbarItems::get_parts_of_deprecated_inline_command("node    ");
        assert_eq!(program, "node");
        assert_eq!(args, "");
    }

    #[test]
    fn should_handle_complex_quoted_arguments_as_single_string() {
        let (program, args) = TaskbarItems::get_parts_of_deprecated_inline_command(
            "node \"arg with spaces\" 'another arg' --flag=\"value\"",
        );
        assert_eq!(
            program,
            "node \"arg with spaces\" 'another arg' --flag=\"value\""
        );
        assert_eq!(args, "");
    }

    #[test]
    fn should_handle_complex_quoted_arguments_as_single_string_2() {
        let (program, args) = TaskbarItems::get_parts_of_deprecated_inline_command(
            "\"node\" \"arg with spaces\" 'another arg' --flag=\"value\"",
        );
        assert_eq!(program, "node");
        assert_eq!(args, "\"arg with spaces\" 'another arg' --flag=\"value\"");
    }
}
