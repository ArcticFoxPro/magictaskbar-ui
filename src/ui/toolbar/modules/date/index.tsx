import { FuncCommand, invoke } from "@magic-ui/lib";
import { useSyncClockInterval } from "@shared/hooks";
import moment from "moment";
import "moment/locale/zh-cn";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import "./styles.css";

const DATE_FORMAT = "M月D日 ddd";
const TIME_FORMAT = "HH:mm";

export function DateModule() {
  const {
    i18n: { language },
  } = useTranslation();

  const locale = useMemo(() => language, [language]);

  // 根据语言选择日期格式
  const dateFormat = useMemo(() => {
    return language === 'zh-CN' ? 'M月D日 ddd' : 'ddd, M/D';
  }, [language]);

  const [dateOnly, setDateOnly] = useState(
    moment().locale(locale).format(dateFormat),
  );
  const [timeOnly, setTimeOnly] = useState(
    moment().locale(locale).format(TIME_FORMAT),
  );
  const [isSelected, setIsSelected] = useState(false);
  const pollingRef = useRef<number | null>(null);
  const timeoutRef = useRef<number | null>(null);
  const disposedRef = useRef(false);

  useEffect(() => {
    setDateOnly(moment().locale(locale).format(dateFormat));
    setTimeOnly(moment().locale(locale).format(TIME_FORMAT));
  }, [locale, dateFormat]);

  useSyncClockInterval(
    () => {
      setDateOnly(moment().locale(locale).format(dateFormat));
      setTimeOnly(moment().locale(locale).format(TIME_FORMAT));
    },
    "seconds",
    [locale, dateFormat],
  );

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

  // 检查日历窗口是否可见
  const checkVisibility = async () => {
    if (disposedRef.current) return;
    
    try {
      const visible = await invoke(FuncCommand.CalendarIsVisible, undefined);
      if (disposedRef.current) return;
      
      if (!visible) {
        // 窗口不可见，取消选中态并停止轮询
        setIsSelected(false);
        cleanup();
      }
    } catch (e) {
      console.warn("[Date] Check visibility failed:", e);
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
      await invoke(FuncCommand.ReportClickComponent, { content: "日期" });

      // Backend guarantees: check exe exists -> run; else -> Win+N
      await invoke(FuncCommand.HonorCalendarWidgetOpen, undefined);
      
      // 设置选中态并开始轮询
      setIsSelected(true);
      startPolling();
    } catch (error) {
      console.error("Failed to launch calendar widget:", error);
    }
  };

  return (
    <div className={`taskbar-item taskbar-module date-module${isSelected ? ' selected' : ''}`} onClick={handleClick}>
      <div className="date-container">
        <div className="date-content horizontal">
          <div className="date-line date-only">{dateOnly}</div>
          <div className="date-line time-only">{timeOnly}</div>
        </div>
      </div>
    </div>
  );
}
