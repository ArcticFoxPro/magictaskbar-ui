use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use slu_ipc::{messages::SvcAction, ServiceIpc, IPC};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use windows::Win32::System::TaskScheduler::{IExecAction2, ITaskService, TaskScheduler};
use windows_core::Interface;

use crate::{
    app::get_app_handle, error::Result, get_tokio_handle, utils::pwsh::PwshScript, windows_api::Com,
};

pub struct ServicePipe;

impl ServicePipe {
    /// will ignore any response
    pub fn request(message: SvcAction) -> Result<()> {
        get_tokio_handle().spawn(async move {
            if let Err(err) = ServiceIpc::send(message.clone()).await {
                log::error!("Error sending message to service {err}. Message: {message:?}");
            }
        });
        Ok(())
    }

    /// request and wait for response data
    pub async fn request_with_response(message: SvcAction) -> Result<Option<String>> {
        let response = ServiceIpc::send(message)
            .await
            .map_err(|e| format!("IPC error: {}", e))?;
        response
            .get_data()
            .map_err(|e| format!("Get data error: {}", e).into())
    }

    /// Synchronous wrapper for legacy window-event paths.
    ///
    /// Some WinEvent/UIA callbacks are synchronous, but can be invoked from a Tokio runtime
    /// worker. Calling Handle::block_on directly from those callbacks panics, so hop to a
    /// short-lived OS thread when a runtime is already driving the current thread.
    pub fn request_with_response_blocking(
        message: SvcAction,
        timeout: Duration,
    ) -> Result<Option<String>> {
        if tokio::runtime::Handle::try_current().is_err() {
            return get_tokio_handle().block_on(Self::request_with_response(message));
        }

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = get_tokio_handle()
                .block_on(Self::request_with_response(message))
                .map_err(|err| err.to_string());
            let _ = tx.send(result);
        });

        match rx.recv_timeout(timeout) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(err.into()),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(format!(
                "Service IPC response timed out after {}ms",
                timeout.as_millis()
            )
            .into()),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("Service IPC response thread disconnected".into())
            }
        }
    }

    pub fn is_running() -> bool {
        ServiceIpc::can_stablish_connection()
    }

    pub fn service_path() -> Result<PathBuf> {
        let service_path = std::env::current_exe()?.with_file_name("magictaskbar-srv.exe");
        Ok(service_path)
    }

    fn start_service_task() -> Result<()> {
        Com::run_with_context(|| unsafe {
            let task_service: ITaskService = Com::create_instance(&TaskScheduler)?;
            task_service.Connect(
                &Default::default(),
                &Default::default(),
                &Default::default(),
                &Default::default(),
            )?;
            let folder = task_service.GetFolder(&"\\MagicTaskbar".into())?;
            let task = folder.GetTask(&"MagicTaskbar Service".into())?;

            let actions = task.Definition()?.Actions()?;
            // ask to microsoft what that hell this start counting from 1 instead 0
            let action: IExecAction2 = actions.get_Item(1)?.cast()?;
            let mut action_path = windows_core::BSTR::new();
            action.Path(&mut action_path)?;

            let service_path = Self::service_path()?.to_string_lossy().to_lowercase();
            match action_path.to_string().to_lowercase() == service_path {
                true => {
                    task.Run(&Default::default())?;
                    Ok(())
                }
                false => {
                    Err("Service task is not pointing to the correct service executable".into())
                }
            }
        })
    }

    // the service should be running since installer will start it or startup task scheduler
    // so if the service is not running, we need to start it (common on msix setup)
    pub async fn start_service() -> Result<()> {
        let service_path = Self::service_path()?;
        if !service_path.exists() {
            log::warn!(
                "Service executable not found, skip starting service: {}",
                service_path.display()
            );
            return Ok(());
        }

        let Err(err) = Self::start_service_task() else {
            return Ok(());
        };

        log::debug!("Can not start service via task scheduler: {err}");

        let script = PwshScript::new(format!(
            "Start-Process '{}' -Verb runAs",
            Self::service_path()?.display()
        ))
        .inline_command()
        .elevated();

        let result = script.execute().await;
        if let Err(err) = result {
            let try_again = get_app_handle()
                .dialog()
                .message(t!("service.not_running_description"))
                .title(t!("service.not_running"))
                .kind(MessageDialogKind::Info)
                .buttons(MessageDialogButtons::OkCustom(
                    t!("service.not_running_ok").to_string(),
                ))
                .blocking_show();
            if try_again {
                script.execute().await?;
            }
            return Err(err);
        }

        let mut counter = 0;
        while !Self::is_running() && counter < 20 {
            log::debug!("Waiting for service IPC...");
            counter += 1;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        if counter == 20 {
            get_app_handle()
                .dialog()
                .message(t!("service.not_running_description"))
                .title(t!("service.not_running"))
                .kind(MessageDialogKind::Error)
                .buttons(MessageDialogButtons::Ok)
                .blocking_show();
            return Err("Service not running".into());
        }

        Ok(())
    }
}
