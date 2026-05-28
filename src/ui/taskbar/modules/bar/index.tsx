import { FuncCommand, FuncEvent, TaskbarMode, TaskbarSide } from "@magic-ui/lib";
import { cx } from "@shared/styles";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

import { BackgroundByLayersV2 } from "@shared/components/BackgroundByLayers/infra";

import { $dock_should_be_hidden, $settings } from "../shared/state/mod";
import { $current_monitor } from "../shared/state/system";
import { DockItems } from "./ItemReordableList";
import { TaskbarCoordinatesTracker } from "../shared/utils/taskbar_save_window_coordinates";

// ContextMenu 命令常量
const CONTEXTMENU_TRIGGER_CMD = "contextmenu_trigger";
const CONTEXTMENU_ITEM_CLICK_EVENT = "contextmenu::item_click";
const TASKBAR_LAYOUT_REFRESH_EVENT = "taskbar-layout-refresh-request";

const normalizeZoomEffectType = (value: unknown): 'wave' | 'singleIcon' =>
  typeof value === 'string' && value.toLowerCase() === 'singleicon'
    ? 'singleIcon'
    : 'wave';

export function Taskbar() {
  const { t } = useTranslation();
  const menuCallbacksRef = useRef<Record<string, () => void>>({});
  const taskbarRef = useRef<HTMLDivElement>(null);

  const zoomEffectType = normalizeZoomEffectType(($settings.value as any).zoomEffectType);
  const enableZoomEffect = ($settings.value as any).enableZoomEffect ?? true;
  const dockItemsRenderKey = `zoom-${enableZoomEffect ? 'on' : 'off'}-${zoomEffectType}`;

  // 后端发来的 DPI 值（通过 TaskbarContainerRefresh 事件更新）
  // recalculate() 用它做 DPI 一致性检查，避免在 WebView2 DPI 尚未更新时用错误值计算
  const backendDpiRef = useRef<number>(0);

  // 供 ContainerRefresh 事件触发强制 recalculate（跨 useEffect 通信）
  const forceRecalculateRef = useRef<((reason?: string) => void) | null>(null);

  // 监听 taskbar 容器宽度变化，更新后端窗口尺寸
  // 依赖 dockItemsRenderKey：当 DockItems 因 key 变化而重建时，
  // 旧的 .taskbar-items DOM 节点被移除，需要重新绑定 Observer 到新节点，
  // 否则波浪效果引起的宽度变化不会触发 recalculate，亚克力模糊区域不同步。
  useEffect(() => {
    const taskbarEl = taskbarRef.current;
    if (!taskbarEl) return;

    let lastPhysicalLeft = 0;
    let lastPhysicalTop = 0;
    let lastPhysicalWidth = 0;
    let lastPhysicalHeight = 0;
    let isDragging = false;
    let isRecalculating = false;
    let pendingRecalculate = false;
    let recalculateFrame: number | null = null;
    const SCALE_MARGIN = 20; // 屏幕两侧预留空间（CSS 像素）

    // 计算缩放因子：当容器自然宽度超过 (屏幕宽度 - SCALE_MARGIN) 时等比例缩小
    const calculateScale = (naturalWidth: number): number => {
      const screenWidth = window.innerWidth;
      const maxWidth = screenWidth - SCALE_MARGIN;
      if (naturalWidth > maxWidth && naturalWidth > 0) {
        return maxWidth / naturalWidth;
      }
      return 1;
    };

    // 应用缩放到 taskbar 容器（使用 zoom 而非 transform，zoom 同时影响视觉和布局）
    // 注意：不设显式 width，让 CSS min-content 自然跟随内容宽度变化
    // 否则波浪效果释放时 min-width:100% 会继承固定值，容器无法收缩
    const applyScale = (scale: number) => {
      if (scale < 1) {
        taskbarEl.style.zoom = `${scale}`;
        taskbarEl.style.maxWidth = 'none';
        taskbarEl.style.width = '';
      } else {
        taskbarEl.style.zoom = '';
        taskbarEl.style.maxWidth = '';
        taskbarEl.style.width = '';
      }
    };

    const resetScaleForDisplayChange = () => {
      taskbarEl.style.zoom = '';
      taskbarEl.style.maxWidth = '';
      taskbarEl.style.width = '';
      void taskbarEl.offsetHeight;
    };

    // 获取 .taskbar-items 的真实内容宽度（不受父元素 inline style 影响）
    // 只需：1) 移除 zoom（避免 getBoundingClientRect 返回缩放后值）
    //       2) 在 .taskbar-items 上设 min-width:0（打断 min-width:100% 百分比链）
    const getNaturalWidth = (): number => {
      const itemsEl = taskbarEl.querySelector('.taskbar-items') as HTMLElement;
      if (!itemsEl) return 0;

      const prevZoom = taskbarEl.style.zoom;
      const prevItemsMinWidth = itemsEl.style.minWidth;
      taskbarEl.style.zoom = '';
      itemsEl.style.minWidth = '0';

      let naturalWidth = itemsEl.getBoundingClientRect().width;

      // 加上 .taskbar-items-container 的 padding
      const containerEl = taskbarEl.querySelector('.taskbar-items-container') as HTMLElement;
      if (containerEl) {
        const cs = getComputedStyle(containerEl);
        naturalWidth += parseFloat(cs.paddingLeft) + parseFloat(cs.paddingRight);
      }

      itemsEl.style.minWidth = prevItemsMinWidth;
      taskbarEl.style.zoom = prevZoom;
      return naturalWidth;
    };

    // 更新容器位置，使其相对于屏幕居中
    const updateContainerPosition = (visibleWidth: number): number | null => {
      const rootEl = document.getElementById('root');
      if (!rootEl) return null;

      const screenCenterX = window.innerWidth / 2;
      const containerLeft = Math.round(screenCenterX - visibleWidth / 2);
      rootEl.style.left = `${containerLeft}px`;
      return containerLeft;
    };

    const recalculate = (reason = 'unknown') => {
      if (isDragging) return;

      // Transform 守卫：taskbar 隐藏（overlapped）时有 CSS transform，
      // getBoundingClientRect() 包含 transform 偏移会导致 top 值错误。
      // 此时跳过测量，等 transitionend 恢复可见后再测。
      const rootEl = document.getElementById('root');
      let isRootTransformed = false;
      if (rootEl) {
        const transform = getComputedStyle(rootEl).transform;
        if (transform && transform !== 'none' && transform !== 'matrix(1, 0, 0, 1, 0, 0)') {
          isRootTransformed = true;
        }
      }

      // DPI 一致性守卫：如果后端已通知了新 DPI，但 WebView2 的 devicePixelRatio
      // 还没更新，此时 getBoundingClientRect() 返回的是旧 DPI 坐标空间的值，
      // 用旧 DPI 转物理像素会算出错误尺寸（如 h=144 而非 126）
      const browserDpi = globalThis.devicePixelRatio || 1;
      if (backendDpiRef.current > 0 && Math.abs(browserDpi - backendDpiRef.current) > 0.01) {
        return;
      }

      const naturalWidth = getNaturalWidth();
      if (naturalWidth <= 0) {
        return;
      }

      // 计算并应用缩放
      const scale = calculateScale(naturalWidth);
      applyScale(scale);

      // 关键：先更新容器居中位置，再测量 getBoundingClientRect
      // 否则 containerLeft 是旧位置下测量的，与实际居中位置偏移几十 px
      const visibleWidth = naturalWidth * scale;
      const containerLeft = updateContainerPosition(visibleWidth);
      // root hidden/slide transform pollutes visualRect top/left. Centering and scale
      // above are still safe, so keep them fresh and defer only the physical glass rect.
      if (isRootTransformed) {
        return;
      }

      // 现在测量的是最终渲染位置（zoom + 居中都已生效）
      const visualRect = taskbarEl.getBoundingClientRect();
      const dpi = globalThis.devicePixelRatio || 1;
      // 用 right - left 而非 round(width*dpi) 避免舍入误差导致 blur 宽度偏移
      const physicalContainerLeft = Math.round(visualRect.left * dpi);
      const physicalContainerRight = Math.round((visualRect.left + visualRect.width) * dpi);
      const physicalWidth = physicalContainerRight - physicalContainerLeft;
      const physicalContainerTop = Math.round(visualRect.top * dpi);
      const physicalContainerHeight = Math.round(visualRect.height * dpi);

      // 位置和尺寸都小于 2px 时才跳过，避免亚像素抖动。
      // DPI 切换后可能只有 top 变化；若只按宽高去重，glass blur y 会停在旧坐标。
      if (
        Math.abs(physicalContainerLeft - lastPhysicalLeft) < 2 &&
        Math.abs(physicalContainerTop - lastPhysicalTop) < 2 &&
        Math.abs(physicalWidth - lastPhysicalWidth) < 2 &&
        Math.abs(physicalContainerHeight - lastPhysicalHeight) < 2
      ) {
        return;
      }
      lastPhysicalLeft = physicalContainerLeft;
      lastPhysicalTop = physicalContainerTop;
      lastPhysicalWidth = physicalWidth;
      lastPhysicalHeight = physicalContainerHeight;

      // 通知后端更新窗口和亚克力模糊区域
      invoke("taskbar_update_window_size", {
        width: physicalWidth,
        containerLeft: physicalContainerLeft,
        containerTop: physicalContainerTop,
        containerHeight: physicalContainerHeight,
      }).catch(err => {
        console.error('[Taskbar] 更新窗口宽度失败:', err);
      });
    };

    // 防反馈循环的 recalculate 包装。
    // item 增减时 MutationObserver 与 ResizeObserver 可能在同一帧连续触发；
    // 如果第一下测到旧宽度，后续触发不能丢，否则视觉会只向右自然扩展。
    const safeRecalculate = (reason = 'unknown') => {
      if (isRecalculating || recalculateFrame !== null) {
        pendingRecalculate = true;
        return;
      }

      recalculateFrame = requestAnimationFrame(() => {
        recalculateFrame = null;
        isRecalculating = true;
        recalculate(reason);
        requestAnimationFrame(() => {
          isRecalculating = false;
          if (pendingRecalculate) {
            pendingRecalculate = false;
            safeRecalculate('pending');
          }
        });
      });
    };

    const resetLastMeasurements = () => {
      lastPhysicalLeft = 0;
      lastPhysicalTop = 0;
      lastPhysicalWidth = 0;
      lastPhysicalHeight = 0;
    };

    // DPI/分辨率变化后，后端事件、WebView DPR、DOM 布局可能分步稳定。
    // 连续短间隔重测，避免必须等鼠标移入/图标放大才恢复。
    let dpiSettleTimers: ReturnType<typeof setTimeout>[] = [];
    const clearDpiSettleTimers = () => {
      dpiSettleTimers.forEach((timer) => clearTimeout(timer));
      dpiSettleTimers = [];
    };
    const debouncedRecalculate = (reason = 'debounced') => {
      clearDpiSettleTimers();
      resetScaleForDisplayChange();
      [0, 100, 300, 800, 1500, 3000, 5000].forEach((delay) => {
        const timer = setTimeout(() => {
          dpiSettleTimers = dpiSettleTimers.filter((item) => item !== timer);
          resetLastMeasurements();
          safeRecalculate(`${reason}:settle-${delay}`);
        }, delay);
        dpiSettleTimers.push(timer);
      });
    };

    // 注册强制 recalculate 供 ContainerRefresh 事件使用
    forceRecalculateRef.current = debouncedRecalculate;

    // 1. MutationObserver 监听 taskbar 子树。
    //    热插拔后 DockItems 可能重建 .taskbar-items 节点；如果只观察旧节点，
    //    后续 item 增减会自然撑宽 DOM，但不会重新居中。
    const itemsEl = taskbarEl.querySelector('.taskbar-items');
    const mutationObserver = new MutationObserver(() => {
      safeRecalculate('mutation');
    });
    mutationObserver.observe(taskbarEl, { childList: true, subtree: true });

    // 2. ResizeObserver 作为补充：捕获非 childList 触发的尺寸变化（如 item 内容变宽）。
    //    同时观察 taskbarEl，避免 .taskbar-items 被替换后 observer 留在旧节点。
    const resizeObserver = new ResizeObserver(() => {
      safeRecalculate('resize');
    });
    resizeObserver.observe(taskbarEl);
    if (itemsEl) {
      resizeObserver.observe(itemsEl);
    }

    const handleLayoutRefreshRequest = () => {
      debouncedRecalculate('layout-refresh-request');
    };
    window.addEventListener(TASKBAR_LAYOUT_REFRESH_EVENT, handleLayoutRefreshRequest);

    // 监听拖拽事件，拖拽过程中暂停窗口更新
    const handleDragStart = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      if (target.closest('.taskbar-item-drag-container')) {
        isDragging = true;
      }
    };
    const handleDragEnd = () => {
      if (isDragging) {
        isDragging = false;
        safeRecalculate('drag-end');
      }
    };

    document.addEventListener('mousedown', handleDragStart);
    document.addEventListener('mouseup', handleDragEnd);

    // 初始化
    recalculate('initial');

    // 3. 监听 taskbar 显示/隐藏状态变化
    //    当 taskbar 从隐藏恢复显示时，强制重新测量并更新后端的 glass 尺寸/blur 区域。
    //    修复：模式切换后 glass blur region 在隐藏期间更新未生效，
    //    show 后需要前端重新发送最新测量值（与鼠标移动触发 recalculate 同理）。
    const rootEl = document.getElementById('root');
    const onTransitionEnd = (e: Event) => {
      if (e.target !== rootEl || (e as TransitionEvent).propertyName !== 'transform') return;
      const transform = getComputedStyle(rootEl!).transform;
      if (transform === 'none' || transform === 'matrix(1, 0, 0, 1, 0, 0)') {
        // 滑入动画完成，taskbar 完全可见，强制刷新 blur region
        resetLastMeasurements();
        safeRecalculate('transitionend');
      }
    };
    rootEl?.addEventListener('transitionend', onTransitionEnd);

    // 4. 监听 DPI 变化：WebView2 更新 devicePixelRatio 后自动触发重算
    //    ResizeObserver 不响应纯 DPI 变化，需要 matchMedia 补充
    let dpiMediaQuery = window.matchMedia(`(resolution: ${globalThis.devicePixelRatio}dppx)`);
    const onDpiChange = () => {
      // DPI 已更新，同步 backendDpiRef 以解除 recalculate 的 DPI 守卫
      backendDpiRef.current = globalThis.devicePixelRatio;
      // 立即 recalculate：纯 DPI 切换时 DOM 是稳定的，立即测量正确且快速
      resetLastMeasurements();
      safeRecalculate('dpi-change');
      // 防抖兜底：DPI+分辨率同时变时，立即测量可能因 DOM 不稳定而出错，
      // 300ms 后再测一次以修正（纯 DPI 变化时不影响，dedup 会跳过相同值）
      debouncedRecalculate('dpi-change-debounced');
      // matchMedia 是一次性的，需要重新创建监听器以匹配新 DPI
      dpiMediaQuery.removeEventListener('change', onDpiChange);
      dpiMediaQuery = window.matchMedia(`(resolution: ${globalThis.devicePixelRatio}dppx)`);
      dpiMediaQuery.addEventListener('change', onDpiChange);
    };
    dpiMediaQuery.addEventListener('change', onDpiChange);

    return () => {
      forceRecalculateRef.current = null;
      clearDpiSettleTimers();
      if (recalculateFrame !== null) {
        cancelAnimationFrame(recalculateFrame);
        recalculateFrame = null;
      }
      mutationObserver.disconnect();
      resizeObserver.disconnect();
      window.removeEventListener(TASKBAR_LAYOUT_REFRESH_EVENT, handleLayoutRefreshRequest);
      document.removeEventListener('mousedown', handleDragStart);
      document.removeEventListener('mouseup', handleDragEnd);
      rootEl?.removeEventListener('transitionend', onTransitionEnd);
      dpiMediaQuery.removeEventListener('change', onDpiChange);
    };
  }, [dockItemsRenderKey]);


  // Listen for backend event to refresh container position (DPI/monitor config changed)
  useEffect(() => {
    const unlistenRefresh = getCurrentWebviewWindow().listen<[number, number]>(FuncEvent.TaskbarContainerRefresh, (event) => {
      const [screenCenterX, dpiTimes100] = event.payload;
      const dpi = dpiTimes100 / 100;
      const browserDpi = globalThis.devicePixelRatio || 1;
      const currentMonitor = $current_monitor.value;
      const expectedCenterX = currentMonitor
        ? Math.round((currentMonitor.rect.left + currentMonitor.rect.right) / 2)
        : null;

      if (expectedCenterX !== null && Math.abs(screenCenterX - expectedCenterX) > 2) {
        return;
      }

      // 更新后端 DPI 参考值，供 recalculate() 做 DPI 守卫
      backendDpiRef.current = dpi;

      if (Math.abs(browserDpi - dpi) > 0.01) {
        // DPI 不匹配时不能立即测量，但仍需调度防抖：
        // 300ms 后 DPI 应已一致，可以安全测量修正 blur 位置
        if (forceRecalculateRef.current) {
          forceRecalculateRef.current('container-refresh-dpi-mismatch');
        }
        return;
      }

      // 复用标准 recalculate() 而非独立测量，避免两套测量代码结果不一致
      // （独立测量在 DPI/分辨率切换后可能因布局未稳定而得到错误 Y 坐标）
      if (forceRecalculateRef.current) {
        forceRecalculateRef.current('container-refresh');
      }
    });
    return () => {
      unlistenRefresh.then((fn) => fn());
    };
  }, []);


  // Listen for menu item click event
  useEffect(() => {
    const unlisten = listen(CONTEXTMENU_ITEM_CLICK_EVENT, (event: any) => {
      const { key, menuType } = event.payload;
      if (menuType === "taskbar") {
        const callback = menuCallbacksRef.current[key];
        if (callback) {
          callback();
        }
      }
    });
    return () => {
      unlisten.then(fn => fn());
    };
  }, []);

  const settings = $settings.value;
  const isHorizontal = settings.position === TaskbarSide.Top ||
    settings.position === TaskbarSide.Bottom;
  const handleContextMenu = async (e: any) => {
    e.preventDefault();
    e.stopPropagation();
    const clientX = e.clientX;
    const clientY = e.clientY;

    // 构建菜单项
    const callbacks: Record<string, () => void> = {};
    const serializedItems: any[] = [];

    serializedItems.push({
      key: "task_manager",
      label: t("taskbar_menu.task_manager"),
    });
    callbacks["task_manager"] = () => {
      invoke(FuncCommand.OpenFile, { path: "Taskmgr.exe" });
    };

    menuCallbacksRef.current = callbacks;

    // 计算屏幕坐标
    const currentMonitor = $current_monitor.value;
    const dpiScale = currentMonitor?.dpi || globalThis.window.devicePixelRatio || 1;
    const webviewPos = await getCurrentWebviewWindow().outerPosition();

    const x = webviewPos.x + Math.round(clientX * dpiScale);
    const y = webviewPos.y + Math.round(clientY * dpiScale);

    invoke(CONTEXTMENU_TRIGGER_CMD, {
      payload: {
        menuType: "taskbar",
        items: serializedItems,
        position: { x, y, placement: "top" },
        monitorId: currentMonitor?.id ?? null,
        monitorDpi: currentMonitor?.dpi ?? null,
        monitorRect: currentMonitor?.rect ?? null,
      },
    });
  };

  return (
    <>
      <TaskbarCoordinatesTracker />
      <div
        ref={taskbarRef}
        className={cx("taskbar", settings.position.toLowerCase(), {
          horizontal: isHorizontal,
          vertical: !isHorizontal,
          "full-width": settings.mode === TaskbarMode.FullWidth,
          "white-backplate": settings.iconBackplateStyle === 'White',
          hidden: $dock_should_be_hidden.value,
          "zoom-enabled": ($settings.value as any).enableZoomEffect ?? true,
          singleIcon: zoomEffectType === 'singleIcon',
          wave: zoomEffectType === 'wave',
        })}
        onContextMenu={handleContextMenu}
      >
        <BackgroundByLayersV2 prefix="taskbar" />
        <div className="taskbar-items-container">
          <DockItems
            key={dockItemsRenderKey}
            isHorizontal={isHorizontal}
            zoomEffectType={zoomEffectType}
          />
        </div>
      </div>
    </>
  );
}
