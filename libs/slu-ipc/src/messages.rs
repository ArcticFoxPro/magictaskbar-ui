use std::collections::HashMap;

use libs_core::{rect::Rect, system_state::WifiNetwork};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Windows 任务栏对齐方式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskbarAlignment {
    /// 左对齐 (值为 0)
    Left = 0,
    /// 居中对齐 (值为 1)
    Center = 1,
    /// 未找到值 (值为 2)
    NotFound = 2,
}

impl From<u32> for TaskbarAlignment {
    fn from(value: u32) -> Self {
        match value {
            0 => TaskbarAlignment::Left,
            1 => TaskbarAlignment::Center,
            2 => TaskbarAlignment::NotFound,
            _ => TaskbarAlignment::Center, // 其他值默认居中对齐
        }
    }
}

impl From<TaskbarAlignment> for u32 {
    fn from(alignment: TaskbarAlignment) -> Self {
        alignment as u32
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcResponse {
    Success,
    Err(String),
    Data(String),
}

impl IpcResponse {
    pub fn ok(self) -> Result<()> {
        match self {
            IpcResponse::Success => Ok(()),
            IpcResponse::Err(err) => Err(Error::IpcResponseError(err)),
            IpcResponse::Data(_) => Ok(()),
        }
    }

    pub fn get_data(self) -> Result<Option<String>> {
        match self {
            IpcResponse::Data(data) => Ok(Some(data)),
            IpcResponse::Success => Ok(None),
            IpcResponse::Err(err) => Err(Error::IpcResponseError(err)),
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Ok(serde_json::from_slice(bytes)?)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(self)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AppMessage {
    /// Command-line messages
    Cli(Vec<String>),
    /// Debug message for logging and diagnostics
    Debug(String),
    /// Window event forwarded by the service process.
    WinEvent { event: u32, hwnd: isize },
    /// Global mouse movement forwarded by the service process.
    GlobalMouseMove { x: i32, y: i32, emitted_at: u64 },
    /// Low-level Win key press forwarded by the service process.
    KeyboardWinKeyDown { pressed: bool },
    /// Recycle bin content state forwarded by the service process.
    RecycleBinContentChanged {
        recycle_bin_empty: bool,
        recycle_bin_count: u32,
    },
    /// Game fullscreen suppress-hover state forwarded by the service process.
    GameFullscreenChanged { blocked: bool },
    /// Default render endpoint volume state forwarded by the service process.
    SystemVolumeChanged { volume: u8, muted: bool },
    /// Bluetooth radio state forwarded by the service process.
    SystemBluetoothStateChanged { enabled: bool },
    /// WLAN network list forwarded by the service process.
    SystemNetworksChanged { networks: Vec<WifiNetwork> },
    /// UIA WindowVisualStatePropertyId forwarded by the service process.
    WindowVisualStateChanged { hwnd: isize, state: i32 },
}

impl AppMessage {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Ok(serde_json::from_slice(bytes)?)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(self)?)
    }
}

// ==============================================

/// UI Service Actions
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SvcAction {
    Stop,
    /// Request srv to restart UI process (triggered by WebView2 crash)
    RestartUI,
    ShowWindow {
        hwnd: isize,
        command: i32,
    },
    ShowWindowAsync {
        hwnd: isize,
        command: i32,
    },
    SetWindowPosition {
        hwnd: isize,
        rect: Rect,
        flags: u32,
    },
    DeferWindowPositions {
        list: HashMap<isize, Rect>,
        animated: bool,
        animation_duration: u64,
        easing: String,
    },
    SetForeground(isize),
    GetProcessName {
        hwnd: isize,
    },
    GetProcessPath {
        hwnd: isize,
    },
    GetWindowIconPng {
        hwnd: isize,
        large: bool,
    },
    CloseWindow {
        hwnd: isize,
    },
    PostMessage {
        hwnd: isize,
        message: u32,
        wparam: usize,
        lparam: isize,
    },
    SetWindowTopmost {
        hwnd: usize,
        rect: Rect,
        flags: u32,
    },
    SetWindowNoTopmost {
        hwnd: usize,
        rect: Rect,
        flags: u32,
    },
    MoveWindowByOverlap {
        hwnd: usize,
        overlap_h: i32,
    },
    StartTsfWatcher {
        hwnd: isize,
    },
    SwitchInputMethodHotkey {
        guid_profile: String,
    },
    ExecuteBackendCommand {
        command: String,
        args: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SvcMessage {
    pub token: String,
    pub action: SvcAction,
}

impl SvcMessage {
    pub fn signature() -> &'static str {
        std::env!("SLU_SERVICE_CONNECTION_TOKEN")
    }

    pub fn is_signature_valid(&self) -> bool {
        self.token == SvcMessage::signature()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Ok(serde_json::from_slice(bytes)?)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(self)?)
    }
}
