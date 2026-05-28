use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{Builder, Wry};
use tauri_plugin_log::{fern, Target, TargetKind};
use time::format_description;
use time::OffsetDateTime;

/// 日志目录路径
const LOG_DIR: &str = r"C:\ProgramData\Comms\MagicAnimation\MagicBarUI";

/// 检查并尝试创建日志目录，必要时请求管理员权限
fn ensure_log_dir_access(log_dir: &Path) {
    if !log_dir.exists() {
        match fs::create_dir_all(log_dir) {
            Ok(_) => {
                println!("Log directory created successfully");
            }
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                println!("No permission to create log directory, attempting with admin privileges");
                let _ = std::process::Command::new("powershell")
                    .args([
                        "-NoProfile",
                        "-Command",
                        "Start-Process",
                        "-FilePath",
                        "powershell",
                        "-ArgumentList",
                        &format!(r#"-NoProfile -Command \"New-Item -ItemType Directory -Force -Path '{}' -ErrorAction SilentlyContinue\""#, log_dir.display()),
                        "-Verb",
                        "RunAs",
                        "-WindowStyle",
                        "Hidden"
                    ])
                    .creation_flags(0x08000000)
                    .output();
            }
            Err(err) => {
                println!("Failed to create log directory: {}", err);
            }
        }
    }
}

/// 检查目录是否可写，必要时请求管理员权限修复
fn ensure_log_dir_writable(log_dir: &Path) {
    let temp_file = log_dir.join(".write_test");
    match fs::write(&temp_file, "test") {
        Ok(_) => {
            let _ = fs::remove_file(temp_file);
            println!("Log directory is writable");
        }
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            println!("Log directory is not writable, attempting to fix permissions with admin privileges");
            if let Some(log_dir_str) = log_dir.to_str() {
                let _ = std::process::Command::new("powershell")
                    .args([
                        "-NoProfile",
                        "-Command",
                        "Start-Process",
                        "-FilePath",
                        "powershell",
                        "-ArgumentList",
                        &format!(r#"-NoProfile -Command \"icacls '{}' /grant Users:(OI)(CI)F /T -ErrorAction SilentlyContinue\""#, log_dir_str),
                        "-Verb",
                        "RunAs",
                        "-WindowStyle",
                        "Hidden"
                    ])
                    .creation_flags(0x08000000)
                    .output();
                println!("Permissions fixed for log directory");
            }
        }
        Err(err) => {
            println!("Failed to write to log directory: {}", err);
        }
    }
}

/// 自定义轮转日志写入器
/// - 支持追加模式
/// - 文件大小超过限制时自动创建新文件
struct RotatingFileWriter {
    log_dir: PathBuf,
    file_prefix: String,
    max_size: u64,
    max_files: usize,
    current_file: Mutex<Option<File>>,
    current_path: Mutex<PathBuf>,
    current_size: Mutex<u64>,
}

impl RotatingFileWriter {
    fn new(log_dir: PathBuf, file_prefix: String, max_size: u64, max_files: usize) -> Self {
        let initial_path = Self::generate_log_path(&log_dir, &file_prefix);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&initial_path)
            .expect("Failed to open log file");

        let initial_size = file.metadata().map(|m| m.len()).unwrap_or(0);

        println!(
            "[RotatingFileWriter] Initial log file: {}",
            initial_path.display()
        );

        let writer = Self {
            log_dir,
            file_prefix,
            max_size,
            max_files,
            current_file: Mutex::new(Some(file)),
            current_path: Mutex::new(initial_path),
            current_size: Mutex::new(initial_size),
        };

        // 修复：启动时立即清理过期日志文件
        // 这样即使日志文件很小不需要轮转，也会在每次启动时清理
        writer.cleanup_old_files();
        println!(
            "[RotatingFileWriter] Startup cleanup completed, max files: {}",
            max_files
        );

