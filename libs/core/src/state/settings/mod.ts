import {
  FuncCommand,
  FuncEvent,
  type UnSubscriber,
} from "../../handlers/mod.ts";

import type {
  FancyToolbarSettings,
  FancyToolbarSide,
  HideMode,
  /*   SeelenLauncherMonitor,
  SeelenLauncherSettings, */
  Settings as ISettings,
  TaskbarMode,
  TaskbarSettings,
  TaskbarSide,
  ThirdPartyWidgetSettings,
  UpdateChannel,
  WidgetId,
} from "@magic-ui/types";
import { newFromInvoke, newOnEvent } from "../../utils/State.ts";
import type { Enum } from "../../utils/enums.ts";
import { invoke } from "../../handlers/mod.ts";

// Widget IDs for bundled widgets
export const MagicTaskbarWidgetId = "@magic/taskbar" as WidgetId;
/* export const SeelenToolbarWidgetId = "@magic/toolbar" as WidgetId; */

export interface Settings extends ISettings {}
export class Settings {
  constructor(public inner: ISettings) {
    Object.assign(this, this.inner);
  }

  static default(): Promise<Settings> {
    return newFromInvoke(this, FuncCommand.StateGetDefaultSettings);
  }

  static getAsync(): Promise<Settings> {
    return newFromInvoke(this, FuncCommand.StateGetSettings);
  }

  static onChange(cb: (payload: Settings) => void): Promise<UnSubscriber> {
    return newOnEvent(cb, this, FuncEvent.StateSettingsChanged);
  }

  static loadCustom(path: string): Promise<Settings> {
    return newFromInvoke(this, FuncCommand.StateGetSettings, { path });
  }

  /**
   * Returns the settings for the current widget (simplified version without Widget dependency)
   */
  getCurrentWidgetConfig(): ThirdPartyWidgetSettings {
    // This method is deprecated and kept for compatibility
    return {} as ThirdPartyWidgetSettings;
  }

  get fancyToolbar(): FancyToolbarSettings {
    // Note: toolbar is not implemented in this simplified version
    return {} as FancyToolbarSettings;
  }

  get magicTaskbar(): TaskbarSettings {
    return this.inner.taskbar; //zhang
  }

  /** Will store the settings on disk */
  save(): Promise<void> {
    return invoke(FuncCommand.StateWriteSettings, { settings: this.inner });
  }
}

// =================================================================================
//    From here some enums as helpers like @magic-ui/types only contains types
// =================================================================================

const FancyToolbarSide: Enum<FancyToolbarSide> = {
  Top: "Top",
  Bottom: "Bottom",
};

const TaskbarMode: Enum<TaskbarMode> = {
  FullWidth: "FullWidth",
  MinContent: "MinContent",
};

const HideMode: Enum<HideMode> = {
  Never: "Never",
  Always: "Always",
  OnOverlap: "OnOverlap",
};

const TaskbarSide: Enum<TaskbarSide> = {
  Left: "Left",
  Right: "Right",
  Top: "Top",
  Bottom: "Bottom",
};

const UpdateChannel: Enum<UpdateChannel> = {
  Release: "Release",
  Beta: "Beta",
  Nightly: "Nightly",
};

export { FancyToolbarSide, HideMode, TaskbarMode, TaskbarSide, UpdateChannel };

export * from "./settings_by_monitor.ts";
