import { useEffect, useState } from "preact/hooks";
import { cloneElement, createPortal } from "preact/compat";
import { $settings, $open_popups } from "../shared/state/mod";
import { invoke as invokeCommand } from "@tauri-apps/api/core";
import { FuncCommand } from "@magic-ui/lib";
import { $is_this_webview_focused } from "@shared/signals";
import { useSignalEffect } from "@preact/signals";
import "./ContextMenu.css";

export function ToolbarContextMenuTrigger({ children }: { children: preact.VNode<any> }) {
  const [open, setOpen] = useState(false);
  const [menuPos, setMenuPos] = useState({ x: 0, y: 0 });
  useEffect(() => {
    $open_popups.value = { ...$open_popups.value, contextMenu: open };
  }, [open]);
  useEffect(() => {
    const onDocMouseDown = (e: MouseEvent) => {
      if (!open) return;
      const target = e.target as HTMLElement | null;
      const inPopup = !!target?.closest('.ft-bar-context-menu');
      if (!inPopup) setOpen(false);
    };
    document.addEventListener('mousedown', onDocMouseDown, true);
    return () => document.removeEventListener('mousedown', onDocMouseDown, true);
  }, [open]);
  useSignalEffect(() => {
    if (!$is_this_webview_focused.value && open) {
      setOpen(false);
    }
  });

  const toggleMode = async (next: "Never" | "OnOverlap") => {
    const prevMode = $settings.value.hideMode;
    try {
      const settings: any = await invokeCommand<any>(FuncCommand.StateGetSettings);
      const newSettings = {
        ...settings,
        byWidget: {
          ...(settings?.byWidget ?? {}),
          fancyToolbar: {
            ...(settings?.byWidget?.fancyToolbar ?? {}),
            hideMode: next,
          },
        },
      };
      await invokeCommand(FuncCommand.StateWriteSettings, { settings: newSettings });
      $settings.value = { ...$settings.value, hideMode: next } as any;
    } catch (e) {
      console.error('[Toolbar] toggle hideMode failed', e);
      $settings.value = { ...$settings.value, hideMode: prevMode } as any;
    }
  };

  const rowOuterStyle = {
    width: '200px',
    height: '40px',
    display: 'flex',
    flexDirection: 'column' as const,
    padding: '4px',
    zIndex: 0,
  };

  const rowInnerStyle = {
    width: '192px',
    height: '32px',
    display: 'grid',
    alignItems: 'center',
    padding: '6px 12px',
    zIndex: 0,
    borderRadius: '8px',
    cursor: 'pointer',
    transition: 'background 120ms ease',
  } as const;

  const stopEvt = (e: any) => { try { e.preventDefault(); e.stopPropagation(); } catch {} };

  const getMenuPosition = () => {
    const menuWidth = 200;
    const menuHeight = 80;
    const toolbar = document.querySelector('.ft-bar');
    const toolbarRect = toolbar?.getBoundingClientRect();
    let x = menuPos.x;
    let y = toolbarRect ? toolbarRect.bottom + 2 : menuPos.y + 4;
    if (x + menuWidth > window.innerWidth) {
      x = window.innerWidth - menuWidth;
    }
    x = Math.max(0, x);
    y = Math.max(0, y);
    return {
      position: 'fixed' as const,
      left: `${x}px`,
      top: `${y}px`,
      zIndex: 1000,
    };
  };

  const content = (
    <div
      className="toolbar-popup ft-bar-context-menu"
      style={getMenuPosition()}
      onMouseDown={stopEvt}
      onClick={stopEvt}
      onContextMenu={stopEvt}
    >
      <div style={rowOuterStyle}>
        <div
          style={{
            ...rowInnerStyle,
            gridTemplateColumns: $settings.value.hideMode === 'Never' ? 'auto 8px 16px' : 'auto',
          }}
          className="ft-bar-context-menu-item"
          onClick={(e) => {
            e.stopPropagation();
            toggleMode("Never");
            setOpen(false);
          }}
        >
          <div className="ft-bar-context-menu-text">始终显示工具栏</div>
          {$settings.value.hideMode === 'Never' && (
            <img src="/static/icons/confirm.svg" alt="选中" width={16} height={16} draggable={false} style={{ gridColumn: 3 }} />
          )}
        </div>
      </div>
      <div style={rowOuterStyle}>
        <div
          style={{
            ...rowInnerStyle,
            gridTemplateColumns: $settings.value.hideMode === 'OnOverlap' ? 'auto 8px 16px' : 'auto',
          }}
          className="ft-bar-context-menu-item"
          onClick={(e) => {
            e.stopPropagation();
            toggleMode("OnOverlap");
            setOpen(false);
          }}
        >
          <div className="ft-bar-context-menu-text">智能自动隐藏</div>
          {$settings.value.hideMode === 'OnOverlap' && (
            <img src="/static/icons/confirm.svg" alt="选中" width={16} height={16} draggable={false} style={{ gridColumn: 3 }} />
          )}
        </div>
      </div>
    </div>
  );

  return (
    <>
      {cloneElement(children, {
        onContextMenu: (e: MouseEvent) => {
          e.preventDefault();
          e.stopPropagation();
          const target = e.target as HTMLElement | null;
          const isInToolbar = !!target?.closest('.ft-bar');
          const isOnModule = !!target?.closest('.taskbar-module, .ft-bar-left .taskbar-module, .ft-bar-right .taskbar-module');
          if (isInToolbar && !isOnModule) {
            setMenuPos({ x: e.clientX, y: e.clientY });
            setOpen(true);
          }
        },
      })}
      {open && createPortal(content, document.body)}
    </>
  );
}
