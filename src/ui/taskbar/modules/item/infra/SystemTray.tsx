import { BackgroundByLayersV2 } from "@shared/components/BackgroundByLayers/infra";
import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { info as logInfo } from "@tauri-apps/plugin-log";
import { $open_popups, setTaskbarOpenPopups } from '../../shared/state/mod';
import { $current_monitor } from '../../shared/state/system';

interface SystemTrayProps {
  item: any;
}

export function SystemTray({ item }: SystemTrayProps) {
  const [isOpen, setIsOpen] = useState(false);
  const [iconSrc, setIconSrc] = useState<string>("");
  const trayRef = useRef<HTMLDivElement>(null);
  const isMountedRef = useRef(true);
  const openedByThisTaskbarRef = useRef(false);

  // Sync tray overflow state with open_popups to prevent taskbar from hiding
  useEffect(() => {
    if (isOpen) {
      setTaskbarOpenPopups({ ...$open_popups.value, trayOverflow: true }, 'tray-overflow-open');
    } else {
      const { trayOverflow, ...rest } = $open_popups.value;
      setTaskbarOpenPopups(rest, 'tray-overflow-close');
    }
  }, [isOpen]);

  // Load the tray icon
  useEffect(() => {
    const loadIcon = async () => {
      try {
        const processName = isOpen ? "opentray" : "closetray";
        const result = await invoke('get_local_icon' as any, { processName} as any);
        const base64 = typeof result === 'string' ? result : '';
        if (base64 && isMountedRef.current) {
          setIconSrc(base64);
        }
      } catch (e) {
        console.error("Failed to load tray icon:", e);
      }
    };
    loadIcon();
  }, [isOpen]);

  // Check initial visibility state on mount
  useEffect(() => {
    const checkVisibility = async () => {
      try {
        const visible = await invoke<boolean>('is_tray_overflow_visible');
        if (isMountedRef.current && (openedByThisTaskbarRef.current || !visible)) {
          if (!visible) openedByThisTaskbarRef.current = false;
          setIsOpen(visible);
        }
      } catch (e) {
        console.error('[SystemTray] Failed to check visibility:', e);
      }
    };
    checkVisibility();

    // Poll for visibility changes (backup mechanism)
    const interval = setInterval(async () => {
      try {
        const visible = await invoke<boolean>('is_tray_overflow_visible');
        if (isMountedRef.current && (openedByThisTaskbarRef.current || !visible)) {
          if (!visible) openedByThisTaskbarRef.current = false;
          setIsOpen(visible);
        }
      } catch {
        // Ignore polling errors
      }
    }, 500);

    return () => {
      clearInterval(interval);
    };
  }, []);

  const handleClick = useCallback(async (event?: any) => {
    if (!trayRef.current) return;

    try {
      // Get the position of the tray icon
      const rect = trayRef.current.getBoundingClientRect();
      const currentMonitor = $current_monitor.value;
      const dpi = currentMonitor?.dpi || globalThis.devicePixelRatio || 1;
      const cursor = await invoke<[number, number]>('get_mouse_position').catch(() => null);

      // 使用 Tauri 的 outerPosition 获取窗口物理像素位置（比 window.screenX/Y 更准确）
      const webview = getCurrentWebviewWindow();
      const pos = await webview.outerPosition();

      // pos.x/y 是物理像素，rect 是 CSS 像素，乘以 DPI 转换为物理像素
      const rectAnchorCenterX = pos.x + Math.round((rect.left + rect.width / 2) * dpi);
      const rectAnchorTopY = pos.y + Math.round(rect.top * dpi);
      const cursorAnchorCenterX = cursor ? Math.round(cursor[0]) : rectAnchorCenterX;
      const cursorAnchorTopY = cursor ? Math.round(cursor[1] - (rect.height * dpi) / 2) : rectAnchorTopY;
      const anchorCenterX = cursorAnchorCenterX;
      const anchorTopY = cursorAnchorTopY;
      const gap = 16;

      const diagnostics = {
        anchorCenterX,
        anchorTopY,
        rectAnchorCenterX,
        rectAnchorTopY,
        cursorAnchorCenterX,
        cursorAnchorTopY,
        gap,
        currentState: isOpen,
        monitorId: currentMonitor?.id ?? null,
        monitorDpi: currentMonitor?.dpi ?? null,
        webviewDpr: globalThis.devicePixelRatio || 1,
        webviewPosition: pos,
        iconRect: {
          left: rect.left,
          top: rect.top,
          width: rect.width,
          height: rect.height,
        },
        clickClient: event ? { x: event.clientX, y: event.clientY } : null,
        cursor,
      };

      // Call the native overflow function - it toggles visibility
      const result = await invoke<[boolean, boolean]>('show_native_tray_overflow', {
        anchorCenterX,
        anchorTopY,
        gap,
      });

      const [success, newVisibleState] = result;
      console.log('[SystemTray] show_native_tray_overflow result:', { success, newVisibleState });

      if (success && isMountedRef.current) {
        openedByThisTaskbarRef.current = newVisibleState;
        setIsOpen(newVisibleState);
      }
    } catch (error) {
      console.error('[SystemTray] Failed to toggle tray overflow:', error);
    }
  }, [isOpen]);

  return (
    <div
      ref={trayRef}
      className="taskbar-item system-tray-module"
      onClick={handleClick}
      onContextMenu={(e) => {
        e.preventDefault();
      }}
    >
      <BackgroundByLayersV2 prefix="item" />
      {iconSrc ? (
        <img
          className="taskbar-item-icon"
          src={iconSrc}
          data-shape="square"
          data-local="true"
          style={{
            width: '100%',
            height: '100%',
            objectFit: 'contain',
          }}
        />
      ) : null}
    </div>
  );
}
