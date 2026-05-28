import { signal } from "@preact/signals";
import { TaskbarItems, TaskbarItemType } from "@magic-ui/lib";
import { TaskbarItem } from "@magic-ui/lib/types";
import { invoke } from "@tauri-apps/api/core";
import { $current_monitor } from "./system";


import { SeparatorTaskbarItem } from "../store/domain";

const decodeBase64Url = (value: string) => {
  const normalized = value.replace(/_/g, '/').replace(/-/g, '+');
  const padding = (4 - (normalized.length % 4)) % 4;
  return atob(normalized.padEnd(normalized.length + padding, '='));
};

// Get monitor ID: try from URL params first, then from window label
let monitorId: string | null = null;

// Method 1: From URL query parameters
const params = new URLSearchParams(window.location.search);
monitorId = params.get('monitorId');
console.log('[items.ts] monitorId from URL:', monitorId);

// Method 2: From window label (fallback, base64 encoded)
if (!monitorId) {
  const windowLabel = (window as any).__TAURI_INTERNALS__?.metadata?.label || '';
  console.log('[items.ts] raw windowLabel:', windowLabel);
  if (windowLabel) {
    try {
      const decodedLabel = decodeBase64Url(windowLabel);
      const match = decodedLabel.match(/monitorId=([^&]+)/);
      monitorId = match?.[1] ? decodeURIComponent(match[1]) : null;
      console.log('[items.ts] monitorId from label:', monitorId);
    } catch (e) {
      console.error('[items.ts] Failed to decode from label:', e);
    }
  }
}

console.log('[items.ts] final monitorId:', monitorId);

interface DockState {
  isReorderDisabled: boolean;
  items: TaskbarItem[];
}

export const HardcodedSeparator1: SeparatorTaskbarItem = {
  id: "hardcoded-separator-1",
  type: TaskbarItemType.Separator,
};

export const HardcodedSeparator2: SeparatorTaskbarItem = {
  id: "hardcoded-separator-2",
  type: TaskbarItemType.Separator,
};


function getStateFromStored(raw: TaskbarItems): DockState {
  const systemTrayItem = {
    id: "system-tray",
    type: TaskbarItemType.SystemTray,
  };
  const currentMonitor = $current_monitor.value;
  const isPrimaryMonitor = !monitorId || (
    currentMonitor?.rect.left === 0 && currentMonitor?.rect.top === 0
  );
  const centerItems = isPrimaryMonitor
    ? [...raw.inner.center, systemTrayItem]
    : [...raw.inner.center];

  // 暂时添加系统托盘，后续会根据托盘图标数量动态调整
  const items = [
    ...raw.inner.left, // 固定的应用显示在左侧区域
    HardcodedSeparator1,
    ...centerItems, // 未固定的应用显示在中间区域，系统托盘只显示在主屏
    HardcodedSeparator2,
    ...raw.inner.right,
  ];

  return {
    isReorderDisabled: raw.inner.isReorderDisabled,
    items: items,
  };
}

export function stateToStored(state: DockState): TaskbarItems {
  const index1 = state.items.indexOf(HardcodedSeparator1);
  const index2 = state.items.indexOf(HardcodedSeparator2);

  // 左侧区域保存所有位于Separator1左侧的项目（包括用户拖拽的任何类型）
  const left = state.items.slice(0, index1).filter(item =>
    item.type !== TaskbarItemType.Separator && item.type !== TaskbarItemType.SystemTray
  );

  // 中间区域保存所有位于Separator1和Separator2之间的项目
  const center = state.items.slice(index1 + 1, index2).filter(item =>
    item.type !== TaskbarItemType.Separator && item.type !== TaskbarItemType.SystemTray
  );

  // 右侧区域保存所有位于Separator2右侧的项目
  const right = state.items.slice(index2 + 1).filter(item =>
    item.type !== TaskbarItemType.Separator && item.type !== TaskbarItemType.SystemTray
  );

  return new TaskbarItems({
    isReorderDisabled: state.isReorderDisabled,
    left: left,
    center: center,
    right: right,
  });
}