        writer
    }

    fn generate_log_path(log_dir: &Path, file_prefix: &str) -> PathBuf {
        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
        let file_name = format!(
            "{}_{:04}_{:02}_{:02}_{:02}{:02}{:02}_{:03}.log",
            file_prefix,
            now.year(),
            now.month() as u8,
            now.day(),
            now.hour(),
            now.minute(),
            now.second(),
            now.millisecond()
        );
        log_dir.join(file_name)
    }

    fn rotate(&self) {
        // 关闭当前文件
        {
            let mut file_guard = self.current_file.lock().unwrap();
            *file_guard = None;
        }

        // 生成新文件路径
        let new_path = Self::generate_log_path(&self.log_dir, &self.file_prefix);

        // 打开新文件
        let new_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&new_path)
            .expect("Failed to open new log file");

        println!(
            "[RotatingFileWriter] Rotated to new log file: {}",
            new_path.display()
        );

        // 更新状态
        {
            let mut file_guard = self.current_file.lock().unwrap();
            *file_guard = Some(new_file);
        }
        {
            let mut path_guard = self.current_path.lock().unwrap();
            *path_guard = new_path;
        }
        {
            let mut size_guard = self.current_size.lock().unwrap();
            *size_guard = 0;
        }

        // 清理过期的日志文件
        self.cleanup_old_files();
    }

    fn cleanup_old_files(&self) {
        if let Ok(entries) = fs::read_dir(&self.log_dir) {
            let mut log_files: Vec<(PathBuf, std::time::SystemTime)> = entries
                .filter_map(|entry| {
                    let entry = entry.ok()?;
                    let path = entry.path();
                    if path.is_file() {
                        if let Some(name) = path.file_name() {
                            if let Some(name_str) = name.to_str() {
                                if name_str.starts_with(&self.file_prefix)
                                    && name_str.ends_with(".log")
                                {
                                    if let Ok(metadata) = fs::metadata(&path) {
                                        if let Ok(modified) = metadata.modified() {
                                            return Some((path, modified));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    None
                })
                .collect();

            log_files.sort_by(|a, b| b.1.cmp(&a.1)); // 按修改时间降序排列（最新的在前）

            let total_files = log_files.len();
            let files_to_keep = self.max_files;
            let files_to_delete = if total_files > files_to_keep {
                total_files - files_to_keep
            } else {
                0
            };

            println!("[RotatingFileWriter] Cleanup check: found {} log files, max_files={}, will delete {}", 
                     total_files, files_to_keep, files_to_delete);

            for (path, _modified) in log_files.iter().skip(self.max_files) {
                match fs::remove_file(path) {
                    Ok(_) => {
                        println!("[RotatingFileWriter] ✓ Deleted old log: {}", path.display());
                    }
                    Err(e) => {
                        println!(
                            "[RotatingFileWriter] ✗ Failed to delete {}: {:?}",
                            path.display(),
                            e
                        );
                    }
                }
            }
        } else {
            println!(
                "[RotatingFileWriter] ✗ Failed to read log directory: {}",
                self.log_dir.display()
            );
        }
    }
}

impl Write for RotatingFileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // 检查是否需要轮转
        {
            let size = self.current_size.lock().unwrap();
            if *size >= self.max_size {
                drop(size);
                self.rotate();
            }
        }

        // 写入数据
        let mut file_guard = self.current_file.lock().unwrap();
        if let Some(ref mut file) = *file_guard {
            let written = file.write(buf)?;
            drop(file_guard);

            // 更新大小
            let mut size_guard = self.current_size.lock().unwrap();
            *size_guard += written as u64;

            Ok(written)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "No log file open",
            ))
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut file_guard = self.current_file.lock().unwrap();
        if let Some(ref mut file) = *file_guard {
            file.flush()
        } else {
            Ok(())
        }
    }
}

pub fn register_plugins(app_builder: Builder<Wry>) -> Builder<Wry> {
    // 创建时间格式
    let time_format = format_description::parse("[year]-[month]-[day]").unwrap();
    let time_format_hourms =
        format_description::parse("[hour]:[minute]:[second].[subsecond digits:3]").unwrap();

    // 日志文件限制配置
    const MAX_LOG_FILE_SIZE: u64 = 5 * 1024 * 1024; // 5MB
    const MAX_LOG_FILES: usize = 10; // 最多保留10个日志文件

    // 确保日志目录可访问
    let log_dir = Path::new(LOG_DIR);
    ensure_log_dir_access(log_dir);
    ensure_log_dir_writable(log_dir);

    // 创建自定义轮转日志写入器
    let rotating_writer = RotatingFileWriter::new(
        log_dir.to_path_buf(),
        "MagicBarUI".to_string(),
        MAX_LOG_FILE_SIZE,
        MAX_LOG_FILES,
    );

    // 使用 fern 包装自定义写入器
    let file_dispatch =
        fern::Dispatch::new().chain(Box::new(rotating_writer) as Box<dyn Write + Send>);

    // 构建日志目标列表
    // debug 版本保留 Stdout 便于开发调试，release 版本移除避免控制台阻塞
    let targets: Vec<Target> = vec![
        #[cfg(debug_assertions)]
        Target::new(TargetKind::Stdout), // debug 版本添加控制台输出
        Target::new(TargetKind::Dispatch(file_dispatch)), // 文件日志
        Target::new(TargetKind::Webview),                 // Webview 控制台
    ];

    let log_plugin_builder = tauri_plugin_log::Builder::new()
        .targets(targets)
        .level(if cfg!(debug_assertions) {
            log::LevelFilter::Trace
        } else {
            log::LevelFilter::Info
        })
        .level_for("tao", log::LevelFilter::Off)
        .level_for("os_info", log::LevelFilter::Off)
        .level_for("notify", log::LevelFilter::Off)
        .level_for("notify_debouncer_full", log::LevelFilter::Off);

    let log_plugin = log_plugin_builder
        .format(move |out, message, record| {
            let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
            let date_str = now.format(&time_format).unwrap_or_default();
            let time_str = now.format(&time_format_hourms).unwrap_or_default();
            out.finish(format_args!(
                "[{}][{}][{}][{}] {}",
                date_str,
                time_str,
                record.level(),
                record.target(),
                message
            ))
        })
        .build();

    app_builder
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(log_plugin)
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_http::init())
}
