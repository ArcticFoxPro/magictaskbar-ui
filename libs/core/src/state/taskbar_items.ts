import type {
  MonitorId,
  TaskbarItem,
  TaskbarItems as ITaskbarItems,
} from "@magic-ui/types";
import {
  FuncCommand,
  FuncEvent,
  invoke,
  subscribe,
  type UnSubscriber,
} from "../handlers/mod.ts";
import { newFromInvoke } from "../utils/State.ts";
import type { Enum } from "../utils/enums.ts";

export class TaskbarItems {
  constructor(public inner: ITaskbarItems) {}

  /** Will return the taskbar items state without filtering by monitor */
  static getNonFiltered(): Promise<TaskbarItems> {
    return newFromInvoke(this, FuncCommand.StateGetTaskbarItems);
  }

  /** Will return the taskbar items state for a specific monitor */
  static getForMonitor(monitorId: MonitorId): Promise<TaskbarItems> {
    return newFromInvoke(this, FuncCommand.StateGetTaskbarItems, { monitorId });
  }

  static onChange(cb: () => void): Promise<UnSubscriber> {
    return subscribe(FuncEvent.StateTaskbarItemsChanged, () => cb());
  }

  /** Will store the taskbar items placeoments on disk */
  save(): Promise<void> {
    return invoke(FuncCommand.StateWriteTaskbarItems, { items: this.inner });
  }
}

// =================================================================================
//    From here some enums as helpers like @magic-ui/types only contains types
// =================================================================================

const TaskbarItemType: Enum<TaskbarItem["type"]> = {
  Pinned: "Pinned",
  Temporal: "Temporal",
  Separator: "Separator",
  StartMenu: "StartMenu",
  RecycleBin: "RecycleBin",
  SystemTray: "SystemTray",
};

export { TaskbarItemType };
