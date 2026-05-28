import { FuncCommand } from "@magic-ui/lib";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { info as logInfo } from "@tauri-apps/plugin-log";
import { useCallback, useEffect, useRef, useState } from "react";

import { PreviewList } from "./components/PreviewList";

// 预览窗口数据类型
interface PreviewWindowInfo {
  handle: number;
  title: string;
  iconPngBase64?: string;
  isFocused: boolean;
}

interface PreviewPosition {
  x: number;
  y: number;
  placement: string;
}

interface PreviewMonitorRect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

interface PreviewShowPayload {
  itemId: string;
  displayName: string;
  windows: PreviewWindowInfo[];
  position: PreviewPosition;
  monitorId?: string | null;
  monitorRect?: PreviewMonitorRect | null;
  monitorDpi?: number | null;
  path?: string;
  umid?: string;
  appIconBase64?: string;
  appIconSrc?: string;
}

// 事件名称常量
const PREVIEW_SHOW_EVENT = "preview::show";
const PREVIEW_HIDE_EVENT = "preview::hide";
const PREVIEW_MOUSE_ENTER_EVENT = "preview::mouse_enter";
const PREVIEW_MOUSE_LEAVE_EVENT = "preview::mouse_leave";
const PREVIEW_WINDOW_OPEN_EVENT = "preview::window_open";

// 命令名称常量
const PREVIEW_SET_POSITION_CMD = "preview_set_position";
const PREVIEW_SHOW_CMD = "preview_show";
const PREVIEW_HIDE_CMD = "preview_hide";

const decodeBase64Url = (value: string) => {
  const normalized = value.replace(/_/g, '/').replace(/-/g, '+');
  const padding = (4 - (normalized.length % 4)) % 4;
  return atob(normalized.padEnd(normalized.length + padding, '='));
};

const getCurrentMonitorId = () => {
  try {
    const decoded = decodeBase64Url(getCurrentWebviewWindow().label);
    const monitorIdMatch = decoded.match(/monitorId=([^&]+)/);
    return monitorIdMatch?.[1] ? decodeURIComponent(monitorIdMatch[1]) : null;
  } catch {
    return null;
  }
};

const clampToMonitorX = (x: number, width: number, monitorRect?: PreviewMonitorRect | null) => {
  if (!monitorRect) {
    return Math.round(x);
  }

  return Math.max(monitorRect.left, Math.min(Math.round(x), monitorRect.right - width));
};

const getMonitorBottomY = (
  height: number,
  dpiScale: number,
  monitorRect?: PreviewMonitorRect | null,
) => {
  const bottom = monitorRect?.bottom ?? Math.round(globalThis.screen.height * dpiScale);
  return Math.max(monitorRect?.top ?? 0, bottom - height - Math.round(90 * dpiScale));
};

const getPayloadDpi = (payload?: PreviewShowPayload | null) => {
  return payload?.monitorDpi || globalThis.devicePixelRatio || 1;
};

