import { FuncCommand, IconPackManager, invoke, Settings } from "@magic-ui/lib";
import { FuncCommandGetIconArgs } from "@magic-ui/lib/types";
import { cx } from "@shared/styles";
import { UnlistenFn } from "@tauri-apps/api/event";
import React, { ImgHTMLAttributes } from "react";

import { iconPackManager } from "./common";
import cs from "./index.module.css";

// 获取 missing icon 作为备用图标
const getMissingIconSrc = () => {
  const missingIcon = iconPackManager.getMissingIcon();
  if (missingIcon) {
    return missingIcon.base;
  }
  return null;
};

// 🔧 全局图标加载失败计数器
const ICON_FAILURE_THRESHOLD = 3;
const iconFailureCounts = new Map<string, number>();

// 获取应用的唯一标识符（用于追踪失败次数）
const getAppIdentifier = (path?: string | null, umid?: string | null): string => {
  if (umid) return `umid:${umid}`;
  if (path) return `path:${path}`;
  return 'unknown';
};

// 检查是否需要使用备用提取方法
const shouldUseFallbackExtraction = (identifier: string): boolean => {
  const count = iconFailureCounts.get(identifier) || 0;
  return count >= ICON_FAILURE_THRESHOLD;
};

// 记录失败次数
const recordIconFailure = (identifier: string): void => {
  const count = (iconFailureCounts.get(identifier) || 0) + 1;
  iconFailureCounts.set(identifier, count);
  console.error(`[FileIcon] Icon failure count for ${identifier}: ${count}`);
};

// 重置失败次数
const resetIconFailureCount = (identifier: string): void => {
  iconFailureCounts.delete(identifier);
};

interface FileIconProps extends Omit<ImgHTMLAttributes<HTMLImageElement>, "src"> {
  path?: string | null;
  umid?: string | null;
  /** 🔧 形状变化回调，当图标提取完成后通知父组件 */
  onShapeChange?: (isSquare: boolean, isFromLocal: boolean) => void;
}

interface FileIconState {
  src: string | null;
  mask: string | null;
  isAproximatelySquare: boolean;
  loadFailed: boolean; // 标记图标是否加载失败
  isFromLocal: boolean; // 🔧 标记图标是否来自本地目录
}

const darkModeQuery = globalThis.matchMedia("(prefers-color-scheme: dark)");
function getIcon(args: FuncCommandGetIconArgs): FileIconState {
  const icon = iconPackManager.getIcon(args);
  if (icon) {
      const src = (darkModeQuery.matches ? icon.dark : icon.light) || icon.base;
      // 过滤掉 missing icon，不显示占位图标
      if (src && src.includes('missing-icon.png')) {
        console.debug(`[FileIcon] Filtering out missing icon for path: ${args.path || '(no path)'}, umid: ${args.umid || '(no umid)'}`);
        return { src: null, mask: null, isAproximatelySquare: false, loadFailed: false, isFromLocal: false };
      }

    return {
      src: (darkModeQuery.matches ? icon.dark : icon.light) || icon.base,
      mask: icon.mask,
      isAproximatelySquare: icon.isAproximatelySquare || false,
      loadFailed: false,
      isFromLocal: false,
    };
  }
  return { src: null, mask: null, isAproximatelySquare: false, loadFailed: false, isFromLocal: false };
}
export class FileIcon extends React.Component<FileIconProps, FileIconState> {
  unlistener: UnlistenFn | null = null;
  private timeoutId: number | null = null;
  private lastExtractionTime: number = 0;
  private readonly EXTRACTION_COOLDOWN = 1000; // 1秒冷却时间
  private isWhiteBackplateMode: boolean = false; // 当前是否为白色背板模式
  private backplateStyleChangePending: boolean = false; // 🔧 标记背板风格切换正在进行中（防止竞态条件）
  private backplateStyleChangeListener: (event: Event) => void; // 背板风格变化事件监听器
  private backplatePendingTimerId: number | null = null; // 🔧 保存 pending 重置定时器 ID，防止竞态

  private getIcon(args: FuncCommandGetIconArgs): FileIconState {
    return getIcon(args);
  }

