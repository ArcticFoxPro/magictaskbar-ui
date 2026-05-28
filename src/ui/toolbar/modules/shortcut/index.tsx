import { invoke } from "@magic-ui/lib";
import { FuncCommand } from "@magic-ui/lib";
import { useEffect, useState, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { MouseEvent } from 'preact/compat';

import "./styles.css";

// 快捷键常量定义
const SHORTCUT_ICONS: Record<string, string> = {
  "SC_IDS_MA_YOYO_MEETING": "SC_IDS_MA_YOYO_MEETING.svg",
  "SC_SCREEN_CAST": "SC_SCREEN_CAST.svg", 
  "SC_HONOR_SHARE": "SC_HONOR_SHARE.svg",
  "SC_SCREEN_SHOT": "SC_SCREEN_SHOT.svg",
  "SC_SCREEN_RECORD": "SC_SCREEN_RECORD.svg",
  "SC_NOTEPAD": "SC_NOTEPAD.svg",
  "SC_YOYO_ASSISTANT": "yoyo.svg",
  "SC_AI_SUBTITILE": "SC_AI_SUBTITILE.svg",
  "SC_DISPLAY_EYE_COMFORT": "SC_DISPLAY_EYE_COMFORT.svg",
  "SC_VOICE_INPUT": "SC_VOICE_INPUT.svg",
  "SC_TOUCH_SCREEN": "SC_TOUCH_SCREEN.svg",
  "SC_SMART_SEARCH": "SC_SMART_SEARCH.svg",
  "SC_MAGIC_TEXT": "SC_MAGIC_TEXT.svg",
  "SC_IDS_DARK_THEME": "SC_IDS_DARK_THEME.svg",
  "SC_LID_POWER_ON": "SC_LID_POWER_ON.svg",
  "SC_CALCULATOR": "SC_CALCULATOR.svg",
  "SC_TOF": "SC_TOF.svg",
  "SC_MAGICTEXT_AI_TOOLBAR": "SC_MAGICTEXT_AI_TOOLBAR.svg",
  "SC_MAGICTEXT_OCR": "SC_MAGICTEXT_OCR.svg",
  "SC_MAGICANIMATION": "SC_MAGICANIMATION.svg",
  "SC_AI_DESKTOP": "SC_AI_DESKTOP.svg",
  "SC_SMART_AUDIO": "SC_SMART_AUDIO.svg",
  "SC_SYSTEM_OPTIMIZE": "SC_SYSTEM_OPTIMIZE.svg",
  "SC_EYE_PROTECTION_MODE": "SC_EYE_PROTECTION_MODE.svg",
  "SC_MAGIC_TOUCH_PAD_CONTROL_CENTER": "SC_MAGIC_TOUCH_PAD_CONTROL_CENTER.svg",
  "SC_GAME_ASSISTANT": "SC_GAME_ASSISTANT.svg"
};

export function ShortcutModule() {
    const [shortcuts, setShortcuts] = useState<string[]>([]);
    const [contextMenu, setContextMenu] = useState<{x: number, y: number, shortcutId: string} | null>(null);
    const contextMenuRef = useRef<HTMLDivElement | null>(null);
    const [animatingShortcutId, setAnimatingShortcutId] = useState<string | null>(null);



  useEffect(() => {
    // 初始化时从注册表读取快捷键
    const loadShortcutsFromRegistry = async () => {
      try {
        const savedShortcuts = await invoke(FuncCommand.ShortcutGetKeys, undefined);
        if (savedShortcuts && Array.isArray(savedShortcuts)) {
          setShortcuts(savedShortcuts.filter(id => SHORTCUT_ICONS[id]));
        }
      } catch (err) {
        console.warn('[Shortcut] Failed to load shortcuts from registry:', err);
      }
    };

    loadShortcutsFromRegistry();

    const unlisten = listen('shortcut-message', (event: any) => {
      const shortcutId = event.payload;
  
      if (!SHORTCUT_ICONS[shortcutId]) return;
      setShortcuts(prev => {
        // 去重（已经存在就不再加）
        if (prev.includes(shortcutId)) {
          return prev;
        }

        // 新的放左边
        const newShortcuts = [shortcutId, ...prev];

        // 上报新增 shortcut
        (invoke as any)(FuncCommand.ReportShortcutOperation, { 
          operation: '1', toolName: shortcutId
        });

        // 触发动画
        setAnimatingShortcutId(shortcutId);
        // 跨两帧移除动画态，触发 transition
        requestAnimationFrame(() => {
            requestAnimationFrame(() => {
            setAnimatingShortcutId(current =>
                current === shortcutId ? null : current
            );
            });
        });

        // 保存到注册表
        invoke(FuncCommand.ShortcutSaveKeys, { shortcutIds: newShortcuts }).catch((err: any) => {
          console.warn('[Shortcut] Failed to save shortcuts to registry:', err);
        });
        
        return newShortcuts;
      });
    });

    }, []);
  

  const onClick = async (shortcutId: string) => {
    try {
      await (invoke as any)(FuncCommand.ReportClickComponent, { 
        content: `快捷键-${shortcutId}`
      });
  
      await (invoke as any)(
        FuncCommand.SendShortcutMessage,
        { shortcutId }
      );
    } catch (err) {
      console.warn(`[Shortcut] trigger ${shortcutId} failed`, err);
    }
  };

  const handleContextMenu = (e: MouseEvent<HTMLDivElement>, shortcutId: string) => {
    e.preventDefault();
    e.stopPropagation();
    
    setContextMenu({
      x: e.clientX,
      y: e.clientY,
      shortcutId: shortcutId
    });
  };

  const handleRemoveShortcut = (shortcutId: string) => {
    //ignoreNextShortcutRef.current = shortcutId;
    setShortcuts(prev => {
      const newShortcuts = prev.filter(id => id !== shortcutId);
      
      // 上报移除 shortcut
      (invoke as any)(FuncCommand.ReportShortcutOperation, { 
        operation: '0', toolName: shortcutId
      });
      
      // 保存到注册表
      invoke(FuncCommand.ShortcutSaveKeys, { shortcutIds: newShortcuts }).catch((err: any) => {
        console.warn('[Shortcut] Failed to save shortcuts to registry:', err);
      });
      
      return newShortcuts;
    });
    setContextMenu(null);
  };
  

  if (shortcuts.length === 0) {
    return null;
  }
  
  return (
    <>
      {shortcuts.map(shortcutId => {
        const iconPath = `/static/icons/${SHORTCUT_ICONS[shortcutId]}`;
  
        return (
          <div
            key={shortcutId}
            className={`taskbar-item taskbar-module shortcut-module ${
                animatingShortcutId === shortcutId ? 'shortcut-animate-in' : ''
            }`}
            onClick={() => onClick(shortcutId)}
            onContextMenu={(e) => handleContextMenu(e, shortcutId)}
          >
            <img
              className="shortcut-icon"
              src={iconPath}
              alt={shortcutId}
            />
          </div>
        );
      })}
      
      {contextMenu && (
        <div
            className="shortcut-context-mask"
            style={{
            position: 'fixed',
            inset: 0,
            zIndex: 999
            }}
            onClick={() => setContextMenu(null)}
        >
        <div
        className="shortcut-context-menu"
        style={{
            left: contextMenu.x,
            top: contextMenu.y,
        }}
        onClick={(e) => e.stopPropagation()}
        >
        <div className="context-menu-outer">
            <div
            className="context-menu-inner"
            onClick={() => handleRemoveShortcut(contextMenu.shortcutId)}
            >
            从顶部Bar移除
            </div>
        </div>
        </div>

        </div>
        )}

    </>
  );
  
  
}

export default ShortcutModule;