import { BackgroundByLayersV2 } from "@shared/components/BackgroundByLayers/infra";
import { FuncCommand, invoke, TaskbarSide } from "@magic-ui/lib";
import { memo, useCallback, useEffect, useRef, useState } from "react";
import { useSelector } from "react-redux";
import { useTranslation } from "react-i18next";
import { cx } from "@shared/styles";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useWindowFocusChange } from "@shared/hooks";
import { $settings } from "../../shared/state/mod";
import { $current_monitor } from "../../shared/state/system";
import { RecycleBinTaskbarItem } from "../../shared/store/domain";
import { Selectors } from "../../shared/store/app";
import { dockCoordinatesTracker, saveCoordinatesImmediately, saveSimpleWindowCoordinatesToDisk } from "../../shared/utils/taskbar_save_window_coordinates";

// Preview 窗口事件常量
const PREVIEW_SHOW_EVENT = "preview::show";
const PREVIEW_HIDE_CMD = "preview_hide";
const PREVIEW_MOUSE_ENTER_EVENT = "preview::mouse_enter";
const PREVIEW_MOUSE_LEAVE_EVENT = "preview::mouse_leave";

// ContextMenu 窗口命令常量
const CONTEXTMENU_TRIGGER_CMD = "contextmenu_trigger";
const CONTEXTMENU_ITEM_CLICK_EVENT = "contextmenu::item_click";

// 模块级变量：追踪当前活跃的预览窗口属于哪个图标
let activePreviewItemId: string | null = null;

interface Props {
  item: RecycleBinTaskbarItem;
}

