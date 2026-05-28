import { $system_colors } from "@shared/signals";
import { useDarkMode } from "@shared/styles";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { ConfigProvider, theme } from "antd";
import { useEffect } from "react";

import { ErrorBoundary } from "../taskbar/components/Error";
import { ErrorFallback } from "./components/Error";
import { FancyToolbar } from "./modules/main/Toolbar";

const waitForNextFrame = () =>
  new Promise<void>((resolve) => {
    requestAnimationFrame(() => resolve());
  });

async function waitForToolbarFirstLayout() {
  const fontsReady = document.fonts?.ready;
  if (fontsReady) {
    await Promise.race([
      fontsReady.catch(() => undefined),
      new Promise((resolve) => setTimeout(resolve, 300)),
    ]);
  }

  await waitForNextFrame();
  await waitForNextFrame();
}

export function App() {
  const isDarkMode = useDarkMode();

  useEffect(() => {
    let cancelled = false;

    void waitForToolbarFirstLayout().then(() => {
      if (!cancelled) {
        void getCurrentWebviewWindow().show();
      }
    });

    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <ConfigProvider
      theme={{
        token: {
          colorPrimary: isDarkMode ? $system_colors.value.accent_light : $system_colors.value.accent_dark,
        },
        components: {
          Calendar: {
            fullBg: "transparent",
            fullPanelBg: "transparent",
            itemActiveBg: "transparent",
          },
        },
        algorithm: isDarkMode ? theme.darkAlgorithm : theme.defaultAlgorithm,
      }}
    >
      <ErrorBoundary fallback={<ErrorFallback />}>
        <FancyToolbar />
      </ErrorBoundary>
    </ConfigProvider>
  );
}
