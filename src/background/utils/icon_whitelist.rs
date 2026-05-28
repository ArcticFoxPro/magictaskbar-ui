use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use libs_core::state::IconBackplateStyle;

/// 通用安装程序列表 - 这些程序易重名，需要特殊的路径感知处理
const GENERIC_INSTALLERS: &[&str] = &[
    "setup.exe",
    "install.exe",
    "uninstall.exe",
    "launcher.exe",
    "browser.exe",
];

/// 判断是否为通用安装程序
pub fn is_generic_installer(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    // browser.exe 也需要特殊处理，防止wegame的browser.exe被错误匹配到其他浏览器
    GENERIC_INSTALLERS.iter().any(|name| lower == *name)
}

/// 进程白名单条目
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessWhitelistEntry {
    pub name: String,             // 进程名称
    pub requires_backplate: bool, // 是否需要显示背板
}

/// 窗口白名单条目
#[derive(Debug, Clone, PartialEq)]
pub struct WindowWhitelistEntry {
    pub class: String,
    pub titles: Vec<String>,      // 支持多个标题匹配
    pub requires_backplate: bool, // 新增字段：是否需要显示背板
}

// 全局白名单变量
static PROCESS_WHITELIST: RwLock<Vec<ProcessWhitelistEntry>> = RwLock::new(Vec::new());
static WINDOW_WHITELIST: RwLock<Vec<WindowWhitelistEntry>> = RwLock::new(Vec::new());

/// 获取白名单配置文件路径
fn get_whitelist_config_path() -> String {
    // 使用相对于可执行文件的路径
    let path = crate::utils::get_app_dir()
        .join("static")
        .join("whitelist.txt");
    path.to_string_lossy().to_string()
}

