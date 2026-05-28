import { useEffect } from "react";
import { applyTextScaleCompensation, getRootContainer } from "@shared";
import { removeDefaultWebviewActions } from "@shared/setup";
import { createRoot } from "react-dom/client";
import { I18nextProvider } from "react-i18next";
import { Provider } from "react-redux";
import { store } from "../toolbar/modules/shared/store/infra";
import i18n, { loadTranslations } from "../taskbar/i18n";
import { Settings } from "@magic-ui/lib";
import { AboutModal } from "../toolbar/modules/about";
import "@shared/styles/colors.css";
import "../toolbar/styles/variables.css";
import "@shared/styles/reset.css";
import "../toolbar/styles/global.css";

removeDefaultWebviewActions();
await applyTextScaleCompensation();
await loadTranslations();

try {
  const settings = await Settings.getAsync();
  const lang = settings.inner.language || "en";
  await i18n.changeLanguage(lang);
} catch (e) {
  console.warn("[About] Failed to load language from settings");
}

function App() {
  useEffect(() => {
    console.log('[About] Component mounted, will show window in 100ms');
    // 页面加载完成后再显示窗口，解决白屏闪烁问题
    import("@tauri-apps/api/webviewWindow").then(({ getCurrentWebviewWindow }) => {
      const win = getCurrentWebviewWindow();
      console.log('[About] Got webview window, showing...');
      // 给渲染留一点缓冲时间
      setTimeout(() => {
        win.show();
        console.log('[About] Window shown');
      }, 100);
    }).catch((e) => {
      console.error('[About] Failed to show window:', e);
    });
  }, []);

  return <AboutModal />;
}

const container = getRootContainer();
createRoot(container).render(
  <Provider store={store}>
    <I18nextProvider i18n={i18n}>
      <App />
    </I18nextProvider>
  </Provider>,
);
