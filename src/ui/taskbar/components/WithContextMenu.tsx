import { AnimatedDropdown } from "@shared/components/AnimatedWrappers";
import { useWindowFocusChange } from "@shared/hooks";
import { Menu } from "antd";
import { ItemType, MenuItemType } from "antd/es/menu/interface";
import { PropsWithChildren, useEffect, useRef, useState } from "react";
import { flushSync } from "react-dom";

import { BackgroundByLayersV2 } from "@shared/components/BackgroundByLayers/infra";

// 全局变量来追踪当前打开的菜单及其ID
let currentOpenMenuRef: { close: () => void; id: string } | null = null;

// 导出关闭所有图标菜单的函数
export function closeAllIconMenus() {
  if (currentOpenMenuRef) {
    currentOpenMenuRef.close();
    currentOpenMenuRef = null;
  }
}

interface Props extends PropsWithChildren {
  items: ItemType<MenuItemType>[];
  onOpenChange?: (isOpen: boolean) => void;
  placement?: "top" | "bottom" | "topCenter" | "bottomCenter" | "topLeft" | "topRight" | "bottomLeft" | "bottomRight";
}

export function WithContextMenu({ children, items, onOpenChange, placement = "topLeft" }: Props) {
  const [openContextMenu, setOpenContextMenu] = useState(false);
  const closeMenuRef = useRef<() => void>(() => {});
  const menuIdRef = useRef<string>(Math.random().toString(36).substring(7));
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    // 创建关闭当前菜单的函数
    closeMenuRef.current = () => {
      setOpenContextMenu(false);
      if (onOpenChange) {
        onOpenChange(false);
      }
    };
  }, [onOpenChange]);

  useWindowFocusChange((focused) => {
    if (!focused) {
      setOpenContextMenu(false);
    }
  });

  const handleContextMenu = (e: any) => {
    e.preventDefault();
    e.stopPropagation();

    // 关闭其他打开的菜单
    const menuId = menuIdRef.current;

    // 如果有其他菜单打开，强制同步关闭它
    if (currentOpenMenuRef && currentOpenMenuRef.id !== menuId) {
      // 使用 flushSync 强制立即更新 DOM，确保旧菜单立即消失
      flushSync(() => {
        currentOpenMenuRef!.close();
      });
      currentOpenMenuRef = null;
    }

    // 立即打开新菜单
    setOpenContextMenu(true);
    currentOpenMenuRef = { close: closeMenuRef.current, id: menuId };

    if (onOpenChange) {
      onOpenChange(true);
    }
  };

  return (
    <AnimatedDropdown
      animationDescription={{
        openAnimationName: "taskbar-context-menu-container-open",
        closeAnimationName: "taskbar-context-menu-container-close",
        maxAnimationTimeMs: 0,  // 禁用动画，避免跳动
      }}
      placement={placement}
      open={openContextMenu}
      onOpenChange={(isOpen) => {
        setOpenContextMenu(isOpen);
        if (!isOpen && currentOpenMenuRef?.id === menuIdRef.current) {
          currentOpenMenuRef = null;
        }
        if (onOpenChange) {
          onOpenChange(isOpen);
        }
      }}
      trigger={[]}
      popupRender={() => {
        const menuStyle: React.CSSProperties = {
          width: "fit-content",
          maxWidth: "300px",
        };

        return (
        <BackgroundByLayersV2
          className="taskbar-context-menu-container"
          prefix="menu"
          style={menuStyle}
          onContextMenu={(e) => {
            e.stopPropagation();
            e.preventDefault();
          }}
        >
          <Menu
            className="taskbar-context-menu"
            onMouseMoveCapture={(e) => e.stopPropagation()}
            items={items}
          />
        </BackgroundByLayersV2>
        );
      }}
    >
      <div
        ref={containerRef}
        onContextMenu={handleContextMenu}
        onClick={(e) => e.stopPropagation()}  // 阻止点击事件冒泡
        style={openContextMenu ? { pointerEvents: "none" } : undefined}
      >
        {children}
      </div>
    </AnimatedDropdown>
  );
}