/// 从配置文件加载白名单
fn load_whitelist_from_file(
) -> Result<(Vec<ProcessWhitelistEntry>, Vec<WindowWhitelistEntry>), Box<dyn std::error::Error>> {
    let config_path = get_whitelist_config_path();
    let path_obj = Path::new(&config_path);
    log::debug!(
        "Path object exists: {}, is file: {}",
        path_obj.exists(),
        path_obj.is_file()
    );

    let mut process_whitelist = Vec::new();
    let mut window_whitelist: Vec<WindowWhitelistEntry> = Vec::new();

    if path_obj.exists() {
        let content = fs::read_to_string(&config_path)?;
        log::debug!(
            "Successfully read file, content length: {}\nWhitelist file content:\n{}",
            content.len(),
            content
        );

        let lines: Vec<&str> = content.lines().collect();
        let mut in_process_section = false;
        let mut in_window_section = false;
        let mut current_class = String::new();
        let mut current_titles = Vec::new();
        let mut current_requires_backplate = false;
        let mut current_process_name = String::new();
        let mut process_requires_backplate = false;
        let mut in_process_entry = false;

        for line in lines {
            let trimmed = line.trim();

            // 跳过空行和注释
            if trimmed.is_empty() || trimmed.starts_with("//") {
                continue;
            }

            // 检查是否进入新的部分
            if trimmed.starts_with("let process = [") {
                in_process_section = true;
                in_window_section = false;
                continue;
            } else if trimmed.starts_with("let window = [") {
                in_process_section = false;
                in_window_section = true;
                continue;
            }

            // 根据当前部分处理条目
            if in_process_section {
                // 跳过结束符 "];" 和空行
                if trimmed == "];" || trimmed.is_empty() {
                    in_process_section = false;
                    continue;
                }

                // 匹配进程条目开始
                if trimmed.contains("{") {
                    in_process_entry = true;
                    // 检查是否是单行条目（如 {name: process.exe, requires_backplate:true},）
                    if trimmed.contains("}") {
                        // 提取 name 字段
                        if let Some(name_start) = trimmed.find("name:") {
                            let name_part = &trimmed[name_start + 5..];
                            let name_end = name_part.find(",").unwrap_or(name_part.len());
                            let process_name = name_part[..name_end].trim();
                            current_process_name =
                                if process_name.starts_with('"') && process_name.ends_with('"') {
                                    process_name[1..process_name.len() - 1].to_string()
                                } else {
                                    process_name.to_string()
                                };
                        }
                        // 提取 requires_backplate 字段
                        if let Some(rb_start) = trimmed.find("requires_backplate:") {
                            let rb_part = &trimmed[rb_start + 19..];
                            let rb_end = rb_part.find("}").unwrap_or(rb_part.len());
                            let rb_value = rb_part[..rb_end].trim();
                            process_requires_backplate = rb_value.parse().unwrap_or(false);
                        }
                        // 添加到白名单
                        if !current_process_name.is_empty() {
                            process_whitelist.push(ProcessWhitelistEntry {
                                name: current_process_name.clone(),
                                requires_backplate: process_requires_backplate,
                            });
                            log::debug!(
                                "Added process whitelist entry: {}, requires_backplate={}",
                                current_process_name,
                                process_requires_backplate
                            );
                            current_process_name = String::new();
                            process_requires_backplate = false;
                        }
                        in_process_entry = false;
                    }
                    continue;
                }

                // 匹配进程条目结束
                if trimmed.contains("}") {
                    if !current_process_name.is_empty() {
                        process_whitelist.push(ProcessWhitelistEntry {
                            name: current_process_name.clone(),
                            requires_backplate: process_requires_backplate,
                        });
                        log::debug!(
                            "Added process whitelist entry: {}, requires_backplate={}",
                            current_process_name,
                            process_requires_backplate
                        );
                        current_process_name = String::new();
                        process_requires_backplate = false;
                    }
                    in_process_entry = false;
                    continue;
                }

                if in_process_entry {
                    // 匹配 name: "process.exe", 或 name: process.exe,
                    if trimmed.starts_with("name:") {
                        let name_part = trimmed["name:".len()..].trim();
                        let name_without_suffix = if name_part.ends_with(",") {
                            &name_part[..name_part.len() - 1]
                        } else {
                            name_part
                        };

                        // 处理带引号和不带引号的情况
                        current_process_name = if name_without_suffix.starts_with('"')
                            && name_without_suffix.ends_with('"')
                        {
                            // 带引号的格式 "process.exe"
                            name_without_suffix[1..name_without_suffix.len() - 1].to_string()
                        } else {
                            // 不带引号的格式 process.exe
                            name_without_suffix.to_string()
                        };
                        continue;
                    }

                    // 匹配 requires_backplate: true,
                    if trimmed.starts_with("requires_backplate:") {
                        let value = trimmed["requires_backplate:".len()..].trim();
                        if let Some(stripped) = value.strip_suffix(",") {
                            process_requires_backplate = stripped.parse().unwrap_or(false);
                        } else {
                            process_requires_backplate = value.parse().unwrap_or(false);
                        }
                        continue;
                    }
                } else {
                    // 匹配简单格式：process.exe, 或 process.exe（不带逗号）
                    if !trimmed.contains("{") && !trimmed.contains("}") {
                        // 移除可能存在的末尾逗号
                        let process_entry = if trimmed.ends_with(",") {
                            trimmed[..trimmed.len() - 1].trim()
                        } else {
                            trimmed.trim()
                        };

                        // 跳过空条目（如只有逗号的行）
                        if process_entry.is_empty() {
                            continue;
                        }

                        let process_name =
                            if process_entry.starts_with('"') && process_entry.ends_with('"') {
                                // 带引号的格式 "process.exe"
                                process_entry[1..process_entry.len() - 1].to_string()
                            } else {
                                // 不带引号的格式 process.exe
                                process_entry.to_string()
                            };
                        process_whitelist.push(ProcessWhitelistEntry {
                            name: process_name.clone(),
                            requires_backplate: false, // 默认不需要背板
                        });
                        log::debug!("Added process whitelist entry: {}", process_name);
                    }
                }
                // 检查是否结束进程部分
                if trimmed == "];" {
                    in_process_section = false;
                }
            } else if in_window_section {
                // 匹配 WindowWhitelistEntry {
                if trimmed.contains("WindowWhitelistEntry") && trimmed.contains("{") {
                    current_class = String::new();
                    current_titles = Vec::new();
                    current_requires_backplate = false;
                    continue;
                }

                // 匹配 class: "CabinetWClass",
                if trimmed.starts_with("class:") {
                    // 提取引号之间的内容
                    if let Some(start_quote) = trimmed.find('"') {
                        if let Some(end_quote) = trimmed[start_quote + 1..].find('"') {
                            let class =
                                trimmed[start_quote + 1..start_quote + 1 + end_quote].to_string();
                            current_class = class.clone();
                        }
                    }
                    continue;
                }

                // 匹配标题列表中的条目
                if trimmed.contains('"')
                    && !trimmed.starts_with("class:")
                    && !trimmed.starts_with("requires_backplate:")
                {
                    // 提取引号之间的内容
                    if let Some(start_quote) = trimmed.find('"') {
                        if let Some(end_quote) = trimmed[start_quote + 1..].find('"') {
                            let title =
                                trimmed[start_quote + 1..start_quote + 1 + end_quote].to_string();
                            current_titles.push(title.clone());
                        }
                    }
                    continue;
                }

                // 匹配requires_backplate字段
                if trimmed.starts_with("requires_backplate:") {
                    let value = trimmed["requires_backplate:".len()..].trim();
                    // 移除可能存在的末尾逗号
                    let stripped = value.strip_suffix(",").unwrap_or(value);
                    let parsed_value = stripped.parse().unwrap_or(false);
                    current_requires_backplate = parsed_value;
                    log::debug!("Parsed requires_backplate: {}", parsed_value);
                    continue;
                }

                // 匹配结束的 }
                if trimmed == "}," || trimmed == "}" {
                    if !current_class.is_empty() {
                        window_whitelist.push(WindowWhitelistEntry {
                            class: current_class.clone(),
                            titles: current_titles.clone(),
                            requires_backplate: current_requires_backplate,
                        });
                        log::debug!("Added window whitelist entry: class={}, titles={:?}, requires_backplate={}", current_class, current_titles, current_requires_backplate);
                        current_class = String::new();
                        current_titles = Vec::new();
                        current_requires_backplate = false;
                    }
                    // 检查是否是最后一个条目
                    if trimmed == "}" {
                        in_window_section = false;
                    }
                    continue;
                }
            }
        }

        log::debug!(
            "Finished parsing, found {} process entries and {} window entries",
            process_whitelist.len(),
            window_whitelist.len()
        );
        Ok((process_whitelist, window_whitelist))
    } else {
        log::warn!("Whitelist config file not found at: {}", config_path);
        // 当配置文件不存在时，返回空的白名单而不是错误
        Ok((process_whitelist, window_whitelist))
    }
}

