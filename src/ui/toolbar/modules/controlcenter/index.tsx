import { useState, useEffect, useRef } from "react";
import { invoke, FuncCommand } from "@magic-ui/lib";
import "./styles.css";

export function ControlCenterModule() {
  const [isSelected, setIsSelected] = useState(false);
  const pollingRef = useRef<number | null>(null);
  const timeoutRef = useRef<number | null>(null);
  const disposedRef = useRef(false);

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

  // 检查控制中心窗口是否可见
  const checkVisibility = async () => {
    if (disposedRef.current) return;
    
    try {
      const visible = await invoke(FuncCommand.ControlCenterIsVisible, undefined);
      if (disposedRef.current) return;
      
      if (!visible) {
        // 窗口不可见，取消选中态并停止轮询
        setIsSelected(false);
        cleanup();
      }
    } catch (e) {
      console.warn("[ControlCenter] Check visibility failed:", e);
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

  const onClick = async () => {
    try {
      setIsSelected(true);
      // 点击控制中心时提前唤醒独显，确保后续操作不冻屏
      invoke(FuncCommand.GpuWakeAsync, undefined).catch(() => {});
      await invoke(FuncCommand.ControlCenterPostTrayClick, undefined);
      // 上报点击事件
      await (invoke as any)(FuncCommand.ReportClickComponent, { content: "控制中心" })
        .catch((err: any) => console.error('report click failed', err));
      // 开始轮询检查窗口状态
      startPolling();
    } catch (e) {
      console.warn("control center post tray click failed", e);
    }
  };

  return (
    <div className={`taskbar-item taskbar-module controlcenter-module${isSelected ? ' selected' : ''}`} onClick={onClick}>
      <img className="cc-icon" src="/static/icons/controlCenter.svg" alt="Control Center" />
    </div>
  );
}

export default ControlCenterModule;
