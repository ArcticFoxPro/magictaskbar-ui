import { computed } from "@preact/signals";
import { invoke, FuncCommand, FuncEvent, subscribe } from "@magic-ui/lib";
import { FancyToolbarSide, type PhysicalMonitor } from "@magic-ui/lib/types";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { lazySignal } from "@shared/LazySignal";

// Get monitor ID from window label (format: @magic/fancy-toolbar?monitorId=xxx)
const getCurrentMonitorId = async () => {
  try {
    const currentWindow = getCurrentWebviewWindow();
    const windowLabel = currentWindow.label;

    const decoded = atob(windowLabel.replace(/_/g, '/').replace(/-/g, '+'));
    const monitorIdMatch = decoded.match(/monitorId=([^&]+)/);
    const id = monitorIdMatch?.[1] ? decodeURIComponent(monitorIdMatch[1]) : null;
    return id;
  } catch (e) {
    console.error('[Toolbar System] Failed to get monitor ID:', e);
    return null;
  }
};
const currentMonitorId = await getCurrentMonitorId();

const $monitors = lazySignal(() => invoke(FuncCommand.SystemGetMonitors) as unknown as PhysicalMonitor[]);
await subscribe(FuncEvent.SystemMonitorsChanged, (e) => {
  $monitors.value = e.payload as PhysicalMonitor[];
});
await $monitors.init();

export const $current_monitor = computed(() => {
  const monitor = $monitors.value.find((m) => m.id === currentMonitorId);
  if (!monitor) {
    console.warn('[Toolbar System] Current monitor not found for ID:', currentMonitorId);
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


// Detect if mouse is within 5px hot zone of the toolbar side on the current monitor
export const $mouse_at_edge = computed<FancyToolbarSide | null>(() => {
  const currentMonitor = $current_monitor.value;
  if (!currentMonitor) {
    return null;
  }
  const rect = currentMonitor.rect;
  const pos = $mouse_pos.value;

  if (pos.y === rect.top) {
    return "Top";
  }
  if (pos.y === rect.bottom - 1) {
    return "Bottom";
  }
  return null;
});
