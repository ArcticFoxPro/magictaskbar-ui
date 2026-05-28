use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// 预览窗口中显示的单个窗口信息
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PreviewWindowInfo {
    /// 窗口句柄
    pub handle: isize,
    /// 窗口标题
    pub title: String,
    /// 窗口图标的 Base64 编码 PNG 数据
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_png_base64: Option<String>,
    /// 是否是当前焦点窗口
    pub is_focused: bool,
}

/// 预览窗口的位置信息
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PreviewPosition {
    /// X 坐标（屏幕坐标）
    pub x: i32,
    /// Y 坐标（屏幕坐标）
    pub y: i32,
    /// 弹出方向: "top" | "bottom" | "left" | "right"
    pub placement: String,
}

/// 托盘区域位置信息
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TrayAreaRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// 显示预览窗口的事件载荷
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PreviewShowPayload {
    /// Taskbar item 的唯一标识
    pub item_id: String,
    /// 应用显示名称
    pub display_name: String,
    /// 应用的窗口列表
    pub windows: Vec<PreviewWindowInfo>,
    /// 预览窗口的位置
    pub position: PreviewPosition,
    /// 应用路径（用于获取图标）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// 应用 UMID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub umid: Option<String>,
    /// 托盘区域的位置（仅在托盘模式下有效）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tray_area_rect: Option<TrayAreaRect>,
}