export function App() {
  const [visible, setVisible] = useState(false);
  const [data, setData] = useState<PreviewShowPayload | null>(null);
  const [hasClosedWindow, setHasClosedWindow] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const resizeObserverRef = useRef<ResizeObserver | null>(null);
  const adjustFrameRef = useRef<number | null>(null);
  const settleFrameRef = useRef<number | null>(null);
  const suppressedLeaveHideTimerRef = useRef<number | null>(null);
  const closeInteractionUntilRef = useRef(0);
  const pendingPositionRef = useRef<{ x: number; y: number; placement: string } | null>(null);
  const currentPositionRef = useRef<{ x: number; y: number; placement: string } | null>(null);
  const pendingPositionSeqRef = useRef(0);
  const latestDataRef = useRef<PreviewShowPayload | null>(null);
  const contextMenuItemIdRef = useRef<string | null>(null);
  const contextMenuDisplayNameRef = useRef<string | null>(null);
  const visibleRef = useRef(false);

  // 同步更新 visible 状态和 ref（ref 供 useEffect([]) 闭包中读取最新值）
  const updateVisible = useCallback((v: boolean) => {
    visibleRef.current = v;
    setVisible(v);
  }, []);

  const cancelScheduledAdjustWindowSize = useCallback(() => {
    if (adjustFrameRef.current !== null) {
      cancelAnimationFrame(adjustFrameRef.current);
      adjustFrameRef.current = null;
    }

    if (settleFrameRef.current !== null) {
      cancelAnimationFrame(settleFrameRef.current);
      settleFrameRef.current = null;
    }
  }, []);

  const clearSuppressedLeaveHideTimer = useCallback(() => {
    if (suppressedLeaveHideTimerRef.current !== null) {
      clearTimeout(suppressedLeaveHideTimerRef.current);
      suppressedLeaveHideTimerRef.current = null;
    }
  }, []);

  const hidePreviewWindow = useCallback((itemId?: string) => {
    clearSuppressedLeaveHideTimer();
    closeInteractionUntilRef.current = 0;
    cancelScheduledAdjustWindowSize();
    setHasClosedWindow(false);
    if (itemId) {
      emit(PREVIEW_MOUSE_LEAVE_EVENT, { itemId });
    }
    updateVisible(false);
    emit(PREVIEW_WINDOW_OPEN_EVENT, { open: false, monitorId: data?.monitorId ?? null });
    invoke(PREVIEW_HIDE_CMD, { monitorId: data?.monitorId ?? null }).catch((err) => {
      console.error("[Preview] Failed to hide:", err);
    });
  }, [cancelScheduledAdjustWindowSize, clearSuppressedLeaveHideTimer, data?.monitorId, updateVisible]);

  // 鼠标进入预览窗口
  const handleMouseEnter = useCallback(() => {
    if (data) {
      clearSuppressedLeaveHideTimer();
      closeInteractionUntilRef.current = 0;
      const rect = containerRef.current?.getBoundingClientRect();
      emit(PREVIEW_MOUSE_ENTER_EVENT, { itemId: data.itemId });
      // 通知 taskbar 预览窗口正在显示
      emit(PREVIEW_WINDOW_OPEN_EVENT, { open: true, monitorId: data.monitorId ?? null });
    }
  }, [clearSuppressedLeaveHideTimer, data]);

  // 鼠标离开预览窗口：立即隐藏
  const handleMouseLeave = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    if (data) {
      const rect = containerRef.current?.getBoundingClientRect();

      if (performance.now() < closeInteractionUntilRef.current) {
        clearSuppressedLeaveHideTimer();
        suppressedLeaveHideTimerRef.current = window.setTimeout(() => {
          hidePreviewWindow(data.itemId);
        }, 1200);
        return;
      }
      hidePreviewWindow(data.itemId);
    }
  }, [clearSuppressedLeaveHideTimer, data, hidePreviewWindow]);

  // 处理窗口点击
  const handleWindowClick = useCallback(async (handle: number, isFocused: boolean) => {
    try {
      await invoke(FuncCommand.SetForegroundWindow, { hwnd: handle });
      hidePreviewWindow(data?.itemId);
    } catch (e) {
      console.error("[Preview] Failed to set foreground window:", e);
    }
  }, [data?.itemId, hidePreviewWindow]);

  // 调整 preview 窗口大小和位置
  const adjustWindowSize = useCallback(async () => {
    if (!containerRef.current || !currentPositionRef.current) return;

    const latestData = latestDataRef.current;
    if (!latestData?.monitorId) return;

    const position = currentPositionRef.current;
    const dpiScale = getPayloadDpi(latestData);

    // 获取内容实际大小
    const rect = containerRef.current.getBoundingClientRect();
    const width = Math.ceil(rect.width * dpiScale);
    const height = Math.ceil(rect.height * dpiScale);

    // 计算窗口位置，确保不超出当前显示器边界
    const windowX = clampToMonitorX(position.x, width, latestData.monitorRect);

    const windowY = position.placement === "top"
      ? position.y - height - 28
      : position.y + 28;

    try {
      await invoke(PREVIEW_SET_POSITION_CMD, {
        x: windowX,
        y: windowY,
        width,
        height,
        monitorId: latestData.monitorId,
      });
    } catch (e) {
      console.error("[Preview] Failed to adjust window size:", e);
    }
  }, []);

  // 处理关闭窗口
  const handleCloseWindow = useCallback(async (handle: number) => {
    // 标记已在 preview 中关闭过窗口
    setHasClosedWindow(true);
    clearSuppressedLeaveHideTimer();
    closeInteractionUntilRef.current = performance.now() + 400;
    const isClosingLastWindow = data?.windows.length === 1;

    // 立即从前端列表中移除，实现即时刷新
    setData((prevData) => {
      if (!prevData) return null;
      const updatedWindows = prevData.windows.filter((w) => w.handle !== handle);
      // 如果关闭后没有窗口了或窗口数量<=1，隐藏 preview
      if (updatedWindows.length === 0) {
        return null;
      }
      return { ...prevData, windows: updatedWindows };
    });

    if (isClosingLastWindow) {
      console.debug('[Preview] No windows left after closing, hiding');
      hidePreviewWindow(data?.itemId);
    }

    // 延迟调整窗口大小，等待 DOM 更新完成
    // Closed-window resizing is scheduled by a follow-up effect after the DOM settles.
    try {
      await invoke(FuncCommand.TaskbarCloseApp, { hwnd: handle });
    } catch (e) {
      console.error("[Preview] Failed to close window:", e);
    }
  }, [clearSuppressedLeaveHideTimer, data, hidePreviewWindow]);

  useEffect(() => {
    const webview = getCurrentWebviewWindow();

    // 初始化主题
    invoke<boolean>("get_is_dark_mode").then((isDark) => {
      if (isDark) document.body.classList.add("dark");
      else document.body.classList.remove("dark");
    }).catch(() => {});

    // 监听主题切换
    const unlistenTheme = listen<{ is_dark: boolean }>("theme::changed", (event) => {
      if (event.payload.is_dark) document.body.classList.add("dark");
      else document.body.classList.remove("dark");
    });

    // 监听 ContextMenu 图标变化事件
    const unlistenContextMenu = listen<{ itemId: string | null; displayName: string | null }>("contextmenu::item_changed", (event) => {
      contextMenuItemIdRef.current = event.payload?.itemId;
      contextMenuDisplayNameRef.current = event.payload?.displayName || null;
    });

    // 监听显示事件
    const unlistenShow = listen<PreviewShowPayload>(PREVIEW_SHOW_EVENT, async (event) => {
      const payload = event.payload;

      // 检查窗口数量，如果<=1 则隐藏已有 preview 并退出
      // 修复：鼠标从多窗口图标滑到单窗口图标时，旧 preview 必须被主动隐藏，
      // 否则旧图标的 hideTimer 因 activePreviewItemId 已变更而无法执行隐藏
      if (payload.windows.length <= 1) {
        if (visibleRef.current) {
          clearSuppressedLeaveHideTimer();
          closeInteractionUntilRef.current = 0;
          cancelScheduledAdjustWindowSize();
          setHasClosedWindow(false);
          updateVisible(false)
          setData(null);
          latestDataRef.current = null;
          emit(PREVIEW_WINDOW_OPEN_EVENT, { open: false, monitorId: payload.monitorId ?? null });
          invoke(PREVIEW_HIDE_CMD, { monitorId: payload.monitorId ?? null }).catch((e) => {
            console.error("[Preview] Failed to hide on single-window switch:", e);
          });
        }
        return;
      }

      // 先清空 data 再设置，确保触发 React 状态更新
      cancelScheduledAdjustWindowSize();
      pendingPositionRef.current = null;
      setData(null);
      latestDataRef.current = null;
      await new Promise(resolve => setTimeout(resolve, 0));

      // 先检查 ContextMenu 是否可见
      const contextMenuItemIdBeforeDelay = contextMenuItemIdRef.current;
      if (contextMenuItemIdBeforeDelay !== null) {
        await new Promise(resolve => setTimeout(resolve, 200));
      }

      // 延迟后主动查询后端 ContextMenu 状态（避免事件丢失）
      let contextMenuItemId = contextMenuItemIdRef.current;
      let contextMenuDisplayName = contextMenuDisplayNameRef.current;
      try {
        const contextMenuState = await invoke<{ itemId: string | null; displayName: string | null }>("contextmenu_get_state");
        if (contextMenuState) {
          contextMenuItemIdRef.current = contextMenuState.itemId;
          contextMenuDisplayNameRef.current = contextMenuState.displayName || null;
          contextMenuItemId = contextMenuState.itemId;
          contextMenuDisplayName = contextMenuState.displayName || null;
        }
      } catch (e) {
        logInfo("[Preview] Failed to get contextmenu state from backend:");
      }

      // 检查 ContextMenu 是否正在显示，且是否为同一个图标
      if (contextMenuItemId !== null && contextMenuItemId === payload.itemId) {
        return;
      }
      // 重置关闭窗口标记
      setHasClosedWindow(false);

      // 如果没有图标数据，尝试通过 path 获取应用图标
      if (!payload.appIconBase64 && payload.path) {
        try {
          const processName = payload.path.split('\\').pop()?.split('.')[0];
          if (processName) {
            const iconBase64 = await invoke('get_local_icon', { processName });
            if (iconBase64) {
              payload.appIconBase64 = (iconBase64 as string).replace(/^data:image\/png;base64,/, '');
            }
          }
        } catch (e) {
          console.debug("[Preview] Failed to get local icon:", e);
        }
      }

      // 更新数据
      const positionSeq = pendingPositionSeqRef.current + 1;
      pendingPositionSeqRef.current = positionSeq;
      latestDataRef.current = payload;
      setData(payload);

      // 保存位置信息，等待 ResizeObserver 触发后再设置
      pendingPositionRef.current = { ...payload.position, seq: positionSeq } as any;
      currentPositionRef.current = payload.position;
    });

    // 监听隐藏事件
    const unlistenHide = listen(PREVIEW_HIDE_EVENT, async () => {
      cancelScheduledAdjustWindowSize();
      clearSuppressedLeaveHideTimer();
      closeInteractionUntilRef.current = 0;
      setHasClosedWindow(false);
      updateVisible(false)
      latestDataRef.current = null;
      emit(PREVIEW_WINDOW_OPEN_EVENT, { open: false, monitorId: data?.monitorId ?? null });
      // 不清空 data，让内容保留以便下次快速显示
      try {
        await invoke(PREVIEW_HIDE_CMD, { monitorId: data?.monitorId ?? null });
      } catch (e) {
        console.error("[Preview] Failed to hide:", e);
      }
    });

    // 初始隐藏窗口
    webview.hide();

    // 通知后端前端已就绪，可以接收事件了
    Promise.all([unlistenShow, unlistenHide]).then(() => {
      invoke(FuncCommand.PreviewReady, { monitorId: getCurrentMonitorId() }).catch((e) => {
        console.error('[Preview] Failed to notify ready:', e);
      });
    });

    return () => {
      clearSuppressedLeaveHideTimer();
      closeInteractionUntilRef.current = 0;
      cancelScheduledAdjustWindowSize();
      unlistenShow.then((fn) => fn());
      unlistenHide.then((fn) => fn());
      unlistenContextMenu.then((fn) => fn());
      unlistenTheme.then((fn) => fn());
    };
  }, [cancelScheduledAdjustWindowSize, clearSuppressedLeaveHideTimer]);

  // ResizeObserver: 自动根据内容大小调整窗口
  useEffect(() => {
    if (!hasClosedWindow || !data || pendingPositionRef.current || !currentPositionRef.current) {
      return;
    }

    cancelScheduledAdjustWindowSize();

    // Wait for React to commit and layout to settle before reading the resized preview.
    adjustFrameRef.current = requestAnimationFrame(() => {
      adjustFrameRef.current = null;
      settleFrameRef.current = requestAnimationFrame(() => {
        settleFrameRef.current = null;
        adjustWindowSize();
      });
    });

    return cancelScheduledAdjustWindowSize;
  }, [adjustWindowSize, cancelScheduledAdjustWindowSize, data, hasClosedWindow]);

  useEffect(() => {
    if (!containerRef.current) return;

    const handleResize = async (entries: ResizeObserverEntry[]) => {
      const entry = entries[0];
      if (!entry || !pendingPositionRef.current) return;
      const latestData = latestDataRef.current;
      const pending = pendingPositionRef.current as PreviewPosition & { seq?: number };
      if (!latestData?.monitorId || pending.seq !== pendingPositionSeqRef.current) return;

      if (latestData.windows.length === 0) {
        // 应用窗口模式：检查窗口数量，如果<=1 则不显示 preview
        updateVisible(false)
        emit(PREVIEW_WINDOW_OPEN_EVENT, { open: false, monitorId: latestData.monitorId });
        invoke(PREVIEW_HIDE_CMD, { monitorId: latestData.monitorId }).catch((e) => {
          console.error("[Preview] Failed to hide:", e);
        });
        return;
      }

      // 检查 ContextMenu 是否正在显示同一个图标
      const contextMenuItemId = contextMenuItemIdRef.current;
      if (contextMenuItemId !== null && contextMenuItemId === latestData.itemId) {
          return;
      }

      if (
        latestData.windows.length > 1 &&
        !containerRef.current.querySelector(".preview-window-list")
      ) {
        return;
      }

      const position = pending;
      const dpiScale = getPayloadDpi(latestData);

      // 获取内容实际大小
      const width = Math.ceil(entry.contentRect.width * dpiScale);
      const height = Math.ceil(entry.contentRect.height * dpiScale);

      // 计算窗口位置，确保不超出屏幕边界
      // 窗口左对齐，并限制在当前显示器范围内
      const windowX = clampToMonitorX(position.x, width, latestData.monitorRect);

      // 窗口底部距离屏幕底部固定距离
      const windowY = getMonitorBottomY(height, dpiScale, latestData.monitorRect);

      try {
        await invoke(PREVIEW_SET_POSITION_CMD, {
          x: windowX,
          y: windowY,
          width,
          height,
          monitorId: latestData.monitorId,
        });

        updateVisible(true)
        // 立即通知 taskbar 预览窗口已打开（防止任务栏隐藏）
        emit(PREVIEW_WINDOW_OPEN_EVENT, { open: true, monitorId: latestData.monitorId });
        await invoke(PREVIEW_SHOW_CMD, { monitorId: latestData.monitorId });

        // 清除 pending 位置，但保留 currentPosition 用于后续调整
        pendingPositionRef.current = null;
      } catch (e) {
        console.error("[Preview] Failed to resize window:", e);
      }
    };

    resizeObserverRef.current = new ResizeObserver(handleResize);
    resizeObserverRef.current.observe(containerRef.current, { box: "border-box" });

    return () => {
      if (resizeObserverRef.current) {
        resizeObserverRef.current.disconnect();
      }
    };
  }, [data]); // 只依赖 data

  return (
    <div
      ref={containerRef}
      className="preview-container"
      onMouseEnter={handleMouseEnter}
      onMouseLeave={handleMouseLeave}
      tabIndex={-1}
    >
      {data && (data.windows.length > 1 || hasClosedWindow) ? (
        <PreviewList
          windows={data.windows}
          appIconBase64={data.appIconBase64}
          appIconSrc={data.appIconSrc}
          onWindowClick={handleWindowClick}
          onCloseWindow={handleCloseWindow}
        />
      ) : (
        <div className="preview-header preview-header-only" style={{ visibility: 'hidden' }}>
          <span className="preview-header-title">Placeholder</span>
        </div>
      )}
    </div>
  );
}