  constructor(props: FileIconProps) {
    super(props);
    this.updateSrc = this.updateSrc.bind(this);

    // 白色背板模式下，先初始化为非本地图标，避免使用缓存的本地图标
    // 稍后会根据设置决定是否重新获取
    this.state = {
      src: null,
      mask: null,
      isAproximatelySquare: false,
      loadFailed: false,  // 🔧 改为 false，等待超时或 onError 触发
      isFromLocal: false
    };

    darkModeQuery.addEventListener("change", this.updateSrc);

    console.log(`[FileIcon] Constructor called for path: ${this.props.path || '(no path)'}, umid: ${this.props.umid || '(no umid)'}`);

    // 🔧 立即检查本地图标，不等待异步 Promise，避免初始渲染时 isFromLocal 为 false
    this.initLocalIconCheck();

    // 监听背板风格变化事件
    this.backplateStyleChangeListener = (event: Event) => {
      const customEvent = event as CustomEvent;
      console.log(`[FileIcon] Backplate style changed to: ${customEvent.detail.style}, reloading icon for path: ${this.props.path || '(no path)'}, umid: ${this.props.umid || '(no umid)'}`);
      // 更新背板模式状态
      const newStyle = customEvent.detail.style as 'Transparent' | 'White';
      this.isWhiteBackplateMode = newStyle === 'White';

      // 🔧 取消上一次切换的 pending 重置定时器，防止旧定时器偷走新 pending
      if (this.backplatePendingTimerId !== null) {
        window.clearTimeout(this.backplatePendingTimerId);
        this.backplatePendingTimerId = null;
      }

      // 🔧 核心修复：不清空图标，保持旧图标显示，等新图标就绪后再替换
      this.backplateStyleChangePending = true;
      this.lastExtractionTime = 0;

      // 优先尝试获取本地图标
      this.getLocalIcon().then(localApplied => {
        if (!localApplied) {
          // 本地图标不存在，发起异步提取，但不清除当前显示的图标
          this.requestIconExtraction();
        }
        // 延迟重置 pending 标志，确保提取完成后的 updateSrc 不被冷却阻止
        this.backplatePendingTimerId = window.setTimeout(() => {
          this.backplateStyleChangePending = false;
          this.backplatePendingTimerId = null;
        }, 3000);
      });
    };
    window.addEventListener('backplate-style-changed', this.backplateStyleChangeListener);
    console.log(`[FileIcon] Event listener registered for backplate-style-changed`);

    iconPackManager.onChange(this.updateSrc).then((unlistener) => {
      this.unlistener = unlistener;

      // 从设置中获取当前背板模式
      Settings.getAsync().then(settings => {
        this.isWhiteBackplateMode = settings.magicTaskbar.iconBackplateStyle === 'White';

        // 优先尝试获取本地图标
        this.getLocalIcon().then(applied => {
          if (!applied && !this.state.src) {
            // 本地图标不存在且缓存也没有，发起异步提取
            this.safeRequestIconExtraction();
          }
        });
      });

      // 设置较长的超时时间，给图标提取更多时间
      if (!this.state.src) {
        this.timeoutId = window.setTimeout(() => {
          if (!this.state.src && !this.state.loadFailed) {
            console.error(`[FileIcon] Icon extraction timeout for path: ${this.props.path || '(no path)'}, umid: ${this.props.umid || '(no umid)'}`);

            // 🔧 记录失败次数并检查是否需要备用提取
            const appIdentifier = getAppIdentifier(this.props.path, this.props.umid);
            recordIconFailure(appIdentifier);

            if (shouldUseFallbackExtraction(appIdentifier)) {
              console.log(`[FileIcon] Threshold reached (timeout), using fallback extraction for ${appIdentifier}`);
              invoke(FuncCommand.IconExtractWithFallback, {
                path: this.props.path ?? null,
                umid: this.props.umid ?? null,
              }).catch((err) => {
                console.error(`[FileIcon] Fallback extraction failed:`, err);
              });
            }

            this.setState({ loadFailed: true });
          }
        }, 5000); // 5秒超时
      }
    });
  }

  componentWillUnmount(): void {
    this.unlistener?.();
    this.unlistener = null;
    darkModeQuery.removeEventListener("change", this.updateSrc);
    window.removeEventListener('backplate-style-changed', this.backplateStyleChangeListener);
    // 清理超时定时器
    if (this.timeoutId !== null) {
      window.clearTimeout(this.timeoutId);
      this.timeoutId = null;
    }
    // 🔧 清理 pending 重置定时器
    if (this.backplatePendingTimerId !== null) {
      window.clearTimeout(this.backplatePendingTimerId);
      this.backplatePendingTimerId = null;
    }
  }

