import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { FuncCommand, FuncEvent, subscribe } from '@magic-ui/lib';
import type { PhysicalMonitor } from '@magic-ui/lib/types';
import { $dock_state } from '../state/items';

// 单个显示器上的坐标信息
interface MonitorCoordinate {
  monitorName: string;  // 显示器设备名(如"\\.\DISPLAY1")
  x: number;
  y: number;
  width: number;
  'x-relative': number;  // x坐标占屏幕宽度的百分比
  'y-relative': number;  // y坐标占屏幕高度的百分比
}

// 窗口在所有显示器上的坐标
interface WindowCoordinate {
  hwnd: number;
  title: string;
  monitors: MonitorCoordinate[];  // 该窗口在各个显示器上的坐标
}

interface CoordinatesData {
  timestamp: string;
  iconCount: number;
  coordinates: WindowCoordinate[];
  activeHwnds?: number[];  // 当前活跃的窗口hwnd列表，用于后端清理已关闭的窗口
  recycleBinIcon?: {  // 回收站图标位置（用于将标题为"回收站"的窗口坐标修正为图标位置）
    monitorName: string;
    x: number;
    y: number;
    width: number;
    'x-relative': number;
    'y-relative': number;
  };
}

class DockCoordinatesTracker {
  private coordinates: WindowCoordinate[] = [];
  private shouldSaveCoordinates = false;
  private pendingUpdates: Map<number, {hwnd: number; title: string; monitors: Map<string, MonitorCoordinate>}> = new Map();
  private processingTimer: NodeJS.Timeout | null = null;
  // 保存回调，由外部设置
  private onSaveNeeded: (() => void) | null = null;

  // 设置保存回调
  setOnSaveNeeded(callback: () => void) {
    this.onSaveNeeded = callback;
  }

  addOrUpdateCoordinate(
    hwnd: number,
    title: string,
    monitorName: string,
    x: number,
    y: number,
    width: number,
    xRelative: number,
    yRelative: number
  ) {
    // 使用待处理队列，而不是立即更新
    if (!this.pendingUpdates.has(hwnd)) {
      this.pendingUpdates.set(hwnd, {
        hwnd,
        title,
        monitors: new Map()
      });
    }

    const pending = this.pendingUpdates.get(hwnd)!;
    pending.title = title;
    pending.monitors.set(monitorName, {
      monitorName,
      x,
      y,
      width,
      'x-relative': xRelative,
      'y-relative': yRelative
    });

    this.shouldSaveCoordinates = true;
    this.scheduleProcessing();
  }

  private scheduleProcessing() {
    if (this.processingTimer) {
      return;  // 已有处理任务
    }

    // 延迟 200ms 后批量处理所有待处理更新，并触发保存
    this.processingTimer = setTimeout(() => {
      this.flushPendingUpdates();
      this.processingTimer = null;
      // 处理完成后触发保存
      this.triggerSave();
    }, 200);
  }

  private flushPendingUpdates() {
    // 一次性处理所有待处理的坐标更新
    this.pendingUpdates.forEach((pending) => {
      let existingCoord = this.coordinates.find(c => c.hwnd === pending.hwnd);

      if (!existingCoord) {
        existingCoord = {
          hwnd: pending.hwnd,
          title: pending.title,
          monitors: []
        };
        this.coordinates.push(existingCoord);
      } else {
        existingCoord.title = pending.title;
      }
      
      // 合并 monitors
      pending.monitors.forEach((monitorCoord, monitorName) => {
        const existing = existingCoord!.monitors.find(m => m.monitorName === monitorName);
        if (existing) {
          Object.assign(existing, monitorCoord);
        } else {
          existingCoord!.monitors.push(monitorCoord);
        }
      });
    });

    this.pendingUpdates.clear();
  }

  // 立即刷新待处理队列（不等待延迟）
  flushPendingUpdatesNow() {
    if (this.processingTimer) {
      clearTimeout(this.processingTimer);
      this.processingTimer = null;
    }
    this.flushPendingUpdates();
  }

  // 触发保存（通过回调通知外部）
  private triggerSave() {
    if (this.shouldSaveCoordinates && this.onSaveNeeded) {
      this.onSaveNeeded();
    }
  }

  clearAllCoordinates() {
    if (this.coordinates.length > 0) {
      this.coordinates = [];
      this.shouldSaveCoordinates = true;
      this.triggerSave();
    }
  }

  getAllCoordinates(): WindowCoordinate[] {
    return [...this.coordinates];
  }

  // 标记窗口已关闭
  markWindowClosed(hwnd: number) {
    // 立即从坐标数据中删除
    const beforeCount = this.coordinates.length;
    this.coordinates = this.coordinates.filter(coord => coord.hwnd !== hwnd);
    const afterCount = this.coordinates.length;

    if (beforeCount !== afterCount) {
      this.shouldSaveCoordinates = true;
      this.triggerSave();
    }
  }


