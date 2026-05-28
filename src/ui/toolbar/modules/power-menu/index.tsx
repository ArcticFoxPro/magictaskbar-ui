import { useState, useEffect, useRef, useCallback } from "react";
import { $open_popups, $check_update_modal_open, $check_update_version } from "../shared/state/mod";
import { invoke, FuncCommand } from "@magic-ui/lib";
import { SlPopup } from "@shared/components/SlPopup";
import { useTranslation } from "react-i18next";
import "./styles.css";

// 毛玻璃效果相关常量
const POPUP_CORNER_RADIUS = 12;
import lockIcon from "../../../../static/icons/lock.svg";
import sleepIcon from "../../../../static/icons/sleep.svg";
import shutdownIcon from "../../../../static/icons/shutdown.svg";
import restartIcon from "../../../../static/icons/restart.svg";
import quitIcon from "../../../../static/icons/quit.svg";
import alertIcon from "../../../../static/icons/Alert.svg";
import settingIcon from "../../../../static/icons/setting.svg";
import aboutIcon from "../../../../static/icons/About.svg";
import accountIcon from "../../../../static/icons/Account.svg";
import feedbackIcon from "../../../../static/icons/Feedback.svg";
interface MenuItemType {
  id: string;
  labelKey: string;
  icon: string;
}

const MENU_ITEMS: MenuItemType[] = [
  { id: "account", labelKey: "power_menu.account", icon: accountIcon },
  { id: "settings", labelKey: "power_menu.settings", icon: settingIcon },
  { id: "lock", labelKey: "power_menu.lock", icon: lockIcon },
  { id: "sleep", labelKey: "power_menu.sleep", icon: sleepIcon },
  { id: "shutdown", labelKey: "power_menu.shutdown", icon: shutdownIcon },
  { id: "restart", labelKey: "power_menu.restart", icon: restartIcon },
  { id: "exit", labelKey: "power_menu.exit", icon: quitIcon },
  { id: "checkupdate", labelKey: "power_menu.checkupdate", icon: alertIcon },
  { id: "feedback", labelKey: "power_menu.feedback", icon: feedbackIcon },
  { id: "about", labelKey: "power_menu.about", icon: aboutIcon },
];