// 1. 初始化为空状态，不阻塞模块加载
export const $dock_state = signal<DockState>({
  isReorderDisabled: false,
  items: []  // 空数组，taskbar会先显示空白
});

// 2. 添加加载状态标记
let isFirstLoad = true;
let isLoadingInProgress = false;
let taskbarItemsRefreshSeq = 0;
let isTaskbarItemsRefreshInProgress = false;
let pendingTaskbarItemsRefreshReason: string | null = null;
let pendingLocalTaskbarItemsSave: Promise<void> | null = null;

function countStoredItems(raw: TaskbarItems) {
  return raw.inner.left.length + raw.inner.center.length + raw.inner.right.length;
}

function describeDockState(state: DockState): string {
  const counts = state.items.reduce<Record<string, number>>((acc, item) => {
    acc[item.type] = (acc[item.type] ?? 0) + 1;
    return acc;
  }, {});
  return `total=${state.items.length}, counts=${JSON.stringify(counts)}`;
}

async function fetchDockStateFromBackend(reason: string): Promise<DockState> {
  const startedAt = performance.now();
  const rawItems = monitorId
    ? await TaskbarItems.getForMonitor(monitorId)
    : await TaskbarItems.getNonFiltered();
  const state = getStateFromStored(rawItems);
  const elapsed = performance.now() - startedAt;
  return state;
}

async function syncDockStateFromBackend(reason: string) {
  if (pendingLocalTaskbarItemsSave) {
    pendingTaskbarItemsRefreshReason = reason;
    return;
  }

  if (isTaskbarItemsRefreshInProgress) {
    pendingTaskbarItemsRefreshReason = reason;
    return;
  }

  isTaskbarItemsRefreshInProgress = true;
  const seq = ++taskbarItemsRefreshSeq;
  try {
    const previous = describeDockState($dock_state.value);
    const nextState = await fetchDockStateFromBackend(`${reason}#${seq}`);
    $dock_state.value = nextState;
  } catch (error) {
    console.error("[TaskbarItems][Frontend] sync failed:", error);
  } finally {
    isTaskbarItemsRefreshInProgress = false;
    if (pendingTaskbarItemsRefreshReason && !pendingLocalTaskbarItemsSave) {
      const queuedReason = pendingTaskbarItemsRefreshReason;
      pendingTaskbarItemsRefreshReason = null;
      void syncDockStateFromBackend(`${queuedReason}:queued`);
    }
  }
}

export function saveDockStateToBackend(state: DockState): Promise<void> {
  const storedState = stateToStored(state);
  const savePromise = storedState.save();
  pendingLocalTaskbarItemsSave = savePromise;

  return savePromise.finally(() => {
    if (pendingLocalTaskbarItemsSave !== savePromise) {
      return;
    }

    pendingLocalTaskbarItemsSave = null;
    if (pendingTaskbarItemsRefreshReason) {
      const queuedReason = pendingTaskbarItemsRefreshReason;
      pendingTaskbarItemsRefreshReason = null;
      void syncDockStateFromBackend(`${queuedReason}:after-local-save`);
    }
  });
}

// 3. 启动异步加载流程（不阻塞）
(async () => {
  try {
    console.log('[ItemsLoader] 开始异步加载图标数据...');

    // 从后端获取完整数据
    let fullState = await fetchDockStateFromBackend("initial");

    console.log('[ItemsLoader] 加载到的完整状态:', fullState);
    const allItems = fullState.items;
    console.log('[ItemsLoader] 所有图标类型:', allItems.map(i => i.type));
    // 标记正在加载
    isLoadingInProgress = true;

    // 根据图标数量动态调整延迟时间
    let delayPerItem: number;
    if (allItems.length <= 5) {
      delayPerItem = 80;  // 图标少，可以慢一点，让用户看清效果
    } else if (allItems.length <= 10) {
      delayPerItem = 50;  // 中等数量
    } else {
      delayPerItem = 30;  // 图标多，加快速度
    }

    console.log(`[ItemsLoader] 使用延迟: ${delayPerItem}ms/图标`);

    // 4. 逐个添加图标（模拟渐进式加载）
    for (let i = 0; i < allItems.length; i++) {
      // 每次添加一个图标
      $dock_state.value = {
        isReorderDisabled: fullState.isReorderDisabled,
        items: allItems.slice(0, i + 1)  // 截取前 i+1 个
      };

      // 延迟一段时间（控制显示速度）
      if (i < allItems.length - 1) {  // 最后一个不需要延迟
        await new Promise(resolve => setTimeout(resolve, delayPerItem));
      }
    }

    isFirstLoad = false;
    isLoadingInProgress = false;

  } catch (error) {
    console.error('[ItemsLoader] 加载图标失败:', error);
    isLoadingInProgress = false;
    // 失败时尝试直接加载完整状态
    try {
      let fullState = await fetchDockStateFromBackend("initial-retry");

      $dock_state.value = fullState;
      isFirstLoad = false;
    } catch (retryError) {
      console.error('[ItemsLoader] 重试加载也失败:', retryError);
    }
  }
})();

