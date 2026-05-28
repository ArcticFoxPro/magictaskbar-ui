import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { info as logInfo } from "@tauri-apps/plugin-log";
import { useCallback, useEffect, useRef, useState } from "react";

// 序列化的菜单项类型
interface SerializedMenuItem {
  key: string;
  label: string;
  iconSvgUrl?: string;       // SVG icon URL (e.g. /static/icons/xxx.svg)
  iconImgSrc?: string;       // img src (e.g. data:image/png;base64,...)
  danger?: boolean;
  disabled?: boolean;
  divider?: boolean;
}

interface ContextMenuPosition {
  x: number;
  y: number;
  placement: string;
}

interface ContextMenuShowPayload {
  menuType: "app" | "taskbar";
  items: SerializedMenuItem[];
  position: ContextMenuPosition;
  sourceItemId?: string;
  sourceDisplayName?: string;
  monitorId?: string | null;
  monitorDpi?: number | null;
  monitorRect?: {
    left: number;
    top: number;
    right: number;
    bottom: number;
  } | null;
}

interface PendingPosition {
  position: ContextMenuPosition;
  token: number;
}

// 事件名称常量
const CONTEXTMENU_SHOW_EVENT = "contextmenu::show";
const CONTEXTMENU_ITEM_CLICK_EVENT = "contextmenu::item_click";
const CONTEXTMENU_WINDOW_OPEN_EVENT = "contextmenu::window_open"; // 通知 taskbar 窗口打开/关闭状态

// 命令名称常量
const CONTEXTMENU_SET_POSITION_CMD = "contextmenu_set_position";
const CONTEXTMENU_SHOW_CMD = "contextmenu_show";
const CONTEXTMENU_HIDE_CMD = "contextmenu_hide";
const CONTEXTMENU_READY_CMD = "contextmenu_ready";
const CONTEXTMENU_DESTROY_CMD = "contextmenu_destroy";

// 30 秒延迟销毁
const DESTROY_DELAY_MS = 30_000;