export const RecycleBin = memo(({ item }: Props) => {
  const { id } = item;
  const [isOpen, setIsOpen] = useState(false);
  const [isEmpty, setIsEmpty] = useState((item as any).is_empty ?? true);
  const [iconSrc, setIconIconSrc] = useState<string>("");
  const [isHovering, setIsHovering] = useState(false);
  const [previewHovering, setPreviewHovering] = useState(false);
  const { t } = useTranslation();
  const focusedApp = useSelector(Selectors.focusedApp) as any;
  const itemRef = useRef<HTMLDivElement>(null);
  const hoverTimerRef = useRef<number | null>(null);
  const hideTimerRef = useRef<number | null>(null);
  const isHoveringRef = useRef(false);
  const previewHoveringRef = useRef(false);
  const menuCallbacksRef = useRef<Record<string, () => void>>({});
  const clickDebounceRef = useRef<{
    timeoutId: number | null;
    isClickable: boolean;
  }>({ timeoutId: null, isClickable: true });

  const isFocused = (() => {
    const title = (focusedApp?.title || "") as string;
    return (
      title.startsWith("回收站") ||
      title.startsWith("Recycle Bin") ||
      title.startsWith("資源回收筒")
    );
  })();

  const calculatePlacement = (position: any) => {
    switch (position) {
      case TaskbarSide.Bottom:
        return "top" as const;
      case TaskbarSide.Top:
        return "bottom" as const;
      case TaskbarSide.Left:
        return "right" as const;
      case TaskbarSide.Right:
        return "left" as const;
      default:
        return "top" as const;
    }
  };

  // 发送预览显示事件
  const showPreview = useCallback(async () => {
    if (!itemRef.current) return;
    
    const rect = itemRef.current.getBoundingClientRect();
    const placement = calculatePlacement($settings.value.position);
    const currentMonitor = $current_monitor.value;
    const monitorId = currentMonitor?.id ?? null;
    if (!monitorId) {
      console.warn("[RecycleBin] Preview show skipped: missing monitorId");
      return;
    }

    // 计算屏幕坐标
    const dpiScale = currentMonitor.dpi || globalThis.window.devicePixelRatio || 1;
    const webviewPos = await getCurrentWebviewWindow().outerPosition();
    
    const x = webviewPos.x + Math.round((rect.left + rect.width / 2) * dpiScale);
    const y = placement === "top" 
      ? webviewPos.y + Math.round(rect.top * dpiScale)
      : webviewPos.y + Math.round(rect.bottom * dpiScale);
    
    const payload = {
      itemId: item.id,
      displayName: t("taskbar.recycle_bin", { defaultValue: "回收站" }),
      windows: [],
      position: { x, y, placement },
      appIconBase64: iconSrc ? iconSrc.replace('data:image/png;base64,', '') : null,
      appIconSrc: null,
      monitorId,
      monitorRect: currentMonitor.rect,
      monitorDpi: currentMonitor.dpi,
    };

    // 发送事件通知其他 webview（ContextMenu）当前预览的图标 ID
    emit("preview::item_changed", { itemId: item.id }).catch((e) => {
      console.error("[RecycleBin] Failed to emit preview item changed:", e);
    });

    invoke(FuncCommand.PreviewTriggerShow, { payload, monitorId }).catch((e) => {
      console.error("[RecycleBin] Failed to trigger preview show:", e);
    });
  }, [item.id, t, iconSrc]);

  // 隐藏预览窗口
  const hidePreview = useCallback(async () => {
    if (activePreviewItemId !== item.id) {
      return;
    }
    if (!previewHoveringRef.current && !isHoveringRef.current) {
      activePreviewItemId = null;
      await invoke(PREVIEW_HIDE_CMD as any, { monitorId: $current_monitor.value?.id ?? null });
    }
  }, [item.id]);

  // 鼠标进入处理
  const handleMouseEnter = useCallback(() => {
    if (hoverTimerRef.current) {
      clearTimeout(hoverTimerRef.current);
    }
    if (hideTimerRef.current) {
      clearTimeout(hideTimerRef.current);
      hideTimerRef.current = null;
    }
    
    hoverTimerRef.current = window.setTimeout(() => {
      if (hideTimerRef.current) {
        clearTimeout(hideTimerRef.current);
        hideTimerRef.current = null;
      }
      
      activePreviewItemId = item.id;
      isHoveringRef.current = true;
      setIsHovering(true);
      showPreview();
    }, 250);
  }, [showPreview, item.id]);

  // 鼠标离开处理
  const handleMouseLeave = useCallback(() => {
    if (hoverTimerRef.current) {
      clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
    
    isHoveringRef.current = false;
    setIsHovering(false);
    hideTimerRef.current = window.setTimeout(() => {
      if (!previewHoveringRef.current) {
        hidePreview();
      }
    }, 300);
  }, [hidePreview]);

  // 窗口焦点变化处理
  useWindowFocusChange((focused) => {
    if (!focused) {
      if (isHovering) {
        isHoveringRef.current = false;
        setIsHovering(false);
        hidePreview();
      }
    }
  });

  // 组件卸载时清理
  useEffect(() => {
    return () => {
      if (hoverTimerRef.current) {
        clearTimeout(hoverTimerRef.current);
      }
      if (hideTimerRef.current) {
        clearTimeout(hideTimerRef.current);
      }
      if (clickDebounceRef.current.timeoutId) {
        clearTimeout(clickDebounceRef.current.timeoutId);
      }
    };
  }, []);

  // 监听预览窗口鼠标事件
  useEffect(() => {
    const unlistenEnter = listen(PREVIEW_MOUSE_ENTER_EVENT, (event: any) => {
      if (event.payload.itemId === item.id) {
        previewHoveringRef.current = true;
        setPreviewHovering(true);
        if (hideTimerRef.current) {
          clearTimeout(hideTimerRef.current);
          hideTimerRef.current = null;
        }
      }
    });

    const unlistenLeave = listen(PREVIEW_MOUSE_LEAVE_EVENT, (event: any) => {
      if (event.payload.itemId === item.id) {
        previewHoveringRef.current = false;
        setPreviewHovering(false);
        hidePreview();
      }
    });

    return () => {
      unlistenEnter.then(fn => fn());
      unlistenLeave.then(fn => fn());
    };
  }, [item.id, hidePreview]);

  // 监听 ContextMenu 菜单项点击事件
  useEffect(() => {
    const unlisten = listen(CONTEXTMENU_ITEM_CLICK_EVENT, (event: any) => {
      const { key, sourceItemId } = event.payload;
      if (sourceItemId === item.id) {
        const callback = menuCallbacksRef.current[key];
        if (callback) {
          callback();
        }
      }
    });
    return () => {
      unlisten.then(fn => fn());
    };
  }, [item.id]);

  useEffect(() => {
    const loadIcon = async () => {
      try {
        const processName = isEmpty ? "RecycleBin_ Empty" : "RecycleBin_NotEmpty";
        const base64 = await invoke('get_local_icon' as any, { processName } as any);
        if (base64) {
          setIconIconSrc(base64);
        }
      } catch (e) {
        console.error("Failed to load recycle bin icon:", e);
      }
    };
    loadIcon();
  }, [isEmpty]);

  // 监听回收站窗口状态变化（事件驱动）
  useEffect(() => {
    // 初始检查状态
    const checkStatus = async () => {
      try {
        const [open, empty] = await Promise.all([
          invoke('system_is_recycle_bin_open' as any),
          invoke('system_is_recycle_bin_empty' as any),
        ]);
        setIsOpen(!!open);
        setIsEmpty(!!empty);
      } catch (e) {
        console.error("Failed to check recycle bin open/empty status:", e);
      }
    };
    checkStatus();

    // 监听回收站窗口状态变化事件
    const unlisten = listen<boolean>('recycle-bin-state-changed', async (event) => {
      setIsOpen(event.payload);
      // 状态变化时同时更新 is_empty 状态
      if (event.payload) {
        // 打开时，重新获取 empty 状态
        try {
          const empty = await invoke('system_is_recycle_bin_empty' as any);
          setIsEmpty(!!empty);
        } catch (e) {
          console.error("Failed to check recycle bin empty status:", e);
        }
      }
    });

    // 监听回收站内容变化事件（文件删除、清空等操作）
    const unlistenContent = listen<boolean>('recycle-bin-content-changed', async (event) => {
      setIsEmpty(event.payload);

      // 同时更新 item 的 is_empty 状态
      await emit('taskbar-item-state-changed', {
        itemId: item.id,
        state: { is_empty: event.payload }
      }).catch(() => {}); // 忽略错误，静默失败
    });

    return () => {
      unlisten.then(fn => fn());
      unlistenContent.then(fn => fn());
    };
  }, []);

  // 当外部 item 状态变化时同步内部状态
  useEffect(() => {
    if ((item as any).is_empty !== undefined) {
      setIsEmpty((item as any).is_empty);
    }
  }, [(item as any).is_empty]);

  // 保存回收站图标坐标（参考普通应用图标，增加 DOM 监听）
  useEffect(() => {
    // 防抖 timer，避免短时间内多次触发产生并发 invoke
    let debounceTimer: number | null = null;

    const updateCoordinates = async () => {
      if (!itemRef.current) {
        return;
      }

      // 先 await 获取 hwnd，挂起期间 #root.style.left 有充足时间完成更新
      let recycleBinHwnd = -1;
      try {
        recycleBinHwnd = await invoke('system_get_recycle_bin_hwnd' as any) as number;
      } catch (e) {
        console.error('[RecycleBin] 获取回收站窗口句柄失败:', e);
      }

      // await 完成后重新检查组件是否仍挂载
      if (!itemRef.current) {
        return;
      }

      // await 完成后再读 rect，此时布局（#root.style.left）已是最新值
      const rect = itemRef.current.getBoundingClientRect();
      const currentMonitor = $current_monitor.value;

      if (!currentMonitor || rect.width === 0 || rect.height === 0) return;

      const dpiScale = globalThis.window.devicePixelRatio || 1;
      // 获取 WebView 窗口在显示器上的偏移 (相对于显示器左上角)
      const webviewScreenY = globalThis.window.screenY || globalThis.window.screenTop || 0;

      // 获取图标的实际尺寸（排除 padding）
      const computedStyle = window.getComputedStyle(itemRef.current);
      const paddingLeft = parseFloat(computedStyle.paddingLeft) || 0;
      const paddingTop = parseFloat(computedStyle.paddingTop) || 0;
      const paddingRight = parseFloat(computedStyle.paddingRight) || 0;
      const paddingBottom = parseFloat(computedStyle.paddingBottom) || 0;

      // 计算内容区域（图标实际位置）
      const contentWidth = rect.width - paddingLeft - paddingRight;
      const contentHeight = rect.height - paddingTop - paddingBottom;

      // rect.left/top 是相对于 WebView 窗口的坐标
      // 需要加上 WebView 在显示器上的偏移得到相对于显示器的坐标
      // 加上 paddingLeft/Top 得到内容区域的起点，再加上内容区域的一半得到中心
      const relativeCenterX = rect.left + paddingLeft + contentWidth / 2;
      const relativeCenterY = webviewScreenY + rect.top + paddingTop + contentHeight / 2;

      // 转换为物理像素
      const physicalCenterX = Math.round(relativeCenterX * dpiScale);
      let physicalCenterY = Math.round(relativeCenterY * dpiScale);
      const physicalWidth = Math.round(rect.width * dpiScale);

      // 限制 Y 坐标不超过屏幕高度
      const maxY = currentMonitor.rect.bottom;
      const configPadding = parseInt(getComputedStyle(itemRef.current).getPropertyValue('--config-padding')) || 0;
      const paddingInPixels = Math.round(configPadding * dpiScale);

      if (physicalCenterY > maxY) {
        physicalCenterY = Math.round(maxY - physicalWidth / 2 - paddingInPixels);
      }

      const relativeX = physicalCenterX;
      const relativeY = physicalCenterY;

      // 计算相对百分比
      const monitorWidth = currentMonitor.rect.right - currentMonitor.rect.left;
      const monitorHeight = currentMonitor.rect.bottom - currentMonitor.rect.top;
      const xRelative = relativeX / monitorWidth;
      const yRelative = relativeY / monitorHeight;

      // 保存回收站图标位置到窗口句柄（无论是否获取都保存，使用获取到的值）
      dockCoordinatesTracker.addOrUpdateCoordinate(
        recycleBinHwnd,
        t("taskbar.recycle_bin", { defaultValue: "回收站" }),
        currentMonitor.name,
        relativeX,
        relativeY,
        physicalWidth,
        xRelative,
        yRelative
      );
      // 立即刷新待处理队列（不等待 200ms 延迟）
      dockCoordinatesTracker.flushPendingUpdatesNow();
      // 立即保存到 JSON 文件（不经过防抖延迟）
      dockCoordinatesTracker.setShouldSaveCoordinates(true);
      await saveSimpleWindowCoordinatesToDisk();
    };

    // 防抖触发：100ms 内多次触发只执行最后一次，但最多等待 500ms（maxWait）
    // 避免容器持续变化时防抖被无限重置导致坐标长时间不更新
    let maxWaitTimer: number | null = null;
    const scheduleUpdate = () => {
      if (debounceTimer !== null) {
        clearTimeout(debounceTimer);
      }
      debounceTimer = window.setTimeout(() => {
        debounceTimer = null;
        if (maxWaitTimer !== null) {
          clearTimeout(maxWaitTimer);
          maxWaitTimer = null;
        }
        updateCoordinates();
      }, 100);
      // 如果还没有 maxWait 定时器，启动一个 500ms 的兜底执行
      if (maxWaitTimer === null) {
        maxWaitTimer = window.setTimeout(() => {
          maxWaitTimer = null;
          if (debounceTimer !== null) {
            clearTimeout(debounceTimer);
            debounceTimer = null;
          }
          updateCoordinates();
        }, 500);
      }
    };

    // 初始化时等待 DOM 完全渲染后再计算坐标
    // 使用 requestAnimationFrame 确保在浏览器重绘之后执行
    let rafId: number;
    const initCoordinates = () => {
      rafId = requestAnimationFrame(() => {
        requestAnimationFrame(updateCoordinates);
      });
    };
    initCoordinates();

    // 监听整个任务栏的变化（不仅仅是父容器，还包括兄弟图标节点的变化）
    const observer = new MutationObserver(() => {
      scheduleUpdate();
    });

    // 监听多个层级：父容器 + 整个任务栏容器
    const parentElement = itemRef.current?.parentElement;
    if (parentElement) {
      observer.observe(parentElement, { childList: true, subtree: true, attributes: true, characterData: true });
    }

    // 同时监听整个 dock 容器（捕获所有图标的变化）
    const dockContainer = document.querySelector('.taskbar-items-container');
    if (dockContainer && dockContainer !== parentElement) {
      observer.observe(dockContainer, { childList: true, subtree: true, attributes: true, characterData: true });
    }

    // 使用 ResizeObserver 监听任务栏尺寸变化（中间区域图标变化会导致回收站位置偏移）
    const resizeObserver = new ResizeObserver(() => {
      scheduleUpdate();
    });
    if (dockContainer) {
      resizeObserver.observe(dockContainer);
    }

    // 监听后端推送的容器刷新事件（DPI/显示器变化等导致 containerLeft 改变）
    // 此事件不经过 MutationObserver/ResizeObserver，需单独监听
    const unlistenContainerRefresh = listen('taskbar::container-refresh', () => {
      scheduleUpdate();
    });

    // 组件卸载时清理
    return () => {
      observer.disconnect();
      resizeObserver.disconnect();
      unlistenContainerRefresh.then(fn => fn());
      if (rafId) {
        cancelAnimationFrame(rafId);
      }
      if (debounceTimer !== null) {
        clearTimeout(debounceTimer);
      }
      if (maxWaitTimer !== null) {
        clearTimeout(maxWaitTimer);
      }
    };
  }, [t, $current_monitor.value]);  // 移除不可靠的 $dock_state.value.items.length，完全依赖 MutationObserver

  const handleClick = () => {
    // 防重复点击处理，点击后300ms内不响应
    if (!clickDebounceRef.current.isClickable) {
      return;
    }

    // 立即设置为不可点击
    clickDebounceRef.current.isClickable = false;

    // 300ms后恢复可点击
    if (clickDebounceRef.current.timeoutId) {
      clearTimeout(clickDebounceRef.current.timeoutId);
    }
    clickDebounceRef.current.timeoutId = window.setTimeout(() => {
      clickDebounceRef.current.isClickable = true;
    }, 300);

    // 立即执行操作
    (async () => {
      try {
        const hwndResult = await invoke('system_get_recycle_bin_hwnd' as any);
        const hwnd = Number(hwndResult);
        if (hwnd === -1) {
          // 窗口未打开，执行打开
          await invoke('system_open_recycle_bin' as any);
        } else {
          // 窗口已打开，与普通图标行为一致：切换最小化/恢复/置前
          const wasFocused = isFocused;
          await (invoke as any)('taskbar_toggle_window_state', { hwnd, wasFocused });
          if (wasFocused) {
            emit("hidden::remove-focused-color");
          }
        }
      } catch (e) {
        console.error('[RecycleBin] 操作失败:', e);
      }
    })();
  };

  const handleClear = async () => {
    try {
      await invoke('system_empty_recycle_bin' as any);
    } catch (error) {
      console.error('Failed to empty recycle bin:', error);
    }
  };

  const handleClose = () => {
    // 立即更新前端状态
    setIsOpen(false);

    // 调用后端关闭
    invoke('system_close_recycle_bin' as any).catch((e: any) => {
      console.error('[RecycleBin] 关闭RecycleBin窗口失败:', e);
    });
  };

  // 右键菜单处理
  const handleContextMenu = useCallback(async (e: any) => {
    e.preventDefault();
    e.stopPropagation();
    const clientX = e.clientX;
    const clientY = e.clientY;

    // 立即隐藏预览窗口
    if (isHovering) {
      isHoveringRef.current = false;
      setIsHovering(false);
    }
    if (hoverTimerRef.current) {
      clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
    if (hideTimerRef.current) {
      clearTimeout(hideTimerRef.current);
      hideTimerRef.current = null;
    }
    activePreviewItemId = null;
    invoke(PREVIEW_HIDE_CMD as any, { monitorId: $current_monitor.value?.id ?? null }).catch(() => {});

    // 构建序列化菜单项和回调映射
    const callbacks: Record<string, () => void> = {};
    const serializedItems: any[] = [];

    // 打开/关闭回收站
    serializedItems.push({
      key: "open",
      label: t("taskbar.recycle_bin", { defaultValue: "回收站" }),
    });
    callbacks["open"] = handleClick;

    // 如果回收站窗口已打开，添加关闭选项
    if (isOpen) {
      serializedItems.push({ key: "divider_1", label: "", divider: true });
      serializedItems.push({
        key: "close",
        label: t("recycle_bin.close"),
      });
      callbacks["close"] = handleClose;
      serializedItems.push({ key: "divider_2", label: "", divider: true });
    }

    // 清空回收站（仅在回收站非空时显示）
    if (!isEmpty) {
      serializedItems.push({
        key: "clear",
        label: t("recycle_bin.empty"),
      });
      callbacks["clear"] = handleClear;
    }

    // 保存回调映射
    menuCallbacksRef.current = callbacks;

    // 计算屏幕坐标
    const currentMonitor = $current_monitor.value;
    const dpiScale = currentMonitor?.dpi || globalThis.window.devicePixelRatio || 1;
    const webviewPos = await getCurrentWebviewWindow().outerPosition();
    const placement = calculatePlacement($settings.value.position);

    const rect = itemRef.current?.getBoundingClientRect();
    const x = rect
      ? webviewPos.x + Math.round(rect.left * dpiScale)
      : webviewPos.x + Math.round(clientX * dpiScale);
    const y = rect
      ? (placement === "top"
        ? webviewPos.y + Math.round(rect.top * dpiScale)
        : webviewPos.y + Math.round(rect.bottom * dpiScale))
      : webviewPos.y + Math.round(clientY * dpiScale);

    // 通过后端命令触发 contextmenu
    (invoke as any)(CONTEXTMENU_TRIGGER_CMD, {
      payload: {
        menuType: "app",
        items: serializedItems,
        position: { x, y, placement },
        sourceItemId: item.id,
        monitorId: currentMonitor?.id ?? null,
        monitorDpi: currentMonitor?.dpi ?? null,
        monitorRect: currentMonitor?.rect ?? null,
      },
    });
  }, [item.id, isHovering, isOpen, t, handleClick, handleClose, handleClear]);

  return (
    <div
      ref={itemRef}
      className="taskbar-item taskbar-item-recycle-bin"
      onClick={handleClick}
      onMouseEnter={handleMouseEnter}
      onMouseLeave={handleMouseLeave}
      onContextMenu={handleContextMenu}
      data-id={id}
    >
      <BackgroundByLayersV2 prefix="item" />
      {iconSrc && (
        <img
          className="taskbar-item-icon"
          src={iconSrc}
          style={{ width: '100%', height: '100%', objectFit: 'contain' }}
        />
      )}
      <div
        className={cx("taskbar-item-open-sign", {
          "taskbar-item-open-sign-active": isOpen,
        })}
      />
    </div>
  );
});
