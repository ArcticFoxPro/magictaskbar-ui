use std::{ops::Index, path::PathBuf};

use libs_core::state::{RelaunchArguments, TaskbarItem};
use serde::{Deserialize, Serialize};
use tauri_plugin_shell::ShellExt;
use windows::Win32::UI::WindowsAndMessaging::SW_MINIMIZE;

use crate::{
    app::get_app_handle,
    error::Result,
    trace_lock,
    utils::constants::VAR_COMMON,
    widgets::taskbar::taskbar_items_impl::{TaskbarState, TASKBAR_STATE},
    windows_api::{monitor::Monitor, window::Window, WindowsApi},
};

/// dock commands
#[derive(Debug, Serialize, Deserialize, clap::Args)]
pub struct TaskbarCli {
    #[command(subcommand)]
    pub subcommand: TaskbarCommand,
}

#[derive(Debug, Serialize, Deserialize, clap::Subcommand)]
pub enum TaskbarCommand {
    /// Set foreground to the application which is idx-nth on the taskbar. If it is not started, then starts it.
    ForegroundOrRunApp {
        /// Which index should be started on taskbar.
        index: usize,
    },
}

impl TaskbarCli {
    pub fn process(self) -> Result<()> {
        #[allow(irrefutable_let_patterns)]
        if let TaskbarCommand::ForegroundOrRunApp { index } = self.subcommand {
            let id = Monitor::from(WindowsApi::monitor_from_cursor_point()).stable_id2()?;

            // 先获取 items 的克隆，然后立即释放锁
            // 避免在持有锁期间调用可能耗时的 get_filtered_by_monitor
            let items = {
                let guard = trace_lock!(TASKBAR_STATE);
                guard.items.clone()
            };
            let temp_state = TaskbarState { items };
            let filtered = temp_state.get_filtered_by_monitor()?;

            if let Some(taskbaritems) = filtered.get(&id) {
                let all_items: Vec<&TaskbarItem> = taskbaritems
                    .left
                    .iter()
                    .chain(taskbaritems.center.iter())
                    .chain(taskbaritems.right.iter())
                    .filter(|item| {
                        matches!(item, TaskbarItem::Pinned(_) | TaskbarItem::Temporal(_))
                    })
                    .collect();

                if all_items.len() <= index {
                    return Ok(());
                }

                let item = all_items.index(index);

                if let TaskbarItem::Pinned(inner_data) | TaskbarItem::Temporal(inner_data) = item {
                    if let Some(item) = inner_data.windows.first() {
                        let window = Window::from(item.handle);
                        if !window.is_window() {
                            return Ok(());
                        }

                        if window.is_focused() {
                            window.show_window_async(SW_MINIMIZE)?;
                        } else {
                            window.focus()?;
                        }
                    } else {
                        let args = match &inner_data.relaunch_args {
                            Some(args) => match args {
                                RelaunchArguments::String(args) => args.clone(),
                                RelaunchArguments::Array(args) => args.join(" ").trim().to_owned(),
                            },
                            None => String::new(),
                        };

                        // we create a link file to trick with explorer into a separated process
                        // and without elevation in case UI was running as admin
                        // this could take some delay like is creating a file but just are some milliseconds
                        // and this exposed funtion is intended to just run certain times
                        let lnk_file = WindowsApi::create_temp_shortcut(
                            &PathBuf::from(&inner_data.relaunch_program),
                            &args,
                            inner_data.relaunch_in.as_deref(),
                        )?;
                        tokio::spawn(async move {
                            let _ = get_app_handle()
                                .shell()
                                .command(VAR_COMMON.system_dir().join("explorer.exe"))
                                .arg(&lnk_file)
                                .status()
                                .await;
                            let _ = std::fs::remove_file(&lnk_file);
                        });
                    }
                }
            }
        }
        Ok(())
    }
}
