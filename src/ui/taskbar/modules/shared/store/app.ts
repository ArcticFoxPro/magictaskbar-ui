import { createSlice } from "@reduxjs/toolkit";
import { TaskbarItemType } from "@magic-ui/lib";
import { StateBuilder } from "@shared/StateBuilder";

import { PinnedTaskbarItem, RootState, SwItem, TemporalTaskbarItem } from "./domain";

const initialState: RootState = {
  devTools: false,
  focusedApp: null,
};

export const RootSlice = createSlice({
  name: "root",
  initialState,
  reducers: {
    ...StateBuilder.reducersFor(initialState),
  },
});

export const RootActions = RootSlice.actions;
export const Selectors = StateBuilder.compositeSelector(initialState);

export const isPinnedApp = (item: SwItem): item is PinnedTaskbarItem => {
  return item.type === TaskbarItemType.Pinned;
};

export const isTemporalApp = (item: SwItem): item is TemporalTaskbarItem => {
  return item.type === TaskbarItemType.Temporal;
};
