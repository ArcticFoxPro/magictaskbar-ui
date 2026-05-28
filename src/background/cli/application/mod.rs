mod debugger;

use std::sync::atomic::Ordering;

use clap::Parser;
use debugger::DebuggerCli;
use serde::{Deserialize, Serialize};
use slu_ipc::{messages::AppMessage, AppIpc};
use windows::Win32::System::Console::{AttachConsole, GetConsoleWindow, ATTACH_PARENT_PROCESS};

use crate::{error::Result, resources::cli::ResourceManagerCli, widgets::taskbar::cli::TaskbarCli};

/// Command Line Interface
#[derive(Debug, Serialize, Deserialize, clap::Parser)]
#[command(version, name = "MagicTaskbar UI")]
pub struct AppCli {
    /// Indicates that the app was invoked from the start up action.
    #[arg(long, default_value_t)]
    startup: bool,
    /// Unused flag
    #[arg(long, default_value_t)]
    silent: bool,
    /// Prints some extra information on the console.
    #[arg(long, default_value_t)]
    verbose: bool,
    /// Path or URI to load.
    uri: Option<String>,
    #[command(subcommand)]
    command: Option<AppCliCommand>,
}

#[derive(Debug, Serialize, Deserialize, clap::Subcommand)]
pub enum AppCliCommand {
    Debugger(DebuggerCli),
    Taskbar(TaskbarCli),
    Resource(ResourceManagerCli),
    /// Stop and exit the application
    Stop,
}

// attach console could fail if not console to attach is present
pub fn attach_console() -> bool {
    let already_attached = unsafe { !GetConsoleWindow().is_invalid() };
    already_attached || unsafe { AttachConsole(ATTACH_PARENT_PROCESS).is_ok() }
}

/// Handles the CLI and will exit the process if needed.\
/// Performs redirection to the instance if needed too, will fail if no instance is running.
pub async fn handle_console_client() -> Result<()> {
    let matches = match AppCli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            // (help, --help or -h) and other sugestions are managed as error
            attach_console();
            e.exit();
        }
    };

    if matches.startup {
        crate::STARTUP.store(true, Ordering::SeqCst);
    }

    if matches.silent {
        crate::SILENT.store(true, Ordering::SeqCst);
    }

    if matches.verbose {
        crate::VERBOSE.store(true, Ordering::SeqCst);
        println!("Received {:#?}", std::env::args());
        println!("Parsed {matches:#?}");
    }

    if matches.should_be_redirected() {
        attach_console();
        matches.send_to_main_instance().await?;
        std::process::exit(0);
    }

    if matches.command.is_some() {
        matches.process()?;
        std::process::exit(0);
    }
    Ok(())
}

impl AppCli {
    pub fn should_be_redirected(&self) -> bool {
        if let Some(command) = &self.command {
            return matches!(command, AppCliCommand::Stop);
        }
        false
    }

    /// intended to be called on the main instance
    pub fn process(self) -> Result<()> {
        match self.command {
            Some(cmd) => cmd.process(),
            None => Ok(()),
        }
    }

    /// will fail if no instance is running
    pub async fn send_to_main_instance(self) -> Result<()> {
        let mut args = Vec::new();
        let working_dir = std::env::current_dir()?;

        for arg in std::env::args() {
            if arg.starts_with("./")
                || arg.starts_with(".\\")
                || arg.starts_with("../")
                || arg.starts_with("..\\")
            {
                args.push(working_dir.join(arg).to_string_lossy().to_string());
                continue;
            }
            args.push(arg);
        }

        if self.verbose {
            println!("Sending {args:#?}");
        }

        AppIpc::send(AppMessage::Cli(args))
            .await
            .map_err(|_| "Can't stablish connection, ensure UI is running.")?;
        Ok(())
    }
}

impl AppCliCommand {
    pub fn process(self) -> Result<()> {
        match self {
            AppCliCommand::Debugger(command) => {
                command.process()?;
            }
            AppCliCommand::Taskbar(command) => {
                command.process()?;
            }
            AppCliCommand::Resource(command) => {
                command.process()?;
            }
            AppCliCommand::Stop => {
                log::info!("Received stop command, exiting application...");
                crate::report_ui_process_exit("CliStop");
                // exit(0) 不会触发 RunEvent::Exit，需要在此手动清理 Explorer 中的 Hook 和子类化
                std::process::exit(0);
            }
        }
        Ok(())
    }
}
