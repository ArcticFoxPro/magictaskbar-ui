import { FuncCommand, TaskbarItemType, TaskbarSide } from "@magic-ui/lib";
import { FileIcon, Icon } from "@shared/components/Icon";
import { useWindowFocusChange } from "@shared/hooks";
import { cx } from "@shared/styles";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { memo, useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSelector } from "react-redux";

import { BackgroundByLayersV2 } from "@shared/components/BackgroundByLayers/infra";

import { Selectors } from "../../shared/store/app";

import { PinnedTaskbarItem, TemporalTaskbarItem } from "../../shared/store/domain";

import { $dock_state, $dock_state_actions } from "../../shared/state/items";
import { $settings } from "../../shared/state/mod";
import { $current_monitor, $monitors } from "../../shared/state/system";
import { dockCoordinatesTracker } from "../../shared/utils/taskbar_save_window_coordinates";

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

// 模块级变量：存储需要隐藏圆点的进程名列表
let hiddenIndicatorProcesses: Set<string> = new Set();

// 模块级变量：事件监听器是否已注册
let prelaunchListenerRegistered = false;

// 模块级变量：存储需要强制更新的回调函数列表
const forceUpdateCallbacks: Set<() => void> = new Set();

// 模块级变量：记录上一次窗口的最小化状态，用于检测状态变化
let lastWindowIconicState: Map<string, boolean> = new Map();

// 注册预启动事件监听器（只注册一次）
function registerPrelaunchListener() {
  if (prelaunchListenerRegistered) {
    return;
  }
  prelaunchListenerRegistered = true;

  listen<string>("prelaunch::hide_indicator", (event) => {
    const processName = event.payload;
    hiddenIndicatorProcesses.add(processName.toLowerCase());

    // 通知所有注册的组件更新
    forceUpdateCallbacks.forEach(cb => cb());
  }).catch((e) => {
    prelaunchListenerRegistered = false;
  });
}

interface Props {
  item: PinnedTaskbarItem | TemporalTaskbarItem;
  isOverlay?: boolean;
}

