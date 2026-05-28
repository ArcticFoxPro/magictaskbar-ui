import { useSignal, useSignalEffect } from "@preact/signals";
import { useDebounce } from "@shared/hooks";
import { $is_this_webview_focused } from "@shared/signals";
import { cx } from "@shared/styles";
import { cloneElement, ComponentChild, VNode } from "preact";
import { ForwardedRef, forwardRef, JSX } from "preact/compat";
import { createPortal, CSSProperties, HTMLAttributes, useCallback, useEffect, useRef } from "preact/compat";

import { LegacyCustomAnimationProps } from "../AnimatedWrappers/domain";

import { mergeRefs } from "../mergeRefs";
import { calculateElementPosition } from "./positioning";

import "./base.css";

type BasicElementProps =
  & HTMLAttributes<HTMLElement>
  & { [x in `data-${string}`]: string };

export interface SlPopupProps<TriggerProps extends BasicElementProps> extends BasicElementProps {
  debug?: boolean;
  animationDescription?: LegacyCustomAnimationProps;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  content: ComponentChild;
  children: VNode<TriggerProps>;
  placement?: "bottom" | "top" | "left" | "right";
  offset?: number;
  /**
   * Alignment on the axis perpendicular to `placement`.
   * For `top`/`bottom`, this affects horizontal alignment (left/center/right).
   * Defaults to `center` to preserve existing behavior.
   */
  align?: "start" | "center" | "end";
  trigger?: "click" | "hover" | "manual";
  mouseEnterDelay?: number;
  mouseLeaveDelay?: number;
}

