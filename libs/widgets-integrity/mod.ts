// the console logger to capture any error on the main script

import { WebviewInformation } from "libs/widgets-integrity/_tauri";

import { wrapConsoleV2 } from "./ConsoleWrapper";

// 包装所有操作在一个异步函数中，避免顶层await
(async function initializeWidget() {
  try {
    // 设置控制台日志包装器
    wrapConsoleV2();
    // 获取当前widget ID
    const currentWidgetId = new WebviewInformation().widgetId;
    // 创建最小化的widget对象以保持兼容性
    window.__SLU_WIDGET = {
      id: currentWidgetId,
      icon: null,
      metadata: {
        displayName: currentWidgetId,
        description: "",
        author: "",
        tags: [],
        internal: {
          bundled: true,
          path: "",
        },
      },
      instances: "Single",
      settings: [],
      js: null,
      css: null,
      html: null,
    } as any;
    // 加载index.js
    const response = await fetch("./index.js");
    if (!response.ok) {
      throw new Error(`Failed to fetch index.js: ${response.status}`);
    }
    const indexJsCode = await response.text();
    const script = document.createElement("script");
    script.type = "module";
    script.textContent = indexJsCode;
    document.head.appendChild(script);
  } catch (error) {
    console.error("Error initializing widget:", error);
  }
})();
