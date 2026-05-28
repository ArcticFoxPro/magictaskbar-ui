import { FuncCommand, TaskbarSide } from "@magic-ui/lib";
import { SpecificIcon } from "@shared/components/Icon";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { memo, useCallback, useRef } from "react";
import { useSelector } from "react-redux";
import { useTranslation } from "react-i18next";

import { BackgroundByLayersV2 } from "@shared/components/BackgroundByLayers/infra";
import { Selectors } from "../../shared/store/app";
import { StartMenuTaskbarItem } from "../../shared/store/domain";
import { $settings } from "../../shared/state/mod";

const PREVIEW_SHOW_EVENT = "preview::show";
const PREVIEW_HIDE_EVENT = "preview::hide";

interface Props {
  item: StartMenuTaskbarItem;
}

const startMenuExes = ["SearchHost.exe", "StartMenuExperienceHost.exe"];

const calculatePlacement = (position: any) => {
  switch (position) {
    case "Top":
    case TaskbarSide.Top:
      return "bottom";
    case "Bottom":
    case TaskbarSide.Bottom:
      return "top";
    case "Left":
    case TaskbarSide.Left:
      return "right";
    case "Right":
    case TaskbarSide.Right:
      return "left";
    default:
      return "top";
  }
};

export const StartMenu = memo(({ item }: Props) => {
  const { t } = useTranslation();
  const focused = useSelector(Selectors.focusedApp);
  const itemRef = useRef<HTMLDivElement>(null);
  const hoverTimerRef = useRef<number | null>(null);

  // 获取当前背板模式
  const isWhiteBackplate = $settings.value.iconBackplateStyle === 'White';

  const isStartMenuOpen = startMenuExes.some((program) => ((focused as any)?.exe || "").endsWith(program));

  const showPreview = useCallback(() => {
    if (!itemRef.current) return;

    const rect = itemRef.current.getBoundingClientRect();
    const placement = calculatePlacement($settings.value.position);
    const dpiScale = globalThis.window.devicePixelRatio || 1;
    const screenX = globalThis.window.screenX || globalThis.window.screenLeft || 0;
    const screenY = globalThis.window.screenY || globalThis.window.screenTop || 0;

    const x = Math.round((screenX + rect.left + rect.width / 2) * dpiScale);
    const y = placement === "top"
      ? Math.round((screenY + rect.top) * dpiScale)
      : Math.round((screenY + rect.bottom) * dpiScale);

    emit(PREVIEW_SHOW_EVENT, {
      itemId: "start-menu",
      displayName: t("start_menu.name"),
      windows: [],
      position: { x, y, placement },
    });
  }, [t]);

  const hidePreview = useCallback(() => {
    emit(PREVIEW_HIDE_EVENT, {});
  }, []);

  const handleMouseEnter = useCallback(() => {
    if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
    hoverTimerRef.current = window.setTimeout(() => {
      showPreview();
    }, 300);
  }, [showPreview]);

  const handleMouseLeave = useCallback(() => {
    if (hoverTimerRef.current) {
      clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
    hidePreview();
  }, [hidePreview]);

  return (
    <div
      ref={itemRef}
      className="taskbar-item taskbar-item-start"
      onClick={() => {
        if (!isStartMenuOpen) {
          invoke(FuncCommand.SendKeys, { keys: "{win}" });
        }
      }}
      onMouseEnter={handleMouseEnter}
      onMouseLeave={handleMouseLeave}
    >
      {/* 白色背板模式：直接使用 png 图标，不添加背板 */}
      {isWhiteBackplate ? (
        <img
          className="taskbar-item-icon taskbar-item-start-icon"
          src="/static/icons/start-menu.png"
          alt="Start Menu"
          data-shape="square"
        />
      ) : (
        <>
          <BackgroundByLayersV2 />
          <SpecificIcon
            className="taskbar-item-icon taskbar-item-start-icon"
            name="@effect/taskbar::start-menu"
          />
        </>
      )}
    </div>
  );
});
