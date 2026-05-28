import { invoke } from "@magic-ui/lib";
import { FuncCommand } from "@magic-ui/lib";
import "./styles.css";

export function ScreenshotModule() {
  const onClick = async () => {
    try {
      await (invoke as any)(FuncCommand.ReportClickComponent, { content: "截图" });
      // 统一走 invoke（与 yoyo 模块一致），后端已固定传 \p
      await (invoke as any)("screenshot_launch", undefined);
    } catch (err) {
      console.warn("[Screenshot] launch failed", err);
    }
  };

  return (
    <div className="taskbar-item taskbar-module screenshot-module" onClick={onClick}>
      <img className="screenshot-icon" src="/static/icons/screenshot.svg" alt="截图" />
    </div>
  );
}

export default ScreenshotModule;