  getShouldSaveCoordinates(): boolean {
    return this.shouldSaveCoordinates;
  }

  setShouldSaveCoordinates(value: boolean) {
    this.shouldSaveCoordinates = value;
  }
}

export const dockCoordinatesTracker = new DockCoordinatesTracker();

// 立即保存坐标到磁盘（不经过防抖延迟）
export const saveCoordinatesImmediately = async () => {
  // 先立即刷新待处理队列（不等待200ms延迟）
  dockCoordinatesTracker.flushPendingUpdatesNow();
  dockCoordinatesTracker.setShouldSaveCoordinates(true);
  await saveSimpleWindowCoordinatesToDisk();
};

function debounce<T extends (...args: any[]) => any>(func: T, wait: number): (...args: Parameters<T>) => void {
  let timeout: NodeJS.Timeout;

  return function(...args: Parameters<T>) {
    clearTimeout(timeout);
    timeout = setTimeout(() => func(...args), wait);
  };
}

export const saveSimpleWindowCoordinatesToDisk = async () => {
  const shouldSave = dockCoordinatesTracker.getShouldSaveCoordinates();

  if (!shouldSave) {
    return;
  }

  // 获取当前活跃的窗口hwnd列表
  const currentItems = $dock_state.value.items;
  const activeHwnds: number[] = [];

  currentItems.forEach(item => {
    if ((item.type === 'Pinned' || item.type === 'Temporal') && item.windows) {
      item.windows.forEach(window => {
        activeHwnds.push(window.handle);
      });
    }
  });

  const coordinates = dockCoordinatesTracker.getAllCoordinates();

  // 获取回收站图标位置（如果存在）
  // 现在使用真实 hwnd，通过 title 判断是否为回收站
  let recycleBinIcon: CoordinatesData['recycleBinIcon'] | undefined;
  const recycleBinCoord = coordinates.find(c =>
    c.title === '回收站' || c.title.startsWith('回收站') || c.title === 'Recycle Bin'
  );
  if (recycleBinCoord && recycleBinCoord.monitors.length > 0) {
    const monitor = recycleBinCoord.monitors[0];
    if (monitor) {
      recycleBinIcon = {
        monitorName: monitor.monitorName,
        x: monitor.x,
        y: monitor.y,
        width: monitor.width,
        'x-relative': monitor['x-relative'],
        'y-relative': monitor['y-relative']
      };
    }
  }

  try {
    const data: CoordinatesData = {
      timestamp: new Date().toISOString(),
      iconCount: coordinates.length,
      coordinates: coordinates,
      activeHwnds: activeHwnds,  // 传递活跃窗口列表，让后端清理已关闭的数据
      recycleBinIcon: recycleBinIcon  // 传递回收站图标位置
    };

    const filename = 'dock-icons-coordinates.json';

    await invoke(FuncCommand.TaskbarSaveWindowCoordinates, {
      content: JSON.stringify(data, null, 2),
      filename: filename
    });

    dockCoordinatesTracker.setShouldSaveCoordinates(false);
  } catch (error) {
    console.error('保存任务栏图标坐标失败:', error);
  }
};

const debouncedSaveCoordinates = debounce(saveSimpleWindowCoordinatesToDisk, 200); // 减少到 200ms

export const TaskbarCoordinatesTracker = () => {
  const lastMonitorCountRef = useRef<number>(0);

  useEffect(() => {
    // 监听显示器变化事件
    const unsubscribe = subscribe(FuncEvent.SystemMonitorsChanged, (e) => {
      const monitors = e.payload as PhysicalMonitor[];
      const currentMonitorCount = monitors.length;

      // 检测到显示器数量变化,特别是减少到1个时
      if (currentMonitorCount < lastMonitorCountRef.current) {
        dockCoordinatesTracker.clearAllCoordinates();
        saveSimpleWindowCoordinatesToDisk();
      }

      lastMonitorCountRef.current = currentMonitorCount;
    });

    const checkAndSaveCoordinates = () => {
      if (document.hidden) {
        return;
      }

      const shouldSave = dockCoordinatesTracker.getShouldSaveCoordinates();
      if (shouldSave) {
        debouncedSaveCoordinates();
      }
    };

    // 设置回调：当类内部触发保存时调用
    dockCoordinatesTracker.setOnSaveNeeded(checkAndSaveCoordinates);

    // 页面可见时检查是否有积压的保存需求
    const handleVisibilityChange = () => {
      if (!document.hidden) {
        checkAndSaveCoordinates();
      }
    };
    document.addEventListener('visibilitychange', handleVisibilityChange);

    return () => {
      document.removeEventListener('visibilitychange', handleVisibilityChange);
      unsubscribe.then(unsub => unsub());
      // 组件卸载时保存最终状态
      saveSimpleWindowCoordinatesToDisk();
    };
  }, []);

  return null;
};