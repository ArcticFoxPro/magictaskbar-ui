import { computed, effect, signal } from "@preact/signals";
import { HideMode, FuncEvent, Settings, IconPackManager, FuncCommand } from "@magic-ui/lib";
import { TaskbarSettings } from "@magic-ui/lib/types";
import { $is_this_webview_focused } from "@shared/signals";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { invoke } from "@tauri-apps/api/core";
import { debounce } from "lodash";

import { $current_monitor, $mouse_at_edge } from "./system";

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
    console.info('[Taskbar] Hover check (cached): blocked=', $game_mode_fullscreen_blocked.value);
    return;
  }
  
  try {
    const blocked = await invoke(FuncCommand.SystemIsGameFullscreenBlocked);
    console.info('[Taskbar] Hover check: blocked=', blocked);
    $game_mode_fullscreen_blocked.value = blocked as boolean;
    lastCheckTime = now;
  } catch (e) {
    console.warn('[Taskbar] Failed to check game fullscreen blocked', e);
  }
}

export const $settings = signal<TaskbarSettings>(
  (await Settings.getAsync()).magicTaskbar,
);
Settings.onChange((settings) => ($settings.value = settings.magicTaskbar));

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
    console.info('[Taskbar] GameFullscreenChanged event:', event.payload);
    $game_mode_fullscreen_blocked.value = !!event.payload;
    lastCheckTime = Date.now();
  },
);

// 监听背板风格变更事件（来自设置窗口）
// 这确保即使后端没有触发 StateSettingsChanged 事件，
// 任务栏的 CSS 类也会被立即更新
await getCurrentWebviewWindow().listen<{ style: string }>(
  'backplate-style-changed',
  async (event) => {
    console.log(`[Taskbar] Received backplate-style-changed event: ${event.payload.style}`);
    const newStyle = event.payload.style as 'Transparent' | 'White';
    if ($settings.value.iconBackplateStyle !== newStyle) {
      $settings.value = { ...$settings.value, iconBackplateStyle: newStyle };
    }

    // 🔧 关键修复：先清除图标缓存，确保切换背板后显示正确的图标
    await IconPackManager.clearCachedIcons();

    // 🔧 将 Tauri 事件转发为 DOM 事件，通知 FileIcon 组件重新加载图标
    // FileIcon 使用 window.addEventListener 监听，所以需要发送 DOM 事件
    window.dispatchEvent(new CustomEvent('backplate-style-changed', { detail: { style: newStyle } }));
  },
);

export const $is_dock_overlaped = signal(false);
await getCurrentWebviewWindow().listen<boolean>(
  FuncEvent.TaskbarOverlaped,
  (event) => {
    $is_dock_overlaped.value = event.payload;
  },
);

export const $open_popups = signal<Record<string, boolean>>({});
export const $there_are_open_popups = computed(() => Object.values($open_popups.value).some((v) => v));

const getOpenPopupKeys = () => Object.entries($open_popups.value)
  .filter(([, value]) => value)
  .map(([key]) => key);

export const setTaskbarOpenPopups = (next: Record<string, boolean>, reason: string) => {
  $open_popups.value = next;
  console.info('[Taskbar] Open popup state changed:', {
    reason,
    openPopupKeys: getOpenPopupKeys(),
    openPopupsRaw: $open_popups.value,
  });
};

// Native tray overflow window visibility state
// Updated via WinEvent hook when the window shows/hides
export const $native_tray_overflow_visible = signal(false);

// Listen for native tray overflow window visibility changes
await getCurrentWebviewWindow().listen<boolean>(
  "native_tray_overflow::visibility_changed",
  (event) => {
    $native_tray_overflow_visible.value = event.payload;
    console.log('[Taskbar] Native tray overflow visibility:', event.payload);
  },
);

// 监听预览窗口的打开/关闭状态
await getCurrentWebviewWindow().listen<{ open: boolean; monitorId?: string | null }>(
  "preview::window_open",
  (event) => {
    if (event.payload.monitorId !== $current_monitor.value?.id) {
      return;
    }

    if (event.payload.open) {
      console.log('[Taskbar] Preview window opened');
      setTaskbarOpenPopups({ ...$open_popups.value, preview: true }, 'preview-open');
    } else {
      console.log('[Taskbar] Preview window closed');
      // 关闭时删除键，而不是设置为 false
      const { preview, ...$rest } = $open_popups.value;
      setTaskbarOpenPopups($rest, 'preview-close');
    }
  },
);

// 监听右键菜单窗口的打开/关闭状态
await getCurrentWebviewWindow().listen<{ open: boolean; monitorId?: string | null }>(
  "contextmenu::window_open",
  (event) => {
    const monitorId = event.payload.monitorId ?? null;
    const isForThisMonitor = monitorId === $current_monitor.value?.id;

    if (event.payload.open && !isForThisMonitor) {
      return;
    }

    if (event.payload.open) {
      console.log('[Taskbar] ContextMenu window opened');
      setTaskbarOpenPopups({ ...$open_popups.value, contextmenu: true }, 'contextmenu-open');
    } else {
      console.log('[Taskbar] ContextMenu window closed');
      // 关闭时删除键，而不是设置为 false
      const { contextmenu, ...$rest } = $open_popups.value;
      setTaskbarOpenPopups($rest, 'contextmenu-close');
    }
  },
);

