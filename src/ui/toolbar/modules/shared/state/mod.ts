import { computed, effect, signal } from "@preact/signals";
import { $mouse_at_edge, $current_monitor, $mouse_pos } from "./system";
import { $is_this_webview_focused } from "@shared/signals";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { invoke as invokeCommand } from "@tauri-apps/api/core";
import { FuncCommand } from "@magic-ui/lib";
import { FuncEvent } from "@magic-ui/lib";
import { debounce } from "lodash";

// Game scene blocked state (foreground process is in game whitelist)
// When mouse hovers to edge, we check if game scene is blocked
export const $game_mode_fullscreen_blocked = signal(false);
let lastCheckTime: number = 0;
const CHECK_CACHE_DURATION = 500; // Cache for 500ms to avoid excessive API calls

// Check if game scene is blocked
async function checkGameFullscreenBlocked(): Promise<void> {
  const now = Date.now();
  // Use cached result if still valid
  if (now - lastCheckTime < CHECK_CACHE_DURATION) {
    console.info('[Toolbar] Hover check (cached): blocked=', $game_mode_fullscreen_blocked.value);
    return;
  }
  
  try {
    const blocked = await invokeCommand(FuncCommand.SystemIsGameFullscreenBlocked);
    console.info('[Toolbar] Hover check: blocked=', blocked);
    $game_mode_fullscreen_blocked.value = blocked as boolean;
    lastCheckTime = now;
  } catch (e) {
    console.warn('[Toolbar] Failed to check game fullscreen blocked', e);
  }
}

export interface ToolbarSettings {
  position: "Top" | "Bottom";
  hideMode: "Never" | "Always" | "OnOverlap";
  height: number;
  delayToHide: number;
  delayToShow: number;
}

const initialSettings: ToolbarSettings & { dateFormat: string } = {
  position: "Top",
  hideMode: "Never",
  height: 32,
  delayToHide: 0,
  delayToShow: 120,
  dateFormat: "YYYY-MM-DD HH:mm:ss",
};

export const $settings = signal(initialSettings);

// Trigger check when mouse is at edge
effect(() => {
  const isMouseOverEdge = $mouse_at_edge.value === $settings.value.position;
  if (isMouseOverEdge) {
    checkGameFullscreenBlocked();
  }
});

// Listen for game fullscreen state changes from backend (immediate notification)
await getCurrentWebviewWindow().listen<boolean>(
  FuncEvent.GameFullscreenChanged,
  (event) => {
    console.info('[Toolbar] GameFullscreenChanged event:', event.payload);
    $game_mode_fullscreen_blocked.value = !!event.payload;
    lastCheckTime = Date.now();
  },
);

export const $open_popups = signal<Record<string, boolean>>({});
export const $there_are_open_popups = computed(() => Object.values($open_popups.value).some((v) => v));
export const $is_toolbar_overlaped = signal(false);
export const $check_update_modal_open = signal(false);
export const $check_update_version = signal("");

// Listen overlap state from backend to drive OnOverlap mode
await getCurrentWebviewWindow().listen<boolean>(
  FuncEvent.ToolbarOverlaped,
  (event) => {
    $is_toolbar_overlaped.value = !!event.payload;
  },
);

// 初始化同步：主动查询后端 overlap 状态
// 修复：ToolbarOverlaped 事件可能在 webview 就绪前就已发射，导致前端丢失事件
try {
  const backendOverlapState = await invokeCommand<boolean>('toolbar_get_overlap_state');
  $is_toolbar_overlaped.value = !!backendOverlapState;
} catch (e) {
  console.warn('[Toolbar] Failed to query initial overlap state', e);
}

// Initialize settings from backend (registry-backed) and keep in sync on changes
try {
  const loaded = await invokeCommand<any>(FuncCommand.StateGetSettings);
  if (loaded?.byWidget?.fancyToolbar) {
    const ft = loaded.byWidget.fancyToolbar;
    $settings.value = {
      ...$settings.value,
      position: ft.position ?? $settings.value.position,
      hideMode: ft.hideMode ?? $settings.value.hideMode,
      height: ft.height ?? $settings.value.height,
      delayToHide: ft.delayToHide ?? $settings.value.delayToHide,
      delayToShow: ft.delayToShow ?? $settings.value.delayToShow,
    };
  }
} catch (e) {
  console.warn('[Toolbar] Failed to load settings from backend', e);
}

await getCurrentWebviewWindow().listen<any>(
  FuncEvent.StateSettingsChanged,
  (event) => {
    const payload = event.payload as any;
    const ft = payload?.byWidget?.fancyToolbar;
    if (ft) {
      $settings.value = {
        ...$settings.value,
        position: ft.position ?? $settings.value.position,
        hideMode: ft.hideMode ?? $settings.value.hideMode,
        height: ft.height ?? $settings.value.height,
        delayToHide: ft.delayToHide ?? $settings.value.delayToHide,
        delayToShow: ft.delayToShow ?? $settings.value.delayToShow,
      };
    }
  },
);