function _SlPopup<TProps extends BasicElementProps>(
  props: SlPopupProps<TProps>,
  forwardedRef: ForwardedRef<HTMLElement>,
) {
  const {
    open: openProp,
    debug,
    onOpenChange: onOpenChangeProp,
    content,
    children: trigger,
    trigger: triggerType = "click",
    mouseEnterDelay = 0,
    mouseLeaveDelay = 0,
    animationDescription = {},
    placement: preferredPosition = "bottom",
    offset = 0,
    align = "center",
    ...rest
  } = props;
  const { openAnimationName, closeAnimationName } = animationDescription;
  const isExternallyHandled = openProp !== undefined;

  const unique_trigger_id = useRef(crypto.randomUUID());

  const $was_open = useSignal(false);
  const $is_open = useSignal(openProp);
  const $popup_position_styles = useSignal<CSSProperties>({});

  const triggerRef = useRef<HTMLElement>(null);
  const popupRef = useRef<HTMLDivElement>(null);

  const mouseEnterDelayedAction = useDebounce(
    (cb: () => void) => cb(),
    mouseEnterDelay * 1000,
  );
  const mouseLeaveDelayedAction = useDebounce(
    (cb: () => void) => cb(),
    mouseLeaveDelay * 1000,
  );
  const onOpenChange = useCallback(
    (open: boolean) => {
      if (!isExternallyHandled) {
        $is_open.value = open;
      }
      onOpenChangeProp?.(open);
    },
    [onOpenChangeProp, isExternallyHandled],
  );

  useEffect(() => {
    return () => {
      mouseEnterDelayedAction.cancel();
      mouseLeaveDelayedAction.cancel();
    };
  }, []);

  useEffect(() => {
    const cb = (e: MouseEvent) => {
      const clickedElement = e.target as HTMLElement;
      if (!clickedElement || !document.contains(clickedElement)) {
        return;
      }

      const isTrigger = clickedElement.closest(
        `[data-sl-trigger-id="${unique_trigger_id.current}"]`,
      );
      const isPopup = clickedElement.closest(".sl-popup");
      if (!isTrigger && !isPopup && $is_open.value) {
        onOpenChange(false);
      }
    };
    globalThis.addEventListener("click", cb);
    globalThis.addEventListener("contextmenu", cb);
    return () => {
      globalThis.removeEventListener("click", cb);
      globalThis.removeEventListener("contextmenu", cb);
    };
  }, [onOpenChange]);

  useSignalEffect(() => {
    if (!$is_this_webview_focused.value) {
      onOpenChange(false);
    }
  });

  useEffect(() => {
    const newValue = openProp ?? $is_open.value;
    if (newValue !== $is_open.value) {
      $is_open.value = newValue;
    }
  }, [openProp]);

  useSignalEffect(() => {
    if ($is_open.value && !$was_open.peek()) {
      $was_open.value = true;
    }
  });

  const updatePopupPosition = () => {
    if (debug) {
      console.debug("updatePopupPosition");
    }

    if (
      !$was_open.value || !$is_open.value || !triggerRef.current ||
      !popupRef.current
    ) return;

    // 等待下一帧确保DOM完全渲染后再计算位置
    requestAnimationFrame(() => {
      if (!popupRef.current) return;

      const position = calculateElementPosition(
        triggerRef.current!,
        popupRef.current!,
        preferredPosition,
        offset,
        align,
      );

      if (debug) {
        console.debug("position", position);
      }

      const newStyles = {
        top: `${position.top}px`,
        left: `${position.left}px`,
      };

      // Only update if styles actually changed
      if (
        JSON.stringify(newStyles) !==
          JSON.stringify($popup_position_styles.peek())
      ) {
        $popup_position_styles.value = newStyles;
      }
    });
  };

  // 在组件打开后延迟更新位置，确保内容完全渲染
  useSignalEffect(() => {
    if ($is_open.value && $was_open.value) {
      // 延迟一小段时间确保内容渲染完成
      setTimeout(() => {
        updatePopupPosition();
      }, 10);
    }
  });

  useSignalEffect(updatePopupPosition);

  // 新增：监听popup内容尺寸变化，重新计算位置
  useEffect(() => {
    if (!$was_open.value || !popupRef.current) {
      return;
    }

    const popup = popupRef.current;
    const resizeObserver = new ResizeObserver(() => {
      // 延迟一小段时间，确保DOM更新完成
      setTimeout(updatePopupPosition, 10);
    });

    resizeObserver.observe(popup);
    return () => resizeObserver.disconnect();
  }, [$was_open.value]);

  function onMouseEnter() {
    if (triggerType === "hover") {
      if ($is_open.value) {
        mouseLeaveDelayedAction.cancel();
        return;
      }
      mouseEnterDelayedAction(() => onOpenChange(true));
    }
  }

  function onMouseLeave() {
    if (triggerType === "hover") {
      if (!$is_open.value) {
        mouseEnterDelayedAction.cancel();
        return;
      }
      mouseLeaveDelayedAction(() => onOpenChange(false));
    }
  }

  const { className: _className, ...toForwardDown } = rest;
  const triggerProps = {
    ...toForwardDown,
    ...trigger.props,
    "data-sl-trigger-id": unique_trigger_id.current,
    onClick(e: JSX.TargetedMouseEvent<HTMLElement>) {
      trigger.props.onClick?.(e);
      if (triggerType === "click") {
        onOpenChange(!$is_open.value);
      }
      toForwardDown.onClick?.(e);
    },
    onMouseEnter(e: JSX.TargetedMouseEvent<HTMLElement>) {
      trigger.props.onMouseEnter?.(e);
      if (triggerType === "hover") {
        onMouseEnter();
      }
      toForwardDown.onMouseEnter?.(e);
    },
    onMouseLeave(e: JSX.TargetedMouseEvent<HTMLElement>) {
      trigger.props.onMouseLeave?.(e);
      if (triggerType === "hover") {
        onMouseLeave();
      }
      toForwardDown.onMouseLeave?.(e);
    },
    ref: mergeRefs([trigger.ref, triggerRef, forwardedRef]),
  };

  return (
    <>
      {cloneElement(trigger, triggerProps)}
      {$was_open.value &&
        createPortal(
          <div>
            <div
              id={unique_trigger_id.current}
              ref={popupRef}
              onMouseEnter={onMouseEnter}
              onMouseLeave={onMouseLeave}
              style={{
                ...$popup_position_styles.value,
              }}
              className={cx("sl-popup", {
                "sl-popup-open": $is_open.value,
                "sl-popup-closed": !$is_open.value,
                [openAnimationName ?? "!?"]: openAnimationName &&
                  $is_open.value,
                [closeAnimationName ?? "!?"]: closeAnimationName &&
                  !$is_open.value,
              })}
            >
              {content}
            </div>
          </div>,
          document.body,
        )}
    </>
  );
}

export const SlPopup = forwardRef(_SlPopup);