  // 🔧 组件挂载后立即通知父组件当前形状信息（解决启动时缓存命中不触发回调的问题）
  componentDidMount(): void {
    // 如果图标已经加载完成（从缓存获取），立即通知父组件
    if (this.state.src) {
      this.props.onShapeChange?.(this.state.isAproximatelySquare, this.state.isFromLocal);
    }
  }

  // 🔧 初始化时立即检查本地图标
  private async initLocalIconCheck(): Promise<void> {
    // 先获取当前背板模式
    const settings = await Settings.getAsync();
    this.isWhiteBackplateMode = settings.magicTaskbar.iconBackplateStyle === 'White';
    
    // 检查本地图标
    if (this.props.path || this.props.umid) {
      await this.getLocalIcon();
    }
  }

  componentDidUpdate(
    prevProps: Readonly<FileIconProps>,
    prevState: Readonly<FileIconState>,
  ): void {
    if (
      this.props.path !== prevProps.path || this.props.umid !== prevProps.umid
    ) {
      // 清理旧的超时定时器
      if (this.timeoutId !== null) {
        window.clearTimeout(this.timeoutId);
        this.timeoutId = null;
      }
      this.updateSrc();
    }

    // 🔧 当形状或本地图标状态变化时，通知父组件
    if (
      this.state.src && // 只有当图标加载完成时才通知
      (this.state.isAproximatelySquare !== prevState.isAproximatelySquare ||
       this.state.isFromLocal !== prevState.isFromLocal ||
       (this.state.src && !prevState.src)) // 图标从无到有时也通知
    ) {
      this.props.onShapeChange?.(this.state.isAproximatelySquare, this.state.isFromLocal);
    }
  }

  requestIconExtraction(): void {
    IconPackManager.requestIconExtraction({
      path: this.props.path,
      umid: this.props.umid,
    });
  }

  safeRequestIconExtraction(): void {
    // 🔧 如果背板切换正在进行中，跳过冷却检查
    if (this.backplateStyleChangePending) {
      this.requestIconExtraction();
      return;
    }

    const now = Date.now();
    if (now - this.lastExtractionTime < this.EXTRACTION_COOLDOWN) {
      console.debug(`[FileIcon] Skipping icon extraction due to cooldown for path: ${this.props.path || '(no path)'}, umid: ${this.props.umid || '(no umid)'}`);
      return;
    }
    this.lastExtractionTime = now;
    this.requestIconExtraction();
  }

  async getLocalIcon(): Promise<boolean> {
    if (!this.props.path && !this.props.umid) {
      return false;
    }

    const lookupName = this.props.path || this.props.umid || '';

    try {
      const command = this.isWhiteBackplateMode ? FuncCommand.GetLocalIconWhite : FuncCommand.GetLocalIcon;
      const result = await invoke(command, {
        processName: lookupName
      });

      if (result) {
        this.setState({
          src: result,
          mask: null,
          isAproximatelySquare: true,
          loadFailed: false,
          isFromLocal: true, // 🔧 标记为本地图标
        });
        return true;
      }
    } catch (error) {
      console.error(`[FileIcon] Failed to get local icon for path: ${this.props.path || '(no path)'}, umid: ${this.props.umid || '(no umid)'}`, error);
    }
    return false;
  }