/// 初始化白名单
pub fn init_icon_whitelist() {
    match load_whitelist_from_file() {
        Ok((process_list, window_list)) => {
            *PROCESS_WHITELIST
                .write()
                .expect("process whitelist lock poisoned") = process_list;
            *WINDOW_WHITELIST
                .write()
                .expect("window whitelist lock poisoned") = window_list;
        }
        Err(e) => {
            log::warn!("Failed to load whitelist from file: {}", e);
            // 不使用默认白名单，保持空的白名单
            PROCESS_WHITELIST
                .write()
                .expect("process whitelist lock poisoned")
                .clear();
            WINDOW_WHITELIST
                .write()
                .expect("window whitelist lock poisoned")
                .clear();
        }
    }
}

/// 检查指定的应用程序是否在进程白名单中，并返回是否需要背板
pub fn is_process_whitelisted(process_name: &str) -> (bool, bool) {
    let lowercased = process_name.to_lowercase();
    let whitelist = PROCESS_WHITELIST
        .read()
        .expect("process whitelist lock poisoned");
    for entry in whitelist.iter() {
        if entry.name.to_lowercase() == lowercased {
            return (true, entry.requires_backplate);
        }
    }
    (false, false)
}

/// 获取本地图标目录
fn get_local_icon_dir(mode: IconBackplateStyle) -> Option<PathBuf> {
    let dir_name = match mode {
        IconBackplateStyle::Transparent => "DockIcon",
        IconBackplateStyle::White => "DockIconWhite",
    };

    // 方案 A: 优先尝试相对于 exe 的路径
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let base_paths = vec![
                exe_dir.to_path_buf(),                    // 同级目录
                exe_dir.join("..").join(".."),            // 向上两级
                exe_dir.join("..").join("..").join(".."), // 向上三级
            ];

            for base in base_paths {
                let p = base.join("res").join("drawable").join(dir_name);
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }

    // 仅 debug 模式: 回退到固定路径
    #[cfg(debug_assertions)]
    {
        let fixed_path = PathBuf::from(format!(
            r"C:\Program Files\HONOR\MagicAnimation\res\drawable\{}",
            dir_name
        ));
        if fixed_path.exists() {
            return Some(fixed_path);
        }
    }

    None
}

