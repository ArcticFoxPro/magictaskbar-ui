import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { cx } from "@shared/styles";
import { HTMLAttributes, PropsWithChildren } from "preact/compat";

import { SwItem } from "../shared/store/domain";

interface Props extends PropsWithChildren {
  item: SwItem;
  isInCenterZone?: boolean;
  zoomEffectType?: 'wave' | 'singleIcon';
}

export function DraggableItem({ children, item, isInCenterZone = false, zoomEffectType }: Props) {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({
    id: item.id,
    animateLayoutChanges: () => false,
    disabled: item.type === "Separator" || item.type === "StartMenu" || item.type === "RecycleBin" || item.type === "SystemTray",
  });

  // 补偿 zoom 对排序动画 transform 的影响
  // useSortable 返回的 transform 是视口像素，但在 zoom 容器内需除以 zoom
  const adjustedTransform = (() => {
    if (!transform) return null;
    const taskbarEl = document.querySelector('.taskbar') as HTMLElement;
    const zoom = taskbarEl ? parseFloat(taskbarEl.style.zoom || '1') || 1 : 1;
    if (zoom === 1) return transform;
    return { ...transform, x: transform.x / zoom, y: transform.y / zoom };
  })();

  return (
    <div
      ref={setNodeRef}
      {...(attributes as HTMLAttributes<HTMLDivElement>)}
      {...listeners}
      style={{
        transform: CSS.Translate.toString(adjustedTransform),
        transition,
        opacity: isDragging ? 0.3 : 1,
      }}
      className={cx("taskbar-item-drag-container", {
        dragging: isDragging,
        "center-zone": isInCenterZone,
        wave: zoomEffectType === 'wave',
        singleIcon: zoomEffectType === 'singleIcon',
      })}
      // this was added here to avoid need to pass it to all the items types,
      // this avoid the double context menu of dock menu and dock items.
      onContextMenu={item.type === "Separator" ? undefined : (e) => e.stopPropagation()}
    >
      {children}
    </div>
  );
}
