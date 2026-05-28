use std::sync::Arc;

use super::domain::NotificationWhitelist;
use super::infrastructure::{handle_shell_hook_event, load_notification_whitelist};

/// 应用通知服务
pub struct AppNotificationService {
    whitelist: Arc<NotificationWhitelist>,
}

impl AppNotificationService {
    pub fn new() -> Self {
        let whitelist = load_notification_whitelist();

        Self {
            whitelist: Arc::new(whitelist),
        }
    }

    /// 处理 Shell Hook 事件
    pub fn process_shell_hook_event(&mut self, wparam: u32, lparam: isize) {
        handle_shell_hook_event(wparam, lparam, &self.whitelist);
    }
}
