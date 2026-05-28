pub mod constants;
pub mod icon_extractor;
pub mod icon_whitelist;
pub mod integrity;
pub mod lock_free;
pub mod pwsh;

use std::{
    collections::hash_map::DefaultHasher,
    collections::HashMap,
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc, LazyLock},
    time::{Duration, Instant},
};

use lazy_static::lazy_static;
use parking_lot::Mutex;
use windows::{
    core::GUID,
    Win32::{
        Foundation::RECT,
        UI::Shell::{SHGetKnownFolderPath, KF_FLAG_DEFAULT},
    },
};

use crate::error::Result;

/// Get the directory where the application executable is located
/// Used to locate resource files at the same level as the executable (such as static directory)
pub fn get_app_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn sleep_millis(millis: u64) {
    std::thread::sleep(Duration::from_millis(millis));
}

pub fn are_overlaped(a: &RECT, b: &RECT) -> bool {
    let zeroed = RECT::default();
    if a == &zeroed || b == &zeroed {
        return false;
    }
    // The edge pixel overlapping do not matters. This resolves the shared pixel in between the monitors,
    // hereby a fullscreened app shared pixel collision does not hide other monitor windows.
    if a.right <= b.left || a.left >= b.right || a.bottom <= b.top || a.top >= b.bottom {
        return false;
    }
    true
}

/// Resolve paths with folder ids in the form of "{GUID}\path\to\file"
///
/// https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid
#[allow(dead_code)]
pub fn resolve_guid_path<S: AsRef<str>>(path: S) -> Result<PathBuf> {
    let parts = path.as_ref().split("\\");
    let mut path_buf = PathBuf::new();

    for (idx, part) in parts.into_iter().enumerate() {
        if part.starts_with("{") && part.ends_with("}") {
            let guid = part.trim_start_matches('{').trim_end_matches('}');
            let rfid = GUID::try_from(guid)?;
            let string_path =
                unsafe { SHGetKnownFolderPath(&rfid as _, KF_FLAG_DEFAULT, None)?.to_string()? };

            path_buf.push(string_path);
        } else if idx == 0 {
            return Ok(PathBuf::from(path.as_ref()));
        } else {
            path_buf.push(part);
        }
    }

    Ok(path_buf)
}

pub static TRACE_LOCK_ENABLED: AtomicBool = AtomicBool::new(true);

// ========================================================================
// 活跃锁跟踪：实时记录当前所有正在被持有的锁。
// - 锁 acquire 成功 → 向 ACTIVE_LOCKS[key] 追加一个 LockHolder
// - TracedGuard drop 时 → 对应的 LockTicket drop → 从 ACTIVE_LOCKS 移除
// - 任意一次 acquire 超时 → panic 时 dump_active_locks()
//   直接展示当前哪个线程、在哪里、持锁多久
// 调用方（trace_lock! / trace_read! / trace_write!）零侵入。
// ========================================================================

#[derive(Clone)]
pub struct LockHolder {
    pub key: String,          // "APP_MANAGER::write" / "TASKBAR_STATE" 等
    pub kind: &'static str,   // "read" / "write" / "mutex"
    pub thread_id: String,    // format!("{:?}", thread::current().id())
    pub thread_name: String,  // 线程名，便于辨识
    pub location: String,     // "file.rs:line" — acquire 处
    pub acquired_at: Instant, // 用于计算 held_for
}

pub static ACTIVE_LOCKS: LazyLock<Mutex<HashMap<String, Vec<Arc<LockHolder>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// RAII 登出票据：随 TracedGuard 一同 drop，自动从 ACTIVE_LOCKS 移除该 holder。
pub struct LockTicket {
    holder: Arc<LockHolder>,
}

impl Drop for LockTicket {
    fn drop(&mut self) {
        if let Some(mut map) = ACTIVE_LOCKS.try_lock_for(Duration::from_millis(200)) {
            if let Some(v) = map.get_mut(&self.holder.key) {
                v.retain(|h| !Arc::ptr_eq(h, &self.holder));
                if v.is_empty() {
                    map.remove(&self.holder.key);
                }
            }
        }
        // 拿不到 ACTIVE_LOCKS 时静默跳过，绝不在跟踪逻辑中产生新的死锁。
    }
}

/// 登记一次锁获取，返回 RAII 票据（TracedGuard 持有）。
/// 当 TRACE_LOCK_ENABLED 关闭时返回 None，此时无记录亦无登出动作。
pub fn register_lock_acquire(
    key: &'static str,
    kind: &'static str,
    file: &'static str,
    line: u32,
) -> Option<LockTicket> {
    if !TRACE_LOCK_ENABLED.load(std::sync::atomic::Ordering::Acquire) {
        return None;
    }
    let holder = Arc::new(LockHolder {
        key: key.to_string(),
        kind,
        thread_id: format!("{:?}", std::thread::current().id()),
        thread_name: std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_string(),
        location: format!("{}:{}", file, line),
        acquired_at: Instant::now(),
    });
    if let Some(mut map) = ACTIVE_LOCKS.try_lock_for(Duration::from_millis(200)) {
        map.entry(holder.key.clone())
            .or_insert_with(Vec::new)
            .push(holder.clone());
    } else {
        // 方案 4 监控漏洞补补：抢不到 ACTIVE_LOCKS 时永远不要默默吃下，
        // 否则下一次死锁 dump 将缺失该内层持有者，导致排查无从下手。
        // 仅从跟踪角度记录 warn，不重试以避免在跟踪逻辑自身引入死锁。
        log::warn!(
            "[trace_lock] Failed to register holder into ACTIVE_LOCKS within 200ms: key={} kind={} location={} thread={:?}({})",
            holder.key,
            holder.kind,
            holder.location,
            std::thread::current().id(),
            holder.thread_name,
        );
    }
    Some(LockTicket { holder })
}

