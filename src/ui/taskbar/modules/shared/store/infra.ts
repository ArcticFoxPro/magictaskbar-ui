import { configureStore } from "@reduxjs/toolkit";
import { FuncCommand, FuncEvent, Settings, startThemingTool, subscribe } from "@magic-ui/lib";
import { FocusedApp, TaskbarSettings } from "@magic-ui/lib/types";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { debounce } from "lodash";

import { RootActions, RootSlice } from "./app";

import i18n from "../../../i18n";

export const store = configureStore({
  reducer: RootSlice.reducer,
  middleware(getDefaultMiddleware) {
    return getDefaultMiddleware({
      serializableCheck: false,
    });
  },
});

export async function registerStoreEvents() {
  const view = getCurrentWebviewWindow();

  const onFocusChanged = debounce((app: FocusedApp) => {
    store.dispatch(RootActions.setFocusedApp(app));
  }, 200);
  await view.listen<FocusedApp>(FuncEvent.GlobalFocusChanged, (e) => {
    onFocusChanged(e.payload);
    if (e.payload.name != "MagicTaskbar") {
      onFocusChanged.flush();
    }
  });

  await Settings.onChange(loadSettingsToStore);

  await startThemingTool();
}

function loadSettingsCSS(settings: TaskbarSettings) {
  const styles = document.documentElement.style;

  styles.setProperty("--config-margin", `${settings.margin}px`);
  styles.setProperty("--config-padding", `${settings.padding}px`);

  styles.setProperty("--config-item-size", `${settings.size}px`);
  styles.setProperty("--config-item-zoom-size", `${settings.zoomSize}px`);
  styles.setProperty(
    "--config-space-between-items",
    `${settings.spaceBetweenItems}px`,
  );
}

function loadSettingsToStore(settings: Settings) {
  // 不从 settings 读取语言，保持 i18n 初始化时设置的语言
  store.dispatch(RootActions.setDevTools(settings.inner.devTools));
  loadSettingsCSS(settings.magicTaskbar);
}

export async function loadStore() {
  loadSettingsToStore(await Settings.getAsync());
}
