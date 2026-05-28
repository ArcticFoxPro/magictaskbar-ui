import { FocusedApp, TaskbarItem } from "@magic-ui/lib/types";

export type HWND = number & {};

export type PinnedTaskbarItem = Extract<TaskbarItem, { type: "Pinned" }>;
export type TemporalTaskbarItem = Extract<TaskbarItem, { type: "Temporal" }>;
export type SeparatorTaskbarItem = Extract<TaskbarItem, { type: "Separator" }>;
export type StartMenuTaskbarItem = Extract<TaskbarItem, { type: "StartMenu" }>;
export type RecycleBinTaskbarItem = Extract<TaskbarItem, { type: "RecycleBin" }>;

/** @alias */
export type SwItem = TaskbarItem;

export interface RootState {
  devTools: boolean;
  // ----------------------
  focusedApp: FocusedApp | null;
}
