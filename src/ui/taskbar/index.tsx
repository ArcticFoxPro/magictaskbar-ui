import { FuncCommand, FuncEvent, subscribe } from "@magic-ui/lib";
import { getRootContainer } from "@shared";
import { declareDocumentAsLayeredHitbox } from "@shared/layered";
import { disableAnimationsOnPerformanceMode } from "@shared/performance";
import { removeDefaultWebviewActions } from "@shared/setup";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { info as logInfo } from "@tauri-apps/plugin-log";
import { createRoot } from "react-dom/client";
import { I18nextProvider } from "react-i18next";
import { Provider } from "react-redux";

import { loadStore, registerStoreEvents, store } from "./modules/shared/store/infra";
import "./modules/shared/state/mod"; // Import state module to initialize mouse tracking and visibility logic

import { App } from "./app";

import i18n, { loadTranslations } from "./i18n";

import "@shared/styles/colors.css";
import "./styles/variables.css";
import "@shared/styles/reset.css";
import "./styles/global.css";

removeDefaultWebviewActions();
logInfo("[Taskbar] 开始初始化流程");

logInfo("[Taskbar] 步骤1: 注册 LayeredHitbox 事件监听器");
await declareDocumentAsLayeredHitbox();
logInfo("[Taskbar] LayeredHitbox 注册完成");

logInfo("[Taskbar] 步骤2: 加载 Store");
await loadStore();

logInfo("[Taskbar] 步骤3: 注册 Store 事件");
await registerStoreEvents();

logInfo("[Taskbar] 步骤4: 加载翻译");
await loadTranslations();

logInfo("[Taskbar] 步骤5: 禁用动画（性能模式）");
disableAnimationsOnPerformanceMode();

logInfo("[Taskbar] 步骤6: 渲染 React 组件");
const container = getRootContainer();
createRoot(container).render(
  <Provider store={store}>
    <I18nextProvider i18n={i18n}>
      <App />
    </I18nextProvider>
  </Provider>,
);
logInfo("[Taskbar] React 组件渲染完成");

// WebView窗口位置信息
let webviewWindowPos = { x: 0, y: 0 };
let webviewWindowSize = { width: 0, height: 0 };

// 初始化WebView窗口位置
async function initWebviewWindowPos() {
  try {
    const webview = getCurrentWebviewWindow();
    const pos = await webview.outerPosition();
    const size = await webview.outerSize();
    webviewWindowPos = { x: pos.x, y: pos.y };
    webviewWindowSize = { width: size.width, height: size.height };
  } catch (err) {
    console.error('[initWebviewWindowPos] 获取WebView窗口位置失败:', err);
  }
}

// 初始化WebView窗口位置
await initWebviewWindowPos();

// 监听WebView窗口移动和大小改变
const webview = getCurrentWebviewWindow();
webview.onMoved((pos) => {
  webviewWindowPos = { x: pos.payload.x, y: pos.payload.y };
});

webview.onResized((size) => {
  webviewWindowSize = { width: size.payload.width, height: size.payload.height };
});

// 计算鼠标drop位置在左侧固定区域的目标索引
function calculateTargetIndex(screenX: number, screenY: number): number | null {
  try {
    // 将屏幕坐标转换为WebView相对坐标
    const webviewRelativeX = screenX - webviewWindowPos.x;
    const webviewRelativeY = screenY - webviewWindowPos.y;

    // 考虑DPI缩放
    const dpiScale = globalThis.devicePixelRatio || 1;
    const clientX = webviewRelativeX / dpiScale;
    const clientY = webviewRelativeY / dpiScale;

    const container = document.querySelector('.taskbar-items') as HTMLElement;
    if (!container) {
      console.warn('[calculateTargetIndex] 找不到.taskbar-items容器');
      return null;
    }

    // 获取所有拖拽容器项目
    const allItems = Array.from(container.querySelectorAll('.taskbar-item-drag-container'));
    // 查找开始菜单位置
    const startMenuElement = allItems.find(item => {
      const startMenuDiv = item.querySelector('.taskbar-item-start');
      return startMenuDiv !== null;
    });
    const startMenuIndex = startMenuElement ? allItems.indexOf(startMenuElement) : -1;

    if (allItems.length === 0) {
      console.warn('[calculateTargetIndex] 没有找到任务栏项目');
      return null;
    }

    // 找到分隔符1的位置（标记左侧区域的结束）
    const separator1Index = allItems.findIndex(item => {
      const separatorDiv = item.querySelector('.taskbar-separator');
      return separatorDiv !== null && separatorDiv.classList.contains('taskbar-separator-1');
    });

    // 提取左侧区域的项目（从0到separator1Index）
    let leftItems: Element[];
    let leftItemsStartIndex = 0;  // 左侧区域在allItems中的起始索引

    if (separator1Index > 0) {
      leftItems = allItems.slice(0, separator1Index);
      leftItemsStartIndex = 0;
    } else if (separator1Index === 0) {
      return 0;
    } else {
      // 找不到分隔符，假设所有项目都是左侧区域
      leftItems = allItems;
      leftItemsStartIndex = 0;
    }

    if (leftItems.length === 0) {
      return 0;
    }

    const containerRect = container.getBoundingClientRect();
    const isHorizontal = containerRect.width > containerRect.height;

    // 找到离drop点最近的项目
    let closestIndexInLeftItems = 0;  // 在leftItems中的索引
    let closestDistance = Infinity;

    leftItems.forEach((item, index) => {
      const itemRect = (item as HTMLElement).getBoundingClientRect();
      const itemCenterX = itemRect.left + itemRect.width / 2;
      const itemCenterY = itemRect.top + itemRect.height / 2;
      const itemWidth = itemRect.width;

      // 使用客户端坐标计算距离
      const distX = clientX - itemCenterX;
      const distY = clientY - itemCenterY;
      const distance = Math.sqrt(distX * distX + distY * distY);

      if (distance < closestDistance) {
        closestDistance = distance;
        closestIndexInLeftItems = index;
      }
    });

    // 判断是插入到项目前还是后
    const closestItemRect = (leftItems[closestIndexInLeftItems] as HTMLElement).getBoundingClientRect();

    let insertAfter = false;  // 是否插入到该项目之后
    if (isHorizontal) {
      // 水平任务栏：比较水平坐标
      const closestItemCenterX = closestItemRect.left + closestItemRect.width / 2;
      insertAfter = clientX >= closestItemCenterX;
    } else {
      // 垂直任务栏：比较垂直坐标
      const closestItemCenterY = closestItemRect.top + closestItemRect.height / 2;
      insertAfter = clientY >= closestItemCenterY;
    }

    // 计算最终的目标索引（相对于allItems）
    let targetIndex = leftItemsStartIndex + closestIndexInLeftItems + (insertAfter ? 1 : 0);
    // 智能处理开始菜单附近的拖拽
    // 如果目标索引 <= 开始菜单索引，说明用户拖到了开始菜单或其左侧
    // 应该调整为插入到开始菜单之后
    if (startMenuIndex !== -1 && targetIndex <= startMenuIndex) {
      targetIndex = startMenuIndex + 1;
    }
    return targetIndex;
  } catch (err) {
    console.error('[calculateTargetIndex] 异常:', err);
    return null;
  }
}