  async updateSrc(): Promise<void> {
    // 优化：如果当前已经有图标，检查是否真的需要更新
    if (this.state.src) {
      // 🔧 背板切换进行中时，强制检查新图标
      if (this.backplateStyleChangePending) {
        // 先检查本地图标
        if (this.props.path || this.props.umid) {
          const applied = await this.getLocalIcon();
          if (applied) return;
        }
        // 检查系统提取的图标是否已更新
        const newIconState = this.getIcon({ path: this.props.path, umid: this.props.umid });
        if (newIconState.src && newIconState.src !== this.state.src) {
          this.setState(newIconState);
        }
        return;
      }
      if (this.state.isFromLocal) {
        // 本地图标不会因为普通 icon-packs 事件而改变，跳过更新
        return;
      }
      // 系统图标：检查是否真的变化了
      const newIconState = this.getIcon({ path: this.props.path, umid: this.props.umid });
      if (newIconState.src === this.state.src) {
        return;
      }
    }

    // 🔧 背板切换进行中时，即使 src 为 null 也只做检查，不重复发起提取
    if (this.backplateStyleChangePending) {
      if (this.props.path || this.props.umid) {
        const applied = await this.getLocalIcon();
        if (applied) return;
      }
      const newIconState = this.getIcon({ path: this.props.path, umid: this.props.umid });
      if (newIconState.src) {
        if (this.timeoutId !== null) {
          window.clearTimeout(this.timeoutId);
          this.timeoutId = null;
        }
        this.setState(newIconState);
      }
      // pending 期间不调用 safeRequestIconExtraction，避免冷却雪崩
      return;
    }

    // 优先尝试获取本地图标
    if (this.props.path || this.props.umid) {
      const applied = await this.getLocalIcon();
      if (applied) {
        return;
      }
    }

    // 🔧 背板切换进行中且 src 为 null 时，只做被动检查，不触发 safeRequestIconExtraction
    // backplate handler 已经调用过 requestIconExtraction()，这里不需要重复调用
    if (this.backplateStyleChangePending) {
      const newState = this.getIcon({ path: this.props.path, umid: this.props.umid });
      if (newState.src) {
        if (this.timeoutId !== null) {
          window.clearTimeout(this.timeoutId);
          this.timeoutId = null;
        }
        if (this.state.loadFailed) {
          newState.loadFailed = false;
        }
        this.setState(newState);
      }
      return;
    }

    // 本地图标不存在，获取系统提取的图标
    this.setState({ isFromLocal: false });
    const newState = this.getIcon({ path: this.props.path, umid: this.props.umid });
    if (newState.src) {
      if (this.timeoutId !== null) {
        window.clearTimeout(this.timeoutId);
        this.timeoutId = null;
      }
      if (this.state.loadFailed) {
        newState.loadFailed = false;
      }
      this.setState(newState);
    } else {
      this.setState(newState);
      this.safeRequestIconExtraction();
    }
  }

  render(): React.ReactNode {
    const { path: _path, umid: _umid, ...imgProps } = this.props;

    const dataProps = Object.entries(imgProps)
      .filter(([k]) => k.startsWith("data-"))
      .reduce((acc, [k, v]) => ({ ...acc, [k]: v }), {});

    // 始终返回一个占位容器，避免布局跳动
    const loadingClass = !this.state.src && !this.state.loadFailed ? cs.loading : '';
    const failedClass = this.state.loadFailed ? cs.failed : '';

    // 获取应用标识符
    const appIdentifier = getAppIdentifier(this.props.path, this.props.umid);

    return (
      <figure
        {...imgProps}
        className={cx(cs.outer, imgProps.className, loadingClass, failedClass)}
        data-shape={this.state.isAproximatelySquare ? "square" : "unknown"}
        data-local={this.state.isFromLocal ? "true" : undefined}
      >
        {this.state.src ? (
          <>
            <img
              {...dataProps}
              src={this.state.src}
              className={cx(cs.icon, this.state.isFromLocal ? cs.localIcon : '')}
              onError={async (e) => {
                // 记录失败次数
                recordIconFailure(appIdentifier);

                console.error(`[FileIcon] Image loading failed for path: ${this.props.path || '(no path)'}, umid: ${this.props.umid || '(no umid)'}`);

                // 检查是否需要使用备用提取方法
                if (shouldUseFallbackExtraction(appIdentifier)) {
                  console.log(`[FileIcon] Threshold reached, using fallback extraction for ${appIdentifier}`);
                  // 调用后端备用提取命令
                  try {
                    await invoke(FuncCommand.IconExtractWithFallback, {
                      path: this.props.path ?? null,
                      umid: this.props.umid ?? null,
                    });
                  } catch (err) {
                    console.error(`[FileIcon] Fallback extraction failed:`, err);
                  }
                }

                this.setState({ src: null, mask: null, isAproximatelySquare: false, loadFailed: true });
              }}
              onLoad={() => {
                // 图标加载成功，重置失败计数
                resetIconFailureCount(appIdentifier);
              }}
            />
            {this.state.mask && (
              <div
                {...dataProps}
                className={cx(cs.mask, "sl-mask")}
                style={{ maskImage: `url('${this.state.mask}')` }}
              />
            )}
          </>
        ) : this.state.loadFailed ? (
          // 图标加载失败时，显示 missing.png 作为备用图标
          <>
            {getMissingIconSrc() && (
              <>
                {console.error(`[FileIcon] Displaying missing icon as fallback for path: ${this.props.path || '(no path)'}, umid: ${this.props.umid || '(no umid)'}`)}
                <img
                  {...dataProps}
                  src={getMissingIconSrc() as string}
                  className={cs.icon}
                  style={{ opacity: 0.6 }}
                />
              </>
            )}
          </>
        ) : (
          // 图标加载中，显示占位符
          <div className={cs.placeholder} />
        )}
      </figure>
    );
  }
}
