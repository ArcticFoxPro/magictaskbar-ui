import { configureStore } from "@reduxjs/toolkit";
import { RootSlice } from "./app";

export const store = configureStore({
  reducer: {
    root: RootSlice.reducer,
  },
  middleware: (getDefaultMiddleware) =>
    getDefaultMiddleware({ serializableCheck: false }),
});

export async function registerStoreEvents() {
  // Reserved for future store-related event subscriptions.
  // Note: Layered hitbox cursor handling is managed in `@shared/layered`.
  try {
    // No-op subscription block to keep structure consistent; safe to extend later.
  } catch {}
}
