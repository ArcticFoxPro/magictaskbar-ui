use std::time::Duration;

use slu_ipc::messages::SvcAction;

use crate::{cli::ServicePipe, error::Result};

fn service_feedback_command(
    command: &str,
    args: serde_json::Value,
) -> Result<Option<serde_json::Value>> {
    let data = ServicePipe::request_with_response_blocking(
        SvcAction::ExecuteBackendCommand {
            command: command.to_string(),
            args,
        },
        Duration::from_secs(120),
    )?;

    match data {
        Some(data) if !data.trim().is_empty() => Ok(Some(serde_json::from_str(&data)?)),
        _ => Ok(None),
    }
}

pub fn check_and_increment_daily_count() -> bool {
    service_feedback_command(
        "feedback_check_and_increment_daily_count",
        serde_json::json!({}),
    )
    .ok()
    .flatten()
    .and_then(|value| value.get("value").and_then(|value| value.as_bool()))
    .unwrap_or(true)
}

pub fn collect_and_zip_logs() -> std::result::Result<String, String> {
    service_feedback_command("feedback_collect_and_zip_logs", serde_json::json!({}))
        .map_err(|err| err.to_string())?
        .and_then(|value| {
            value
                .get("value")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .ok_or_else(|| "srv returned empty feedback log path".to_string())
}
