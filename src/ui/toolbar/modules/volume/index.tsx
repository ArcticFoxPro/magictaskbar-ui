import { useEffect, useState, useRef, useCallback } from "react";
import { invoke, FuncCommand, FuncEvent, subscribe } from "@magic-ui/lib";
import { SlPopup } from "@shared/components/SlPopup";
import { useTranslation } from "react-i18next";
import { $open_popups } from "../shared/state/mod";
import "./styles.css";

// 毛玻璃效果相关常量
const POPUP_CORNER_RADIUS = 9;

// 根据音量和静音状态获取对应的图标路径
const getVolumeIconSrc = (volume: number, muted: boolean): string => {
  if (muted || volume === 0) {
    return "/static/icons/SpeakerMute.svg";
  }
  if (volume >= 67) {
    return "/static/icons/Speaker3.svg";
  }
  if (volume >= 33) {
    return "/static/icons/Speaker2.svg";
  }
  return "/static/icons/Speaker1.svg";
};

export function VolumeModule() {
  const { t } = useTranslation();
  const [popupOpen, setPopupOpen] = useState(false);
  const [volume, setVolume] = useState<number>(50);
  const [muted, setMuted] = useState<boolean>(false);
  const uiVolume = muted ? 0 : volume;
  const popupOpenRef = useRef(false);
  const popupGlassRef = useRef<HTMLDivElement | null>(null);
  const glassUpdateSeqRef = useRef(0);
  // 毛玻璃效果：显示弹窗模糊
  const showPopupGlass = useCallback(() => {
    if (!popupOpenRef.current) return;
    const scheduledSeq = glassUpdateSeqRef.current;

    const el = popupGlassRef.current;
    if (!el) return;
    
    const rect = el.getBoundingClientRect();
    // 必须同时检查坐标和尺寸，确保元素已正确渲染
    if (rect.left <= -9999 || rect.top <= -9999 || rect.width <= 0 || rect.height <= 0) return;
    
    requestAnimationFrame(() => {
      if (!popupOpenRef.current || scheduledSeq !== glassUpdateSeqRef.current) return;
      
      (invoke as any)('popup_glass_show', {
        id: 'volume-primary',
        x: Math.round(rect.left),
        y: Math.round(rect.top),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
        cornerRadius: POPUP_CORNER_RADIUS
      }).catch((e: any) => {
        console.warn('[Volume] Failed to show glass effect:', e);
      });
    });
  }, []);
  
  // 毛玻璃效果：隐藏弹窗模糊
  const hidePopupGlass = useCallback(() => {
    glassUpdateSeqRef.current++;
    (invoke as any)('popup_glass_hide', { id: 'volume-primary' }).catch((e: any) => {
      console.warn('[Volume] Failed to hide glass effect:', e);
    });
  }, []);

  useEffect(() => {
    popupOpenRef.current = popupOpen;
    if (!popupOpen) {
      glassUpdateSeqRef.current++;
    }
    $open_popups.value = { ...$open_popups.value, volumePopup: popupOpen };
  }, [popupOpen]);

  // 毛玻璃效果：监听弹窗打开（显示毛玻璃）
  useEffect(() => {
    if (!popupOpen) return;
    
    const checkAndShowGlass = () => {
      const el = popupGlassRef.current;
      if (!el) return false;
      const rect = el.getBoundingClientRect();
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
  }, [popupOpen, showPopupGlass]);

  // 毛玻璃效果：监听弹窗尺寸变化
  useEffect(() => {
    if (!popupOpen) return;
    
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
  }, [popupOpen, showPopupGlass]);

  useEffect(() => {
    let mounted = true;
    let unsub: (() => void) | null = null;
    (async () => {
      try {
        const v = await invoke(FuncCommand.SystemGetMasterVolume, undefined).catch(() => null as any);
        const m = await invoke(FuncCommand.SystemGetMasterMuted, undefined).catch(() => null as any);
        if (!mounted) return;
        if (typeof v === "number") setVolume(v as number);
        if (typeof m === "boolean") setMuted(m as boolean);
      } catch (e) {
        // silent
      }
      // subscribe to system volume changes
      try {
        subscribe(FuncEvent.SystemVolumeChanged, (e) => {
          const payload = (e as any).payload as { volume: number; muted: boolean };
          if (typeof payload?.volume === "number") setVolume(payload.volume);
          if (typeof payload?.muted === "boolean") setMuted(payload.muted);
        }).then(u => { unsub = u; }).catch(() => {});
      } catch {}
    })();
    return () => { mounted = false; if (unsub) unsub(); };
  }, []);

  const onToggleMute = async () => {
    try {
      await invoke(FuncCommand.SystemSetMasterMuted, { muted: !muted });
      setMuted(!muted);
    } catch (e) {}
  };

  const onChangeVolume = async (v: number) => {
    // When dragging up from 0 while muted, update local mute state immediately to avoid UI flicker.
    if (muted && v > 0) setMuted(false);
    if (!muted && v === 0) setMuted(true);
    setVolume(v);
    try {
      // Dragging the slider up from 0 should unmute first, otherwise system events may still report muted=true.
      if (v > 0 && muted) {
        await invoke(FuncCommand.SystemSetMasterMuted, { muted: false }).catch(() => {});
      }

      await invoke(FuncCommand.SystemSetMasterVolume, { volume: v });

      // If slider set to 0, reflect muted state.
      if (v === 0) {
        await invoke(FuncCommand.SystemSetMasterMuted, { muted: true }).catch(() => {});
      }
    } catch (e) {}
  };

  // 获取轨道背景色 - 浅色模式下使用 rgba(0,0,0,0.12)
  const getTrackBackground = (value: number) => {
    const isDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
    const unselectedColor = isDark ? 'rgba(255,255,255,0.12)' : 'rgba(0,0,0,0.12)';
    return `linear-gradient(90deg, #256FFF ${value}%, ${unselectedColor} ${value}%)`;
  };

  const openMore = () => {
    popupOpenRef.current = false;
    hidePopupGlass();
    setPopupOpen(false);
    // 关闭弹窗，打开混音器窗口
    setPopupOpen(false);
    try {
      invoke(FuncCommand.SystemOpenVolumeMixer, undefined).catch(() => {});
    } catch (e) {}
  };

  const iconSrc = getVolumeIconSrc(volume, muted);

  const PopupContent = (
    <div className="volume-popup" ref={popupGlassRef}>
      <div className="volume-top-container">
        <div className="volume-title">{t('volume.title')}</div>
        <div className="volume-slider-block">
          <div className="volume-slider-container">
            <img
              src="/static/icons/Speaker0.svg"
              alt="low"
              className="volume-icon-label"
            />
            <div className="volume-slider">
              <input
                className="volume-slider-track"
                type="range"
                min={0}
                max={100}
                value={uiVolume}
                onChange={(e: React.ChangeEvent<HTMLInputElement>) => onChangeVolume(Number(e.currentTarget.value))}
                style={{
                  background: getTrackBackground(uiVolume)
                }}
              />
            </div>
            <img src="/static/icons/high_volume.svg" alt="high" className="volume-icon-label" />
          </div>
        </div>
      </div>
      <div className="volume-divider"></div>
      <div className="volume-bottom-container">
        <div className="volume-controls-inner" onClick={() => { openMore(); }}>
          <button className="volume-settings">
            {t('volume.more_settings')}
          </button>
        </div>
      </div>
    </div>
  );

  // 毛玻璃效果：监听弹窗状态变化
  const prevPopupOpenRef = useRef(false);
  
  useEffect(() => {
    // 当从 true 变为 false 时，立即隐藏毛玻璃
    if (prevPopupOpenRef.current && !popupOpen) {
      hidePopupGlass();
    }
    prevPopupOpenRef.current = popupOpen;
    
    if (!popupOpen) return;
    showPopupGlass();
  }, [popupOpen, showPopupGlass, hidePopupGlass]);

  // 处理弹窗打开事件
  const handleOpenChange = useCallback((open: boolean) => {
    popupOpenRef.current = open;
    if (open) {
      setPopupOpen(true);
      // 上报点击事件
      (invoke as any)(FuncCommand.ReportClickComponent, { content: "音量" })
        .catch((err: any) => console.error('report click failed', err));
    } else {
      hidePopupGlass();
      setPopupOpen(false);
    }
  }, [hidePopupGlass]);

  return (
    <SlPopup placement="top" offset={4} align="start" content={PopupContent} open={popupOpen} onOpenChange={handleOpenChange}>
      <div className={`taskbar-item taskbar-module volume-module${popupOpen ? ' selected' : ''}`}>
        <div className={`volume-icon-wrap ${muted ? 'muted' : ''}`}>
          <img
            src={iconSrc}
            alt={`Volume ${volume}%${muted ? " muted" : ""}`}
            className="volume-icon-img"
            width="20"
            height="20"
          />
        </div>
      </div>
    </SlPopup>
  );
}
