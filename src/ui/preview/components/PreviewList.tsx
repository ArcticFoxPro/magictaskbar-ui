import { memo } from "react";

interface PreviewWindowInfo {
  handle: number;
  title: string;
  iconPngBase64?: string;
  isFocused: boolean;
}

interface Props {
  windows: PreviewWindowInfo[];
  appIconBase64?: string;
  appIconSrc?: string;
  onWindowClick: (handle: number, isFocused: boolean) => void;
  onCloseWindow: (handle: number) => void;
}

export const PreviewList = memo(function PreviewList({ windows, appIconBase64, appIconSrc, onWindowClick, onCloseWindow }: Props) {
  if (windows.length === 0) {
    return <div className="preview-empty">No windows</div>;
  }

  return (
    <div className="preview-window-list">
      {windows.map((window) => (
        <div
          key={window.handle}
          className={`preview-window-item ${window.isFocused ? "focused" : ""}`}
          onClick={() => onWindowClick(window.handle, window.isFocused)}
        >
          {(() => {
            // 优先使用 app 级图标（与任务栏一致），再降级到窗口自身图标
            if (appIconSrc) {
              return <img className="preview-window-icon" src={appIconSrc} alt="" />;
            }
            const iconBase64 = appIconBase64 || window.iconPngBase64;
            if (iconBase64) {
              const src = iconBase64.startsWith('data:') ? iconBase64 : `data:image/png;base64,${iconBase64}`;
              return <img className="preview-window-icon" src={src} alt="" />;
            }
            return null;
          })()}
          <span className="preview-window-title">
            {window.title}
          </span>
          <button
            className="preview-window-close"
            onClick={(e) => {
              e.stopPropagation();
              onCloseWindow(window.handle);
            }}
          >
            <span>&times;</span>
          </button>
        </div>
      ))}
    </div>
  );
});