export function App() {
  const [visible, setVisible] = useState(false);
  const [data, setData] = useState<ContextMenuShowPayload | null>(null);
  const [previewItemId, setPreviewItemId] = useState<string | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const resizeObserverRef = useRef<ResizeObserver | null>(null);
  const pendingPositionRef = useRef<PendingPosition | null>(null);
  const latestPayloadRef = useRef<ContextMenuShowPayload | null>(null);
  const showTokenRef = useRef(0);
  const positionFrameRef = useRef<number | null>(null);
  const destroyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // 监听 Preview 图标变化事件
  useEffect(() => {
    const unlisten = listen<{ itemId: string }>("preview::item_changed", (event) => {
      console.log("[ContextMenu] Received preview item changed:", event.payload.itemId);
      setPreviewItemId(event.payload.itemId);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // 取消销毁定时器
  const cancelDestroyTimer = useCallback(() => {
    if (destroyTimerRef.current) {
      clearTimeout(destroyTimerRef.current);
      destroyTimerRef.current = null;
    }
  }, []);

  // 启动 30s 延迟销毁定时器
  const startDestroyTimer = useCallback(() => {
    cancelDestroyTimer();
    destroyTimerRef.current = setTimeout(async () => {
      logInfo("[ContextMenu] 30s inactivity, destroying window");
      try {
        await invoke(CONTEXTMENU_DESTROY_CMD);
      } catch (e) {
        console.error("[ContextMenu] Failed to destroy:", e);
        // fallback: 直接关闭窗口
        getCurrentWebviewWindow().close();
      }
    }, DESTROY_DELAY_MS);
  }, [cancelDestroyTimer]);

  // 隐藏菜单
  const positionAndShowMenu = useCallback(async (entryRect: DOMRectReadOnly) => {
    if (!pendingPositionRef.current) return;

    const pending = pendingPositionRef.current;
    const activeData = latestPayloadRef.current;
    if (!activeData || pending.token !== showTokenRef.current) {
      return;
    }

    const position = pending.position;
    const dpiScale = activeData.monitorDpi || globalThis.devicePixelRatio || 1;
    const width = Math.ceil(entryRect.width * dpiScale);
    const height = Math.ceil(entryRect.height * dpiScale);

    const monitorRect = activeData.monitorRect;
    const screenLeft = monitorRect?.left ?? 0;
    const screenTop = monitorRect?.top ?? 0;
    const screenRight = monitorRect?.right ?? Math.round(globalThis.screen.width * dpiScale);
    const screenBottom = monitorRect?.bottom ?? Math.round(globalThis.screen.height * dpiScale);

    let windowX = Math.round(position.x);
    let windowY = Math.round(screenBottom - height - Math.round(90 * dpiScale));

    windowX = Math.max(screenLeft, Math.min(windowX, screenRight - width));
    windowY = Math.max(screenTop, windowY);

    void logInfo(
      `[ContextMenu] set_position monitorId=${activeData.monitorId ?? "null"} monitorDpi=${activeData.monitorDpi ?? "null"} webviewDpr=${globalThis.devicePixelRatio || 1} css=${entryRect.width}x${entryRect.height} physical=${width}x${height} target=${windowX},${windowY}`,
    );

    try {
      await invoke(CONTEXTMENU_SET_POSITION_CMD, {
        x: windowX,
        y: windowY,
        width,
        height,
      });

      if (pending.token !== showTokenRef.current || latestPayloadRef.current !== activeData) {
        return;
      }

      emit(CONTEXTMENU_WINDOW_OPEN_EVENT, {
        open: true,
        monitorId: activeData.monitorId ?? null,
      }).catch((e) => {
        console.error("[ContextMenu] Failed to emit window_open true:", e);
      });

      setVisible(true);
      await invoke(CONTEXTMENU_SHOW_CMD);

      if (pending.token === showTokenRef.current) {
        pendingPositionRef.current = null;
      }
    } catch (e) {
      console.error("[ContextMenu] Failed to position/show window:", e);
    }
  }, []);

  const schedulePositionAndShow = useCallback(() => {
    if (positionFrameRef.current !== null) {
      cancelAnimationFrame(positionFrameRef.current);
    }

    positionFrameRef.current = requestAnimationFrame(() => {
      positionFrameRef.current = null;
      const container = containerRef.current;
      if (!container || !pendingPositionRef.current) return;
      positionAndShowMenu(container.getBoundingClientRect());
    });
  }, [positionAndShowMenu]);

  const hideMenu = useCallback(async () => {
    showTokenRef.current += 1;
    pendingPositionRef.current = null;
    if (positionFrameRef.current !== null) {
      cancelAnimationFrame(positionFrameRef.current);
      positionFrameRef.current = null;
    }
    const monitorId = latestPayloadRef.current?.monitorId ?? data?.monitorId ?? null;
    // 通知 taskbar ContextMenu 窗口已关闭
    emit(CONTEXTMENU_WINDOW_OPEN_EVENT, {
      open: false,
      monitorId,
    }).catch((e) => {
      console.error("[ContextMenu] Failed to emit window_open false:", e);
    });

    // 发送事件通知 taskbar ContextMenu 已隐藏
    emit("contextmenu::item_changed", {
      itemId: null,
      displayName: null
    }).catch((e) => {
      console.error("[ContextMenu] Failed to emit contextmenu hide:", e);
    });

    // 更新后端状态
    invoke("contextmenu_set_state", {
      itemId: null,
      displayName: null
    }).catch((e) => {
      console.error("[ContextMenu] Failed to set contextmenu state:", e);
    });

    setVisible(false);
    try {
      await invoke(CONTEXTMENU_HIDE_CMD);
    } catch (e) {
      console.error("[ContextMenu] Failed to hide:", e);
    }
    // 隐藏后启动延迟销毁定时器
    startDestroyTimer();
  }, [data?.monitorId, startDestroyTimer]);

  // 点击菜单项
  const handleItemClick = useCallback(
    async (item: SerializedMenuItem) => {
      if (item.disabled) return;

      // 通知 taskbar 窗口执行回调
      emit(CONTEXTMENU_ITEM_CLICK_EVENT, {
        key: item.key,
        sourceItemId: data?.sourceItemId,
        menuType: data?.menuType,
      });

      // 隐藏菜单
      await hideMenu();
    },
    [data, hideMenu],
  );

  // 监听显示事件 + mount 时发送 ready 信号
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

    // 通知后端：前端已挂载就绪
    invoke(CONTEXTMENU_READY_CMD).catch((e) => {
      console.error("[ContextMenu] Failed to send ready signal:", e);
    });

    const unlistenShow = listen<ContextMenuShowPayload>(
      CONTEXTMENU_SHOW_EVENT,
      async (event) => {
        const payload = event.payload;
        logInfo(
          `[ContextMenu] Received show event: menuType=${payload.menuType}, items=${payload.items.length}`,
        );

        // 收到新的 show 事件，取消销毁定时器
        cancelDestroyTimer();

        // 更新数据
        const token = showTokenRef.current + 1;
        showTokenRef.current = token;
        latestPayloadRef.current = payload;
        pendingPositionRef.current = {
          position: payload.position,
          token,
        };
        setData(payload);
        schedulePositionAndShow();

        // 发送事件通知 taskbar 当前显示 ContextMenu 的图标 ID
        if (payload.sourceItemId) {
          emit("contextmenu::item_changed", {
            itemId: payload.sourceItemId,
            displayName: payload.sourceDisplayName || null
          }).catch((e) => {
            console.error("[ContextMenu] Failed to emit contextmenu item changed:", e);
          });

          // 更新后端状态
          invoke("contextmenu_set_state", {
            itemId: payload.sourceItemId,
            displayName: payload.sourceDisplayName || null
          }).catch((e) => {
            console.error("[ContextMenu] Failed to set contextmenu state:", e);
          });
        }

        // 保存位置信息，等待 ResizeObserver 触发后再设置
      },
    );

    // 初始隐藏窗口
    webview.hide();

    return () => {
      unlistenShow.then((fn) => fn());
      cancelDestroyTimer();
      if (positionFrameRef.current !== null) {
        cancelAnimationFrame(positionFrameRef.current);
        positionFrameRef.current = null;
      }
      unlistenTheme.then((fn) => fn());
    };
  }, [cancelDestroyTimer, schedulePositionAndShow]);

  // 监听窗口失焦 -> 隐藏菜单
  useEffect(() => {
    const webview = getCurrentWebviewWindow();
    const unlisten = webview.onFocusChanged(async ({ payload: focused }) => {
      if (!focused && visible) {
        // 检查焦点窗口
        let shouldHide = true;
        try {
          const foregroundInfo: [string, string] | null = await invoke("get_foreground_window_info");
          if (foregroundInfo) {
            // 如果窗口是 MagicPreview
            if (foregroundInfo[1] === "MagicPreview") {
              const sourceId = data?.sourceItemId;
              // 如果是同一个图标，不隐藏 ContextMenu，将 ContextMenu 置顶
              if (sourceId && previewItemId === sourceId) {
                shouldHide = false;
                // 隐藏 Preview 窗口
                invoke("preview_hide").catch((e) => {
                  console.error("[ContextMenu] Failed to hide preview:", e);
                });
              }
            }
          } else {
            logInfo(`[ContextMenu] Lost focus, no foreground window`);
          }
        } catch (e) {
          logInfo("[ContextMenu] Failed to get foreground window info:");
        }
        if (shouldHide) {
          hideMenu();
        } else {
          // 将 ContextMenu 窗口置顶并获得焦点
          webview.setFocus().catch((e) => {
            console.error("[ContextMenu] Failed to set focus:", e);
          });
        }
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [visible, hideMenu, data, previewItemId]);

  // ResizeObserver: 自动根据内容大小调整窗口
  useEffect(() => {
    if (!containerRef.current) return;

    const handleResize = async (entries: ResizeObserverEntry[]) => {
      const entry = entries[0];
      if (!entry || !pendingPositionRef.current) return;

      const pending = pendingPositionRef.current;
      const activeData = latestPayloadRef.current;
      if (!activeData || pending.token !== showTokenRef.current) {
        return;
      }

      const position = pending.position;
      const dpiScale = activeData.monitorDpi || globalThis.devicePixelRatio || 1;

      // 获取内容实际大小
      const width = Math.ceil(entry.contentRect.width * dpiScale);
      const height = Math.ceil(entry.contentRect.height * dpiScale);

      // 计算窗口位置，x 是图标左边缘
      const monitorRect = activeData.monitorRect;
      const screenLeft = monitorRect?.left ?? 0;
      const screenTop = monitorRect?.top ?? 0;
      const screenRight = monitorRect?.right ?? Math.round(globalThis.screen.width * dpiScale);
      const screenBottom = monitorRect?.bottom ?? Math.round(globalThis.screen.height * dpiScale);

      let windowX = Math.round(position.x);

      // 窗口底部距离屏幕底部固定 108px（逻辑像素，需要乘以 DPI 缩放）
      let windowY = Math.round(screenBottom - height - Math.round(90 * dpiScale));

      // 确保不超出屏幕边界
      windowX = Math.max(screenLeft, Math.min(windowX, screenRight - width));
      windowY = Math.max(screenTop, windowY);

      void logInfo(
        `[ContextMenu] set_position monitorId=${activeData.monitorId ?? "null"} monitorDpi=${activeData.monitorDpi ?? "null"} webviewDpr=${globalThis.devicePixelRatio || 1} css=${entry.contentRect.width}x${entry.contentRect.height} physical=${width}x${height} target=${windowX},${windowY}`,
      );

      try {
        // 设置窗口位置和大小
        await invoke(CONTEXTMENU_SET_POSITION_CMD, {
          x: windowX,
          y: windowY,
          width,
          height,
        });

        // 通知 taskbar ContextMenu 窗口已打开
        if (pending.token !== showTokenRef.current || latestPayloadRef.current !== activeData) {
          return;
        }

        emit(CONTEXTMENU_WINDOW_OPEN_EVENT, {
          open: true,
          monitorId: activeData.monitorId ?? null,
        }).catch((e) => {
          console.error("[ContextMenu] Failed to emit window_open true:", e);
        });

        // 显示窗口
        setVisible(true);
        await invoke(CONTEXTMENU_SHOW_CMD);

        // 清除 pending 位置
        if (pending.token === showTokenRef.current) {
          pendingPositionRef.current = null;
        }
      } catch (e) {
        console.error("[ContextMenu] Failed to position/show window:", e);
      }
    };

    resizeObserverRef.current = new ResizeObserver(handleResize);
    resizeObserverRef.current.observe(containerRef.current, {
      box: "border-box",
    });

    return () => {
      if (resizeObserverRef.current) {
        resizeObserverRef.current.disconnect();
      }
    };
  }, []);

  // 渲染菜单项图标
  const renderIcon = (item: SerializedMenuItem) => {
    if (item.iconImgSrc) {
      return (
        <div className="contextmenu-item-icon">
          <img src={item.iconImgSrc} alt="" />
        </div>
      );
    }
    if (item.iconSvgUrl) {
      return (
        <div className="contextmenu-item-icon">
          <img src={item.iconSvgUrl} alt="" />
        </div>
      );
    }
    return null;
  };

  return (
    <div
      ref={containerRef}
      className="contextmenu-container"
    >
      {data && data.items.length > 0 ? (
        <div className="contextmenu-list">
          {data.items.map((item, index) => {
            if (item.divider) {
              return <div key={`divider-${index}`} className="contextmenu-divider" />;
            }
            return (
              <div
                key={item.key}
                className={`contextmenu-item${item.danger ? " danger" : ""}${item.disabled ? " disabled" : ""}`}
                onClick={() => handleItemClick(item)}
              >
                {renderIcon(item)}
                <span className="contextmenu-item-label">{item.label}</span>
              </div>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
