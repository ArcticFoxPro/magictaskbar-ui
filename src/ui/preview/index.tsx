import { getRootContainer } from "@shared";
import { removeDefaultWebviewActions } from "@shared/setup";
import { info as logInfo } from "@tauri-apps/plugin-log";
import { createRoot } from "react-dom/client";
import { emit } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

import { App } from "./app";

import "./public/index.css";

removeDefaultWebviewActions();

const decodeBase64Url = (value: string) => {
  const normalized = value.replace(/_/g, '/').replace(/-/g, '+');
  const padding = (4 - (normalized.length % 4)) % 4;
  return atob(normalized.padEnd(normalized.length + padding, '='));
};

const getCurrentMonitorId = () => {
  try {
    const label = getCurrentWebviewWindow().label;
    const decoded = decodeBase64Url(label);
    const monitorIdMatch = decoded.match(/monitorId=([^&]+)/);
    return monitorIdMatch?.[1] ? decodeURIComponent(monitorIdMatch[1]) : null;
  } catch {
    return null;
  }
};

// 🔧 修复：初始化时立即通知 taskbar 预览窗口是关闭状态
// 防止遗留状态导致 taskbar 无法隐藏
const currentMonitorId = getCurrentMonitorId();

emit("preview::window_open", { open: false, monitorId: currentMonitorId }).then(() => {
  logInfo("[Preview] Sent initial closed state to taskbar");
}).catch((e) => {
  console.error("[Preview] Failed to send initial state:", e);
});

// 渲染 React 组件
const container = getRootContainer();
createRoot(container).render(<App />);