export const $dock_should_be_hidden = signal(false);
const setDockAsHidden = computed(() => {
  return debounce(
    () => {
      $dock_should_be_hidden.value = true;
    },
    $settings.value.delayToHide,
  );
});
const setDockAsNotHidden = computed(() => {
  return debounce(
    () => {
      $dock_should_be_hidden.value = false;
    },
    $settings.value.delayToShow,
  );
});

// 标记是否已经初始化过重叠状态
let isOverlapInitialized = false;

effect(() => {
  // 第一次执行时，主动从后端获取重叠状态
  if (!isOverlapInitialized) {
    isOverlapInitialized = true;
    invoke('check_taskbar_overlap_status').then((isOverlaped) => {
      console.debug('[Taskbar] 首次执行，从后端获取重叠状态:', isOverlaped);
      $is_dock_overlaped.value = isOverlaped as boolean;
    }).catch((error) => {
      console.error('[Taskbar] 获取重叠状态失败:', error);
    });
  }

  let hidden = false;
  let flush = false;

  let isMouseOverEdge = $mouse_at_edge.value === $settings.value.position;

  // Game scene + overlap: do not reveal taskbar on hover while a game overlaps it.
  if ($game_mode_fullscreen_blocked.value && $is_dock_overlaped.value) {
      hidden = !$is_this_webview_focused.value && !$there_are_open_popups.value;
    if (hidden) {
      setDockAsNotHidden.peek().cancel();
      setDockAsHidden.peek()();
    } else {
      setDockAsHidden.peek().cancel();
      setDockAsNotHidden.peek()();
    }
    return;
  }

  switch ($settings.value.hideMode) {
    case HideMode.Never:
      hidden = false;
      flush = true;
      break;
    case HideMode.Always:
      hidden = !$is_this_webview_focused.value &&
        !$there_are_open_popups.value && !isMouseOverEdge;
      break;
    case HideMode.OnOverlap:
      hidden = $is_dock_overlaped.value &&
        !$there_are_open_popups.value &&
        !isMouseOverEdge;
      // 打印完整的隐藏逻辑状态
      console.warn('Taskbar Hide Logic (OnOverlap):', {
        hidden,
        overlaped: $is_dock_overlaped.value,
        focused: $is_this_webview_focused.value,
        openPopups: $there_are_open_popups.value,
        openPopupKeys: getOpenPopupKeys(),
        openPopupsRaw: $open_popups.value,
        mouseAtEdge: isMouseOverEdge,
        mouseValue: $mouse_at_edge.value,
        position: $settings.value.position,
        hideMode: $settings.value.hideMode,
	tray: $native_tray_overflow_visible.value
      });
      break;
  }

  if (hidden) {
    setDockAsNotHidden.peek().cancel();
    setDockAsHidden.peek()();
    return;
  }

  setDockAsHidden.peek().cancel();
  setDockAsNotHidden.peek()();
  if (flush) {
    setDockAsNotHidden.peek().flush();
  }
});

// 监听 #root 的 CSS transform transition 来同步亚克力效果
// 直接追踪容器的实际动画状态，比鼠标悬停检测更可靠
{
  const rootEl = document.getElementById('root');
  if (rootEl) {
    // 容器开始滑出时（开始隐藏）→ 立即隐藏亚克力
    rootEl.addEventListener('transitionstart', (e) => {
      if (e.target !== rootEl || (e as TransitionEvent).propertyName !== 'transform') return;
      const taskbar = rootEl.querySelector('.taskbar');
      if (taskbar?.classList.contains('hidden') && !rootEl.matches(':hover')) {
        invoke('taskbar_hide_glass_effect');
      }
    });

    // 容器滑入完成时（恢复可见）→ 显示亚克力
    rootEl.addEventListener('transitionend', (e) => {
      if (e.target !== rootEl || (e as TransitionEvent).propertyName !== 'transform') return;
      const transform = getComputedStyle(rootEl).transform;
      if (transform === 'none' || transform === 'matrix(1, 0, 0, 1, 0, 0)') {
        invoke('taskbar_show_glass_effect');
        invoke('taskbar_bring_to_front');
      }
    });
  }
}

// 阻止键盘导航键的默认滚动行为
// 当 taskbar 隐藏时（CSS transform 偏移），若 WebView 意外获焦，
// 方向键会触发浏览器内部滚动，把 transform 隐藏的内容滚回可视区域
globalThis.addEventListener('keydown', (e) => {
  switch (e.key) {
    case 'ArrowUp':
    case 'ArrowDown':
    case 'ArrowLeft':
    case 'ArrowRight':
    case 'PageUp':
    case 'PageDown':
    case 'Home':
    case 'End':
    case ' ':
      e.preventDefault();
      break;
  }
});