/// 检查本地图标目录是否存在对应进程的图标（内部统一实现）
fn has_local_process_icon_impl(process_name: &str, mode: IconBackplateStyle) -> bool {
    let local_icon_dir = match get_local_icon_dir(mode) {
        Some(dir) => dir,
        None => return false,
    };

    // 采用 DataManager.cpp 的逻辑：按第一个点分割文件名
    let stem = if let Some(pos) = process_name.find('.') {
        if pos == 0 {
            process_name
        } else {
            &process_name[..pos]
        }
    } else {
        process_name
    };

    // 先尝试原始名称
    let mut icon_path = local_icon_dir.join(format!("{}.png", stem));
    if icon_path.exists() {
        return true;
    }

    // 再尝试映射后的名称
    let mapped_stem = get_mapped_process_name(stem);
    if mapped_stem != stem {
        icon_path = local_icon_dir.join(format!("{}.png", mapped_stem));
        if icon_path.exists() {
            return true;
        }
    }

    false
}

/// 检查本地图标目录是否存在对应进程的图标（透明背板模式）
pub fn has_local_process_icon(process_name: &str) -> bool {
    has_local_process_icon_impl(process_name, IconBackplateStyle::Transparent)
}

/// 检查白色背板专用本地图标目录是否存在对应进程的图标
pub fn has_local_process_icon_white(process_name: &str) -> bool {
    has_local_process_icon_impl(process_name, IconBackplateStyle::White)
}

/// 常见文件名映射表
fn get_mapped_process_name(process_name: &str) -> &str {
    match process_name.to_lowercase().as_str() {
        "microsoft edge" | "msedge" => "msedge",
        _ => process_name,
    }
}

/// 获取本地图标数据（内部统一实现）
fn get_local_process_icon_impl(process_name: &str, mode: IconBackplateStyle) -> Option<Vec<u8>> {
    let local_icon_dir = get_local_icon_dir(mode)?;

    // 采用 DataManager.cpp 的逻辑：按第一个点分割文件名
    let stem = if let Some(pos) = process_name.find('.') {
        if pos == 0 {
            process_name
        } else {
            &process_name[..pos]
        }
    } else {
        process_name
    };

    // 先尝试原始名称
    let mut icon_path = local_icon_dir.join(format!("{}.png", stem));
    if icon_path.exists() {
        return fs::read(&icon_path).ok();
    }

    // 再尝试映射后的名称
    let mapped_stem = get_mapped_process_name(stem);
    if mapped_stem != stem {
        icon_path = local_icon_dir.join(format!("{}.png", mapped_stem));
        if icon_path.exists() {
            return fs::read(&icon_path).ok();
        }
    }

    None
}

/// 获取本地图标数据（透明背板模式）
pub fn get_local_process_icon(process_name: &str) -> Option<Vec<u8>> {
    get_local_process_icon_impl(process_name, IconBackplateStyle::Transparent)
}

/// 获取本地图标数据（白色背板模式）
pub fn get_local_process_icon_white(process_name: &str) -> Option<Vec<u8>> {
    get_local_process_icon_impl(process_name, IconBackplateStyle::White)
}

/// 检查指定的窗口是否在窗口白名单中，并返回是否需要背板
pub fn is_window_whitelisted(class: &str, title: &str) -> (bool, bool) {
    let whitelist = WINDOW_WHITELIST
        .read()
        .expect("window whitelist lock poisoned");
    for entry in whitelist.iter() {
        // 类名必须匹配
        if entry.class.eq_ignore_ascii_case(class) {
            // 标题匹配规则：
            // 1. 如果标题列表中有空字符串，则匹配任何该类的窗口
            // 2. 如果标题完全匹配，则匹配
            // 3. 如果不区分大小写匹配，则匹配
            // 4. 如果部分包含，则匹配
            for whitelisted_title in &entry.titles {
                if whitelisted_title.is_empty()
                    || whitelisted_title == title
                    || whitelisted_title.eq_ignore_ascii_case(title)
                    || title.contains(whitelisted_title)
                    || whitelisted_title.contains(title)
                {
                    log::debug!("Window whitelist matched class '{}' and title '{}' with entry class '{}' and title '{}'",
                               class, title, entry.class, whitelisted_title);
                    return (true, entry.requires_backplate);
                }
            }
        }
    }
    (false, false)
}

/// 检查应用程序是否在进程白名单中
pub fn is_app_whitelisted(process_name: &str) -> bool {
    is_process_whitelisted(process_name).0
}

/// 检查应用程序或窗口是否在白名单中，并返回是否需要背板
pub fn is_app_or_window_whitelisted(process_name: &str, class: &str, title: &str) -> (bool, bool) {
    // 首先检查进程白名单
    let (is_process_whitelisted, process_requires_backplate) = is_process_whitelisted(process_name);
    if is_process_whitelisted {
        return (true, process_requires_backplate);
    }

    // 然后检查窗口白名单
    is_window_whitelisted(class, title)
}
