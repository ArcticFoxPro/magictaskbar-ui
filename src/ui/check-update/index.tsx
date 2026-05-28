import { useEffect } from "react";
import { applyTextScaleCompensation, getRootContainer } from "@shared";
import { removeDefaultWebviewActions } from "@shared/setup";
import { createRoot } from "react-dom/client";
import { I18nextProvider } from "react-i18next";
import { Provider } from "react-redux";
import { store } from "../toolbar/modules/shared/store/infra";
import i18n, { loadTranslations } from "../taskbar/i18n";
import { CheckUpdateModal } from "../toolbar/modules/check-update";
import { invoke, FuncCommand, Settings } from "@magic-ui/lib";
import { $check_update_version } from "../toolbar/modules/shared/state/mod";
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
  console.warn("[CheckUpdate] Failed to load language from settings");
}

function App() {
  useEffect(() => {
    // 1. 获取当前版本信息
    invoke(FuncCommand.SystemCheckUpdate, undefined).then((version: any) => {
      console.info('[CheckUpdateWindow] Fetched current version:', version);
      $check_update_version.value = version;
    }).catch((e: any) => {
      console.warn('[CheckUpdateWindow] Failed to fetch version:', e);
      $check_update_version.value = "";
    });

    // 2. 自动检查更新
    // 延迟执行，确保组件已挂载并订阅了事件
    setTimeout(() => {
      console.info('[CheckUpdateWindow] Auto-checking for updates...');
      invoke(FuncCommand.SystemSendCheckUpdateToMagicvisuals, undefined).catch((e: any) => {
        console.warn('[CheckUpdateWindow] Auto-check failed:', e);
      });
    }, 500);

    // 3. 页面加载完成后再显示窗口，解决白屏闪烁问题
    import("@tauri-apps/api/webviewWindow").then(({ getCurrentWebviewWindow }) => {
      const win = getCurrentWebviewWindow();
      // 给渲染留一点缓冲时间
      setTimeout(() => {
        win.show();
      }, 100);
    });
  }, []);

  return (
    <CheckUpdateModal isOpen onClose={() => {
        import("@tauri-apps/api/webviewWindow").then(({ getCurrentWebviewWindow }) => {
            getCurrentWebviewWindow().close();
        });
    }} />
  );
}

const container = getRootContainer();
createRoot(container).render(
  <Provider store={store}>
    <I18nextProvider i18n={i18n}>
      <App />
    </I18nextProvider>
  </Provider>,
);
