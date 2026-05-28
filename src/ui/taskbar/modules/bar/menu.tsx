import { FuncCommand } from "@magic-ui/lib";
import { invoke } from "@tauri-apps/api/core";
import { ItemType } from "antd/es/menu/interface";
import { TFunction } from "i18next";

export function getTaskbarMenu(t: TFunction): ItemType[] {
  return [
    {
      key: "task_manager",
      label: t("taskbar_menu.task_manager"),
      onClick() {
        invoke(FuncCommand.OpenFile, { path: "Taskmgr.exe" });
      },
    },
  ];
}
