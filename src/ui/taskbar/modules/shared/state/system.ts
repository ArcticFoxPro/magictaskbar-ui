import { computed, signal } from "@preact/signals";
import { invoke, FuncCommand, FuncEvent, subscribe } from "@magic-ui/lib";
import { TaskbarSide, type PhysicalMonitor } from "@magic-ui/lib/types";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { lazySignal } from "@shared/LazySignal";

// Get monitor ID from window label (format: @magic/taskbar?monitorId=xxx)
const decodeBase64Url = (value: string) => {
  const normalized = value.replace(/_/g, '/').replace(/-/g, '+');
  const padding = (4 - (normalized.length % 4)) % 4;
  return atob(normalized.padEnd(normalized.length + padding, '='));
};

const getCurrentMonitorId = async () => {
  try {
    const currentWindow = getCurrentWebviewWindow();
    const windowLabel = currentWindow.label;

    // Decode base64 label
    const decoded = decodeBase64Url(windowLabel);

    const monitorIdMatch = decoded.match(/monitorId=([^&]+)/);
    const id = monitorIdMatch?.[1] ? decodeURIComponent(monitorIdMatch[1]) : null;
    return id;
  } catch (e) {
    console.error('[System Init] Failed to get monitor ID:', e);
    return null;
  }
};
const currentMonitorId = await getCurrentMonitorId();

const $monitors = signal(await invoke(FuncCommand.SystemGetMonitors) as unknown as PhysicalMonitor[]);
subscribe(FuncEvent.SystemMonitorsChanged, (e) => {
  $monitors.value = e.payload as PhysicalMonitor[];
});

const $current_monitor = computed(() => {
  const monitor = $monitors.value.find((m) => m.id === currentMonitorId);
  if (!monitor) {
    console.warn('[System] Current monitor not found for ID:', currentMonitorId);
  }
  return monitor;
});

export const $mouse_pos = lazySignal(async () => {
  const [x, y] = await invoke(FuncCommand.GetMousePosition) as unknown as [number, number];
  return { x, y };
});
await subscribe(FuncEvent.GlobalMouseMove, ({ payload: [x, y] }) => {
  $mouse_pos.value = { x, y };
});
await $mouse_pos.init();

export const $mouse_at_edge = computed<TaskbarSide | null>(() => {
  const currentMonitor = $current_monitor.value;
  if (!currentMonitor) {
    return null;
  }

  const rect = currentMonitor.rect;
  const pos = $mouse_pos.value;

  if (pos.y === rect.top) {
    return "Top";
  }
  if (pos.x === rect.left) {
    return "Left";
  }
  if (pos.y === rect.bottom - 1) {
    return "Bottom";
  }
  if (pos.x === rect.right - 1) {
    return "Right";
  }
  return null;
});

// 导出当前显示器信息供其他模块使用
export { $current_monitor, $monitors };
