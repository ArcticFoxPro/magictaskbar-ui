use std::collections::HashSet;
use windows::Win32::Foundation::HWND;

/// 应用通知事件
pub struct AppNotificationEvent {
    pub hwnd: HWND,
    pub process_id: u32,
    pub process_name: String,
    pub window_title: String,
    pub window_class: String,
}

/// 通知白名单
pub struct NotificationWhitelist {
    pub applications: HashSet<String>,
}

impl NotificationWhitelist {
    pub fn new() -> Self {
        Self {
            applications: HashSet::new(),
        }
    }

    pub fn add(&mut self, app: String) {
        self.applications.insert(app);
    }

    pub fn contains(&self, app: &str) -> bool {
        self.applications.contains(app)
    }
}
