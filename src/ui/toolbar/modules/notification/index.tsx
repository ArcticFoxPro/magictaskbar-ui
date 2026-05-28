import { useState, useEffect, useRef } from "preact/hooks";
import { FuncCommand, FuncEvent, invoke, subscribe } from "@magic-ui/lib";

import "./styles.css";

const ICON_NO_NOTIFY = "/static/icons/nonotify.svg";
const ICON_HAVE_NOTIFY = "/static/icons/havenotify.svg";

export function NotificationModule() {
  const [hasNotification, setHasNotification] = useState(false);
  const [isSelected, setIsSelected] = useState(false);
  const pollingRef = useRef<number | null>(null);
  const timeoutRef = useRef<number | null>(null);
  const disposedRef = useRef(false);

  useEffect(() => {
    let unsubscribe: (() => void) | null = null;
    let disposed = false;
    
    subscribe(FuncEvent.NotificationIconChanged, (e) => {
      try {
        const payload = (e as any).payload;
        console.log("[Notification] Icon changed:", payload);
        setHasNotification(!!payload);
      } catch (err) {
        console.error("[Notification] Failed to handle icon change:", err);
      }
    }).then((fn) => {
      if (!disposed) {
        unsubscribe = fn;
      } else {
        fn();
      }
    });

    return () => {
      disposed = true;
      if (unsubscribe) {
        unsubscribe();
      }
    };
  }, []);

  // 清理所有定时器
  const cleanup = () => {
    if (timeoutRef.current) {
      clearTimeout(timeoutRef.current);
      timeoutRef.current = null;
    }
    if (pollingRef.current) {
      clearInterval(pollingRef.current);
      pollingRef.current = null;
    }
  };

  // 检查消息中心窗口是否可见
  const checkVisibility = async () => {
    if (disposedRef.current) return;
    
    try {
      const visible = await invoke(FuncCommand.MessageCenterIsVisible, undefined);
      if (disposedRef.current) return;
      
      if (!visible) {
        // 窗口不可见，取消选中态并停止轮询
        setIsSelected(false);
        cleanup();
      }
    } catch (e) {
      console.warn("[Notification] Check visibility failed:", e);
    }
  };

  // 开始轮询检查窗口状态
  const startPolling = () => {
    // 如果已经在轮询，不要重复启动
    if (pollingRef.current || timeoutRef.current) return;
    
    // 延迟 300ms 后开始轮询，等待窗口弹出
    timeoutRef.current = window.setTimeout(() => {
      timeoutRef.current = null;
      if (disposedRef.current) return;
      
      // 每 500ms 检查一次窗口状态
      pollingRef.current = window.setInterval(checkVisibility, 500);
    }, 300);
  };

  // 组件卸载时清理
  useEffect(() => {
    disposedRef.current = false;
    
    return () => {
      disposedRef.current = true;
      cleanup();
    };
  }, []);

  const handleClick = async () => {
    try {
      setHasNotification(false);
      await invoke(FuncCommand.HonorMessageCenterUiOpen, undefined as any);
      
      // 设置选中态并开始轮询
      setIsSelected(true);
      startPolling();
    } catch (error) {
      console.error("Failed to open notifications (Win+N):", error);
    }
  };

  return (
    <div className={`taskbar-item taskbar-module notification-module${isSelected ? ' selected' : ''}`} onClick={handleClick}>
      <img
        className="notification-icon"
        src={hasNotification ? ICON_HAVE_NOTIFY : ICON_NO_NOTIFY}
        alt="notification"
      />
    </div>
  );
}
