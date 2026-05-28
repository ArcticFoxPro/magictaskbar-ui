import { FuncEvent, subscribe } from '@magic-ui/lib';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { useEffect, useRef } from 'react';

const TASKBAR_LAYOUT_REFRESH_EVENT = "taskbar-layout-refresh-request";

interface DockMagnifierOptions {
  maxScale?: number;
  hoverThreshold?: number;
  smoothFactor?: number;
  enableGapScaling?: boolean;
  targetSelector?: string; // CSS选择器，用于指定哪些区域的图标应用动效
  enabled?: boolean; // 根据设置控制是否启用放大效果
  zoomEffectType?: 'wave' | 'singleIcon'; // 放大效果类型
}

type DockEntrySide = 'top' | 'bottom' | 'left' | 'right' | null;
type HorizontalWaveExpansionMode = 'center' | 'left-fixed' | 'right-fixed';
type HorizontalTravelPhase = 'idle' | 'entry' | 'middle' | 'exit' | 'release';
type HorizontalExitTransition = {
  progress: number;
  fromMode: Exclude<HorizontalWaveExpansionMode, 'center'>;
  toMode: Exclude<HorizontalWaveExpansionMode, 'center'>;
};
type HorizontalExitTransitionState = {
  fromMode: Exclude<HorizontalWaveExpansionMode, 'center'>;
  toMode: Exclude<HorizontalWaveExpansionMode, 'center'>;
  fromPosition: number | null;
  toPosition: number | null;
  extraWidth: number;
};

interface DockBounds {
  left: number;
  right: number;
  top: number;
  bottom: number;
  width: number;
  height: number;
}

const clamp = (value: number, min: number, max: number): number =>
  Math.min(Math.max(value, min), max);

const getHorizontalBiasForMode = (mode: HorizontalWaveExpansionMode): number =>
  mode === 'left-fixed' ? 1 : mode === 'right-fixed' ? -1 : 0;

const smoothstep = (t: number): number => t * t * (3 - 2 * t);
const dockBoundsTolerance = 4;

const getSmoothTransitionProgress = (
  value: number,
  start: number,
  end: number,
): number => {
  if (Math.abs(end - start) < 0.001) {
    return value >= end ? 1 : 0;
  }

  return smoothstep(clamp((value - start) / (end - start), 0, 1));
};

const getHorizontalSwitchHalfRange = (
  centers: Array<{ x: number; y: number }>,
  index: number,
): number => {
  const current = centers[index];
  if (!current) {
    return 18;
  }

  const candidateGaps: number[] = [];
  const prev = centers[index - 1];
  const next = centers[index + 1];

  if (prev) {
    candidateGaps.push(Math.abs(current.x - prev.x));
  }
  if (next) {
    candidateGaps.push(Math.abs(next.x - current.x));
  }

  const baseGap = candidateGaps.length > 0 ? Math.min(...candidateGaps) : 48;
  return clamp(baseGap * 0.32, 10, 26);
};

const getHorizontalSwitchIndexes = (
  count: number,
): {
  leftEntry: number;
  rightEntry: number;
  leftExit: number;
  rightExit: number;
} => {
  if (count <= 1) {
    return { leftEntry: 0, rightEntry: 0, leftExit: 0, rightExit: 0 };
  }

  if (count <= 5) {
    const center = Math.floor((count - 1) / 2);
    return {
      leftEntry: 0,
      rightEntry: count - 1,
      leftExit: center,
      rightExit: center,
    };
  }

  return {
    leftEntry: 0,
    rightEntry: count - 1,
    leftExit: 2,
    rightExit: count - 3,
  };
};

const getRectBasedHalfRange = (
  width: number | undefined,
  ratio: number,
  min: number,
  max: number,
): number => clamp((width ?? 48) * ratio, min, max);

const verticalAttenuation = (distance: number): number => {
  const deadZone = 12;
  const maxDistance = 80;

  if (distance <= deadZone) {
    return 1;
  }

  const t = Math.min((distance - deadZone) / maxDistance, 1);
  // smoothstep
  const s = t * t * (3 - 2 * t);
  // 再压一下，让前半段更稳
  return Math.cos(s * Math.PI * 0.5);
}

const getLerpFactor = (diff: number, attenuation: number): number => {
  if (diff > 0) {
    // 放大：快
    return 0.25;
  }
  // 缩小： 与垂直离开程度相关
  return 0.08 +0.12 * attenuation;
}

/**
 * 波形缩放算法 - 基于macOS Dock的余弦波效果
 * 使用余弦函数实现更平滑的过渡，让相邻图标在鼠标居中时大小更接近
 */
const calculateCosineScale = ({
  curveCenter,
  itemPosition,
  curveRange,
  minScale,
  maxScale,
}: {
  curveCenter: number;
  itemPosition: number;
  curveRange: number;
  minScale: number;
  maxScale: number;
}): number => {
  const distance = Math.abs(itemPosition - curveCenter);

  // 如果距离超出范围，返回最小缩放
  if (distance >= curveRange) {
    return minScale;
  }

  const amplitude = maxScale - minScale;
  // 余弦函数 + 指数幂: 让中心图标显著突出，相邻图标大小差距更明显
  // cosBase: 0（边缘）到 1（中心）的平滑过渡
  const cosBase = (1 + Math.cos(Math.PI * distance / curveRange)) / 2;
  // 使用 0.5 次幂加大中心与边缘的差距
  // 效果对比（amplitude部分占比）:
  //   中心图标: 1.0 → 1.0  (不变)
  //   第1邻居:  0.85 → 0.74 (差距拉大)
  //   第2邻居:  0.50 → 0.29 (差距显著拉大)
  const scale = minScale + amplitude * Math.pow(cosBase, 0.95);
  return scale;
};

const isPointInsideBounds = (x: number, y: number, bounds: DockBounds): boolean =>
  x >= bounds.left &&
  x <= bounds.right &&
  y >= bounds.top &&
  y <= bounds.bottom;

const isHorizontalDockInteractionActive = (
  entrySide: DockEntrySide,
  phase: HorizontalTravelPhase,
): boolean =>
  entrySide === 'left' ||
  entrySide === 'right' ||
  phase === 'entry' ||
  phase === 'middle' ||
  phase === 'exit' ||
  phase === 'release';

const isPointInsideDockInteractionBounds = (
  x: number,
  y: number,
  bounds: DockBounds,
  horizontalInteractionActive: boolean,
): boolean => {
  if (!horizontalInteractionActive) {
    return isPointInsideBounds(x, y, bounds);
  }

  const horizontalTolerance = 10;
  const verticalTolerance = 18;
  return (
    x >= bounds.left - horizontalTolerance &&
    x <= bounds.right + horizontalTolerance &&
    y >= bounds.top - verticalTolerance &&
    y <= bounds.bottom + verticalTolerance
  );
};

const detectDockEntrySide = (
  x: number,
  y: number,
  bounds: DockBounds,
): DockEntrySide => {
  const candidates: Array<{ side: Exclude<DockEntrySide, null>; distance: number }> = [];

  if (x < bounds.left) {
    candidates.push({ side: 'left', distance: bounds.left - x });
  }
  if (x > bounds.right) {
    candidates.push({ side: 'right', distance: x - bounds.right });
  }
  if (y < bounds.top) {
    candidates.push({ side: 'top', distance: bounds.top - y });
  }
  if (y > bounds.bottom) {
    candidates.push({ side: 'bottom', distance: y - bounds.bottom });
  }

  if (!candidates.length) {
    return null;
  }

  candidates.sort((a, b) => a.distance - b.distance);
  return candidates[0]!.side;
};

const detectNearestDockSide = (
  x: number,
  y: number,
  bounds: DockBounds,
): Exclude<DockEntrySide, null> => {
  const candidates: Array<{ side: Exclude<DockEntrySide, null>; distance: number }> = [
    { side: 'left', distance: Math.abs(x - bounds.left) },
    { side: 'right', distance: Math.abs(bounds.right - x) },
    { side: 'top', distance: Math.abs(y - bounds.top) },
    { side: 'bottom', distance: Math.abs(bounds.bottom - y) },
  ];

  candidates.sort((a, b) => a.distance - b.distance);
  return candidates[0]!.side;
};

const detectDockEntrySideFromInside = (
  x: number,
  y: number,
  bounds: DockBounds,
): Exclude<DockEntrySide, null> => {
  const horizontalEdgeThreshold = clamp(bounds.width * 0.12, 18, 42);
  const verticalEdgeThreshold = clamp(bounds.height * 0.45, 10, 24);

  if (x <= bounds.left + horizontalEdgeThreshold) {
    return 'left';
  }
  if (x >= bounds.right - horizontalEdgeThreshold) {
    return 'right';
  }
  if (y <= bounds.top + verticalEdgeThreshold) {
    return 'top';
  }
  if (y >= bounds.bottom - verticalEdgeThreshold) {
    return 'bottom';
  }

  return detectNearestDockSide(x, y, bounds);
};