export const $has_maximized_window = signal(false);

// Listen for maximized window state to control background style
await getCurrentWebviewWindow().listen<boolean>(
  FuncEvent.ToolbarHasMaximizedWindow,
  (event) => {
    $has_maximized_window.value = !!event.payload;
  },
);

// 初始化同步：主动查询后端最大化窗口状态
// 修复：ToolbarHasMaximizedWindow 事件可能在 webview 就绪前就已发射，或在 webview 重载后丢失
// 参考 overlap 状态的初始化同步机制
try {
  const backendMaximizedState = await invokeCommand<boolean>('toolbar_get_maximized_state');
  $has_maximized_window.value = !!backendMaximizedState;
} catch (e) {
  console.warn('[Toolbar] Failed to query initial maximized state', e);
}

const currentWindow = getCurrentWebviewWindow();

export const $bar_should_be_hidden = signal(false);

// Detect if mouse is within the visual toolbar area (height band)
export const $mouse_over_toolbar_area = computed<boolean>(() => {
  const monitor = $current_monitor.value;
  if (!monitor) return false;
  const { rect } = monitor;
  const pos = $mouse_pos.value;
  const height = $settings.value.height;
  const side = $settings.value.position;
  const withinX = pos.x >= rect.left && pos.x <= rect.right;
  if (!withinX) return false;
  if (side === "Top") {
    return pos.y >= rect.top && pos.y <= rect.top + height;
  } else {
    return pos.y <= rect.bottom && pos.y >= rect.bottom - height;
  }
});

const setToolbarAsHidden = computed(() => {
  return debounce(
    () => {
      $bar_should_be_hidden.value = true;
    },
    $settings.value.delayToHide,
  );
});

const setToolbarAsNotHidden = computed(() => {
   return debounce(
    () => {
      $bar_should_be_hidden.value = false;
    },
    $settings.value.delayToShow,
  );
});

effect(() => {
  let hidden = false;
  let flush = false;
  const isMouseOverEdge = $mouse_at_edge.value === $settings.value.position;
  const focused = $is_this_webview_focused.value || document.hasFocus();

  // Game scene + overlap: do not reveal toolbar on hover while a game overlaps it.
  if ($game_mode_fullscreen_blocked.value && $is_toolbar_overlaped.value) {
    hidden = !$there_are_open_popups.value && !focused;
    if (hidden) {
      setToolbarAsNotHidden.peek().cancel();
      setToolbarAsHidden.peek()();
    } else {
      setToolbarAsHidden.peek().cancel();
      setToolbarAsNotHidden.peek()();
    }
    return;
  }
  const isHovered = !$bar_should_be_hidden.peek()
    ? $mouse_over_toolbar_area.value
    : isMouseOverEdge;

  switch ($settings.value.hideMode) {
    case "Never":
      hidden = false;
      flush = true;
      break;
    case "Always":
      hidden = !$there_are_open_popups.value && !isHovered;
      break;
    case "OnOverlap":
      hidden = $is_toolbar_overlaped.value &&
        !focused &&
        !$there_are_open_popups.value &&
        !isHovered;
      break;
  }

  if (hidden) {
    setToolbarAsNotHidden.peek().cancel();
    setToolbarAsHidden.peek()();
    return;
  }

  setToolbarAsHidden.peek().cancel();
  setToolbarAsNotHidden.peek()();
  if (flush) {
    setToolbarAsNotHidden.peek().flush();
  }
});

// Emit edge-active to backend so it can toggle topmost only when needed (OnOverlap & overlapped & at edge)
effect(() => {
  const settings = $settings.value;
    const isOnOverlap = settings.hideMode === 'OnOverlap';
    const overlapped = $is_toolbar_overlaped.value;
    const edgeActive = $mouse_at_edge.value === settings.position;
    // Only emit when OnOverlap; send true at edge while overlapped; otherwise false
    const shouldTopMost = isOnOverlap && overlapped && edgeActive;
    currentWindow.emit('toolbar:edge-active', shouldTopMost).catch(() => {});
});

// Sync CSS variables with runtime settings to avoid visual gaps
effect(() => {
  const root = document.documentElement;
  const height = $settings.value.height;
  root.style.setProperty("--config-height", `${height}px`);
});

// Sync toolbar background color state to CSS for controlling text/icon colors
effect(() => {
  const root = document.documentElement;
  const hasBlackBackground = $has_maximized_window.value;
  root.style.setProperty("--toolbar-background-is-black", hasBlackBackground ? "1" : "0");
});
