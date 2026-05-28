import { FuncCommand, FuncEvent } from "@magic-ui/lib";
import { window as TauriWindow } from "@magic-ui/lib/tauri";
import { invoke } from "@tauri-apps/api/core";
import { info as logInfo } from "@tauri-apps/plugin-log";

declare const LAYERED_VERBOSE: boolean;
declare function log(message?: any, data?: any): void;

class LayeredHitbox {
  private _isIgnoringCursorEvents: boolean = true;
  public firstClick: boolean = true;
  public isLayeredEnabled: boolean = true;

  get isIgnoringCursorEvents(): boolean {
    return this._isIgnoringCursorEvents;
  }

  set isIgnoringCursorEvents(value: boolean) {
    if (value == false) {
      this.firstClick = true;
    }
    this._isIgnoringCursorEvents = value;
  }
}

export async function declareDocumentAsLayeredHitbox(
  shouldAllowMouseEvent: (element: Element) => boolean = (element) => element != document.body,
): Promise<void> {
  const webview = TauriWindow.getCurrentWindow();
  const { x, y } = await webview.outerPosition();
  const { width, height } = await webview.outerSize();

  const webviewRect = { x, y, width, height };

  logInfo(`[LayeredHitbox] 开始初始化 window=${webview.label}, pos=(${x},${y}), size=(${width}x${height})`);

  await webview.setIgnoreCursorEvents(true);
  logInfo("[LayeredHitbox] 初始状态设置为 ignore_cursor_events = true");

  const data = new LayeredHitbox();
  const layeredVerbose =
    typeof globalThis !== "undefined" &&
    typeof (globalThis as typeof globalThis & { LAYERED_VERBOSE?: unknown }).LAYERED_VERBOSE === "boolean" &&
    (globalThis as typeof globalThis & { LAYERED_VERBOSE: boolean }).LAYERED_VERBOSE;

  webview.onMoved((e) => {
    webviewRect.x = e.payload.x;
    webviewRect.y = e.payload.y;
  });

  webview.onResized((e) => {
    webviewRect.width = e.payload.width;
    webviewRect.height = e.payload.height;
  });

  webview.listen<boolean>(FuncEvent.HandleLayeredHitboxes, (event) => {
    data.isLayeredEnabled = event.payload;
    logInfo(`[LayeredHitbox] HandleLayeredHitboxes 事件 enabled=${event.payload}`);
  });

  let outsideWhileIgnoringCount = 0; // ignore=true 时连续判定鼠标在窗口外的次数
  let isRefreshingRect = false; // 防止并发刷新

  const describeElement = (element: Element | null): string => {
    if (!element) return "null";

    const htmlElement = element as HTMLElement;
    const id = htmlElement.id ? `#${htmlElement.id}` : "";
    const className = typeof htmlElement.className === "string" && htmlElement.className.length > 0
      ? `.${htmlElement.className.trim().replace(/\s+/g, ".")}`
      : "";

    return `${element.tagName.toLowerCase()}${id}${className}`;
  };

  let appliedIgnoreCursorEvents = true;
  let desiredIgnoreCursorEvents = true;
  let isApplyingIgnoreCursorEvents = false;
  let ignoreCursorRequestId = 0;
  let lastHitboxSnapshot = "";

  const logHitboxSnapshot = (
    source: string,
    reason: string,
    details: {
      mouseX?: number;
      mouseY?: number;
      adjustedX?: number;
      adjustedY?: number;
      element?: string;
      shouldAllow?: boolean;
      nextIgnore?: boolean;
    } = {},
  ) => {
    const snapshot = [
      reason,
      `mouse=(${details.mouseX ?? "-"},${details.mouseY ?? "-"})`,
      `client=(${details.adjustedX?.toFixed(0) ?? "-"},${details.adjustedY?.toFixed(0) ?? "-"})`,
      `rect=(${webviewRect.x},${webviewRect.y},${webviewRect.width}x${webviewRect.height})`,
      `dpr=${globalThis.devicePixelRatio}`,
      `element=${details.element ?? "-"}`,
      `allow=${details.shouldAllow ?? "-"}`,
      `desiredIgnore=${desiredIgnoreCursorEvents}`,
      `appliedIgnore=${appliedIgnoreCursorEvents}`,
      `jsIgnore=${data.isIgnoringCursorEvents}`,
      `nextIgnore=${details.nextIgnore ?? "-"}`,
    ].join(", ");

    if (source === "GlobalMouseMove" && snapshot === lastHitboxSnapshot) return;
    lastHitboxSnapshot = snapshot;
    logInfo(`[LayeredHitbox][Diag] [${source}] ${snapshot}`);
  };

  const flushIgnoreCursorEvents = async (source: string) => {
    if (isApplyingIgnoreCursorEvents) return;

    isApplyingIgnoreCursorEvents = true;
    try {
      while (appliedIgnoreCursorEvents !== desiredIgnoreCursorEvents) {
        const nextIgnore = desiredIgnoreCursorEvents;
        const requestId = ++ignoreCursorRequestId;
        const startedAt = performance.now();
        await webview.setIgnoreCursorEvents(nextIgnore);
        const elapsed = performance.now() - startedAt;
        appliedIgnoreCursorEvents = nextIgnore;
        if (layeredVerbose || elapsed > 20) {
          logInfo(
            `[LayeredHitbox][Apply] [${source}] id=${requestId}, ignore=${nextIgnore}, elapsed=${elapsed.toFixed(1)}ms, desired=${desiredIgnoreCursorEvents}`,
          );
        }
      }
    } catch (e) {
      logInfo(`[LayeredHitbox] [${source}] setIgnoreCursorEvents(${desiredIgnoreCursorEvents}) 失败: ${e}`);
    } finally {
      isApplyingIgnoreCursorEvents = false;
      if (appliedIgnoreCursorEvents !== desiredIgnoreCursorEvents) {
        logInfo(
          `[LayeredHitbox][Apply] [${source}] queued desired=${desiredIgnoreCursorEvents}, applied=${appliedIgnoreCursorEvents}`,
        );
        void flushIgnoreCursorEvents(`${source}:queued`);
      }
    }
  };

  const setIgnoreCursorEvents = (ignore: boolean, source: string) => {
    data.isIgnoringCursorEvents = ignore;
    desiredIgnoreCursorEvents = ignore;
    void flushIgnoreCursorEvents(source);

    if (!ignore) {
      setTimeout(() => {
        if (desiredIgnoreCursorEvents === false && appliedIgnoreCursorEvents !== false) {
          logHitboxSnapshot(source, "stuck: allow=true but appliedIgnore still true after 50ms");
        }
      }, 50);
    }
  };

  // 异步刷新窗口位置，修复 onMoved 事件丢失或不准导致的 rect 失准
  const refreshWebviewRect = async () => {
    if (isRefreshingRect) return;
    isRefreshingRect = true;
    try {
      const { x, y } = await webview.outerPosition();
      const { width, height } = await webview.outerSize();
      webviewRect.x = x;
      webviewRect.y = y;
      webviewRect.width = width;
      webviewRect.height = height;
    } catch (e) {
      logInfo(`[LayeredHitbox] rect 刷新失败: ${e}`);
    } finally {
      isRefreshingRect = false;
    }
  };

  const evaluateMousePosition = (mouseX: number, mouseY: number, source: string) => {
    if (!data.isLayeredEnabled) {
      if (layeredVerbose) log(`${source} ignored: layered disabled`);
      return;
    }

    const {
      x: windowX,
      y: windowY,
      width: windowWidth,
      height: windowHeight,
    } = webviewRect;

    const isHoverWindow = mouseX >= windowX &&
      mouseX <= windowX + windowWidth &&
      mouseY >= windowY &&
      mouseY <= windowY + windowHeight;

    if (!isHoverWindow) {
      if (!data.isIgnoringCursorEvents) {
        setIgnoreCursorEvents(true, source);
        logHitboxSnapshot(source, "outside-window:restore-ignore", {
          mouseX,
          mouseY,
          nextIgnore: true,
        });
      }

      // 当 ignore=true 且鼠标持续「不在窗口内」，可能是 rect 失准
      if (data.isIgnoringCursorEvents) {
        outsideWhileIgnoringCount++;
        // 连续 20 次（约 2 秒）判定在窗口外，触发 rect 刷新
        if (outsideWhileIgnoringCount === 20) {
          refreshWebviewRect();
          outsideWhileIgnoringCount = 0;
        }
      } else {
        outsideWhileIgnoringCount = 0;
      }

      if (source !== "GlobalMouseMove") {
        logHitboxSnapshot(source, "outside-window", { mouseX, mouseY });
      }
      return;
    }

    outsideWhileIgnoringCount = 0;

    const adjustedX = (mouseX - windowX) / globalThis.devicePixelRatio;
    const adjustedY = (mouseY - windowY) / globalThis.devicePixelRatio;

    const elementAtPoint = document.elementFromPoint(adjustedX, adjustedY);
    if (!elementAtPoint) {
      if (source !== "GlobalMouseMove") {
        logHitboxSnapshot(source, "elementFromPoint-null", { mouseX, mouseY, adjustedX, adjustedY });
      }
      return;
    }

    const shouldAllow = shouldAllowMouseEvent(elementAtPoint);
    const nextIgnore = !shouldAllow;
    const elementDescription = describeElement(elementAtPoint);

    if (shouldAllow == data.isIgnoringCursorEvents) {
      setIgnoreCursorEvents(nextIgnore, source);
      logHitboxSnapshot(source, "state-switch", {
        mouseX,
        mouseY,
        adjustedX,
        adjustedY,
        element: elementDescription,
        shouldAllow,
        nextIgnore,
      });
    } else if (source !== "GlobalMouseMove") {
      logHitboxSnapshot(source, "no-switch", {
        mouseX,
        mouseY,
        adjustedX,
        adjustedY,
        element: elementDescription,
        shouldAllow,
        nextIgnore,
      });
    }
  };

  const evaluateCurrentMousePosition = async (source: string) => {
    await refreshWebviewRect();

    try {
      const [mouseX, mouseY] = await invoke<[number, number]>(FuncCommand.GetMousePosition);
      evaluateMousePosition(mouseX, mouseY, source);
    } catch (e) {
      logInfo(`[LayeredHitbox] [${source}] 获取当前鼠标位置失败: ${e}`);
    }
  };

  setTimeout(() => evaluateCurrentMousePosition("init-200"), 200);
  setTimeout(() => evaluateCurrentMousePosition("init-1000"), 1000);
  setTimeout(() => evaluateCurrentMousePosition("init-3000"), 3000);

  let lastEventLagLogAt = 0;
  let staleMouseRefreshScheduled = false;

  webview.listen<[x: number, y: number, emittedAt?: number]>(
    FuncEvent.GlobalMouseMove,
    (event) => {
      const emittedAt = event.payload[2];
      if (typeof emittedAt === "number") {
        const eventAge = Date.now() - emittedAt;
        if (eventAge > 100 && Date.now() - lastEventLagLogAt > 1000) {
          lastEventLagLogAt = Date.now();
          logInfo(
            `[LayeredHitbox][EventLag] GlobalMouseMove age=${eventAge}ms, mouse=(${event.payload[0]},${event.payload[1]}), desiredIgnore=${desiredIgnoreCursorEvents}, appliedIgnore=${appliedIgnoreCursorEvents}`,
          );
        }
        if (eventAge > 250) {
          if (!staleMouseRefreshScheduled) {
            staleMouseRefreshScheduled = true;
            setTimeout(() => {
              staleMouseRefreshScheduled = false;
              void evaluateCurrentMousePosition("GlobalMouseMove:stale-refresh");
            }, 0);
          }
          return;
        }
      }
      evaluateMousePosition(event.payload[0], event.payload[1], "GlobalMouseMove");
    },
  );

  globalThis.addEventListener("touchstart", (e) => {
    const shouldAllow = shouldAllowMouseEvent(e.target as Element);
    if (shouldAllow == data.isIgnoringCursorEvents) {
      setIgnoreCursorEvents(!shouldAllow, "touchstart");
      logInfo(`[LayeredHitbox] [touchstart] 切换状态: ignore=${!shouldAllow}`);
    }
  });

  const logPointerEvent = (event: Event) => {
    const pointer = event as MouseEvent;
    const target = event.target instanceof Element ? event.target : null;
    logInfo(
      `[LayeredHitbox][Input] ${event.type}: target=${describeElement(target)}, client=(${pointer.clientX.toFixed(0)},${pointer.clientY.toFixed(0)}), desiredIgnore=${desiredIgnoreCursorEvents}, appliedIgnore=${appliedIgnoreCursorEvents}, jsIgnore=${data.isIgnoringCursorEvents}`,
    );
  };

  globalThis.addEventListener("pointerdown", logPointerEvent, true);
  globalThis.addEventListener("mousedown", logPointerEvent, true);
  globalThis.addEventListener("click", logPointerEvent, true);
  globalThis.addEventListener("contextmenu", logPointerEvent, true);

  logInfo("[LayeredHitbox] 所有事件监听器注册完成");
}