export const UserApplication = memo(({ item, isOverlay }: Props) => {
  const [openContextMenu, setOpenContextMenu] = useState(false);
  const [isHovering, setIsHovering] = useState(false);
  const [previewHovering, setPreviewHovering] = useState(false); // 预览窗口是否悬停
  const [hasNotification, setHasNotification] = useState(false); // 是否有通知
  // 🔧 使用 useRef 存储 FileIcon 传递的形状信息，避免组件重渲染时状态丢失
  const fileIconShapeRef = useRef<{ isSquare: boolean; isFromLocal: boolean } | null>(null);
  const [, forceUpdate] = useState(0); // 用于强制更新组件
  const itemRef = useRef<HTMLDivElement>(null);
  const hoverTimerRef = useRef<number | null>(null);
  const hideTimerRef = useRef<number | null>(null); // 延迟隐藏定时器
  const clickDebounceRef = useRef<{
    timeoutId: number | null;
    isClickable: boolean;
  }>({ timeoutId: null, isClickable: true });
  const notificationTimerRef = useRef<number | null>(null); // 通知清除定时器
  const isHoveringRef = useRef(false); // 用于在闭包中读取最新值
  const previewHoveringRef = useRef(false);
  const windowsLengthRef = useRef(item.windows.length);
  // 保存菜单回调的 ref
  const menuCallbacksRef = useRef<Record<string, () => void>>({});

  const devTools = useSelector(Selectors.devTools) as boolean;
  const focusedApp = useSelector(Selectors.focusedApp) as any;

  const { t } = useTranslation();

  useEffect(() => {
    windowsLengthRef.current = item.windows.length;
  }, [item.windows.length]);

  const calculatePlacement = (position: any) => {
    switch (position) {
      case TaskbarSide.Bottom: {
        return "top" as const;
      }
      case TaskbarSide.Top: {
        return "bottom" as const;
      }
      case TaskbarSide.Left: {
        return "right" as const;
      }
      case TaskbarSide.Right: {
        return "left" as const;
      }
      default: {
        return "top" as const;
      }
    }
  };

  // 获取翻译后的显示名称
  const getDisplayName = useCallback(() => {
    if (item.displayName.startsWith('app_menu.')) {
      return t(item.displayName);
    }
    return item.displayName;
  }, [item.displayName, t]);

  // 发送预览显示事件
  const showPreview = useCallback(async () => {
    if (!itemRef.current) return;

    const rect = itemRef.current.getBoundingClientRect();
    const placement = calculatePlacement($settings.value.position);
    const currentMonitor = $current_monitor.value;
    const monitorId = currentMonitor?.id ?? null;
    if (!monitorId) {
      console.warn("[UserApplication] Preview show skipped: missing monitorId");
      return;
    }

    // 计算屏幕坐标
    const dpiScale = currentMonitor.dpi || globalThis.window.devicePixelRatio || 1;
    const webviewPos = await getCurrentWebviewWindow().outerPosition();

    const x = webviewPos.x + Math.round(rect.left * dpiScale);
    const y = placement === "top"
      ? webviewPos.y + Math.round(rect.top * dpiScale)
      : webviewPos.y + Math.round(rect.bottom * dpiScale);

    // 从已渲染的 taskbar 图标中获取图标数据
    let appIconBase64: string | null = (item.windows[0] as any)?.iconPngBase64 || null;
    let appIconSrc: string | null = null;
    if (!appIconBase64 && itemRef.current) {
      // FileIcon 渲染为 <figure class="taskbar-item-icon"><img .../></figure>
      // 普通图标渲染为 <img class="taskbar-item-icon" .../>
      const container = itemRef.current.querySelector('.taskbar-item-icon');
      const iconImg = (
        container?.querySelector('img') || // FileIcon: figure > img
        (container?.tagName === 'IMG' ? container : null) // 直接 img 元素
      ) as HTMLImageElement | null;
      if (iconImg?.src) {
        if (iconImg.src.startsWith('data:image/png;base64,')) {
          appIconBase64 = iconImg.src.replace('data:image/png;base64,', '');
        } else if (iconImg.src.startsWith('data:image')) {
          appIconBase64 = iconImg.src;
        } else if (iconImg.src) {
          // asset 协议或其他 URL，直接传给 preview 窗口使用
          appIconSrc = iconImg.src;
        }
      }
    }

    const payload = {
      itemId: item.id,
      displayName: getDisplayName(),
      windows: item.windows.map((w: any) => ({
        handle: w.handle,
        title: w.title,
        iconPngBase64: w.iconPngBase64 || null,
        isFocused: focusedApp?.hwnd === w.handle,
      })),
      position: { x, y, placement },
      path: item.path,
      umid: item.umid,
      appIconBase64,
      appIconSrc,
      monitorId,
      monitorRect: currentMonitor.rect,
      monitorDpi: currentMonitor.dpi,
    };
    // 发送事件通知其他 webview（ContextMenu）当前预览的图标 ID
    emit("preview::item_changed", { itemId: item.id }).catch((e) => {
      console.error("[UserApplication] Failed to emit preview item changed:", e);
    });
    // 改用 command 触发，通过后端 manager 确保事件不会丢失
    invoke(FuncCommand.PreviewTriggerShow, { payload, monitorId }).catch((e) => {
      console.error("[UserApplication] Failed to trigger preview show:", e);
    });
  }, [item, focusedApp, getDisplayName]);

  // 隐藏预览窗口
  const hidePreview = useCallback(async () => {
    // 只有当前活跃的预览是自己的才能隐藏，避免误杀其他图标的预览
    if (activePreviewItemId !== item.id) {
      return;
    }
    // 使用 ref 读取最新状态，只有当前没有悬停时才隐藏
    if (!previewHoveringRef.current && !isHoveringRef.current) {
      activePreviewItemId = null;
      await invoke(PREVIEW_HIDE_CMD, { monitorId: $current_monitor.value?.id ?? null });
    }
  }, [item.id]);

  // 鼠标进入处理
  const handleMouseEnter = useCallback(() => {
    // 清除自己的所有定时器
    if (hoverTimerRef.current) {
      clearTimeout(hoverTimerRef.current);
    }
    if (hideTimerRef.current) {
      clearTimeout(hideTimerRef.current);
      hideTimerRef.current = null;
    }

    // 延迟 250ms 后显示预览
    hoverTimerRef.current = window.setTimeout(() => {
      // 再次清除隐藏定时器，确保不会在显示后立即隐藏
      if (hideTimerRef.current) {
        clearTimeout(hideTimerRef.current);
        hideTimerRef.current = null;
      }

      // 标记当前图标为活跃预览
      activePreviewItemId = item.id;
      isHoveringRef.current = true;
      setIsHovering(true);
      showPreview();
    }, 250);
  }, [showPreview, item.id]);

  // 鼠标离开处理
  const handleMouseLeave = useCallback(() => {
    // 清除显示定时器
    if (hoverTimerRef.current) {
      clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }

    // 无论 isHovering 状态，都延迟隐藏（给鼠标移动到预览窗口的时间）
    isHoveringRef.current = false;
    setIsHovering(false);
    hideTimerRef.current = window.setTimeout(() => {
      // 使用 ref 读取最新状态
      if (!previewHoveringRef.current) {
        hidePreview();
      }
    }, 300);
  }, [hidePreview]);

  useWindowFocusChange((focused) => {
    if (!focused) {
      setOpenContextMenu(false);
      if (isHovering) {
        isHoveringRef.current = false;
        setIsHovering(false);
        hidePreview();
      }
    }
  });

  // 监听预启动应用隐藏圆点事件（全局只注册一次）
  useEffect(() => {
    registerPrelaunchListener();
    // 注册当前组件的强制更新回调
    const forceUpdateCallback = () => forceUpdate(n => n + 1);
    forceUpdateCallbacks.add(forceUpdateCallback);
    return () => {
      forceUpdateCallbacks.delete(forceUpdateCallback);
    };
  }, []);

  // 监听应用焦点变化，当应用获得焦点时延迟清除红点
  useEffect(() => {
    // 检查当前应用是否有窗口获得焦点
    const hasFocusedWindow = item.windows.some((w: any) => w.handle === focusedApp?.hwnd);


    // 当应用获得焦点时，从预启动隐藏列表中移除（应用已被用户激活，应显示圆点）
    // 注意：必须确认焦点窗口未处于最小化状态，防止最小化时 focusedApp 尚未更新导致的竞态误判
    const focusedWindowNotMinimized = item.windows.some((w: any) => w.handle === focusedApp?.hwnd && !w.isIconic);
    if (focusedWindowNotMinimized) {
      const relaunchProgram = item.relaunchProgram?.toLowerCase() || "";
      const processName = relaunchProgram.split("\\").pop() || relaunchProgram;
      if (processName && hiddenIndicatorProcesses.has(processName)) {
        hiddenIndicatorProcesses.delete(processName);
        forceUpdateCallbacks.forEach(cb => cb());
      }
    }

    // 当应用获得焦点时，延迟 500ms 后清除红点，避免闪烁
    if (hasFocusedWindow && hasNotification) {
      // 清除之前的定时器
      if (notificationTimerRef.current) {
        clearTimeout(notificationTimerRef.current);
      }
      // 设置新的定时器，延迟 400ms 清除红点
      notificationTimerRef.current = window.setTimeout(() => {
        setHasNotification(false);
        console.log(`[UserApplication] Clearing notification badge for focused app: ${item.displayName}`);
      }, 400);
    }

    // 清理函数
    return () => {
      if (notificationTimerRef.current) {
        clearTimeout(notificationTimerRef.current);
        notificationTimerRef.current = null;
      }
    };
  }, [focusedApp, item.windows, hasNotification, item.displayName]);

  // 组件卸载时清理
  useEffect(() => {
    return () => {
      if (hoverTimerRef.current) {
        clearTimeout(hoverTimerRef.current);
      }
      if (hideTimerRef.current) {
        clearTimeout(hideTimerRef.current);
      }
      if (notificationTimerRef.current) {
        clearTimeout(notificationTimerRef.current);
      }
      if (clickDebounceRef.current.timeoutId) {
        clearTimeout(clickDebounceRef.current.timeoutId);
      }
    };
  }, []);

  // 监听背板模式变化，重置 fileIconShape 状态
  useEffect(() => {
    const handleBackplateChange = () => {
      fileIconShapeRef.current = null;
      forceUpdate(n => n + 1);
    };
    window.addEventListener('backplate-style-changed', handleBackplateChange);
    return () => {
      window.removeEventListener('backplate-style-changed', handleBackplateChange);
    };
  }, [item.displayName]);

  // 监听预览窗口鼠标事件
  useEffect(() => {
    const unlistenEnter = listen(PREVIEW_MOUSE_ENTER_EVENT, (event: any) => {
      if (event.payload.itemId === item.id) {
        previewHoveringRef.current = true;
        setPreviewHovering(true);
        // 清除隐藏定时器
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
        // 鼠标离开预览窗口，立即隐藏
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

  // 监听应用通知事件
  useEffect(() => {
    const unlisten = listen('app-notification', (event: any) => {
      const notification = event.payload;
      console.log(`[UserApplication] Received notification event: ${JSON.stringify(notification)}`);
      console.log(`[UserApplication] Item info - umid: ${item.umid}, displayName: ${item.displayName}, relaunchProgram: ${item.relaunchProgram}`);
      console.log(`[UserApplication] Matching - appUmid: ${notification.appUmid}, appName: ${notification.appName}`);

      // 检查是否匹配：通过 umid、displayName 或 relaunchProgram（进程名）
      const relaunchProgramLower = item.relaunchProgram?.toLowerCase() || '';
      const appUmidLower = notification.appUmid?.toLowerCase() || '';
      const appNameLower = notification.appName?.toLowerCase() || '';

      // 对于 UWP 应用（relaunchProgram 是 explorer.exe），需要额外检查 displayName 和 umid
      const isUwpApp = relaunchProgramLower.includes('explorer.exe');
      const displayNameLower = item.displayName?.toLowerCase() || '';
      const umidLower = item.umid?.toLowerCase() || '';
      // 提取基础名称（去掉 .exe 后缀）
      const appUmidBase = appUmidLower.replace('.exe', '');
      const appNameBase = appNameLower.replace('.exe', '');

      // 检查 relaunchProgram 中的进程名是否完全匹配
      const getProcessName = (path: string) => {
        // 使用正则表达式分割路径，处理不同的路径分隔符
        const parts = path.split(/[\\/]/);
        return parts[parts.length - 1] || '';
      };
      const relaunchProcessName = getProcessName(relaunchProgramLower);

      let isMatched = notification.appUmid === item.umid || 
                      notification.appName === item.displayName ||
                      relaunchProcessName === appUmidLower ||
                      relaunchProcessName === appNameLower;

      // UWP 应用需要额外匹配 displayName 和 umid
      if (isUwpApp && !isMatched) {
        isMatched = displayNameLower === appUmidBase ||
                    displayNameLower === appNameBase ||
                    umidLower === appUmidBase ||
                    umidLower === appNameBase;
      }

      if (isMatched) {
        // 只有应用有打开的窗口时才显示红点（退出到托盘时不显示）
        const currentWindowsLength = windowsLengthRef.current;
        if (currentWindowsLength > 0) {
          setHasNotification(true);
        }
      }
    });
    return () => {
      unlisten.then(fn => fn());
    };
  }, [item.umid, item.displayName, item.relaunchProgram]);

  // 点击图标时清除通知状态
  const handleItemClick = useCallback(() => {
    setHasNotification(false);
  }, []);

  // 记录图标坐标信息
  useEffect(() => {
    const updateCoordinates = () => {
      if (itemRef.current && !isOverlay && item.windows.length > 0) {
        const rect = itemRef.current.getBoundingClientRect();
        const currentMonitor = $current_monitor.value;

        // 如果没有显示器信息,不计算坐标
        if (!currentMonitor) {
          return;
        }

        // 校验rect有效性，避免DOM未完全渲染时计算
        if (rect.width === 0 || rect.height === 0) {
          return;
        }

        // 获取DPI缩放系数
        const dpiScale = globalThis.window.devicePixelRatio || 1;
        // 获取WebView窗口在显示器上的偏移(相对于显示器左上角)
        const webviewScreenY = globalThis.window.screenY || globalThis.window.screenTop || 0;

        // rect.left/top 是相对于WebView窗口的坐标
        // 需要加上WebView在显示器上的偏移得到相对于显示器的坐标
        const relativeCenterX = rect.left + rect.width / 2;
        const relativeCenterY = webviewScreenY + rect.top + rect.height / 2;

        const physicalCenterX = Math.round(relativeCenterX * dpiScale);
        let physicalCenterY = Math.round(relativeCenterY * dpiScale);
        const physicalWidth = Math.round(rect.width * dpiScale);

        // 根据显示器实际分辨率限制Y坐标
        // 当Y超过屏幕高度时，设置为：屏幕最大高度 - width/2 - padding
        const maxY = currentMonitor.rect.bottom;
        const configPadding = parseInt(getComputedStyle(itemRef.current).getPropertyValue('--config-padding')) || 0;
        const paddingInPixels = Math.round(configPadding * dpiScale);

        if (physicalCenterY > maxY) {
          physicalCenterY = Math.round(maxY - physicalWidth / 2 - paddingInPixels);
        }

        const relativeX = physicalCenterX;
        const relativeY = physicalCenterY;

        // 计算相对百分比(相对于显示器分辨率)
        const monitorWidth = currentMonitor.rect.right - currentMonitor.rect.left;
        const monitorHeight = currentMonitor.rect.bottom - currentMonitor.rect.top;
        const xRelative = relativeX / monitorWidth;
        const yRelative = relativeY / monitorHeight;

        // 为所有窗口保存坐标信息（修复多窗口坐标保存问题）
        item.windows.forEach((windowItem: any) => {
          if (windowItem && currentMonitor) {
            dockCoordinatesTracker.addOrUpdateCoordinate(
              windowItem.handle,
              (windowItem as any).title || item.displayName,
              currentMonitor.name,
              relativeX,
              relativeY,
              physicalWidth,
              xRelative,
              yRelative
            );
          }
        });
      }
    };

    // 初始化时等待DOM完全渲染后再计算坐标
    // 使用requestAnimationFrame确保在浏览器重绘之后执行
    let rafId: number;
    const initCoordinates = () => {
      rafId = requestAnimationFrame(() => {
        requestAnimationFrame(updateCoordinates);
      });
    };
    initCoordinates();

    // 监听父容器的变化（新窗口添加或移除时）
    const observer = new MutationObserver(() => {
      setTimeout(updateCoordinates, 50);
    });

    // 监听父容器的变化
    const parentElement = itemRef.current?.parentElement;
    if (parentElement) {
      observer.observe(parentElement, { childList: true });
    }

    // 监听拖拽排序完成事件，重新计算坐标
    // 拖拽使用 CSS transform 移动元素，DOM childList 不变，MutationObserver 不会触发
    const handleReorderDone = () => {
      requestAnimationFrame(updateCoordinates);
    };
    window.addEventListener('taskbar-reorder-done', handleReorderDone);

    // 监听波浪放大动画动画每帧DOM更新事件，重新计算坐标
    const handleMagnificationSettled = () => {
      updateCoordinates();
    };
    window.addEventListener('taskbar-magnification-settled', handleMagnificationSettled);

    // 组件卸载时清理
    return () => {
      observer.disconnect();
      window.removeEventListener('taskbar-reorder-done', handleReorderDone);
      window.removeEventListener('taskbar-magnification-settled', handleMagnificationSettled);
      if (rafId) {
        cancelAnimationFrame(rafId);
      }
    };
  }, [item, isOverlay, item.windows.length]);

  const itemLabel = $settings.value.showWindowTitle && item.windows.length ? item.windows[0]!.title : null;

  // 右键菜单处理
  const handleContextMenu = useCallback(async (e: any) => {
    e.preventDefault();
    e.stopPropagation();
    const clientX = e.clientX;
    const clientY = e.clientY;

    // 立即隐藏预览窗口（contextmenu 显示时 preview 必须隐藏）
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
    // 强制隐藏 preview
    activePreviewItemId = null;
    invoke(PREVIEW_HIDE_CMD, { monitorId: $current_monitor.value?.id ?? null }).catch(() => {});

    // 构建序列化菜单项和回调映射
    const callbacks: Record<string, () => void> = {};
    const serializedItems: any[] = [];

    const isActuallyPinned = $dock_state.value.items.some(
      (taskbarItem) => taskbarItem.id === item.id && taskbarItem.type === TaskbarItemType.Pinned
    );

    // 打开（启动新实例）
    // 不支持固定的 item（pinDisabled=true，如 WPS 子应用 / 文档窗口 / 联名产品等）同样不支持“打开”，
    // 避免启动 MSIX 包级共享入口或其他非预期的窗口。
    if (!item.pinDisabled) {
      serializedItems.push({
        key: "taskbar_run_new",
        label: t("app_menu.open", { defaultValue: "打开" }),
      });
      callbacks["taskbar_run_new"] = () => {
        invoke(FuncCommand.Run, {
          program: item.relaunchProgram,
          args: item.relaunchArgs,
          workingDir: item.relaunchIn,
        });
      };
    }

    // 固定/取消固定
    if (!item.pinDisabled) {
      if (isActuallyPinned) {
        serializedItems.push({
          key: "taskbar_unpin_app",
          label: t("app_menu.unpin"),
        });
        callbacks["taskbar_unpin_app"] = () => {
          if (item.windows.length) {
            $dock_state_actions.unpinApp(item.id);
          } else {
            $dock_state_actions.remove(item.id);
          }
        };
      } else {
        serializedItems.push({
          key: "taskbar_pin_app",
          label: t("app_menu.pin"),
        });
        callbacks["taskbar_pin_app"] = () => {
          $dock_state_actions.pinApp(item.id);
        };
      }
    }

    // 固定项没有窗口时不显示关闭选项
    const showCloseOptions = item.windows.length > 0 || item.type !== TaskbarItemType.Pinned;

    if (showCloseOptions) {
      if (devTools) {
        serializedItems.push({
          key: "taskbar_copy_hwnd",
          label: t("app_menu.copy_handles"),
        });
        callbacks["taskbar_copy_hwnd"] = () => {
          navigator.clipboard.writeText(
            JSON.stringify(item.windows.map((w: any) => w.handle.toString(16)))
          );
        };
      }

      // 关闭
      serializedItems.push({
        key: "taskbar_close_app",
        label: t("app_menu.close"),
      });
      callbacks["taskbar_close_app"] = () => {
        item.windows.forEach((w: any) => {
          invoke(FuncCommand.TaskbarCloseApp, { hwnd: w.handle });
        });
      };

      if ($settings.value.showEndTask) {
        serializedItems.push({
          key: "taskbar_kill_app",
          label: item.windows.length > 1 ? t("app_menu.kill_multiple") : t("app_menu.kill"),
          danger: true,
        });
        callbacks["taskbar_kill_app"] = () => {
          item.windows.forEach((w: any) => {
            invoke(FuncCommand.TaskbarKillApp, { hwnd: w.handle });
          });
        };
      }
    }

    // 保存回调映射
    menuCallbacksRef.current = callbacks;

    // 计算屏幕坐标（和 Preview 一样，基于图标位置而非鼠标位置）
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

    // 通过后端命令触发 contextmenu（懒创建）
    invoke(CONTEXTMENU_TRIGGER_CMD, {
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
  }, [item, isHovering, devTools, t]);

  // 获取主窗口图标
  const mainWindow = item.windows[0] as any;
  const mainWindowIcon = (mainWindow as any)?.iconPngBase64 as string | null | undefined;

  // 判断图标是否为正方形（以此决定是否需要背板）
  const isApproximatelySquare = (mainWindow as any)?.isApproximatelySquare ?? (item as any).isApproximatelySquare;

  // 🔧 从后端返回的数据中获取是否是本地图标
  const isFromLocal = (mainWindow as any)?.isFromLocal === true;

  // 获取当前背板模式
  const isWhiteBackplate = $settings.value.iconBackplateStyle === 'White';

  // 白色背板模式下的特殊图标
  const whiteBackplateIcon = isWhiteBackplate ? getWhiteBackplateIcon(item) : null;

  // 判断是否是文件资源管理器（原生资源管理器，不包括通过explorer.exe启动的UWP应用）
  function isFileExplorer(item: PinnedTaskbarItem | TemporalTaskbarItem): boolean {
    const umid = item.umid?.toLowerCase() || '';
    const relaunchProgram = item.relaunchProgram?.toLowerCase() || '';
    const relaunchArgs = (item as any).relaunchArgs?.toLowerCase() || '';
    const displayName = item.displayName?.toLowerCase() || '';

    // 1. UMID 精确匹配（最可靠）
    if (umid === 'microsoft.windows.explorer') {
      return true;
    }

    // 2. 通过 relaunchProgram 判断：是 explorer.exe 且不是 UWP 应用（无 shell:Appsfolder 参数）
    const isExplorerExe = relaunchProgram.endsWith('\\explorer.exe') || relaunchProgram === 'explorer.exe';
    const isUwpApp = relaunchArgs.includes('shell:appsfolder');
    if (isExplorerExe && !isUwpApp) {
      return true;
    }

    // 3. displayName 精确匹配（避免误判包含 explorer 单词的其他应用）
    if (displayName === 'explorer' || displayName === '文件资源管理器' || displayName === 'file explorer') {
      return true;
    }

    return false;
  }

  // 判断是否是系统设置
  function isSystemSettings(item: PinnedTaskbarItem | TemporalTaskbarItem): boolean {
    const displayName = item.displayName?.toLowerCase() || '';
    // 精确匹配，避免误判
    return displayName === '设置' || displayName === 'settings' || displayName === 'system settings';
  }

  // 获取白色背板模式下的特殊图标路径
  function getWhiteBackplateIcon(item: PinnedTaskbarItem | TemporalTaskbarItem): string | null {
    if (isFileExplorer(item)) {
      return '/static/icons/fileExplorer.png';
    }
    if (isSystemSettings(item)) {
      return '/static/icons/systemSetting.png';
    }
    return null;
  }

  // 根据设置和图标属性决定是否显示背板
  const shouldShowBackplate = () => {
    // 白色背板模式下，如果使用特殊图标，不显示背板
    if (isWhiteBackplate && whiteBackplateIcon) {
      return false;
    }

    const willUseFileIcon = !mainWindowIcon;

    // 如果是透明背板模式（保持原有逻辑不变）
    if ($settings.value.iconBackplateStyle === 'Transparent') {
      // 本地图标直接显示，非本地图标添加透明背板
      // 确保只有当 isFromLocal 明确为 true 时，才不显示背板
      return !(isFromLocal === true);
    }
    // 如果是白色背板模式
    else if ($settings.value.iconBackplateStyle === 'White') {
      if (willUseFileIcon && fileIconShapeRef.current) {
        // 使用 FileIcon 传递的实际形状信息
        // 正方形图标不加背板，非正方形图标加白色背板
        return !fileIconShapeRef.current.isSquare;
      } else if (willUseFileIcon) {
        // FileIcon 还没有传递形状信息，暂时不显示背板，等待回调
        return false;
      } else {
        // 使用 mainWindowIcon 渲染时，根据形状决定
        return isApproximatelySquare !== true;
      }
    }
    // 默认不显示背板
    return false;
  };
  const itemNode = (
    <div
      ref={itemRef}
      className="taskbar-item"
      onMouseEnter={handleMouseEnter}
      onMouseLeave={handleMouseLeave}
      onContextMenu={handleContextMenu}
      onClick={() => {
        // 防重复点击处理，点击后3秒内不响应
        if (!clickDebounceRef.current.isClickable) {
          return;
        }

        // 立即设置为不可点击
        clickDebounceRef.current.isClickable = false;

        // 500ms后恢复可点击
        if (clickDebounceRef.current.timeoutId) {
          clearTimeout(clickDebounceRef.current.timeoutId);
        }
        clickDebounceRef.current.timeoutId = window.setTimeout(() => {
          clickDebounceRef.current.isClickable = true;
        }, 500);

        // 立即执行操作
        (async () => {
          try {
            let window = item.windows[0];

            if (!window) {
              await invoke(FuncCommand.Run, {
                program: item.relaunchProgram,
                args: item.relaunchArgs,
                workingDir: item.relaunchIn,
              });
            } else {
              const wasFocused = focusedApp?.hwnd === window.handle;
              await invoke(FuncCommand.TaskbarToggleWindowState, {
                hwnd: window.handle,
                wasFocused,
              });
              // this fix an issue of persisting focused colors when minimizing from dock
              if (wasFocused) {
                emit("hidden::remove-focused-color");
              }
            }
          } catch (e) {
            console.error('[UserApplication] 操作失败:', e);
          }
        })();
      }}
      onAuxClick={(e) => {
        const window = item.windows[0];
        if (e.button === 1 && window) {
          invoke(FuncCommand.TaskbarCloseApp, { hwnd: window.handle });
        }
      }}
    >
      {/* 条件渲染背板：根据设置和图标属性决定是否显示背板 */}
      {shouldShowBackplate() && <BackgroundByLayersV2 prefix="item" />}
      {/* 白色背板模式下的特殊图标处理 */}
      {isWhiteBackplate && whiteBackplateIcon ? (
        <img
          className="taskbar-item-icon"
          src={whiteBackplateIcon}
          alt={item.displayName}
          data-shape="square"
        />
      ) : mainWindowIcon ? (
        <img
          className="taskbar-item-icon"
          src={`data:image/png;base64,${mainWindowIcon}`}
          alt={item.displayName}
          data-shape={isApproximatelySquare === true ? "square" : "unknown"}
          data-local={isFromLocal ? "true" : undefined}
        />
      ) : (
        <FileIcon
          className="taskbar-item-icon"
          path={item.path}
          umid={item.umid}
          onShapeChange={(isSquare, isFromLocal) => {
            fileIconShapeRef.current = { isSquare, isFromLocal };
            forceUpdate(n => n + 1);
          }}
          {...({} as any)}
        />
      )}
      {itemLabel && <div className="taskbar-item-title">{itemLabel}</div>}
      {hasNotification && item.windows.length > 0 && <div className="taskbar-item-notification-badge" />}
      {!$settings.value.showWindowTitle && (() => {
        const relaunchProgram = item.relaunchProgram?.toLowerCase() || "";
        const processName = relaunchProgram.split("\\").pop() || relaunchProgram;

        const isPinned = item.type === TaskbarItemType.Pinned;
        const isInHiddenList = processName ? hiddenIndicatorProcesses.has(processName) : false;
        const allWindowsMinimized = item.windows.length > 0 && item.windows.every((w: any) => w.isIconic);
        const anyWindowNotMinimized = item.windows.some((w: any) => !w.isIconic);
        const isFocused = item.windows.some((w: any) => w.handle === focusedApp?.hwnd);

        // 检测窗口状态变化：从最小化变为非最小化
        const lastState = lastWindowIconicState.get(processName);
        const currentState = allWindowsMinimized;
        const windowRestoredFromMinimized = lastState === true && !currentState && anyWindowNotMinimized;

        // 更新状态记录
        lastWindowIconicState.set(processName, currentState);

        // 只有当窗口从最小化恢复时才从隐藏列表中移除
        // 注意：窗口获得焦点的情况已经在 useEffect 中处理
        if (isInHiddenList && windowRestoredFromMinimized) {
          hiddenIndicatorProcesses.delete(processName);
        }

        const shouldHideIndicator = isPinned && (processName ? hiddenIndicatorProcesses.has(processName) : false);

        return !shouldHideIndicator && (
          <div
            className={cx("taskbar-item-open-sign", {
              "taskbar-item-open-sign-active": !!item.windows.length,
            })}
          />
        );
      })()}
    </div>
  );

  if (isOverlay) {
    return itemNode;
  }

  return itemNode;
});
