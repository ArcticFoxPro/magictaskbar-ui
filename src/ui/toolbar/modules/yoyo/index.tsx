import { invoke } from "@magic-ui/lib";
import { FuncCommand } from "@magic-ui/lib";
import "./styles.css";

export function YoyoModule() {
  const onClick = async () => {
    try {
      await (invoke as any)(FuncCommand.ReportClickComponent, { content: "YOYO" });
      await (invoke as any)("yoyo_launch_assistant", undefined);
    } catch (e) {
      console.warn("yoyo assistant launch failed", e);
    }
  };

  return (
    <div className="taskbar-item taskbar-module yoyo-module" onClick={onClick}>
      <img className="yoyo-icon" src="/static/icons/yoyo.svg" alt="HONOR AI" />
    </div>
  );
}

export default YoyoModule;