export const useDockMagnifier = (
  containerRef: React.RefObject<HTMLElement>,
  {
    maxScale = 1.7,
    hoverThreshold = 300,
    smoothFactor = 0.25,
    enableGapScaling = true,
    targetSelector = '.taskbar-item', // 默认所有图标
    enabled = true, // 默认启用
    zoomEffectType = 'wave', // 默认波浪效果
  }: DockMagnifierOptions = {},
) => {
  const itemsRef = useRef<HTMLElement[]>([]);
  const itemRectsRef = useRef<Array<{ left: number; right: number; width: number }>>([]);
  const gapElementsRef = useRef<HTMLElement[]>([]);
  const centersRef = useRef<{ x: number; y: number }[]>([]);
  const gapCentersRef = useRef<{ x: number; y: number }[]>([]);
  const baseGapSizesRef = useRef<number[]>([]);
  const currentScales = useRef<Float32Array>(new Float32Array(0));
  const currentGapScales = useRef<Float32Array>(new Float32Array(0));
  const targetScales = useRef<Float32Array>(new Float32Array(0));
  const targetGapScales = useRef<Float32Array>(new Float32Array(0));
  const pointer = useRef<{ x: number | null; y: number | null }>({ x: null, y: null });
  const lastPointer = useRef<{ x: number | null; y: number | null }>({ x: null, y: null });
  const rafId = useRef<number>(0);
  const isActive = useRef<boolean>(false);
  const measureTimeoutRef = useRef<number | null>(null);
  const layoutRefreshRetryTimeoutRef = useRef<number | null>(null);
  const layoutRefreshBurstTimeoutsRef = useRef<number[]>([]);
  const pointerMovedRef = useRef<boolean>(false);
  const horizontalLayoutAnimatingRef = useRef<boolean>(false);
  const leaveTimeoutRef = useRef<number | null>(null); // 延迟清除定时器
  const minScale = 1;
  const entrySideRef = useRef<DockEntrySide>(null);
  const dockBoundsRef = useRef<DockBounds | null>(null);
  const horizontalAnchorModeRef = useRef<HorizontalWaveExpansionMode>('center');
  const horizontalAnchorSwitchRef = useRef<{
    leftEntry: number | null;
    rightEntry: number | null;
    leftExit: number | null;
    rightExit: number | null;
    leftEntryHalfRange: number;
    rightEntryHalfRange: number;
    leftExitHalfRange: number;
    rightExitHalfRange: number;
  }>({
    leftEntry: null,
    rightEntry: null,
    leftExit: null,
    rightExit: null,
    leftEntryHalfRange: 10,
    rightEntryHalfRange: 10,
    leftExitHalfRange: 16,
    rightExitHalfRange: 16,
  });
  const currentHorizontalAnchorBiasRef = useRef<number>(0);
  const currentHorizontalOffsetRef = useRef<number>(0);
  const horizontalSideLockRef = useRef<'left' | 'right' | null>(null);
  const horizontalLockedOuterExtraRef = useRef<number>(0);
  const horizontalEntryOuterPeakRef = useRef<number>(0);
  const centeredHorizontalExtraWidthLockRef = useRef<number | null>(null);
  const centeredHorizontalGapExtraLockRef = useRef<number | null>(null);
  const centeredHorizontalOuterBeforeLockRef = useRef<number | null>(null);
  const centeredHorizontalOuterAfterLockRef = useRef<number | null>(null);
  const horizontalExitTransitionProgressRef = useRef<number | null>(null);
  const horizontalExitTransitionStateRef = useRef<HorizontalExitTransitionState | null>(null);
  const fixedHorizontalEdgeRef = useRef<{
    mode: HorizontalWaveExpansionMode;
    position: number | null;
  }>({
    mode: 'center',
    position: null,
  });
  const horizontalReleaseStartEdgeRef = useRef<number | null>(null);
  const horizontalReleaseTargetEdgeRef = useRef<number | null>(null);
  const horizontalReleaseStartExtraWidthRef = useRef<number | null>(null);
  const horizontalReleaseStartOffsetRef = useRef<number | null>(null);
  const horizontalReleaseProgressRef = useRef<number>(0);
  const visualDockBoundsRef = useRef<DockBounds | null>(null);
  const horizontalTravelPhaseRef = useRef<HorizontalTravelPhase>('idle');
  const justEnteredHorizontalRef = useRef<boolean>(false);
  const pendingHorizontalReleaseRef = useRef<boolean>(false);
  const measuredItemCountRef = useRef<number>(0);
  const windowScreenPositionRef = useRef<{ x: number; y: number }>({ x: 0, y: 0 });
  const globalPointerRef = useRef<{ x: number | null; y: number | null }>({ x: null, y: null });

  const syncHorizontalAnchorModeWithEntrySide = (entrySide: DockEntrySide) => {
    if (entrySide === 'left') {
      horizontalAnchorModeRef.current = 'left-fixed';
      currentHorizontalAnchorBiasRef.current = 1;
      return;
    }

    if (entrySide === 'right') {
      horizontalAnchorModeRef.current = 'right-fixed';
      currentHorizontalAnchorBiasRef.current = -1;
      return;
    }

    horizontalAnchorModeRef.current = 'center';
    currentHorizontalAnchorBiasRef.current = 0;
  };

  const syncHorizontalTravelPhaseWithEntrySide = (entrySide: DockEntrySide) => {
    horizontalTravelPhaseRef.current =
      entrySide === 'left' || entrySide === 'right' ? 'entry' : 'idle';
  };

  const syncHorizontalStateWithPointerEntry = (
    entrySide: DockEntrySide,
    pointerX: number | null,
    dockBounds: DockBounds | null,
  ) => {
    horizontalSideLockRef.current = null;
    horizontalLockedOuterExtraRef.current = 0;
    horizontalEntryOuterPeakRef.current = 0;

    if (
      (entrySide !== 'left' && entrySide !== 'right') ||
      pointerX === null ||
      dockBounds === null
    ) {
      syncHorizontalAnchorModeWithEntrySide(entrySide);
      syncHorizontalTravelPhaseWithEntrySide(entrySide);
      captureHorizontalFixedEdge(horizontalAnchorModeRef.current);
      justEnteredHorizontalRef.current = false;
      return;
    }

    const leftEntryX =
      horizontalAnchorSwitchRef.current.leftEntry ?? dockBounds.left + dockBounds.width * 0.02;
    const rightEntryX =
      horizontalAnchorSwitchRef.current.rightEntry ?? dockBounds.right - dockBounds.width * 0.02;
    const initialMode =
      entrySide === 'left'
        ? pointerX >= leftEntryX
          ? 'right-fixed'
          : 'left-fixed'
        : pointerX <= rightEntryX
          ? 'left-fixed'
          : 'right-fixed';
    const initialPhase =
      initialMode === 'left-fixed'
        ? entrySide === 'left'
          ? 'entry'
          : 'middle'
        : entrySide === 'right'
          ? 'entry'
          : 'middle';

    horizontalAnchorModeRef.current = initialMode;
    horizontalTravelPhaseRef.current = initialPhase;
    currentHorizontalAnchorBiasRef.current =
      initialMode === 'left-fixed'
        ? 1
        : initialMode === 'right-fixed'
          ? -1
          : 0;
    captureHorizontalFixedEdge(initialMode);
    justEnteredHorizontalRef.current = false;
  };

  const getOppositeHorizontalFixedMode = (
    mode: HorizontalWaveExpansionMode,
  ): HorizontalWaveExpansionMode => {
    if (mode === 'left-fixed') {
      return 'right-fixed';
    }
    if (mode === 'right-fixed') {
      return 'left-fixed';
    }
    return 'center';
  };

  const getHorizontalReleaseMode = (
    exitSide: DockEntrySide,
    fallbackMode: HorizontalWaveExpansionMode,
  ): HorizontalWaveExpansionMode => {
    if (exitSide === 'left') {
      return 'left-fixed';
    }
    if (exitSide === 'right') {
      return 'right-fixed';
    }

    return getOppositeHorizontalFixedMode(fallbackMode);
  };

  const applyHorizontalDockOffset = (offset: number) => {
    const container = containerRef.current;
    const taskbar = container?.closest('.taskbar') as HTMLElement | null;
    if (!taskbar) return;

    if (Math.abs(offset) < 0.1) {
      currentHorizontalOffsetRef.current = 0;
      taskbar.style.removeProperty('left');
      taskbar.style.removeProperty('position');
      // 不再使用 transform，因为 transform 会为后代元素创建新的 containing block，
      // 导致 DragOverlay 的 position:fixed 不再相对于 viewport，破坏拖拽定位
      return;
    }

    currentHorizontalOffsetRef.current = offset;
    // 使用 position:relative + left 替代 transform: translate3d
    // 效果相同，但不会影响后代的 position:fixed 行为
    taskbar.style.position = 'relative';
    taskbar.style.left = `${offset}px`;
  };

  const readScreenDockBounds = (): DockBounds | null => {
    const dpr = globalThis.devicePixelRatio || 1;
    const windowPos = windowScreenPositionRef.current;
    const visualBounds = visualDockBoundsRef.current;
    const baseBounds = dockBoundsRef.current;

    if (visualBounds) {
      return {
        left: windowPos.x + visualBounds.left * dpr - dockBoundsTolerance * dpr,
        right: windowPos.x + visualBounds.right * dpr + dockBoundsTolerance * dpr,
        top: windowPos.y + visualBounds.top * dpr - dockBoundsTolerance * dpr,
        bottom: windowPos.y + visualBounds.bottom * dpr + dockBoundsTolerance * dpr,
        width: visualBounds.width * dpr + dockBoundsTolerance * dpr * 2,
        height: visualBounds.height * dpr + dockBoundsTolerance * dpr * 2,
      };
    }

    if (baseBounds) {
      const horizontalOffset = currentHorizontalOffsetRef.current * dpr;
      return {
        left: windowPos.x + baseBounds.left * dpr + horizontalOffset - dockBoundsTolerance * dpr,
        right: windowPos.x + baseBounds.right * dpr + horizontalOffset + dockBoundsTolerance * dpr,
        top: windowPos.y + baseBounds.top * dpr - dockBoundsTolerance * dpr,
        bottom: windowPos.y + baseBounds.bottom * dpr + dockBoundsTolerance * dpr,
        width: baseBounds.width * dpr + dockBoundsTolerance * dpr * 2,
        height: baseBounds.height * dpr + dockBoundsTolerance * dpr * 2,
      };
    }

    const currentBounds = readDockBounds();
    if (!currentBounds) {
      return null;
    }

    return {
      left: windowPos.x + currentBounds.left * dpr - dockBoundsTolerance * dpr,
      right: windowPos.x + currentBounds.right * dpr + dockBoundsTolerance * dpr,
      top: windowPos.y + currentBounds.top * dpr - dockBoundsTolerance * dpr,
      bottom: windowPos.y + currentBounds.bottom * dpr + dockBoundsTolerance * dpr,
      width: currentBounds.width * dpr + dockBoundsTolerance * dpr * 2,
      height: currentBounds.height * dpr + dockBoundsTolerance * dpr * 2,
    };
  };

  const readDockBounds = (): DockBounds | null => {
    const container = containerRef.current;
    if (!container) return null;

    const taskbar = container.closest('.taskbar') as HTMLElement | null;
    const rect = (taskbar ?? container).getBoundingClientRect();
    return {
      left: rect.left,
      right: rect.right,
      top: rect.top,
      bottom: rect.bottom,
      width: rect.width,
      height: rect.height,
    };
  };

  const captureHorizontalFixedEdge = (
    mode: HorizontalWaveExpansionMode,
    anchor: 'current' | 'resting' = 'current',
  ) => {
    if (mode === 'center') {
      fixedHorizontalEdgeRef.current = { mode: 'center', position: null };
      return;
    }

    const fallbackBounds = dockBoundsRef.current;
    const actualBounds = anchor === 'current' ? readDockBounds() : null;
    const position =
      mode === 'left-fixed'
        ? actualBounds?.left ?? fallbackBounds?.left ?? null
        : actualBounds?.right ?? fallbackBounds?.right ?? null;

    fixedHorizontalEdgeRef.current = { mode, position };
  };

  const syncHorizontalExitTransitionState = (
    transition: HorizontalExitTransition | null,
    extraWidth: number,
  ) => {
    if (!transition) {
      horizontalExitTransitionStateRef.current = null;
      return;
    }

    const existing = horizontalExitTransitionStateRef.current;
    if (
      existing &&
      existing.fromMode === transition.fromMode &&
      existing.toMode === transition.toMode
    ) {
      return;
    }

    const actualBounds = readDockBounds();
    const fallbackBounds = dockBoundsRef.current;
    horizontalExitTransitionStateRef.current = {
      fromMode: transition.fromMode,
      toMode: transition.toMode,
      fromPosition:
        transition.fromMode === 'left-fixed'
          ? actualBounds?.left ?? fallbackBounds?.left ?? null
          : actualBounds?.right ?? fallbackBounds?.right ?? null,
      toPosition:
        transition.toMode === 'left-fixed'
          ? actualBounds?.left ?? fallbackBounds?.left ?? null
          : actualBounds?.right ?? fallbackBounds?.right ?? null,
      extraWidth,
    };
  };

  const stabilizeHorizontalExitTransition = (
    transition: HorizontalExitTransition | null,
    deltaX: number,
  ): HorizontalExitTransition | null => {
    if (!transition) {
      horizontalExitTransitionProgressRef.current = null;
      return null;
    }

    const previousProgress = horizontalExitTransitionProgressRef.current;
    const movingTowardExit =
      (entrySideRef.current === 'left' && deltaX >= 0) ||
      (entrySideRef.current === 'right' && deltaX <= 0);
    const stabilizedProgress =
      previousProgress === null
        ? transition.progress
        : movingTowardExit
          ? Math.max(previousProgress, transition.progress)
          : Math.min(previousProgress, transition.progress);

    horizontalExitTransitionProgressRef.current = stabilizedProgress;
    return {
      ...transition,
      progress: stabilizedProgress,
    };
  };

  const resetHorizontalReleaseState = () => {
    pendingHorizontalReleaseRef.current = false;
    justEnteredHorizontalRef.current = false;
    horizontalLayoutAnimatingRef.current = false;
    horizontalSideLockRef.current = null;
    horizontalLockedOuterExtraRef.current = 0;
    horizontalEntryOuterPeakRef.current = 0;
    centeredHorizontalExtraWidthLockRef.current = null;
    centeredHorizontalGapExtraLockRef.current = null;
    centeredHorizontalOuterBeforeLockRef.current = null;
    centeredHorizontalOuterAfterLockRef.current = null;
    horizontalExitTransitionProgressRef.current = null;
    horizontalExitTransitionStateRef.current = null;
    horizontalReleaseStartEdgeRef.current = null;
    horizontalReleaseTargetEdgeRef.current = null;
    horizontalReleaseStartExtraWidthRef.current = null;
    horizontalReleaseStartOffsetRef.current = null;
    horizontalReleaseProgressRef.current = 0;
  };

  const finalizeHorizontalRelease = () => {
    resetHorizontalReleaseState();
    entrySideRef.current = null;
    syncHorizontalAnchorModeWithEntrySide(null);
    syncHorizontalTravelPhaseWithEntrySide(null);
    captureHorizontalFixedEdge('center');
  };

  const resetMagnifierRuntimeState = () => {
    finalizeHorizontalRelease();
    pointer.current.x = null;
    pointer.current.y = null;
    lastPointer.current.x = null;
    lastPointer.current.y = null;
    globalPointerRef.current.x = null;
    globalPointerRef.current.y = null;
    pointerMovedRef.current = false;
    horizontalLayoutAnimatingRef.current = false;
    dockBoundsRef.current = null;
    visualDockBoundsRef.current = null;
    itemsRef.current = [];
    itemRectsRef.current = [];
    gapElementsRef.current = [];
    centersRef.current = [];
    gapCentersRef.current = [];
    currentScales.current = new Float32Array(0);
    targetScales.current = new Float32Array(0);
    currentGapScales.current = new Float32Array(0);
    targetGapScales.current = new Float32Array(0);
  };

  const clearScheduledLayoutRefresh = () => {
    if (layoutRefreshRetryTimeoutRef.current !== null) {
      clearTimeout(layoutRefreshRetryTimeoutRef.current);
      layoutRefreshRetryTimeoutRef.current = null;
    }
    layoutRefreshBurstTimeoutsRef.current.forEach((timerId) => {
      clearTimeout(timerId);
    });
    layoutRefreshBurstTimeoutsRef.current = [];
  };

  const isLayoutRefreshSafe = () => {
    if (pointer.current.x !== null || pointer.current.y !== null) {
      return false;
    }

    if (
      isActive.current ||
      pendingHorizontalReleaseRef.current ||
      horizontalLayoutAnimatingRef.current
    ) {
      return false;
    }

    if (Math.abs(currentHorizontalOffsetRef.current) > 0.1) {
      return false;
    }

    for (let i = 0; i < currentScales.current.length; i++) {
      if (Math.abs((currentScales.current[i] ?? minScale) - minScale) > 0.02) {
        return false;
      }
    }

    for (let i = 0; i < currentGapScales.current.length; i++) {
      if (Math.abs((currentGapScales.current[i] ?? minScale) - minScale) > 0.02) {
        return false;
      }
    }

    const taskbar = containerRef.current?.closest('.taskbar') as HTMLElement | null;
    if (taskbar?.classList.contains('hidden')) {
      return false;
    }

    return true;
  };

  const runStructuralLayoutRefresh = () => {
    if (!enabled || !containerRef.current) {
      clearScheduledLayoutRefresh();
      return;
    }

    if (!isLayoutRefreshSafe()) {
      if (layoutRefreshRetryTimeoutRef.current !== null) {
        clearTimeout(layoutRefreshRetryTimeoutRef.current);
      }
      layoutRefreshRetryTimeoutRef.current = window.setTimeout(() => {
        layoutRefreshRetryTimeoutRef.current = null;
        runStructuralLayoutRefresh();
      }, 80);
      return;
    }

    if (layoutRefreshRetryTimeoutRef.current !== null) {
      clearTimeout(layoutRefreshRetryTimeoutRef.current);
      layoutRefreshRetryTimeoutRef.current = null;
    }

    resetMagnifierRuntimeState();
    applyHorizontalDockOffset(0);
    refreshItems();
    window.dispatchEvent(new Event(TASKBAR_LAYOUT_REFRESH_EVENT));
  };

  const scheduleStructuralLayoutRefresh = (delays: number[] = [0, 90, 220]) => {
    clearScheduledLayoutRefresh();
    delays.forEach((delay) => {
      const timerId = window.setTimeout(() => {
        layoutRefreshBurstTimeoutsRef.current = layoutRefreshBurstTimeoutsRef.current.filter(
          (id) => id !== timerId,
        );
        runStructuralLayoutRefresh();
      }, delay);
      layoutRefreshBurstTimeoutsRef.current.push(timerId);
    });
  };

  const detectHorizontalFarExitSide = (localX: number | null): DockEntrySide => {
    if (
      localX === null ||
      pendingHorizontalReleaseRef.current ||
      (entrySideRef.current !== 'left' && entrySideRef.current !== 'right') ||
      horizontalTravelPhaseRef.current !== 'exit' ||
      itemRectsRef.current.length === 0
    ) {
      return null;
    }

    const firstRect = itemRectsRef.current[0] ?? null;
    const lastRect = itemRectsRef.current[itemRectsRef.current.length - 1] ?? null;
    const firstItem = itemsRef.current[0] ?? null;
    const lastItem = itemsRef.current[itemsRef.current.length - 1] ?? null;
    if (!firstRect || !lastRect) {
      return null;
    }

    const currentDockOffset = currentHorizontalOffsetRef.current;
    const visualFirstLeft = firstItem
      ? firstItem.getBoundingClientRect().left - currentDockOffset
      : firstRect.left;
    const visualLastRight = lastItem
      ? lastItem.getBoundingClientRect().right - currentDockOffset
      : lastRect.right;

    if (localX < visualFirstLeft) {
      return 'left';
    }
    if (localX > visualLastRight) {
      return 'right';
    }

    return null;
  };

  const beginHorizontalRelease = (exitSide: DockEntrySide) => {
    if (pendingHorizontalReleaseRef.current) {
      return;
    }

    const currentMode = horizontalAnchorModeRef.current;
    const desiredReleaseMode = getHorizontalReleaseMode(exitSide, currentMode);
    const shouldPreserveCurrentFixedMode =
      (entrySideRef.current === 'left' || entrySideRef.current === 'right') &&
      horizontalTravelPhaseRef.current === 'exit' &&
      (currentMode === 'left-fixed' || currentMode === 'right-fixed');
    const releaseMode =
      shouldPreserveCurrentFixedMode
        ? currentMode
        : desiredReleaseMode === currentMode
          ? desiredReleaseMode
          : currentMode;
    const currentBounds = readDockBounds();
    const restingBounds = dockBoundsRef.current;

    pendingHorizontalReleaseRef.current = true;
    justEnteredHorizontalRef.current = false;
    horizontalTravelPhaseRef.current = 'release';
    horizontalAnchorModeRef.current = releaseMode;
    horizontalReleaseStartEdgeRef.current =
      releaseMode === 'left-fixed'
        ? currentBounds?.left ?? restingBounds?.left ?? null
        : releaseMode === 'right-fixed'
          ? currentBounds?.right ?? restingBounds?.right ?? null
          : null;
    horizontalReleaseTargetEdgeRef.current =
      releaseMode === 'left-fixed'
        ? restingBounds?.left ?? null
        : releaseMode === 'right-fixed'
          ? restingBounds?.right ?? null
          : null;
    horizontalReleaseStartExtraWidthRef.current = null;
    horizontalReleaseStartOffsetRef.current = currentHorizontalOffsetRef.current;
    fixedHorizontalEdgeRef.current = {
      mode: releaseMode,
      position: horizontalReleaseStartEdgeRef.current,
    };
    pointer.current.x = null;
    pointer.current.y = null;
    isActive.current = false;
  };

  const measureCenters = () => {
    if (!itemsRef.current.length) return;
    applyHorizontalDockOffset(0);
    const taskbar = containerRef.current?.closest('.taskbar');
    const isVerticalDock =
      taskbar?.classList.contains('left') || taskbar?.classList.contains('right');

    // 暂时重置所有图标和容器的样式，以便准确测量初始位置
    itemsRef.current.forEach((item) => {
      item.style.transform = 'none';
      item.style.transformOrigin = 'center center';
      item.style.zIndex = '1';
      // 重置位移margin
      item.style.marginTop = '0px';
      item.style.marginBottom = '0px';
      item.style.marginLeft = '0px';
      item.style.marginRight = '0px';

      // 重置容器的额外空间
      const dragContainer = item.closest('.taskbar-item-drag-container') as HTMLElement;
      if (dragContainer) {
        dragContainer.style.setProperty('--mag-extra-before', '0px');
        dragContainer.style.setProperty('--mag-extra-after', '0px');
        dragContainer.style.setProperty('--gap-scale', '1');
      }
    });

    // 强制重排以确保样式应用
    void document.body.offsetHeight;

    centersRef.current = itemsRef.current.map((item) => {
      const rect = item.getBoundingClientRect();
      return {
        x: rect.left + rect.width / 2,
        y: rect.top + rect.height / 2,
      };
    });
    itemRectsRef.current = itemsRef.current.map((item) => {
      const rect = item.getBoundingClientRect();
      return {
        left: rect.left,
        right: rect.right,
        width: rect.width,
      };
    });

    if (enableGapScaling && gapElementsRef.current.length) {
      gapCentersRef.current = gapElementsRef.current.map((gap) => {
        const rect = gap.getBoundingClientRect();
        return {
          x: rect.left + rect.width / 2,
          y: rect.top + rect.height / 2,
        };
      });
      baseGapSizesRef.current = gapElementsRef.current.map((gap) => {
        const style = globalThis.getComputedStyle(gap);
        const gapValue = isVerticalDock ? style.marginBottom : style.marginRight;
        return Number.parseFloat(gapValue) || 0;
      });
    } else {
      baseGapSizesRef.current = [];
    }

    if (itemRectsRef.current.length > 0) {
      const {
        leftEntry,
        rightEntry,
        leftExit,
        rightExit,
      } = getHorizontalSwitchIndexes(itemRectsRef.current.length);
      const leftEntryRect = itemRectsRef.current[leftEntry] ?? null;
      const rightEntryRect = itemRectsRef.current[rightEntry] ?? null;
      const leftExitRect = itemRectsRef.current[leftExit] ?? null;
      const rightExitRect = itemRectsRef.current[rightExit] ?? null;
      const leftExitPrevRect = itemRectsRef.current[leftExit - 1] ?? null;
      const leftExitNextRect = itemRectsRef.current[leftExit + 1] ?? null;
      const rightExitPrevRect = itemRectsRef.current[rightExit - 1] ?? null;
      const rightExitNextRect = itemRectsRef.current[rightExit + 1] ?? null;

      horizontalAnchorSwitchRef.current = {
        leftEntry: leftEntryRect
          ? leftEntryRect.left + clamp(leftEntryRect.width * 0.02, 1, 3)
          : null,
        rightEntry: rightEntryRect
          ? rightEntryRect.right - clamp(rightEntryRect.width * 0.02, 1, 3)
          : null,
        leftExit:
          leftExitRect && leftExitPrevRect
            ? (leftExitPrevRect.right + leftExitRect.left) / 2
            : leftExitRect
              ? leftExitRect.left + leftExitRect.width * 0.68
              : null,
        rightExit:
          rightExitRect && rightExitNextRect
            ? (rightExitRect.right + rightExitNextRect.left) / 2
            : rightExitRect
              ? rightExitRect.right - rightExitRect.width * 0.68
              : null,
        leftEntryHalfRange: getRectBasedHalfRange(leftEntryRect?.width, 0.14, 6, 14),
        rightEntryHalfRange: getRectBasedHalfRange(rightEntryRect?.width, 0.14, 6, 14),
        leftExitHalfRange: getRectBasedHalfRange(leftExitRect?.width, 0.2, 10, 20),
        rightExitHalfRange: getRectBasedHalfRange(rightExitRect?.width, 0.2, 10, 20),
      };
    } else {
      horizontalAnchorSwitchRef.current = {
        leftEntry: null,
        rightEntry: null,
        leftExit: null,
        rightExit: null,
        leftEntryHalfRange: 10,
        rightEntryHalfRange: 10,
        leftExitHalfRange: 16,
        rightExitHalfRange: 16,
      };
    }

    dockBoundsRef.current = readDockBounds();
    visualDockBoundsRef.current = dockBoundsRef.current;
  };

  const updateDOM = () => {
    const container = containerRef.current;
    const pointerX = pointer.current.x;
    const localPointerX =
      pointerX === null ? null : pointerX - currentHorizontalOffsetRef.current;

    // 优化：检测Dock方向 - 这个函数在needsUpdate为true时才调用，频率已降低
    const taskbar = container?.closest('.taskbar');
    const isBottom = taskbar?.classList.contains('bottom');
    const isTop = taskbar?.classList.contains('top');
    const isLeft = taskbar?.classList.contains('left');
    const isRight = taskbar?.classList.contains('right');
    const dockBounds = dockBoundsRef.current ?? readDockBounds();
    // 默认为底部Dock（如果没有检测到方向类）
    const isDefaultBottom = !isTop && !isLeft && !isRight;
    const switchLockHysteresis = 6;
    let targetHorizontalAnchorMode: HorizontalWaveExpansionMode = 'center';
    let targetHorizontalAnchorBias = 0;
    let shouldSnapHorizontalAnchor = false;
    let sideEntryPivotIndex: number | null = null;
    let sideEntryUsesOuterSplit = false;
    let pendingSideLock: 'left' | 'right' | null = horizontalSideLockRef.current;
    const {
      leftExit: defaultLeftPivotIndex,
      rightExit: defaultRightPivotIndex,
    } = getHorizontalSwitchIndexes(itemRectsRef.current.length);
    const sideEntryReleaseSplitBlend =
      !isLeft &&
      !isRight &&
      pendingHorizontalReleaseRef.current &&
      (entrySideRef.current === 'left' || entrySideRef.current === 'right')
        ? 1 - getSmoothTransitionProgress(horizontalReleaseProgressRef.current, 0.04, 0.22)
        : 0;

    if (!isLeft && !isRight && dockBounds && localPointerX !== null && !pendingHorizontalReleaseRef.current) {
      const currentX = localPointerX;
      const deltaX =
        lastPointer.current.x !== null ? pointerX - lastPointer.current.x : 0;
      const firstItemRect = itemRectsRef.current[0] ?? null;
      const lastItemRect = itemRectsRef.current[itemRectsRef.current.length - 1] ?? null;
      const leftExitX =
        horizontalAnchorSwitchRef.current.leftExit ?? dockBounds.left + dockBounds.width * 0.18;
      const rightExitX =
        horizontalAnchorSwitchRef.current.rightExit ?? dockBounds.right - dockBounds.width * 0.18;
      const {
        leftExit: leftPivotIndex,
        rightExit: rightPivotIndex,
      } = getHorizontalSwitchIndexes(itemRectsRef.current.length);
      const rightExitSwitchToExitX = rightExitX + switchLockHysteresis;
      const leftExitSwitchToExitX = leftExitX - switchLockHysteresis;
      const leftReturnX = firstItemRect
        ? firstItemRect.left - clamp(firstItemRect.width * 0.08, 4, 10)
        : dockBounds.left + 6;
      const rightReturnX = lastItemRect
        ? lastItemRect.right + clamp(lastItemRect.width * 0.08, 4, 10)
        : dockBounds.right - 6;

      if (entrySideRef.current === 'left') {
        sideEntryUsesOuterSplit = true;
        sideEntryPivotIndex = leftPivotIndex;
        const pivotRect = itemRectsRef.current[leftPivotIndex] ?? null;
        const pivotLockX = pivotRect
          ? pivotRect.left + pivotRect.width / 2
          : leftExitX;
        const previousLock = horizontalSideLockRef.current;
        let nextLock = previousLock;

        if (justEnteredHorizontalRef.current) {
          justEnteredHorizontalRef.current = false;
        } else if (
          previousLock !== 'left' &&
          deltaX >= 0 &&
          currentX >= pivotLockX
        ) {
          nextLock = 'left';
        } else if (previousLock === 'left' && deltaX <= 0 && currentX <= leftReturnX) {
          nextLock = null;
        }

        pendingSideLock = nextLock;
        shouldSnapHorizontalAnchor = nextLock !== previousLock;
        targetHorizontalAnchorMode = nextLock === 'left' ? 'left-fixed' : 'right-fixed';
        horizontalTravelPhaseRef.current =
          nextLock === null
            ? 'entry'
            : currentX >= rightExitSwitchToExitX
              ? 'exit'
              : 'middle';
      } else if (entrySideRef.current === 'right') {
        sideEntryUsesOuterSplit = true;
        sideEntryPivotIndex = rightPivotIndex;
        const pivotRect = itemRectsRef.current[rightPivotIndex] ?? null;
        const pivotLockX = pivotRect
          ? pivotRect.left + pivotRect.width / 2
          : rightExitX;
        const previousLock = horizontalSideLockRef.current;
        let nextLock = previousLock;

        if (justEnteredHorizontalRef.current) {
          justEnteredHorizontalRef.current = false;
        } else if (
          previousLock !== 'right' &&
          deltaX <= 0 &&
          currentX <= pivotLockX
        ) {
          nextLock = 'right';
        } else if (previousLock === 'right' && deltaX >= 0 && currentX >= rightReturnX) {
          nextLock = null;
        }

        pendingSideLock = nextLock;
        shouldSnapHorizontalAnchor = nextLock !== previousLock;
        targetHorizontalAnchorMode = nextLock === 'right' ? 'right-fixed' : 'left-fixed';
        horizontalTravelPhaseRef.current =
          nextLock === null
            ? 'entry'
            : currentX <= leftExitSwitchToExitX
              ? 'exit'
              : 'middle';
      }
    } else if (pendingHorizontalReleaseRef.current) {
      horizontalTravelPhaseRef.current = 'release';
      targetHorizontalAnchorMode = horizontalAnchorModeRef.current;
      targetHorizontalAnchorBias = currentHorizontalAnchorBiasRef.current;
      if (sideEntryReleaseSplitBlend > 0.001) {
        sideEntryUsesOuterSplit = true;
        sideEntryPivotIndex =
          entrySideRef.current === 'left'
            ? defaultLeftPivotIndex
            : defaultRightPivotIndex;
      }
    } else if (targetHorizontalAnchorBias > 0.12) {
      horizontalTravelPhaseRef.current = 'idle';
      targetHorizontalAnchorMode = 'left-fixed';
    } else if (targetHorizontalAnchorBias < -0.12) {
      horizontalTravelPhaseRef.current = 'idle';
      targetHorizontalAnchorMode = 'right-fixed';
    } else {
      horizontalTravelPhaseRef.current = 'idle';
    }

    targetHorizontalAnchorBias = getHorizontalBiasForMode(targetHorizontalAnchorMode);

    if (
      targetHorizontalAnchorMode !== horizontalAnchorModeRef.current ||
      fixedHorizontalEdgeRef.current.mode !== targetHorizontalAnchorMode ||
      (
        targetHorizontalAnchorMode !== 'center' &&
        fixedHorizontalEdgeRef.current.position === null
      )
    ) {
      captureHorizontalFixedEdge(targetHorizontalAnchorMode);
    }
    horizontalAnchorModeRef.current = targetHorizontalAnchorMode;
    const horizontalAnchorBiasDiff =
      targetHorizontalAnchorBias - currentHorizontalAnchorBiasRef.current;
    const horizontalAnchorBiasFactor =
      entrySideRef.current === null
        ? 0.16
        : Math.abs(targetHorizontalAnchorBias) > 0.92
          ? 0.28
          : 0.22;
    const maxHorizontalAnchorBiasStep =
      entrySideRef.current === null ? 0.18 : 0.26;
    const horizontalAnchorBias = shouldSnapHorizontalAnchor ||
      Math.abs(horizontalAnchorBiasDiff) < 0.01
        ? targetHorizontalAnchorBias
        : currentHorizontalAnchorBiasRef.current +
            clamp(
              horizontalAnchorBiasDiff * horizontalAnchorBiasFactor,
              -maxHorizontalAnchorBiasStep,
              maxHorizontalAnchorBiasStep,
            );
    currentHorizontalAnchorBiasRef.current = horizontalAnchorBias;
    const centerStableStartRect =
      itemRectsRef.current[Math.min(2, Math.max(itemRectsRef.current.length - 1, 0))] ?? null;
    const centerStableEndRect =
      itemRectsRef.current[Math.max(itemRectsRef.current.length - 3, 0)] ?? null;
    const isInCenteredStableZone =
      !isLeft &&
      !isRight &&
      !pendingHorizontalReleaseRef.current &&
      targetHorizontalAnchorMode === 'center' &&
      (entrySideRef.current === 'top' || entrySideRef.current === 'bottom' || entrySideRef.current === null) &&
      localPointerX !== null &&
      itemRectsRef.current.length >= 5 &&
      !!centerStableStartRect &&
      !!centerStableEndRect &&
      centerStableStartRect.right < centerStableEndRect.left &&
      localPointerX >= centerStableStartRect.right &&
      localPointerX <= centerStableEndRect.left;
    const isHorizontalSideEntryActive =
      !isLeft &&
      !isRight &&
      !pendingHorizontalReleaseRef.current &&
      (entrySideRef.current === 'left' || entrySideRef.current === 'right');
    const isInSideEntryStableZone =
      isHorizontalSideEntryActive &&
      localPointerX !== null &&
      horizontalTravelPhaseRef.current === 'middle' &&
      horizontalSideLockRef.current !== null;
    const shouldLockHorizontalExtraWidth =
      isInCenteredStableZone || isInSideEntryStableZone;
    const rawItemExtraSpaces = itemsRef.current.map((item, i) => {
      const scale = Number(currentScales.current[i] ?? minScale);
      const itemSize = item.offsetWidth || 40;
      return Math.max(0, (scale - 1) * itemSize);
    });
    const rawTotalHorizontalExtraWidth = rawItemExtraSpaces.reduce((sum, value) => sum + value, 0);
    let itemExtraWidthNormalizationFactor = 1;
    let totalHorizontalExtraWidth = rawTotalHorizontalExtraWidth;

    if (shouldLockHorizontalExtraWidth) {
      const lockedExtraWidth = Math.max(
        centeredHorizontalExtraWidthLockRef.current ?? 0,
        rawTotalHorizontalExtraWidth,
      );
      centeredHorizontalExtraWidthLockRef.current = lockedExtraWidth;
      if (rawTotalHorizontalExtraWidth > 0.001) {
        itemExtraWidthNormalizationFactor = lockedExtraWidth / rawTotalHorizontalExtraWidth;
      }
      totalHorizontalExtraWidth = lockedExtraWidth;
    } else {
      centeredHorizontalExtraWidthLockRef.current = null;
    }

    const effectiveGapScales = new Float32Array(gapElementsRef.current.length);
    if (enableGapScaling && gapElementsRef.current.length > 0) {
      let rawGapExtraTotal = 0;
      for (let i = 0; i < gapElementsRef.current.length; i++) {
        const gapScale = Number(currentGapScales.current[i] ?? 1);
        rawGapExtraTotal += Math.max(0, gapScale - 1);
      }

      let gapExtraNormalizationFactor = 1;
      if (shouldLockHorizontalExtraWidth) {
        const lockedGapExtra = Math.max(
          centeredHorizontalGapExtraLockRef.current ?? 0,
          rawGapExtraTotal,
        );
        centeredHorizontalGapExtraLockRef.current = lockedGapExtra;
        if (rawGapExtraTotal > 0.001) {
          gapExtraNormalizationFactor = lockedGapExtra / rawGapExtraTotal;
        }
      } else {
        centeredHorizontalGapExtraLockRef.current = null;
      }

      for (let i = 0; i < gapElementsRef.current.length; i++) {
        const currentGapScale = Number(currentGapScales.current[i] ?? 1);
        effectiveGapScales[i] =
          1 + Math.max(0, currentGapScale - 1) * gapExtraNormalizationFactor;
      }
    } else {
      centeredHorizontalGapExtraLockRef.current = null;
    }

    let totalHorizontalGapExtraWidth = 0;
    let splitLeftGapDemand = 0;
    let splitRightGapDemand = 0;
    if (enableGapScaling && effectiveGapScales.length > 0) {
      for (let i = 0; i < effectiveGapScales.length; i++) {
        const baseGap = baseGapSizesRef.current[i] ?? 0;
        const gapExtraWidth = baseGap * Math.max(0, Number(effectiveGapScales[i] ?? 1) - 1);
        totalHorizontalGapExtraWidth += gapExtraWidth;

        if (sideEntryUsesOuterSplit && sideEntryPivotIndex !== null) {
          if (i < sideEntryPivotIndex) {
            splitLeftGapDemand += gapExtraWidth;
          } else {
            splitRightGapDemand += gapExtraWidth;
          }
        }
      }
    }

    const shouldLockCenteredHorizontalEdges =
      isInCenteredStableZone &&
      targetHorizontalAnchorMode === 'center' &&
      Math.abs(horizontalAnchorBias) < 0.12;
    if (!shouldLockCenteredHorizontalEdges) {
      centeredHorizontalOuterBeforeLockRef.current = null;
      centeredHorizontalOuterAfterLockRef.current = null;
    }

    const lastItemIndex = itemsRef.current.length - 1;
    let appliedHorizontalExtraWidth = 0;
    let assignedLeftOuterDemand = 0;
    let assignedRightOuterDemand = 0;

    itemsRef.current.forEach((item, i) => {
      const scale = Number(currentScales.current[i]);

      // 计算图标放大后需要的额外空间（单侧）
      // 使用图标实际宽度计算
      const totalExtraSpace = rawItemExtraSpaces[i]! * itemExtraWidthNormalizationFactor;
      const usesSideEntryOuterSplit =
        sideEntryUsesOuterSplit &&
        sideEntryPivotIndex !== null;
      let extraBefore = totalExtraSpace * (1 - horizontalAnchorBias) / 2;
      let extraAfter = totalExtraSpace * (1 + horizontalAnchorBias) / 2;

      if (usesSideEntryOuterSplit) {
        let splitExtraBefore = totalExtraSpace / 2;
        let splitExtraAfter = totalExtraSpace / 2;
        if (i < sideEntryPivotIndex) {
          splitExtraBefore = totalExtraSpace;
          splitExtraAfter = 0;
        } else if (i > sideEntryPivotIndex) {
          splitExtraBefore = 0;
          splitExtraAfter = totalExtraSpace;
        }

        if (pendingHorizontalReleaseRef.current && sideEntryReleaseSplitBlend > 0.001) {
          extraBefore =
            extraBefore * (1 - sideEntryReleaseSplitBlend) +
            splitExtraBefore * sideEntryReleaseSplitBlend;
          extraAfter =
            extraAfter * (1 - sideEntryReleaseSplitBlend) +
            splitExtraAfter * sideEntryReleaseSplitBlend;
        } else {
          extraBefore = splitExtraBefore;
          extraAfter = splitExtraAfter;
        }
      }

      if (shouldLockCenteredHorizontalEdges) {
        if (i === 0) {
          const lockedOuterBefore = Math.max(
            centeredHorizontalOuterBeforeLockRef.current ?? 0,
            extraBefore,
          );
          centeredHorizontalOuterBeforeLockRef.current =
            Math.round(lockedOuterBefore * 2) / 2;
          extraBefore = centeredHorizontalOuterBeforeLockRef.current;
        }

        if (i === lastItemIndex) {
          const lockedOuterAfter = Math.max(
            centeredHorizontalOuterAfterLockRef.current ?? 0,
            extraAfter,
          );
          centeredHorizontalOuterAfterLockRef.current =
            Math.round(lockedOuterAfter * 2) / 2;
          extraAfter = centeredHorizontalOuterAfterLockRef.current;
        }
      }

      if (usesSideEntryOuterSplit) {
        assignedLeftOuterDemand += extraBefore;
        assignedRightOuterDemand += extraAfter;
      }

      appliedHorizontalExtraWidth += extraBefore + extraAfter;

      // 计算上移距离：基于缩放比例，最大上移30像素
      // 当 scale = 1 时，moveOffset = 0；当 scale = maxScale 时，moveOffset = 30
      const scaleRatio = maxScale > 1 ? (scale - 1) / (maxScale - 1) : 0;
      const maxMoveOffset = 8; // 最大上移像素
      const moveOffset = scaleRatio * maxMoveOffset;

      // 使用 transform 只做缩放，位移用 margin 实现
      let marginTop = '0px';
      let marginBottom = '0px';
      let marginLeft = '0px';
      let marginRight = '0px';

      if (isBottom || isDefaultBottom) {
        // 底部Dock：图标向上移动（用负margin-bottom实现，因为align-items: flex-end）
        marginBottom = `${moveOffset}px`;
      } else if (isTop) {
        // 顶部Dock：图标向下移动
        marginTop = `${moveOffset}px`;
      } else if (isLeft) {
        // 左侧Dock：图标向右移动
        marginLeft = `${moveOffset}px`;
      } else if (isRight) {
        // 右侧Dock：图标向左移动
        marginRight = `${moveOffset}px`;
      }

      // 只用 transform 做缩放
      item.style.transform = `scale(${scale})`;
      // 用 margin 实现位移
      let transformOrigin = 'center center';

      if (usesSideEntryOuterSplit) {
        if (i < sideEntryPivotIndex) {
          transformOrigin = 'right center';
        } else if (i > sideEntryPivotIndex) {
          transformOrigin = 'left center';
        }
      } else if (!isLeft && !isRight) {
        if (horizontalAnchorBias > 0.35) {
          transformOrigin = 'left center';
        } else if (horizontalAnchorBias < -0.35) {
          transformOrigin = 'right center';
        }
      }

      item.style.transformOrigin = transformOrigin;
      item.style.marginTop = marginTop;
      item.style.marginBottom = marginBottom;
      item.style.marginLeft = marginLeft;
      item.style.marginRight = marginRight;
      item.style.zIndex = scale > 1.05 ? '10' : '1';

      // 给容器设置 CSS 变量，让 CSS 中的 margin 可以动态调整
      const dragContainer = item.closest('.taskbar-item-drag-container') as HTMLElement;
      if (dragContainer) {
        dragContainer.style.setProperty('--mag-extra-before', `${extraBefore}px`);
        dragContainer.style.setProperty('--mag-extra-after', `${extraAfter}px`);
      }
    });

    if (enableGapScaling) {
      gapElementsRef.current.forEach((gap, i) => {
        const gapScale = Number(effectiveGapScales[i] || currentGapScales.current[i] || 1);
        gap.style.setProperty('--gap-scale', gapScale.toString());
      });
    }

    if (sideEntryUsesOuterSplit) {
      assignedLeftOuterDemand += splitLeftGapDemand;
      assignedRightOuterDemand += splitRightGapDemand;
    }

    let effectiveFixedHorizontalEdgePosition = fixedHorizontalEdgeRef.current.position;
    let horizontalReleaseTailBoost = 0;
    let horizontalReleaseOffsetBlend = 1;
    const resolvedHorizontalExtraWidth =
      totalHorizontalGapExtraWidth +
      Math.max(totalHorizontalExtraWidth, appliedHorizontalExtraWidth);
    if (
      sideEntryUsesOuterSplit &&
      !pendingHorizontalReleaseRef.current &&
      pendingSideLock !== horizontalSideLockRef.current
    ) {
      const currentEntryOuterDemand =
        entrySideRef.current === 'left'
          ? assignedLeftOuterDemand
          : assignedRightOuterDemand;
      horizontalSideLockRef.current = pendingSideLock;
      if (pendingSideLock === null) {
        horizontalLockedOuterExtraRef.current = 0;
        horizontalEntryOuterPeakRef.current = currentEntryOuterDemand;
      } else {
        horizontalLockedOuterExtraRef.current = currentEntryOuterDemand;
        horizontalEntryOuterPeakRef.current = currentEntryOuterDemand;
      }
    }

    if (sideEntryUsesOuterSplit && !pendingHorizontalReleaseRef.current) {
      const currentEntryOuterDemand =
        entrySideRef.current === 'left'
          ? assignedLeftOuterDemand
          : assignedRightOuterDemand;
      horizontalEntryOuterPeakRef.current =
        horizontalSideLockRef.current === null
          ? currentEntryOuterDemand
          : horizontalLockedOuterExtraRef.current;
    }
    if (
      pendingHorizontalReleaseRef.current &&
      horizontalReleaseStartEdgeRef.current !== null &&
      horizontalReleaseTargetEdgeRef.current !== null
    ) {
      const releaseStartExtraWidth =
        horizontalReleaseStartExtraWidthRef.current ??
        Math.max(resolvedHorizontalExtraWidth, 0.0001);
      horizontalReleaseStartExtraWidthRef.current = releaseStartExtraWidth;
      const releaseProgress =
        1 - clamp(resolvedHorizontalExtraWidth / releaseStartExtraWidth, 0, 1);
      horizontalReleaseProgressRef.current = releaseProgress;
      horizontalReleaseTailBoost = getSmoothTransitionProgress(releaseProgress, 0.8, 1);
      horizontalReleaseOffsetBlend = getSmoothTransitionProgress(releaseProgress, 0.08, 0.42);
      effectiveFixedHorizontalEdgePosition =
        horizontalReleaseStartEdgeRef.current +
        (horizontalReleaseTargetEdgeRef.current - horizontalReleaseStartEdgeRef.current) *
          releaseProgress;
    } else {
      horizontalReleaseProgressRef.current = 0;
    }

    const currentVisualBounds = readDockBounds();
    const effectiveHorizontalExpansion =
      dockBounds && currentVisualBounds
        ? Math.max(
          resolvedHorizontalExtraWidth,
          Math.max(0, currentVisualBounds.width - dockBounds.width),
        )
        : resolvedHorizontalExtraWidth;
    let targetHorizontalOffset = resolvedHorizontalExtraWidth * horizontalAnchorBias / 2;
    if (sideEntryUsesOuterSplit && !pendingHorizontalReleaseRef.current) {
      let targetLeftOuterExpansion = assignedLeftOuterDemand;
      if (entrySideRef.current === 'left' && horizontalSideLockRef.current === 'left') {
        targetLeftOuterExpansion = Math.min(
          resolvedHorizontalExtraWidth,
          horizontalLockedOuterExtraRef.current,
        );
      } else if (entrySideRef.current === 'right' && horizontalSideLockRef.current === 'right') {
        targetLeftOuterExpansion = Math.max(
          0,
          resolvedHorizontalExtraWidth -
            Math.min(resolvedHorizontalExtraWidth, horizontalLockedOuterExtraRef.current),
        );
      }
      // Without translation, width growth expands around the dock center.
      // To realize a desired "left outer" expansion, we only need to offset by
      // the delta between the centered growth (total / 2) and the left-side demand.
      targetHorizontalOffset =
        resolvedHorizontalExtraWidth / 2 - targetLeftOuterExpansion;
    } else if (dockBounds && effectiveFixedHorizontalEdgePosition !== null) {
      if (fixedHorizontalEdgeRef.current.mode === 'left-fixed') {
        targetHorizontalOffset =
          effectiveFixedHorizontalEdgePosition - dockBounds.left + resolvedHorizontalExtraWidth / 2;
      } else if (fixedHorizontalEdgeRef.current.mode === 'right-fixed') {
        targetHorizontalOffset =
          effectiveFixedHorizontalEdgePosition -
          dockBounds.right -
          resolvedHorizontalExtraWidth / 2;
      }
    }

    if (shouldLockCenteredHorizontalEdges) {
      targetHorizontalOffset = 0;
    }

    if (
      pendingHorizontalReleaseRef.current &&
      horizontalReleaseStartOffsetRef.current !== null
    ) {
      targetHorizontalOffset =
        horizontalReleaseStartOffsetRef.current +
        (targetHorizontalOffset - horizontalReleaseStartOffsetRef.current) *
          horizontalReleaseOffsetBlend;
    }

    if (
      fixedHorizontalEdgeRef.current.mode !== 'center' ||
      pendingHorizontalReleaseRef.current
    ) {
      const maxReasonableHorizontalOffset = effectiveHorizontalExpansion / 2 + 12;
      targetHorizontalOffset = clamp(
        targetHorizontalOffset,
        -maxReasonableHorizontalOffset,
        maxReasonableHorizontalOffset,
      );
    }

    const horizontalOffsetDiff =
      targetHorizontalOffset - currentHorizontalOffsetRef.current;
    const horizontalOffsetFactor = pendingHorizontalReleaseRef.current
      ? clamp(
          smoothFactor * (1.5 + horizontalReleaseTailBoost * 0.65),
          0.28,
          0.64,
        )
      : shouldSnapHorizontalAnchor
        ? clamp(smoothFactor * 1.4, 0.28, 0.55)
        : clamp(smoothFactor, 0.16, 0.32);
    const maxHorizontalOffsetStep = pendingHorizontalReleaseRef.current
      ? 26 + horizontalReleaseTailBoost * 8
      : shouldSnapHorizontalAnchor
        ? 20
        : 10;
    const nextHorizontalOffset =
      shouldLockCenteredHorizontalEdges
        ? 0
        : Math.abs(horizontalOffsetDiff) < 0.05
        ? targetHorizontalOffset
        : currentHorizontalOffsetRef.current +
            clamp(
              horizontalOffsetDiff * horizontalOffsetFactor,
              -maxHorizontalOffsetStep,
              maxHorizontalOffsetStep,
            );

    const releaseFinishThreshold =
      horizontalReleaseTailBoost > 0.01 ? 0.75 : 0.5;
    if (
      pendingHorizontalReleaseRef.current &&
      resolvedHorizontalExtraWidth < releaseFinishThreshold
    ) {
      finalizeHorizontalRelease();
      applyHorizontalDockOffset(0);
      visualDockBoundsRef.current = readDockBounds() ?? dockBoundsRef.current;
      return;
    }
    applyHorizontalDockOffset(nextHorizontalOffset);
    visualDockBoundsRef.current = readDockBounds() ?? dockBoundsRef.current;
  };

  const tick = () => {
    rafId.current = 0;
    const { x, y } = pointer.current;
    const localHorizontalX =
      x === null ? null : x - currentHorizontalOffsetRef.current;
    const itemCount = itemsRef.current.length;
    const gapCount = gapElementsRef.current.length;

    // 优化：缓存Dock方向检测，避免每帧都查询DOM
    // 使用闭包变量缓存，只有容器变化时才重新检测
    const container = containerRef.current;

    // 检测Dock方向，决定使用x还是y坐标
    const taskbar = container?.closest('.taskbar');
    const isVertical = taskbar?.classList.contains('left') || taskbar?.classList.contains('right');
    const isBottom = taskbar?.classList.contains('bottom');
    const isTop = taskbar?.classList.contains('top');
    const isLeft = taskbar?.classList.contains('left');
    const isRight = taskbar?.classList.contains('right');
    const dockBounds = dockBoundsRef.current ?? readDockBounds();
    const isHorizontalDockTopBottomEntry =
      !isVertical &&
      !isLeft &&
      !isRight &&
      (entrySideRef.current === 'top' || entrySideRef.current === 'bottom') &&
      !pendingHorizontalReleaseRef.current;
    const isPointerInsideHorizontalDockBand =
      !!dockBounds &&
      y !== null &&
      y >= dockBounds.top &&
      y <= dockBounds.bottom;

    // 计算鼠标与Dock栏的垂直距离，用于衰减效果
    let distanceAttenuation =  1;  // 1 = 完全效果, 0 = 无效果
    const fadeOutDistance = 60;    // 衰减距离（超过此距离效果完全消失）
    const fadeStartDistance = 12;  // 离开Dock多远才开始衰减（像素）

    if (x !== null && y !== null && dockBounds) {
      let distanceFromDock = 0;

      if (isBottom) {
        // 底部Dock：只在鼠标向上移出时衰减，向下移动时保持效果
        if (y < dockBounds.top) {
          distanceFromDock = dockBounds.top - y;
        }
        // 向下移动不衰减，保持效果
      } else if (isTop) {
        // 顶部Dock：只在鼠标向下移出时衰减，向上移动时保持效果
        if (y > dockBounds.bottom) {
          distanceFromDock = y - dockBounds.bottom;
        }
        // 向上移动不衰减，保持效果
      } else if (isLeft) {
        // 左侧Dock: 只在鼠标向右移出时衰减，向左移动时保持效果
        if (x  > dockBounds.right) {
          distanceFromDock = x - dockBounds.right;
        }
        // 向左移动不衰减，保持效果
      } else if (isRight) {
        // 右侧Dock: 只在鼠标向左移出时衰减，向右移动时保持效果
        if (x < dockBounds.left) {
          distanceFromDock = dockBounds.left - x;
        }
        // 向右移动不衰减，保持效果
      }

      distanceAttenuation = verticalAttenuation(distanceFromDock);
    }

    const horizontalPointerWaveBias =
      !isVertical && !isLeft && !isRight ? -1.5 : 0;
    const firstHorizontalCenter = centersRef.current[0]?.x ?? null;
    const lastHorizontalCenter =
      centersRef.current[centersRef.current.length - 1]?.x ?? null;
    const currentDockOffset = currentHorizontalOffsetRef.current;
    const firstHorizontalLeft =
      itemsRef.current[0]
        ? itemsRef.current[0]!.getBoundingClientRect().left - currentDockOffset
        : itemRectsRef.current[0]?.left ?? firstHorizontalCenter;
    const lastHorizontalRight =
      itemsRef.current[itemCount - 1]
        ? itemsRef.current[itemCount - 1]!.getBoundingClientRect().right - currentDockOffset
        : itemRectsRef.current[itemCount - 1]?.right ?? lastHorizontalCenter;
    const getEffectiveHorizontalPointerPos = (rawPointerPos: number): number => {
      if (isVertical || isLeft || isRight) {
        return rawPointerPos;
      }

      if (
        entrySideRef.current === 'left' &&
        horizontalTravelPhaseRef.current === 'exit' &&
        lastHorizontalRight !== null
      ) {
        return Math.min(rawPointerPos, lastHorizontalRight);
      }

      if (
        entrySideRef.current === 'right' &&
        horizontalTravelPhaseRef.current === 'exit' &&
        firstHorizontalLeft !== null
      ) {
        return Math.max(rawPointerPos, firstHorizontalLeft);
      }

      return rawPointerPos;
    };

    for (let i = 0; i < itemCount; i++) {
      if (x === null || y === null) {
        targetScales.current[i] = minScale;
      } else {
        const center = centersRef.current[i];
        if (!center) continue;

        // 根据Dock方向选择使用x或y坐标
        const pointerPos = isVertical
          ? y
          : getEffectiveHorizontalPointerPos(
              (localHorizontalX ?? x) + horizontalPointerWaveBias,
            );
        const itemPos = isVertical ? center.y : center.x;

        let scale = minScale;

        if (zoomEffectType === 'wave') {
          // 波浪效果：使用余弦波算法，多个图标都有缩放效果
          const rawScale = calculateCosineScale({
              curveCenter: pointerPos,
              itemPosition: itemPos,
              curveRange: hoverThreshold,
              minScale,
              maxScale,
          });

          // 应用距离衰减
          const easedAttenuation = Math.pow(distanceAttenuation, 1.5);
          scale = minScale + (rawScale - minScale) * easedAttenuation;
        } else if (zoomEffectType === 'singleIcon') {
          // 单图标效果：只让鼠标悬停的图标有缩放效果
          const distance = Math.abs(itemPos - pointerPos);
          if (distance < hoverThreshold / 2) {
            // 计算鼠标与图标中心的距离，只对最近的图标应用缩放
            const normalizedDistance = distance / (hoverThreshold / 2);
            const easedAttenuation = Math.pow(distanceAttenuation, 1.5);
            scale = minScale + (maxScale - minScale) * (1 - normalizedDistance) * easedAttenuation;
          }
        }

        targetScales.current[i] = scale;
      }
    }

    if (enableGapScaling && gapCount > 0 && zoomEffectType === 'wave') {
      // 只有波浪效果才对 gap 元素应用缩放
      for (let i = 0; i < gapCount; i++) {
        if (x === null || y === null) {
          targetGapScales.current[i] = minScale;
        } else {
          const gapCenter = gapCentersRef.current[i];
          if (!gapCenter) continue;

          const pointerPos = isVertical
            ? y
            : getEffectiveHorizontalPointerPos(
                (localHorizontalX ?? x) + horizontalPointerWaveBias,
              );
          const gapPos = isVertical ? gapCenter.y : gapCenter.x;

          const rawGapScale = calculateCosineScale({
            curveCenter: pointerPos,
            itemPosition: gapPos,
            curveRange: hoverThreshold,
            minScale,
            maxScale,
          });

          // 应用距离衰减
          const gapScale = minScale + (rawGapScale - minScale) * distanceAttenuation;
          targetGapScales.current[i] = gapScale;
        }
      }
    } else if (gapCount > 0) {
      // 单图标效果时，gap 元素保持最小缩放
      for (let i = 0; i < gapCount; i++) {
        targetGapScales.current[i] = minScale;
      }
    }

    let needsUpdate = false;

    // 丝滑缩放：使用缓动因子，接近目标时速度变慢
    // 优化：将函数移出循环外定义，避免重复创建
    const getEasedFactor = (current: number, target: number, isEnlarging: boolean): number => {
      if (isEnlarging) {
        if (isHorizontalDockTopBottomEntry && isPointerInsideHorizontalDockBand) {
          return 0.6;
        }
        // 放大：使用较快的响应
        return 0.22;
      }

      // 缩小：实现ease-out效果，接近目标时减速
      const diff = Math.abs(current - target);
      const maxDiff = maxScale - minScale; // 最大可能差值
      const normalizedDiff = diff / maxDiff; // 归一化到0-1

      // 基础因子 + 根据距离调整的因子
      // 距离越大，速度越快；距离越小，速度越慢
      const isHorizontalRelease =
        pendingHorizontalReleaseRef.current && !isLeft && !isRight;
      const releaseTailBoost = isHorizontalRelease
        ? getSmoothTransitionProgress(horizontalReleaseProgressRef.current, 0.8, 1)
        : 0;
      const baseFactor = isHorizontalRelease
        ? 0.13 + releaseTailBoost * 0.08
        : 0.06; // 基础最小速度
      const dynamicFactor =
        (isHorizontalRelease ? 0.17 + releaseTailBoost * 0.08 : 0.12) *
        Math.pow(normalizedDiff, 0.5); // 使用平方根让减速更平滑

      return baseFactor + dynamicFactor;
    };

    for (let i = 0; i < itemCount; i++) {
      const targetScale = targetScales.current[i] ?? minScale;
      const currentScale = currentScales.current[i] ?? minScale;
      const scaleDiff = targetScale - currentScale;

      // 使用缓动因子实现丝滑效果
      const factor = getEasedFactor(currentScale, targetScale, scaleDiff > 0);
      currentScales.current[i]! += scaleDiff * factor;

      if (Math.abs(scaleDiff) > 0.001) {
        needsUpdate = true;
      }
    }

    if (enableGapScaling && gapCount > 0) {
      for (let i = 0; i < gapCount; i++) {
        const targetGapScale = targetGapScales.current[i] ?? minScale;
        const currentGapScale = currentGapScales.current[i] ?? minScale;
        const gapScaleDiff = targetGapScale - currentGapScale;

        // Gap元素也使用相同的缓动因子
        const factor = getEasedFactor(currentGapScale, targetGapScale, gapScaleDiff > 0);
        currentGapScales.current[i]! += gapScaleDiff * factor;

        if (Math.abs(gapScaleDiff) > 0.001) {
          needsUpdate = true;
        }
      }
    }

    // 优化：只有需要更新时才调用 updateDOM
    if (needsUpdate) {
      updateDOM();
      // 图标位置已变化，通知坐标追踪系统更新
      // 坐标追踪器内部有 200ms 防抖，不会每帧都写入文件
      window.dispatchEvent(new Event('taskbar-magnification-settled'));
    }

    // 优化：只有当真正需要继续动画时才请求下一帧
    const shouldContinue = needsUpdate || pendingHorizontalReleaseRef.current || horizontalLayoutAnimatingRef.current;
    if (shouldContinue) {
      rafId.current = requestAnimationFrame(tick);
    }
  };

  const startAnimation = () => {
    if (!rafId.current) {
      rafId.current = requestAnimationFrame(tick);
    }
  };

  const refreshItems = () => {
    const container = containerRef.current;
    if (!container) return;

    // 使用targetSelector过滤特定区域的图标
    const nextItems = Array.from(
      container.querySelectorAll<HTMLElement>(targetSelector),
    ).filter((item: HTMLElement) => {
      if (item.classList.contains('taskbar-separator')) {
        return false;
      }
      const style = globalThis.getComputedStyle(item);
      if (style.display === 'none' || style.visibility === 'hidden') {
        return false;
      }
      const rect = item.getBoundingClientRect();
      return rect.width > 0 && rect.height > 0;
    });
    const itemCountChanged =
      measuredItemCountRef.current !== 0 &&
      measuredItemCountRef.current !== nextItems.length;
    itemsRef.current = nextItems;

    if (enableGapScaling) {
      // 只获取中间区域的Gap元素
      gapElementsRef.current = Array.from(
        new Set(
          itemsRef.current
            .map((item) => item.closest('.taskbar-item-drag-container') as HTMLElement | null)
            .filter((dragContainer): dragContainer is HTMLElement => !!dragContainer),
        ),
      );
    }

    const len = itemsRef.current.length;
    const gapLen = gapElementsRef.current.length;

    currentScales.current = new Float32Array(len).fill(1);
    targetScales.current = new Float32Array(len).fill(1);

    if (enableGapScaling) {
      currentGapScales.current = new Float32Array(gapLen).fill(1);
      targetGapScales.current = new Float32Array(gapLen).fill(1);
    }

    if (itemCountChanged) {
      finalizeHorizontalRelease();
      applyHorizontalDockOffset(0);
    }

    measureCenters();
    measuredItemCountRef.current = len;
    if (isActive.current || pendingHorizontalReleaseRef.current) {
      startAnimation();
    }
  };

  useEffect(() => {
    const container = containerRef.current;
    if (!container || !enabled) {
      // 如果禁用，清除所有缩放效果
      if (container && !enabled) {
        // 🔧 关键修复：使用选择器查找所有匹配的图标，确保清除干净
        const allItems = container.querySelectorAll(targetSelector);
        allItems.forEach((item) => {
          const el = item as HTMLElement;
          el.style.transform = '';  // 清除内联 transform，让 CSS :hover 生效
          el.style.transformOrigin = '';
          el.style.marginTop = '';
          el.style.marginBottom = '';
          el.style.marginLeft = '';
          el.style.marginRight = '';
          el.style.zIndex = '';
          const dragContainer = el.closest('.taskbar-item-drag-container') as HTMLElement;
          if (dragContainer) {
            dragContainer.style.removeProperty('--mag-extra-before');
            dragContainer.style.removeProperty('--mag-extra-after');
            dragContainer.style.removeProperty('--gap-scale');
          }
        });
        resetMagnifierRuntimeState();
        applyHorizontalDockOffset(0);
      }
      return;
    }

    let el: HTMLElement | null = container;
    while (el) {
      el.style.overflow = 'visible';
      el = el.parentElement;
    }

    const webviewWindow = getCurrentWebviewWindow();
    let disposeGlobalMouse: (() => void) | null = null;

    const syncWindowScreenPosition = async () => {
      try {
        const pos = await webviewWindow.outerPosition();
        windowScreenPositionRef.current = { x: pos.x, y: pos.y };
      } catch (error) {
        console.warn('[DockMagnifier] Failed to read outerPosition:', error);
      }
    };

    void syncWindowScreenPosition();

    const unlistenWindowMoved = webviewWindow.onMoved((pos) => {
      windowScreenPositionRef.current = { x: pos.payload.x, y: pos.payload.y };
    });

    const globalMouseSubscription = subscribe(FuncEvent.GlobalMouseMove, ({ payload: [x, y] }) => {
      const screenDockBounds = readScreenDockBounds();
      const prevX = globalPointerRef.current.x;
      const prevY = globalPointerRef.current.y;
      const horizontalInteractionActive = isHorizontalDockInteractionActive(
        entrySideRef.current,
        horizontalTravelPhaseRef.current,
      );
      const currentClientX = (x - windowScreenPositionRef.current.x) / (globalThis.devicePixelRatio || 1);
      const farExitSide = detectHorizontalFarExitSide(
        currentClientX - currentHorizontalOffsetRef.current,
      );

      if (farExitSide) {
        beginHorizontalRelease(farExitSide);
        globalPointerRef.current = { x, y };
        startAnimation();
        return;
      }

      const wasInsideDock = !!(
        screenDockBounds &&
        prevX !== null &&
        prevY !== null &&
        isPointInsideDockInteractionBounds(
          prevX,
          prevY,
          screenDockBounds,
          horizontalInteractionActive,
        )
      );
      const isInsideDock = !!(
        screenDockBounds &&
        isPointInsideDockInteractionBounds(
          x,
          y,
          screenDockBounds,
          horizontalInteractionActive,
        )
      );

      if (screenDockBounds && isInsideDock && !wasInsideDock) {
        resetHorizontalReleaseState();
        entrySideRef.current =
          prevX !== null && prevY !== null
            ? detectDockEntrySide(prevX, prevY, screenDockBounds) ?? detectDockEntrySideFromInside(x, y, screenDockBounds)
            : detectDockEntrySideFromInside(x, y, screenDockBounds);
        syncHorizontalStateWithPointerEntry(
          entrySideRef.current,
          (x - windowScreenPositionRef.current.x) / (globalThis.devicePixelRatio || 1),
          dockBoundsRef.current ?? readDockBounds(),
        );
      } else if (screenDockBounds && isInsideDock && entrySideRef.current === null) {
        resetHorizontalReleaseState();
        entrySideRef.current = detectDockEntrySideFromInside(x, y, screenDockBounds);
        syncHorizontalStateWithPointerEntry(
          entrySideRef.current,
          (x - windowScreenPositionRef.current.x) / (globalThis.devicePixelRatio || 1),
          dockBoundsRef.current ?? readDockBounds(),
        );
      } else if (screenDockBounds && !isInsideDock && wasInsideDock) {
        const exitSide =
          detectDockEntrySide(x, y, screenDockBounds) ??
          detectNearestDockSide(x, y, screenDockBounds);
        beginHorizontalRelease(exitSide);
      }

      globalPointerRef.current = { x, y };
    }).then((unsubscribe) => {
      if (typeof unsubscribe === 'function') {
        disposeGlobalMouse = unsubscribe;
      }
    }).catch((error) => {
      console.warn('[DockMagnifier] Failed to subscribe GlobalMouseMove:', error);
    });

    refreshItems();
    setTimeout(measureCenters, 100);
    if (measureTimeoutRef.current !== null) {
      clearTimeout(measureTimeoutRef.current);
    }
    measureTimeoutRef.current = window.setTimeout(() => {
      measureCenters();
      measureTimeoutRef.current = null;
      if (isActive.current || pendingHorizontalReleaseRef.current) {
        startAnimation();
      }
    }, 100);

    const observer = new MutationObserver(() => {
      refreshItems();
      requestAnimationFrame(() => {
        measureCenters();
      });
    });
    observer.observe(container, { childList: true, subtree: true });

    const taskbarElement = container.closest('.taskbar') as HTMLElement | null;
    let wasTaskbarHidden = taskbarElement?.classList.contains('hidden') ?? false;
    let wasWhiteBackplate = taskbarElement?.classList.contains('white-backplate') ?? false;
    const taskbarClassObserver = new MutationObserver(() => {
      if (!taskbarElement) return;

      const isTaskbarHidden = taskbarElement.classList.contains('hidden');
      const isWhiteBackplate = taskbarElement.classList.contains('white-backplate');
      const becameVisible = wasTaskbarHidden && !isTaskbarHidden;
      const backplateChanged = wasWhiteBackplate !== isWhiteBackplate;

      wasTaskbarHidden = isTaskbarHidden;
      wasWhiteBackplate = isWhiteBackplate;

      if (becameVisible || backplateChanged) {
        scheduleStructuralLayoutRefresh();
      }
    });
    if (taskbarElement) {
      taskbarClassObserver.observe(taskbarElement, {
        attributes: true,
        attributeFilter: ['class'],
      });
    }

    const handleBackplateStyleChanged = () => {
      scheduleStructuralLayoutRefresh();
    };
    window.addEventListener('backplate-style-changed', handleBackplateStyleChanged);

    const handleDisplayLayoutChanged = () => {
      void syncWindowScreenPosition();
      scheduleStructuralLayoutRefresh([0, 100, 300, 800, 1500]);
    };

    const taskbarContainerRefreshSubscription = subscribe(
      FuncEvent.TaskbarContainerRefresh,
      handleDisplayLayoutChanged,
    );

    const handleMove = (e: PointerEvent) => {
      const dockBounds = dockBoundsRef.current ?? readDockBounds();
      const screenDockBounds = readScreenDockBounds();
      const horizontalInteractionActive = isHorizontalDockInteractionActive(
        entrySideRef.current,
        horizontalTravelPhaseRef.current,
      );
      const isInsideDock = !!(
        dockBounds &&
        isPointInsideDockInteractionBounds(
          e.clientX,
          e.clientY,
          dockBounds,
          horizontalInteractionActive,
        )
      );
      const dpr = globalThis.devicePixelRatio || 1;
      const currentScreenX = windowScreenPositionRef.current.x + e.clientX * dpr;
      const currentScreenY = windowScreenPositionRef.current.y + e.clientY * dpr;
      const farExitSide = detectHorizontalFarExitSide(
        e.clientX - currentHorizontalOffsetRef.current,
      );

      if (farExitSide) {
        beginHorizontalRelease(farExitSide);
        globalPointerRef.current = { x: currentScreenX, y: currentScreenY };
        startAnimation();
        return;
      }

      const prevGlobalX = globalPointerRef.current.x;
      const prevGlobalY = globalPointerRef.current.y;
      const wasInsideScreenDock = !!(
        screenDockBounds &&
        prevGlobalX !== null &&
        prevGlobalY !== null &&
        isPointInsideDockInteractionBounds(
          prevGlobalX,
          prevGlobalY,
          screenDockBounds,
          horizontalInteractionActive,
        )
      );
      const isInsideScreenDock = !!(
        screenDockBounds &&
        isPointInsideDockInteractionBounds(
          currentScreenX,
          currentScreenY,
          screenDockBounds,
          horizontalInteractionActive,
        )
      );

      if (screenDockBounds && isInsideScreenDock && !wasInsideScreenDock) {
        resetHorizontalReleaseState();
        entrySideRef.current =
          prevGlobalX !== null && prevGlobalY !== null
            ? detectDockEntrySide(prevGlobalX, prevGlobalY, screenDockBounds) ??
              detectDockEntrySideFromInside(currentScreenX, currentScreenY, screenDockBounds)
            : detectDockEntrySideFromInside(currentScreenX, currentScreenY, screenDockBounds);
        syncHorizontalStateWithPointerEntry(
          entrySideRef.current,
          e.clientX,
          dockBounds,
        );
      } else if (dockBounds && isInsideDock && entrySideRef.current === null) {
        resetHorizontalReleaseState();
        entrySideRef.current = detectDockEntrySideFromInside(e.clientX, e.clientY, dockBounds);
        syncHorizontalStateWithPointerEntry(
          entrySideRef.current,
          e.clientX,
          dockBounds,
        );
      } else if (screenDockBounds && !isInsideScreenDock && wasInsideScreenDock) {
        const exitSide =
          detectDockEntrySide(currentScreenX, currentScreenY, screenDockBounds) ??
          detectNearestDockSide(currentScreenX, currentScreenY, screenDockBounds);
        beginHorizontalRelease(exitSide);
      }
      // 如果有待执行的清除定时器，取消它
      if (leaveTimeoutRef.current !== null) {
        clearTimeout(leaveTimeoutRef.current);
        leaveTimeoutRef.current = null;
      }

      // 优化：如果鼠标位置没有变化，不启动新动画
      const lastX = pointer.current.x;
      const lastY = pointer.current.y;

      lastPointer.current.x = lastX;
      lastPointer.current.y = lastY;
      pointer.current.x = e.clientX;
      pointer.current.y = e.clientY;
      globalPointerRef.current = { x: currentScreenX, y: currentScreenY };
      pointerMovedRef.current = true;

      // 只有当鼠标位置变化或动画未运行时才启动动画
      const hasMoved = lastX !== e.clientX || lastY !== e.clientY;
      isActive.current = true;

      if (hasMoved || !rafId.current) {
        startAnimation();
      }
    };

    const handleLeave = () => {
      // 延迟150ms再清除状态，给用户短暂移出窗口的容错时间
      if (leaveTimeoutRef.current !== null) {
        clearTimeout(leaveTimeoutRef.current);
      }
      leaveTimeoutRef.current = window.setTimeout(() => {
        // 检查全局鼠标位置是否仍在 dock 区域内（即使 webview 失去指针焦点），
        // 避免点击图标激活窗口导致 pointerleave 误触发波浪效果收缩
        const screenDockBounds = readScreenDockBounds();
        const gx = globalPointerRef.current.x;
        const gy = globalPointerRef.current.y;
        if (
          screenDockBounds &&
          gx !== null &&
          gy !== null &&
          isPointInsideBounds(gx, gy, screenDockBounds)
        ) {
          leaveTimeoutRef.current = null;
          return;
        }

        pointer.current.x = null;
        pointer.current.y = null;
        lastPointer.current.x = null;
        lastPointer.current.y = null;
        isActive.current = false;
        if (!pendingHorizontalReleaseRef.current) {
          beginHorizontalRelease(null);
        }
        // 优化：启动动画让图标平滑回到原位，动画会在needsUpdate为false时自动停止
        startAnimation();
        leaveTimeoutRef.current = null;
      }, 150);
    };

    // 阻止滚轮事件，防止Dock被滚动
    const handleWheel = (e: WheelEvent) => {
      e.preventDefault();
      e.stopPropagation();
    };

    // 阻止触摸滚动
    const handleTouchMove = (e: TouchEvent) => {
      e.preventDefault();
    };

    document.addEventListener('pointermove', handleMove, { passive: true });
    document.addEventListener('pointerleave', handleLeave);
    document.addEventListener('wheel', handleWheel, { passive: false });
    document.addEventListener('touchmove', handleTouchMove, { passive: false });
    window.addEventListener('resize', handleDisplayLayoutChanged);

    return () => {
      observer.disconnect();
      taskbarClassObserver.disconnect();
      document.removeEventListener('pointermove', handleMove);
      document.removeEventListener('pointerleave', handleLeave);
      document.removeEventListener('wheel', handleWheel);
      document.removeEventListener('touchmove', handleTouchMove);
      window.removeEventListener('resize', handleDisplayLayoutChanged);
      window.removeEventListener('backplate-style-changed', handleBackplateStyleChanged);
      void unlistenWindowMoved.then((unsubscribe) => unsubscribe());
      void globalMouseSubscription.then(() => disposeGlobalMouse?.());
      void taskbarContainerRefreshSubscription.then((unsubscribe) => unsubscribe());
      if (rafId.current) cancelAnimationFrame(rafId.current);
      if (leaveTimeoutRef.current !== null) clearTimeout(leaveTimeoutRef.current);
      if (measureTimeoutRef.current !== null) clearTimeout(measureTimeoutRef.current);
      clearScheduledLayoutRefresh();
      resetMagnifierRuntimeState();
      applyHorizontalDockOffset(0);
    };
  }, [containerRef, maxScale, hoverThreshold, smoothFactor, enableGapScaling, targetSelector, enabled, zoomEffectType]);
};
