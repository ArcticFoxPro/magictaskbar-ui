import { createSlice } from "@reduxjs/toolkit";
import { StateBuilder } from "@shared/StateBuilder";

export interface RootState {
  version: number;
}

const initialState: RootState = {
  version: 0,
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
