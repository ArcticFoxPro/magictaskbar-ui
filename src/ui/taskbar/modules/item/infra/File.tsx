import { FuncCommand, TaskbarItemType, TaskbarSide } from "@magic-ui/lib";
import { FileIcon } from "@shared/components/Icon";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { memo, useCallback, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";

import { BackgroundByLayersV2 } from "@shared/components/BackgroundByLayers/infra";

import { PinnedTaskbarItem } from "../../shared/store/domain";

import { $dock_state_actions } from "../../shared/state/items";
import { $settings } from "../../shared/state/mod";
import { $current_monitor } from "../../shared/state/system";

// ContextMenu 窗口命令常量
const CONTEXTMENU_TRIGGER_CMD = "contextmenu_trigger";
const CONTEXTMENU_ITEM_CLICK_EVENT = "contextmenu::item_click";

interface Props {
  item: PinnedTaskbarItem;
}

export const FileOrFolder = memo(({ item }: Props) => {
  const { t } = useTranslation();
  const itemRef = useRef<HTMLDivElement>(null);
  const menuCallbacksRef = useRef<Record<string, () => void>>({});

  const calculatePlacement = (position: any) => {
    switch (position) {
      case TaskbarSide.Bottom: return "top" as const;
      case TaskbarSide.Top: return "bottom" as const;
      case TaskbarSide.Left: return "right" as const;
      case TaskbarSide.Right: return "left" as const;
      default: return "top" as const;
    }
  };

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

  // 右键菜单处理
  const handleContextMenu = useCallback(async (e: any) => {
    e.preventDefault();
    e.stopPropagation();
    const clientX = e.clientX;
    const clientY = e.clientY;

    const callbacks: Record<string, () => void> = {};
    const serializedItems: any[] = [];

    if (item.type === TaskbarItemType.Pinned) {
      // 取消固定
      serializedItems.push({
        key: "remove",
        label: t("app_menu.unpin"),
      });
      callbacks["remove"] = () => {
        $dock_state_actions.remove(item.id);
      };

      serializedItems.push({ key: "divider_1", label: "", divider: true });

      // 打开文件位置
      serializedItems.push({
        key: "taskbar_select_file_on_explorer",
        label: t("app_menu.open_file_location"),
      });
      callbacks["taskbar_select_file_on_explorer"] = () => {
        invoke(FuncCommand.SelectFileOnExplorer, { path: item.path });
      };
    }

    menuCallbacksRef.current = callbacks;

    // 计算屏幕坐标（基于图标位置）
    const currentMonitor = $current_monitor.value;
    const dpiScale = currentMonitor?.dpi || globalThis.window.devicePixelRatio || 1;
    const webviewPos = await getCurrentWebviewWindow().outerPosition();
    const placement = calculatePlacement($settings.value.position);

    const rect = itemRef.current?.getBoundingClientRect();
    const x = rect
      ? webviewPos.x + Math.round((rect.left + rect.width / 2) * dpiScale)
      : webviewPos.x + Math.round(clientX * dpiScale);
    const y = rect
      ? (placement === "top"
        ? webviewPos.y + Math.round(rect.top * dpiScale)
        : webviewPos.y + Math.round(rect.bottom * dpiScale))
      : webviewPos.y + Math.round(clientY * dpiScale);

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
  }, [item, t]);

  // 从后端返回的数据中获取是否是本地图标
  const isFromLocal = (item as any).isFromLocal === true;

  // 根据设置和图标属性决定是否显示背板
  const shouldShowBackplate = () => {
    // 本地图标直接显示，非本地图标添加背板（透明或白色由CSS控制）
    if ($settings.value.iconBackplateStyle === 'Transparent' || $settings.value.iconBackplateStyle === 'White') {
      return !isFromLocal;
    }
    return false;
  };

  return (
    <div
      ref={itemRef}
      className="taskbar-item"
      onClick={() => {
        invoke(FuncCommand.OpenFile, { path: item.path });
      }}
      onContextMenu={handleContextMenu}
    >
      {shouldShowBackplate() && <BackgroundByLayersV2 prefix="item" />}
      <FileIcon
        className="taskbar-item-icon"
        path={item.path}
        umid={item.umid}
        {...({} as any)}
      />
    </div>
  );
});