// 记录最后一次系统鼠标位置（来自GlobalMouseMove事件）
let lastSystemMouseX = 0;
let lastSystemMouseY = 0;

// 订阅系统的GlobalMouseMove事件以获取实时鼠标位置
subscribe(FuncEvent.GlobalMouseMove, ({ payload: [x, y] }) => {
  lastSystemMouseX = x;
  lastSystemMouseY = y;
}).catch((err) => {
  console.error('[GlobalMouseMove] 订阅失败:', err);
});

getCurrentWebviewWindow().onDragDropEvent(async (e: any) => {
  if (e.payload.type === "drop") {
    // 1. 坐标转换逻辑 (DPI 感知)
    const dpiScale = globalThis.devicePixelRatio || 1;
    // 使用 payload.position (如果可用) 或者回退到系统记录的位置
    const dropX = e.payload.position?.x ?? lastSystemMouseX;
    const dropY = e.payload.position?.y ?? lastSystemMouseY;

    const clientX = (dropX - webviewWindowPos.x) / dpiScale;
    const clientY = (dropY - webviewWindowPos.y) / dpiScale;

    // 2. 检查是否拖拽到了 right 区域（回收站功能）
    // 获取第二个分隔符的位置，right 区域在其右侧
    const separator2 = document.querySelector('.taskbar-separator-2');
    let rightRegionStartX = Infinity;
    if (separator2) {
      const rect = separator2.getBoundingClientRect();
      rightRegionStartX = rect.right;
    }
    // 判定：clientX 在 right 区域起始位置右侧即算命中回收站
    let isHitRecycleBin = clientX >= rightRegionStartX;

    if (isHitRecycleBin) {
      logInfo(`[onDragDropEvent] 命中回收站，执行删除: ${e.payload.paths.join(', ')}`);
      await invoke("system_recycle_files", { paths: e.payload.paths }).catch(err => {
        console.error("回收站删除失败:", err);
      });
      return;
    }

    // 3. 原有的固定逻辑
    let targetIndex: number | null;
    if (lastSystemMouseX === 0 && lastSystemMouseY === 0) {
      // 未能获取系统鼠标坐标，回退到默认行为（固定到末尾）
      targetIndex = null;
    } else {
      // 使用系统的鼠标位置计算目标索引
      targetIndex = calculateTargetIndex(lastSystemMouseX, lastSystemMouseY);
    }

    for (const path of e.payload.paths) {
      try {
        // 从路径中提取显示名称
        let displayName = path.split(/[\\\/]/).pop() || "Unknown";

        // 检查文件扩展名
        const extension = path.substring(path.lastIndexOf('.')).toLowerCase();
        // 只允许 .exe 和 .lnk 文件
        // .exe 是可执行程序
        // .lnk 是 Windows 快捷方式（可能指向程序或其他应用）
        if (extension !== '.lnk') {
          console.warn(`[onDragDropEvent] 不支持的文件类型: ${extension}。只能拖拽 .exe 或 .lnk 文件到任务栏`);
          continue;  // 跳过这个文件
        }

        // 去掉文件名中的 .lnk 或 .exe 后缀
        displayName = displayName.replace(/\.(lnk)$/i, '');

        // 对于拖拽文件，relaunch_program 应该与 path 相同
        // 后端会根据文件类型（.lnk 等）自动处理
        const relaunch_program = path;
        await invoke(FuncCommand.TaskbarPinItem, {
          umid: null,
          relaunchProgram: relaunch_program,
          displayName: displayName,
          path: path,
          originalId: displayName,
          relaunchArgs: null,
          targetIndex: targetIndex
        });
      } catch (error) {
        console.error(`[onDragDropEvent] 固定应用失败 ${path}:`, error);
      }
    }
  }
});