/// 格式化当前活跃锁快照，用于 panic 消息。
pub fn dump_active_locks() -> String {
    let Some(map) = ACTIVE_LOCKS.try_lock_for(Duration::from_millis(500)) else {
        return "  <failed to acquire ACTIVE_LOCKS snapshot within 500ms>\n".to_string();
    };
    if map.is_empty() {
        return "  <no active locks>\n".to_string();
    }
    let mut out = String::new();
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    for k in keys {
        let holders = &map[k];
        out.push_str(&format!("  {}:\n", k));
        for h in holders {
            out.push_str(&format!(
                "    [thread={}|{}] kind={} acquired at {} held_for={:.3}s\n",
                h.thread_id,
                h.thread_name,
                h.kind,
                h.location,
                h.acquired_at.elapsed().as_secs_f64()
            ));
        }
    }
    out
}

/// 透明包装底层锁守卫，随之携带一个 LockTicket。
/// 字段声明顺序关键：inner 在前，先 drop（释放底层锁），
/// 然后才 drop _ticket（从活跃表登出）——
/// 保证任意时刻表中记录都是“真的还在持有”。
pub struct TracedGuard<G> {
    pub inner: G,
    pub _ticket: Option<LockTicket>,
}

impl<G: std::ops::Deref> std::ops::Deref for TracedGuard<G> {
    type Target = G::Target;
    fn deref(&self) -> &Self::Target {
        &*self.inner
    }
}

impl<G: std::ops::DerefMut> std::ops::DerefMut for TracedGuard<G> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.inner
    }
}

#[macro_export]
macro_rules! trace_lock {
    ($mutex:expr) => {
        trace_lock!($mutex, 5)
    };
    ($mutex:expr, $duration:expr) => {{
        let guard_name: &'static str = stringify!($mutex);
        match $mutex.try_lock_for(std::time::Duration::from_secs($duration)) {
            Some(g) => {
                let ticket = $crate::utils::register_lock_acquire(
                    guard_name, "mutex", file!(), line!(),
                );
                $crate::utils::TracedGuard { inner: g, _ticket: ticket }
            }
            None => {
                let dump = $crate::utils::dump_active_locks();
                let panic_msg = format!(
                    "{} mutex is deadlocked at {}:{}\n=== Active locks snapshot ===\n{}=============================",
                    guard_name, file!(), line!(), dump
                );
                panic!("{:?}", $crate::error::AppError::from(panic_msg));
            }
        }
    }};
}

#[macro_export]
macro_rules! trace_read {
    ($rwlock:expr) => {{
        let guard_name: &'static str = concat!(stringify!($rwlock), "::read");
        match $rwlock.try_read_for(std::time::Duration::from_secs(5)) {
            Some(g) => {
                let ticket = $crate::utils::register_lock_acquire(
                    guard_name, "read", file!(), line!(),
                );
                $crate::utils::TracedGuard { inner: g, _ticket: ticket }
            }
            None => {
                let dump = $crate::utils::dump_active_locks();
                let panic_msg = format!(
                    "{} rwlock read is deadlocked at {}:{}\n=== Active locks snapshot ===\n{}=============================",
                    guard_name, file!(), line!(), dump
                );
                panic!("{:?}", $crate::error::AppError::from(panic_msg));
            }
        }
    }};
}

#[macro_export]
macro_rules! trace_write {
    ($rwlock:expr) => {{
        let guard_name: &'static str = concat!(stringify!($rwlock), "::write");
        match $rwlock.try_write_for(std::time::Duration::from_secs(5)) {
            Some(g) => {
                let ticket = $crate::utils::register_lock_acquire(
                    guard_name, "write", file!(), line!(),
                );
                $crate::utils::TracedGuard { inner: g, _ticket: ticket }
            }
            None => {
                let dump = $crate::utils::dump_active_locks();
                let panic_msg = format!(
                    "{} rwlock write is deadlocked at {}:{}\n=== Active locks snapshot ===\n{}=============================",
                    guard_name, file!(), line!(), dump
                );
                panic!("{:?}", $crate::error::AppError::from(panic_msg));
            }
        }
    }};
}

lazy_static! {
    pub static ref PERFORMANCE_HELPER: Mutex<PerformanceHelper> = Mutex::new(PerformanceHelper {
        time: HashMap::new(),
    });
}

pub struct PerformanceHelper {
    time: HashMap<String, Instant>,
}

impl PerformanceHelper {
    pub fn start(&mut self, name: &str) {
        log::debug!("{name} start");
        self.time.insert(name.to_string(), Instant::now());
    }

    pub fn elapsed(&self, name: &str) -> Duration {
        self.time.get(name).unwrap().elapsed()
    }

    pub fn end(&mut self, name: &str) {
        log::debug!("{} end in: {:.2}s", name, self.elapsed(name).as_secs_f64());
        self.time.remove(name);
    }
}

/// Useful when spawning threads that will allocate a loop or some other blocking operation
pub fn spawn_named_thread<F, T>(id: &str, cb: F) -> Result<std::thread::JoinHandle<T>>
where
    F: FnOnce() -> T,
    F: Send + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new()
        .name(format!("Thread - {id}"))
        .spawn(cb)
        .map_err(|e| format!("Failed to spawn thread: {e}").into())
}

pub fn date_based_hex_id() -> String {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("{since_epoch:x}")
}

pub fn path_based_hash_id(path: &std::path::Path) -> String {
    let mut hasher = DefaultHasher::new();
    path.to_string_lossy().to_lowercase().hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

pub fn umid_based_hash_id(umid: &str) -> String {
    let mut hasher = DefaultHasher::new();
    umid.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}
