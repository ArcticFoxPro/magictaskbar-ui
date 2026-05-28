import { FuncCommand, TaskbarItemType } from "@magic-ui/lib";
import { invoke } from "@tauri-apps/api/core";
import { MenuProps } from "antd";
import { ItemType } from "antd/es/menu/interface";
import { TFunction } from "i18next";

import { PinnedTaskbarItem, TemporalTaskbarItem } from "../../shared/store/domain";

import { $dock_state, $dock_state_actions } from "../../shared/state/items";

export function getUserApplicationContextMenu(
  t: TFunction,
  item: PinnedTaskbarItem | TemporalTaskbarItem,
  devTools: boolean,
  showEndTask: boolean,
): ItemType[] {
  // 检查应用是否真的存在于任务栏的固定区域中
  // 与任务栏菜单保持一致的逻辑：检查全局固定状态，确保显示正确的固定/取消固定选项
  const isActuallyPinned = $dock_state.value.items.some(
    (taskbarItem) => taskbarItem.id === item.id && taskbarItem.type === TaskbarItemType.Pinned
  );

  // 处理窗口标题的辅助函数，提取简洁的应用名称
  const getDisplayName = () => {
      if (item.displayName.startsWith('app_menu.')) {
        return t(item.displayName);
      }
      return item.displayName;
  };

  // 菜单项标签样式
  const menuLabelStyle = {
    width: "100%",
    height: "100%",
    margin: "-10px",
    padding: "10px",
    whiteSpace: "nowrap" as const,
  };

  // 菜单项标签渲染函数
  const renderMenuLabel = (text: string) => (
    <div style={menuLabelStyle}>{text}</div>
  );

  const menu: MenuProps["items"] = [];

  if (!item.pinDisabled) {
    if (isActuallyPinned) {
      menu.push({
        label: renderMenuLabel(t("app_menu.unpin")),
        key: "taskbar_unpin_app",
        onClick: () => {
          if (item.windows.length) {
            $dock_state_actions.unpinApp(item.id);
          } else {
            $dock_state_actions.remove(item.id);
          }
        },
      });
    } else {
      menu.push({
        key: "taskbar_pin_app",
        label: renderMenuLabel(t("app_menu.pin")),
        onClick: () => {
          $dock_state_actions.pinApp(item.id);
        },
      });
    }

    menu.push({
      type: "divider",
    });
  }

  // 获取应用图标的逻辑：白名单应用使用窗口图标，非白名单应用使用FileIcon

  // 不支持固定的 item（pinDisabled=true，如 WPS 子应用 / 文档窗口 / 联名产品等）同样不支持“打开”，
  // 避免启动 MSIX 包级共享入口或其他非预期的窗口。
  if (!item.pinDisabled) {
    menu.push(
      {
        key: "taskbar_run_new",
        label: getDisplayName(),
        onClick: () => {
          invoke(FuncCommand.Run, {
            program: item.relaunchProgram,
            args: item.relaunchArgs,
            workingDir: item.relaunchIn,
          });
        },
      },
    );
  }

  // 固定项没有窗口时不显示关闭选项
  if (!item.windows.length && item.type === TaskbarItemType.Pinned) {
    return menu;
  }

  if (devTools) {
    menu.push({
      key: "taskbar_copy_hwnd",
      label: t("app_menu.copy_handles"),
      onClick: () =>
        navigator.clipboard.writeText(
          JSON.stringify(
            item.windows.map((window: any) => window.handle.toString(16)),
          ),
        ),
    });
  }

  // 添加关闭选项
  menu.push({
    key: "taskbar_close_app",
    label: item.windows.length > 1 ? t("app_menu.close_multiple") : t("app_menu.close"),
    onClick() {
      item.windows.forEach((window: any) => {
        invoke(FuncCommand.TaskbarCloseApp, { hwnd: window.handle });
      });
    },
  });

  if (showEndTask) {
    menu.push({
      key: "taskbar_kill_app",
      label: item.windows.length > 1 ? t("app_menu.kill_multiple") : t("app_menu.kill"),
      onClick() {
        item.windows.forEach((window: any) => {
          // todo replace by enum
          invoke(FuncCommand.TaskbarKillApp, { hwnd: window.handle });
        });
      },
      danger: true,
    });
  }

  return menu;
}
