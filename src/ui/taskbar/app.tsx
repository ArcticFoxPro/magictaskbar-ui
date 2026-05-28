import { $system_colors } from "@shared/signals";
import { useDarkMode } from "@shared/styles";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { ConfigProvider, theme } from "antd";
import { useEffect, useState } from "react";

import { ErrorBoundary } from "./components/Error";
import { Taskbar } from "./modules/bar";
import { switchIconBackplateStyle, getCurrentIconBackplateStyle } from "./modules/shared/store/backplate";
import { IconPackManager } from "@magic-ui/lib";

async function onMount() {
  const view = getCurrentWebviewWindow();
  console.log("About to show window:", view.label);
  await view.show();
  console.log("Window shown:", view.label);
}

export function App() {
  const isDarkMode = useDarkMode();
  const [currentBackplateStyle, setCurrentBackplateStyle] = useState<string>("Transparent");

  useEffect(() => {
    onMount();
    console.debug("taskbar app mounted");
    // 获取当前背板风格
    getCurrentIconBackplateStyle().then(style => {
      setCurrentBackplateStyle(style);
    });
  }, []);

  // 切换背板风格的处理函数
  const handleBackplateStyleChange = async (style: 'Transparent' | 'White') => {
    // 如果设置的风格和当前风格一致，则不执行任何操作
    if (style === currentBackplateStyle) {
      console.log("背板风格未改变，无需切换");
      return;
    }

    try {
      await switchIconBackplateStyle(style);
      setCurrentBackplateStyle(style);
      console.log(`背板风格已切换为: ${style}`);

      // 清除图标缓存，确保使用新的背板模式
      await IconPackManager.clearCachedIcons();
      console.log("Icon cache cleared successfully");

      // 触发自定义事件，通知所有FileIcon组件重新加载
      window.dispatchEvent(new CustomEvent('backplate-style-changed', { detail: { style } }));
    } catch (error) {
      console.error("切换背板风格失败:", error);
    }
  };

  return (
    <ConfigProvider
      componentSize="small"
      theme={{
        token: {
          colorPrimary: isDarkMode ? $system_colors.value.accent_light : $system_colors.value.accent_dark,
        },
        algorithm: isDarkMode ? theme.darkAlgorithm : theme.defaultAlgorithm,
      }}
    >
      <ErrorBoundary fallback={<div>Something went wrong</div>}>
        <Taskbar />
      </ErrorBoundary>
    </ConfigProvider>
  );
}