export function PowerMenuModule() {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = useState(false);
  const isOpenRef = useRef(false);
  const popupGlassRef = useRef<HTMLDivElement | null>(null);
  const glassUpdateSeqRef = useRef(0);
  
  useEffect(() => {
    isOpenRef.current = isOpen;
    if (!isOpen) {
      glassUpdateSeqRef.current++;
    }
    $open_popups.value = { ...$open_popups.value, powerMenu: isOpen };
  }, [isOpen]);

  // 毛玻璃效果：显示弹窗模糊
  const showPopupGlass = useCallback(() => {
    if (!isOpenRef.current) return;
    const scheduledSeq = glassUpdateSeqRef.current;

    const el = popupGlassRef.current;
    if (!el) return;
    
    const rect = el.getBoundingClientRect();
    // 必须同时检查坐标和尺寸，确保元素已正确渲染
    if (rect.left <= -9999 || rect.top <= -9999 || rect.width <= 0 || rect.height <= 0) return;
    
    requestAnimationFrame(() => {
      if (!isOpenRef.current || scheduledSeq !== glassUpdateSeqRef.current) return;
      
      (invoke as any)('popup_glass_show', {
        id: 'power-menu-primary',
        x: Math.round(rect.left),
        y: Math.round(rect.top),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
        cornerRadius: POPUP_CORNER_RADIUS
      }).catch((e: any) => {
        console.warn('[PowerMenu] Failed to show glass effect:', e);
      });
    });
  }, []);
  
  // 毛玻璃效果：隐藏弹窗模糊
  const hidePopupGlass = useCallback(() => {
    glassUpdateSeqRef.current++;
    (invoke as any)('popup_glass_hide', { id: 'power-menu-primary' }).catch((e: any) => {
      console.warn('[PowerMenu] Failed to hide glass effect:', e);
    });
  }, []);

  // 毛玻璃效果：监听弹窗打开/关闭
  useEffect(() => {
    if (isOpen) {
      const checkAndShowGlass = () => {
        const el = popupGlassRef.current;
        if (!el) return false;
        const rect = el.getBoundingClientRect();
        // 必须同时检查坐标和尺寸，确保元素已正确渲染
        if (rect.left > -9999 && rect.top > -9999 && rect.width > 0 && rect.height > 0) {
          showPopupGlass();
          return true;
        }
        return false;
      };
      
      if (!checkAndShowGlass()) {
        requestAnimationFrame(() => {
          if (!checkAndShowGlass()) {
            requestAnimationFrame(() => {
              if (!checkAndShowGlass()) {
                setTimeout(checkAndShowGlass, 50);
              }
            });
          }
        });
      }
      
      return () => {
        hidePopupGlass();
      };
    }
    // 注意：这里不需要 else 分支调用 hidePopupGlass，因为清理函数会处理
  }, [isOpen, showPopupGlass, hidePopupGlass]);

  // 毛玻璃效果：监听弹窗尺寸变化
  useEffect(() => {
    if (!isOpen) return;
    
    const el = popupGlassRef.current;
    if (!el) return;
    
    let rafId: number | null = null;
    const resizeObserver = new ResizeObserver(() => {
      if (rafId) return;
      rafId = requestAnimationFrame(() => {
        rafId = null;
        showPopupGlass();
      });
    });
    
    resizeObserver.observe(el);
    return () => {
      if (rafId) cancelAnimationFrame(rafId);
      resizeObserver.disconnect();
    };
  }, [isOpen, showPopupGlass]);
  const handleMenuClick = async (id: string) => {
    console.log("[PowerMenu] Menu item clicked:", id);
    
    // 统一关闭弹窗函数 - 立即隐藏毛玻璃
    const closeMenu = () => {
      isOpenRef.current = false;
      hidePopupGlass();
      setIsOpen(false);
    };
    
    // 对于打开新窗口的操作，先关闭 power-menu 以立即隐藏毛玻璃
    const openWindowActions = ['account', 'checkupdate', 'settings', 'feedback', 'about'];
    if (openWindowActions.includes(id)) {
      closeMenu();
    }
    
    try {
      switch (id) {
        case "account":
          (invoke as any)(FuncCommand.ReportClickComponent, { content: "账号" });
          invoke(FuncCommand.SystemSendLoginAccountToMagicvisuals, undefined).catch((e: any) => {
            console.warn('[PowerMenu] invoke system_send_login_account_to_magicvisuals failed', e);
          });
          break;
        case "checkupdate":
          (invoke as any)(FuncCommand.ReportClickComponent, { content: "检测更新" });
          invoke(FuncCommand.SystemOpenCheckUpdateWindow, undefined).catch((e: any) => {
            console.warn('[PowerMenu] invoke system_open_check_update_window failed', e);
          });
          break;
        case "lock":
          await (invoke as any)(FuncCommand.ReportClickComponent, { content: "锁定" });
          await invoke(FuncCommand.SystemLockScreen, undefined);
          closeMenu();
          break;
        case "sleep":
          await (invoke as any)(FuncCommand.ReportClickComponent, { content: "睡眠" });
          await invoke(FuncCommand.SystemSleep, undefined);
          closeMenu();
          break;
        case "shutdown":
          await (invoke as any)(FuncCommand.ReportClickComponent, { content: "关机" });
          await invoke(FuncCommand.SystemShutdown, undefined);
          closeMenu();
          break;
        case "restart":
          await (invoke as any)(FuncCommand.ReportClickComponent, { content: "重启" });
          await invoke(FuncCommand.SystemRestart, undefined);
          closeMenu();
          break;
        case "exit":
          await (invoke as any)(FuncCommand.ReportClickComponent, { content: "退出Magic视界" });
          await invoke(FuncCommand.SystemExitToDesktop, undefined);
          closeMenu();
          break;
        case "settings":
          (invoke as any)(FuncCommand.ReportClickComponent, { content: "设置"});
          (invoke as any)("system_open_settings_window", undefined);
          break;
        case "feedback":
          (invoke as any)(FuncCommand.ReportClickComponent, { content: "建议和反馈" });
          invoke(FuncCommand.SystemOpenFeedbackWindow, undefined).catch((e: any) => {
            console.warn('[PowerMenu] invoke SystemOpenFeedbackWindow failed', e);
          });
          break;
        case "about":
          (invoke as any)(FuncCommand.ReportClickComponent, { content: "关于" });
          invoke(FuncCommand.SystemOpenAboutWindow, undefined).catch((e: any) => {
            console.warn('[PowerMenu] invoke SystemOpenAboutWindow failed', e);
          });
          break;
      }
    } catch (e) {
      console.error(`Power menu action '${id}' failed:`, e);
    }
  };

  const PopupContent = (
    <div className="power-menu-popup" ref={popupGlassRef}>
      {MENU_ITEMS.map((item) => (
        <div
          key={item.id}
          className="power-menu-item-container"
          onClick={() => handleMenuClick(item.id)}
        >
          <div className="power-menu-item">
            <img src={item.icon} alt={t(item.labelKey)} className="menu-icon" />
            <span className="menu-label">{t(item.labelKey)}</span>
          </div>
        </div>
      ))}
    </div>
  );

  // 处理弹窗打开/关闭事件 - 直接绑定毛玻璃生命周期
  const handleOpenChange = useCallback((open: boolean) => {
    if (open) {
      // 弹出 HONOR 菜单时提前唤醒独显，确保后续操作（退出/睡眠/关机等）不冻屏
      invoke(FuncCommand.GpuWakeAsync, undefined).catch(() => {});
      isOpenRef.current = true;
      setIsOpen(true);
    } else {
      // 弹窗关闭时立即隐藏毛玻璃，不等待 React 状态更新
      hidePopupGlass();
      isOpenRef.current = false;
      hidePopupGlass();
      setIsOpen(false);
    }
  }, [hidePopupGlass]);

  return (
    <>
      <SlPopup placement="top" offset={4} align="start" content={PopupContent} open={isOpen} onOpenChange={handleOpenChange}>
        <div className={`taskbar-item taskbar-module power-menu-module${isOpen ? ' selected' : ''}`}>
          <img src="/static/icons/HONOR.svg" alt="HONOR" className="honor-icon" />
        </div>
      </SlPopup>
    </>
  );
}

export default PowerMenuModule;