TaskbarItems.onChange(async () => {
  await syncDockStateFromBackend("event");
  return;
});


export const $dock_state_actions = {
  remove(idToRemove: string) {
    const itemToRemove = $dock_state.value.items.find(item => item.id === idToRemove);
    if (itemToRemove && itemToRemove.type === TaskbarItemType.Pinned) {
      // 调用后端的taskbar_unpin_item命令，删除Windows原生固定任务栏目录中的快捷方式
      invoke("taskbar_unpin_item", {
        umid: itemToRemove.umid,
        relaunchProgram: itemToRemove.relaunchProgram,
        originalId: itemToRemove.id // 传递原始ID给后端
      }).catch(err => {
        console.error("取消固定应用时删除快捷方式失败:", err);
      });
    }

    $dock_state.value = {
      ...$dock_state.value,
      items: $dock_state.value.items.filter((item) => item.id !== idToRemove),
    };
  },
  pinApp(id: string) {
    const currentItems = [...$dock_state.value.items];
    const itemIndex = currentItems.findIndex(item => item.id === id);

    if (itemIndex !== -1) {
      const itemToPin = currentItems[itemIndex];

      // 类型守卫：确保是Temporal类型
      if (itemToPin && itemToPin.type === TaskbarItemType.Temporal) {
        const itemData = itemToPin as any;
        // 只调用后端的taskbar_pin_item命令，不手动修改前端状态
        // 完全由后端处理固定逻辑并发送完整的状态更新
        invoke("taskbar_pin_item", {
          umid: itemData.umid,
          relaunchProgram: itemData.relaunchProgram,
          displayName: itemData.displayName,
          path: itemData.path,
          originalId: itemData.id, // 传递原始ID
          relaunchArgs: itemData.relaunchArgs,
          targetIndex: null
        }).catch(err => {
          console.error("固定应用时创建快捷方式失败:", err);
        });

        // 不再手动修改前端状态，完全依赖后端的状态更新
      }
    }
  },
  unpinApp(id: string) {
    const currentItems = [...$dock_state.value.items];
    const itemIndex = currentItems.findIndex(item => item.id === id);

    if (itemIndex !== -1 && currentItems[itemIndex]!.type === TaskbarItemType.Pinned) {
      // 获取要取消固定的项目
      const itemToUnpin = currentItems[itemIndex];

      if (itemToUnpin) {
        const itemData = itemToUnpin as any;
        // 调用后端的taskbar_unpin_item命令，完全由后端处理取消固定逻辑
        // 包括删除Windows原生快捷方式、更新内部状态和通知前端
        invoke("taskbar_unpin_item", {
          umid: itemData.umid,
          relaunchProgram: itemData.relaunchProgram,
          originalId: itemData.id // 传递原始ID
        }).catch(err => {
          console.error("取消固定应用时失败:", err);
        });
      }
    }
  },
  addStartModule() {
    if (
      !$dock_state.value.items.some((current) => current.type === TaskbarItemType.StartMenu)
    ) {
      const newItems = [...$dock_state.value.items];
      newItems.unshift({
        id: crypto.randomUUID(),
        type: TaskbarItemType.StartMenu,
      });
      $dock_state.value = { ...$dock_state.value, items: newItems };
    }
  },
};

// 初始化开始菜单图标
$dock_state_actions.addStartModule();
