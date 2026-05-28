import { FuncCommand } from "@magic-ui/lib";
import { Icon } from "@shared/components/Icon";
import { cx } from "@shared/styles";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { tempDir } from "@tauri-apps/api/path";
import { Spin } from "antd";
import React, { useEffect, useReducer, useState } from "react";

import { HWND } from "../../shared/store/domain";

interface PreviewProps {
  title: string;
  hwnd: HWND;
  isFocused: boolean;
}

const TEMP_FOLDER = await tempDir();

export const UserApplicationPreview = (
  { title, hwnd, isFocused }: PreviewProps,
) => {
  const imageUrl = convertFileSrc(`${TEMP_FOLDER}${hwnd}.png`);

  const [imageSrc, setImageSrc] = useState<string | null>(imageUrl);
  const [_, forceUpdate] = useReducer((x) => x + 1, 0);

  useEffect(() => {
    const unlisten = listen(`taskbar-preview-update-${hwnd}`, () => {
      setImageSrc(imageUrl);
      forceUpdate(_);
    });
    return () => {
      unlisten.then((unlisten) => unlisten()).catch(console.error);
    };
  }, []);

  const onClose = (e: any) => {
    e.stopPropagation();
    invoke(FuncCommand.TaskbarCloseApp, { hwnd });
  };

  return (
    <div
      className="taskbar-item-preview"
      onClick={() => {
        invoke(FuncCommand.TaskbarToggleWindowState, {
          hwnd,
          wasFocused: isFocused,
        });
      }}
    >
      <div className="taskbar-item-preview-topbar">
        <div className="taskbar-item-preview-title">{title}</div>
        <div className="taskbar-item-preview-close" onClick={onClose}>
          <Icon iconName="IoClose" />
        </div>
      </div>
      <div className="taskbar-item-preview-image-container">
        {imageSrc
          ? (
            <img
              className="taskbar-item-preview-image"
              src={imageSrc + `?${new Date().getTime()}`}
              onError={() => setImageSrc(null)}
            />
          )
          : <Spin className="taskbar-item-preview-spin" />}
      </div>
    </div>
  );
};
