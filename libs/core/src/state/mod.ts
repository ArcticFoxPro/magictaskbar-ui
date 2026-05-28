import { FuncCommand, FuncEvent, type UnSubscriber } from "../handlers/mod.ts";
import { newFromInvoke, newOnEvent } from "../utils/State.ts";
import type { LauncherHistory as ILauncherHistory } from "@magic-ui/types";

export * from "./theme/mod.ts";
export * from "./settings/mod.ts";
export * from "./taskbar_items.ts";
export * from "./settings/settings_by_monitor.ts";
export * from "./icon_pack.ts";

export class LauncherHistory {
  constructor(public inner: ILauncherHistory) {}

  static getAsync(): Promise<LauncherHistory> {
    return newFromInvoke(this, FuncCommand.StateGetHistory);
  }

  static onChange(
    cb: (payload: LauncherHistory) => void,
  ): Promise<UnSubscriber> {
    return newOnEvent(cb, this, FuncEvent.StateHistoryChanged);
  }
}
