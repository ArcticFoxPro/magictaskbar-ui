use crate::{cli::ServicePipe, error::Result};
use slu_ipc::messages::SvcAction;

async fn tray_service_command(
    command: &str,
    args: serde_json::Value,
) -> Result<Option<serde_json::Value>> {
    let data = ServicePipe::request_with_response(SvcAction::ExecuteBackendCommand {
        command: command.to_string(),
        args,
    })
    .await?;

    match data {
        Some(data) if !data.trim().is_empty() => Ok(Some(serde_json::from_str(&data)?)),
        _ => Ok(None),
    }
}

/// Shows or hides the native tray overflow window (hidden icons popup).
/// The window will be positioned above the anchor point.
/// Returns a tuple of (success: bool, is_visible: bool) where:
/// - success: whether the operation completed successfully
/// - is_visible: whether the overflow window is now visible
#[tauri::command(async)]
pub async fn show_native_tray_overflow(
    anchor_center_x: i32,
    anchor_top_y: i32,
    gap: i32,
) -> Result<(bool, bool)> {
    log::info!(
        "[TrayOverflow] show_native_tray_overflow called: center_x={}, top_y={}, gap={}",
        anchor_center_x,
        anchor_top_y,
        gap
    );

    let data = tray_service_command(
        "tray_toggle_overflow",
        serde_json::json!({
            "anchorCenterX": anchor_center_x,
            "anchorTopY": anchor_top_y,
            "gap": gap,
        }),
    )
    .await?;
    let result = data
        .as_ref()
        .and_then(|value| value.get("success").and_then(|value| value.as_bool()))
        .unwrap_or(false);
    let is_visible = data
        .as_ref()
        .and_then(|value| value.get("visible").and_then(|value| value.as_bool()))
        .unwrap_or(false);

    log::info!(
        "[TrayOverflow] toggle result: success={}, is_visible={}",
        result,
        is_visible
    );

    Ok((result, is_visible))
}

/// Returns whether the tray overflow window is currently visible.
#[tauri::command(async)]
pub async fn is_tray_overflow_visible() -> bool {
    tray_service_command("tray_is_overflow_visible", serde_json::json!({}))
        .await
        .ok()
        .flatten()
        .and_then(|value| value.get("value").and_then(|value| value.as_bool()))
        .unwrap_or(false)
}

/// Explicitly hides the tray overflow window.
#[tauri::command(async)]
pub async fn hide_tray_overflow() -> Result<bool> {
    Ok(
        tray_service_command("tray_hide_overflow", serde_json::json!({}))
            .await?
            .and_then(|value| value.get("value").and_then(|value| value.as_bool()))
            .unwrap_or(false),
    )
}
