import {
  closestCorners,
  DndContext,
  DragEndEvent,
  DragOverlay,
  DragStartEvent,
  Modifier,
  PointerSensor,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import {
  restrictToParentElement,
  restrictToWindowEdges,
} from "@dnd-kit/modifiers";
import {
  arrayMove,
  horizontalListSortingStrategy,
  SortableContext,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { batch, useSignal, useComputed } from "@preact/signals";
import { FuncEvent, TaskbarItemType } from "@magic-ui/lib";
import { useTranslation } from "react-i18next";
import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { FileOrFolder } from "../item/infra/File";
import { Separator } from "../item/infra/Separator";
import { StartMenu } from "../item/infra/StartMenu";
import { UserApplication } from "../item/infra/UserApplication";

import { RecycleBin } from "../item/infra/RecycleBin";
import { SystemTray } from "../item/infra/SystemTray";

import { SwItem } from "../shared/store/domain";

import { $dock_state, HardcodedSeparator1, HardcodedSeparator2, saveDockStateToBackend } from "../shared/state/items";
import { $settings } from "../shared/state/mod";
import { DraggableItem } from "./DraggableItem";
// 启用波浪缩放动效（对所有区域生效）
import { useDockMagnifier } from "./DockMagnifier";

const normalizeZoomEffectType = (value: unknown): 'wave' | 'singleIcon' =>
  typeof value === 'string' && value.toLowerCase() === 'singleicon'
    ? 'singleIcon'
    : 'wave';
const TASKBAR_LAYOUT_REFRESH_EVENT = "taskbar-layout-refresh-request";

export function DockItems({ isHorizontal, zoomEffectType: zoomEffectTypeProp }: { isHorizontal: boolean, zoomEffectType?: 'wave' | 'singleIcon' }) {
  const $active_id = useSignal<string | null>(null);
  const $pending_pin_id = useSignal<string | null>(null);  // 🔧 标记正在处理固定的项目ID，防止重复
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);

  // 🔧 使用 useComputed 创建计算属性，确保能响应 $settings.value 的变化
  const $zoomEffectTypeFromSettings = useComputed(() => {
    // 🔍 注意：$settings.value 已经是 TaskbarSettings，直接访问 zoomEffectType
const result = normalizeZoomEffectType(($settings.value as any).zoomEffectType);
    return result;
  });

  // 启用波形缩放动效（对所有区域生效）
  // 优先使用传入的 prop，如果没有则从设置中读取
  const zoomEffectType = (zoomEffectTypeProp ?? $zoomEffectTypeFromSettings.value) as 'wave' | 'singleIcon';

  // 🔧 关键修复：只在波浪模式下启用 useDockMagnifier，单点聚焦模式使用 CSS :hover 实现
  // 🔧 同时检查总开关 enableZoomEffect，确保受其控制
  const enableZoomEffect = ($settings.value as any).enableZoomEffect ?? true;

  // 🔧 关键修复：禁止在条件语句中调用 Hook，改用 enabled 参数控制
  useDockMagnifier(containerRef, {
    maxScale: 1.5,          // 最大放大倍数
    hoverThreshold: 180,     // 波浪影响范围（覆盖左右各 2 个图标，共 5 个联动）
    smoothFactor: 0.75,      // 阻尼系数
    enableGapScaling: false,
    targetSelector: '.taskbar-item-drag-container .taskbar-item',
    enabled: enableZoomEffect && zoomEffectType === 'wave', // 🔧 只有波浪模式才启用，且受总开关控制
    zoomEffectType: zoomEffectType, // 根据设置控制放大效果类型
  });

  useEffect(() => {
    const requestOuterLayoutRefresh = () => {
      requestAnimationFrame(() => {
        window.dispatchEvent(new Event(TASKBAR_LAYOUT_REFRESH_EVENT));
      });
    };

    window.addEventListener('resize', requestOuterLayoutRefresh);
    const unlistenRefresh = listen(FuncEvent.TaskbarContainerRefresh, requestOuterLayoutRefresh);

    return () => {
      window.removeEventListener('resize', requestOuterLayoutRefresh);
      unlistenRefresh.then((fn) => fn());
    };
  }, []);

  const pointerSensor = useSensor(PointerSensor, {
    activationConstraint: {
      distance: 5,
    },
  });
  const sensors = useSensors(pointerSensor);

  // 修复 zoom 缩放时拖拽位置偏移
  // DragOverlay 在 zoom 容器内使用 position:fixed 定位，
  // 初始位置(top/left)和 transform delta 都会被 zoom 缩放，需要补偿。
  // 注意：只在 DragOverlay 上应用，不在 DndContext 上应用，
  // 因为 DndContext 的 modifiedTranslate 会传递给 DragOverlay 再次经过 modifier。
  const getTaskbarZoom = () => {
    const taskbarEl = document.querySelector('.taskbar') as HTMLElement;
    return taskbarEl ? parseFloat(taskbarEl.style.zoom || '1') || 1 : 1;
  };
  // 跟踪拖拽过程中 overlay 是否有过非零 transform，用于检测 dragEnd 重置帧
  const hadNonZeroTransform = useRef(false);

  const overlayZoomCompensation: Modifier = ({ transform, activeNodeRect }) => {
    const zoom = getTaskbarZoom();

    // 检测 dragEnd 重置帧
    if (hadNonZeroTransform.current && transform.x === 0 && transform.y === 0) {
      return { ...transform, x: 0, y: -9999 };
    }
    if (Math.abs(transform.x) > 1 || Math.abs(transform.y) > 1) {
      hadNonZeroTransform.current = true;
    }

    if (zoom === 1 || !activeNodeRect) return transform;
    // 补偿 delta: delta/zoom，使得 (delta/zoom)*zoom = delta（视口像素）
    // 补偿初始位置: position:fixed 的 top/left 也被 zoom 缩放，
    //   需要额外偏移 rect.left*(1/zoom-1) 使 (left + offset)*zoom = left
    return {
      ...transform,
      x: transform.x / zoom + activeNodeRect.left * (1 / zoom - 1),
      y: transform.y / zoom + activeNodeRect.top * (1 / zoom - 1),
    };
  };

  const isEmpty = $dock_state.value.items.filter((c) => c.type !== TaskbarItemType.Separator)
    .length === 0;

  function handleDragStart(e: DragStartEvent) {
    hadNonZeroTransform.current = false;
    // 禁止 #root 在拖拽期间应用 transform（自动隐藏动画）
    // 否则 #root 的 transform 会创建 containing block，导致 DragOverlay 的 position:fixed 偏移
    document.getElementById('root')?.style.setProperty('transform', 'none', 'important');
    // 直接在所有 .taskbar-item 上禁用 transition
    const items = containerRef.current?.querySelectorAll('.taskbar-item');
    items?.forEach(el => {
      (el as HTMLElement).style.setProperty('transition', 'none', 'important');
    });
    $active_id.value = e.active.id as string;
  }

  function handleDragEnd(e: DragEndEvent) {
    const { active, over } = e;

    // 延迟两帧恢复 .taskbar-item 的 transition 和 #root 的 transform
    const itemEls = containerRef.current?.querySelectorAll('.taskbar-item');
    const rootEl = document.getElementById('root');
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        itemEls?.forEach(el => {
          (el as HTMLElement).style.removeProperty('transition');
        });
        rootEl?.style.removeProperty('transform');
      });
    });

    // 获取当前项目列表
    const currentItems = [...$dock_state.value.items];

    // 找到开始菜单的位置
    const startMenuIndex = currentItems.findIndex(item => item.type === TaskbarItemType.StartMenu);

    // 如果拖拽的是开始菜单，直接返回，不允许拖拽
    if (startMenuIndex !== -1 && active.id === currentItems[startMenuIndex]?.id) {
      $active_id.value = null;
      return;
    }

    const originalPos = currentItems.findIndex((c) => c.id === active.id);

    // 如果拖拽到桌面（over为null），只重置拖拽状态，不执行取消固定操作
    // 这样用户可以将固定区域的图标拖拽到桌面创建快捷方式，同时保持任务栏中的固定状态
    if (!over) {
      $active_id.value = null;
      return;
    }

    if (active.id === over.id) {
      $active_id.value = null;
      return;
    }

    let newPos = currentItems.findIndex((c) => c.id === over.id);

    // 如果目标位置在开始菜单之前，调整新位置为开始菜单之后
    if (startMenuIndex !== -1 && newPos <= startMenuIndex) {
      newPos = startMenuIndex + 1;
    }

    // 检查区域限制：左边和中间区域的项目不能拖拽到右边区域
    const separator2Index = currentItems.indexOf(HardcodedSeparator2);
    const activeItem = currentItems[originalPos];

    // 如果找不到拖拽的项目，直接返回
    if (!activeItem) {
      $active_id.value = null;
      return;
    }

    // 🔧 关键修复：禁止将非系统托盘图标拖到系统托盘位置及其之后
    // arrayMove(from, to) 在 from < to 时，元素会被插入到 to 位置（移除后的数组），
    // 导致拖拽的图标出现在 systemTray 之后。
    // 解决方案：将 newPos 限制在 systemTray 之前
    const systemTrayIndex = currentItems.findIndex(item => item.type === TaskbarItemType.SystemTray);
    if (systemTrayIndex !== -1 && activeItem.type !== TaskbarItemType.SystemTray && newPos >= systemTrayIndex) {
      newPos = systemTrayIndex - 1;
      // 如果限制后位置与原始位置相同，取消拖拽
      if (newPos === originalPos) {
        $active_id.value = null;
        return;
      }
    }

    // 获取当前区域信息
    const activeRegion = getRegionForItem(activeItem, currentItems);

    // 模拟拖拽后的新区域信息
    const tempItems = arrayMove([...currentItems], originalPos, newPos);
    const newRegion = getRegionForItem(activeItem, tempItems);

    // 检查是否是从中间区域拖拽到左侧固定区域（固定操作）
    if (activeRegion === 'center' && newRegion === 'left') {
      // 从中间拖拽到左侧：固定操作
      // 🔧 设置标志，防止handlePinStateChange再次调用后端
      $pending_pin_id.value = activeItem.id;

      // 计算目标位置
      const separator1Index = currentItems.indexOf(HardcodedSeparator1);
      const separator2Index = currentItems.indexOf(HardcodedSeparator2);
      const targetIndex = newPos > separator1Index ? newPos - separator1Index - 1 : newPos;

      // 🔧 关键修复：执行乐观更新，并删除原始位置的项
      // arrayMove会把项从一个位置移动到另一个位置，其他项目也会改变位置
      // 所以不需要额外删除，并且需要删除原始位置的重复项
      const itemsBeforeMove = [...currentItems];
      const movedItem = itemsBeforeMove[originalPos];

      // 第一步：执行arrayMove
      const newItems = arrayMove([...currentItems], originalPos, newPos);

      // 第二步：改变项目类型从Temporal变为Pinned
      // 这样保存到文件时，后端才能正确识别该项为固定项
      const movedItemIndex = newItems.findIndex(item => item.id === activeItem.id);
      if (movedItemIndex !== -1 && newItems[movedItemIndex]?.type === TaskbarItemType.Temporal) {
        newItems[movedItemIndex] = {
          ...newItems[movedItemIndex],
          type: TaskbarItemType.Pinned
        };
      }

      batch(() => {
        $active_id.value = null;
        $dock_state.value = { ...$dock_state.value, items: newItems };
      });

      // 🔍 详细的位置计算日志
      const leftItems = currentItems.slice(0, separator1Index);
      const centerItems = currentItems.slice(separator1Index + 1, separator2Index);

      // 调用后端API固定应用
      invoke("taskbar_pin_item", {
        umid: (activeItem as any).umid,
        relaunchProgram: (activeItem as any).relaunchProgram,
        displayName: (activeItem as any).displayName,
        path: (activeItem as any).path,
        originalId: activeItem.id,
        relaunchArgs: (activeItem as any).relaunchArgs,
        targetIndex: targetIndex,
      }).catch(err => {
        console.error("固定应用时创建快捷方式失败:", err);
      }).finally(() => {
        // 📌 操作完成后，清除标志
        $pending_pin_id.value = null;
      });

      // 重置拖拽状态
      $active_id.value = null;

      // 拖拽排序后通知所有图标组件重新计算坐标
      requestAnimationFrame(() => {
        window.dispatchEvent(new Event('taskbar-reorder-done'));
      });
      return;
    }

    // 如果拖拽的项目来自左边或中间区域，且目标位置在右边区域，则取消拖拽
    if ((activeRegion === 'left' || activeRegion === 'center') && newRegion === 'right') {
      $active_id.value = null;
      return;
    }

    // 禁止从左侧固定区域向中间区域拖拽
    if (activeRegion === 'left' && newRegion === 'center') {
      $active_id.value = null;
      return;
    }

    const newItems = arrayMove(currentItems, originalPos, newPos);

    batch(() => {
      $active_id.value = null;
      $dock_state.value = { ...$dock_state.value, items: newItems };
    });

    // 同一区域内拖拽完成后，立即保存状态到后端
    // 避免TaskbarItems.onChange用旧状态覆盖前端调整的顺序
    saveDockStateToBackend($dock_state.value).catch(err => {
      console.error("保存拖拽状态失败:", err);
    });

    // 拖拽排序后通知所有图标组件重新计算坐标
    // 拖拽使用 CSS transform，DOM childList 不变，MutationObserver 不会触发
    requestAnimationFrame(() => {
      window.dispatchEvent(new Event('taskbar-reorder-done'));
    });
  }

  // 辅助函数：确定项目所在区域
  function getRegionForItem(item: any, items: any[]) {
    const separator1Index = items.indexOf(HardcodedSeparator1);
    const separator2Index = items.indexOf(HardcodedSeparator2);
    const itemIndex = items.indexOf(item);

    if (itemIndex < separator1Index) return 'left';
    if (itemIndex > separator1Index && itemIndex < separator2Index) return 'center';
    if (itemIndex > separator2Index) return 'right';
    return 'separator';
  }

  // 处理固定/取消固定状态变化
  async function handlePinStateChange(item: SwItem, oldRegion: string, newRegion: string) {
    if ($pending_pin_id.value === item.id) {
      return;
    }
    // 只处理固定项和临时项
    if (item.type !== TaskbarItemType.Pinned && item.type !== TaskbarItemType.Temporal) {
      return;
    }

  }

  const dragginItem = $dock_state.value.items.find((c) => c.id === $active_id.value);

  // 判断item是否在中间区域
  const isInCenterZone = (item: SwItem) => {
    const items = $dock_state.value.items;
    const index1 = items.indexOf(HardcodedSeparator1);
    const index2 = items.indexOf(HardcodedSeparator2);
    const itemIndex = items.indexOf(item);
    return itemIndex > index1 && itemIndex < index2;
  };

  return (
    <DndContext
      collisionDetection={closestCorners}
      onDragStart={handleDragStart}
      onDragEnd={handleDragEnd}
      sensors={sensors}
      modifiers={[restrictToParentElement]} // 限制在 taskbar-items 容器内拖拽（不做 zoom 补偿，避免双重补偿）
    >
      <div className="taskbar-items" ref={containerRef}>
        {isEmpty ? <span className="taskbar-empty-state-label">{t("taskbar.empty")}</span> : (
          <SortableContext
            items={$dock_state.value.items.filter(item => item.type !== TaskbarItemType.StartMenu)}
            strategy={isHorizontal ? horizontalListSortingStrategy : verticalListSortingStrategy}
            disabled={$dock_state.value.isReorderDisabled}
          >
            {$dock_state.value.items.map((item) => (
              <DraggableItem key={item.id} item={item} isInCenterZone={isInCenterZone(item)} zoomEffectType={zoomEffectType}>
                {ItemByType(item, false)}
              </DraggableItem>
            ))}
          </SortableContext>
        )}
        <DragOverlay dropAnimation={null} modifiers={[restrictToWindowEdges, overlayZoomCompensation]}> {/* dropAnimation=null: 拖拽结束时 activeNodeRect 丢失，动画起始位置计算错误; restrictToWindowEdges 在前确保边界裁剪不会覆盖 reset frame 的 y=-9999 */}
          {dragginItem && ItemByType(dragginItem, true)}
        </DragOverlay>
      </div>
    </DndContext>
  );
}

function ItemByType(item: SwItem, isOverlay: boolean) {
  if (item.type === TaskbarItemType.Pinned) {
    if (item.subtype === "App") {
      return <UserApplication key={item.id} item={item} isOverlay={isOverlay} />;
    }
    return <FileOrFolder key={item.id} item={item} />;
  }

  if (item.type === TaskbarItemType.Temporal) {
    return <UserApplication key={item.id} item={item} isOverlay={isOverlay} />;
  }

  if (item.type === TaskbarItemType.StartMenu) {
    return <StartMenu key={item.id} item={item} />;
  }

  if (item.type === TaskbarItemType.Separator) {
    return <Separator key={item.id} item={item} />;
  }

  if (item.type === TaskbarItemType.RecycleBin) {
    return <RecycleBin key={item.id} item={item as any} />;
  }

  if (item.type === TaskbarItemType.SystemTray) {
    return <SystemTray key={item.id} item={item} />;
  }

  return null;
}
