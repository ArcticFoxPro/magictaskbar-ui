import { useState, useEffect, useRef, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { $check_update_modal_open, $check_update_version } from "../shared/state/mod";
import { invoke, FuncCommand, subscribe, FuncEvent } from "@magic-ui/lib";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import "./styles.css";
import magictaskuiIcon from "../../../../static/icons/magictaskui.svg";
import cancelIcon from "../../../../static/icons/cancel.svg";
import buttonIcon from "../../../../static/icons/ButtonCornerRadius.svg";

interface CheckUpdateModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export function CheckUpdateModal({ isOpen, onClose }: CheckUpdateModalProps) {
  const { t } = useTranslation();
  const [visible, setVisible] = useState(false);
  const [version, setVersion] = useState("");
  const [isChecking, setIsChecking] = useState(false);
  const [isDownloading, setIsDownloading] = useState(false);
  const [isInstalling, setIsInstalling] = useState(false);
  const [hasNewVersion, setHasNewVersion] = useState(false);
  const [newVersion, setNewVersion] = useState("");
  const [downloadStatus, setDownloadStatus] = useState<number | null>(null);
  const [downloadProgress, setDownloadProgress] = useState(0);
  
  // 预测动画相关
  const [displayProgress, setDisplayProgress] = useState(0);
  const displayProgressRef = useRef(0);
  const realProgressRef = useRef(0);
  const animationTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // 下载状态码定义
  const DOWNLOAD_ERROR = 1002;
  const DOWNLOAD_PROCESS = 1003;
  const DOWNLOAD_FINISH = 1004;

  // 清理预测动画定时器
  const clearAnimationTimer = useCallback(() => {
    if (animationTimerRef.current) {
      clearInterval(animationTimerRef.current);
      animationTimerRef.current = null;
    }
  }, []);

  // 启动预测动画
  const startPredictAnimation = useCallback((currentRealProgress: number) => {
    clearAnimationTimer();
    
    // 计算下一个真实进度节点（10的倍数）
    const nextMilestone = Math.floor(currentRealProgress / 10) * 10 + 10;
    // 预测动画最多走到下一个节点-1
    const maxPredictProgress = Math.min(nextMilestone - 1, 99);
    
    // 如果已经到达或超过最大预测进度，不启动动画
    if (displayProgressRef.current >= maxPredictProgress) return;
    
    // 每1s增加1%，模拟平滑动画
    animationTimerRef.current = setInterval(() => {
      setDisplayProgress(prev => {
        if (prev >= maxPredictProgress) {
          clearAnimationTimer();
          return prev;
        }
        const newProgress = Math.min(prev + 1, maxPredictProgress);
        displayProgressRef.current = newProgress;
        return newProgress;
      });
    }, 1000);
  }, [clearAnimationTimer]);

  useEffect(() => {
    setVisible(isOpen);
    $check_update_modal_open.value = isOpen;
  }, [isOpen]);

  useEffect(() => {
    // 监听check update消息事件
    let unlistenCheckUpdate: (() => void) | undefined;
    subscribe(FuncEvent.CheckUpdateMessageReceived, (payload: any) => {
      console.info('[CheckUpdate] Received check update message:', payload);
      // Tauri subscribe 回调接收 {event, payload, id} 对象，实际数据在 payload.payload 中
      const data = payload.payload || payload;
      console.info('[CheckUpdate] Extracted data:', data);
      console.info('[CheckUpdate] Setting hasNewVersion to:', data.hasNewVersion);
      setHasNewVersion(data.hasNewVersion);
      if (data.hasNewVersion) {
        console.info('[CheckUpdate] Setting newVersion to:', data.newVersion);
        setNewVersion(data.newVersion);
      } else {
        console.info('[CheckUpdate] Resetting newVersion to empty string');
        setNewVersion("");
      }
      // 收到消息后，停止显示检查状态
      console.info('[CheckUpdate] Setting isChecking to false');
      setIsChecking(false);
    }).then((fn) => {
      unlistenCheckUpdate = fn;
    }).catch((e) => {
      console.warn('[CheckUpdate] Failed to subscribe check update event:', e);
    });

    // 监听download update消息事件
    let unlistenDownloadUpdate: (() => void) | undefined;
    subscribe(FuncEvent.DownloadUpdateMessageReceived, (payload: any) => {
      console.info('[CheckUpdate] Received download update message:', payload);
      // Tauri subscribe 回调接收 {event, payload, id} 对象，实际数据在 payload.payload 中
      const data = payload.payload || payload;
      console.info('[CheckUpdate] Extracted download data:', data);
      
      const status = data.status;
      const progress = data.progress || 0;
      
      // 处理不同的下载状态
      if (status === DOWNLOAD_ERROR) {
        // 1002: 下载失败，重置UI到默认状态
        console.info('[CheckUpdate] Download failed (status=1002), resetting UI');
        setDownloadStatus(null);
        setDownloadProgress(0);
        setDisplayProgress(0);
        displayProgressRef.current = 0;
        realProgressRef.current = 0;
        clearAnimationTimer();
        setHasNewVersion(false);
        setNewVersion("");
        setIsDownloading(false);
      } else if (status === DOWNLOAD_PROCESS) {
        // 1003: 下载进度更新
        console.info('[CheckUpdate] Download progress:', progress, '%');
        realProgressRef.current = progress;
        setDownloadProgress(progress);
        setDisplayProgress(progress);
        displayProgressRef.current = progress;
        // 启动预测动画，让进度条继续缓慢前进
        startPredictAnimation(progress);
      } else if (status === DOWNLOAD_FINISH) {
        // 1004: 下载完成
        console.info('[CheckUpdate] Download finished');
        clearAnimationTimer();
        setDownloadStatus(DOWNLOAD_FINISH);
        setDownloadProgress(100);
        setDisplayProgress(100);
        displayProgressRef.current = 100;
        setIsDownloading(false);
      }
    }).then((fn) => {
      unlistenDownloadUpdate = fn;
    }).catch((e) => {
      console.warn('[CheckUpdate] Failed to subscribe download update event:', e);
    });

    return () => {
      if (unlistenCheckUpdate) unlistenCheckUpdate();
      if (unlistenDownloadUpdate) unlistenDownloadUpdate();
      clearAnimationTimer();
    };
  }, [clearAnimationTimer, startPredictAnimation]);

  const handleClose = () => {
    // 关闭弹窗，但保留所有状态，下次打开时保持当前进度
    setVisible(false);
    onClose();
  };

  const getDownloadStatusText = (status: number | null, installing: boolean, progress: number) => {
    if (installing) {
      return t('check_update.installing');
    }
    switch (status) {
      case DOWNLOAD_FINISH:
        return t('check_update.download_complete');
      case DOWNLOAD_PROCESS:
        return `${t('check_update.downloading')} ${progress}%`;
      default:
        return t('check_update.updating');
    }
  };

  const handleCheckUpdate = async () => {
    try {
      // 在发送检查更新请求前，重置所有状态
      setHasNewVersion(false);
      setNewVersion("");
      setDownloadStatus(null);
      setDownloadProgress(0);
      setDisplayProgress(0);
      displayProgressRef.current = 0;
      realProgressRef.current = 0;
      clearAnimationTimer();
      setIsDownloading(false);
      setIsChecking(true);
      console.info('[CheckUpdate] Reset state and set isChecking to true');
      console.info('[CheckUpdate] Invoking system_send_check_update_to_magicvisuals');
      await invoke(FuncCommand.SystemSendCheckUpdateToMagicvisuals, undefined).catch((e: any) => {
        console.warn('[CheckUpdate] invoke system_send_check_update_to_magicvisuals failed', e);
      });
    } catch (e) {
      console.warn('[CheckUpdate] send check update to magicvisuals failed', e);
    }
  };

  const handleUpdateNow = async () => {
    try {
      setIsDownloading(true);
      setDownloadProgress(0);
      setDisplayProgress(0);
      displayProgressRef.current = 0;
      realProgressRef.current = 0;
      console.info('[CheckUpdate] Clicked update now button, sending download update message');
      // 立即启动预测动画，从0%开始缓慢前进
      startPredictAnimation(0);
      await invoke(FuncCommand.SystemSendDownloadUpdateToMagicvisuals, undefined).catch((e: any) => {
        console.warn('[CheckUpdate] invoke system_send_download_update_to_magicvisuals failed', e);
      });
      // 发送消息后保持"更新中"状态，等待用户手动关闭或后续操作
    } catch (e) {
      console.warn('[CheckUpdate] send download update to magicvisuals failed', e);
      setIsDownloading(false);
      clearAnimationTimer();
    }
  };

  const handleConfirmInstall = async () => {
    try {
      setIsInstalling(true);
      console.info('[CheckUpdate] Clicked confirm install button, sending start install message');
      await invoke(FuncCommand.SystemSendStartInstallToMagicvisuals, undefined).catch((e: any) => {
        console.warn('[CheckUpdate] invoke system_send_start_install_to_magicvisuals failed', e);
      });
      // Close modal immediately after triggering install.
      handleClose();
      // 发送消息后，等待安装完成（暂时不重置，等待后续消息回复）
    } catch (e) {
      console.warn('[CheckUpdate] send start install to magicvisuals failed', e);
      setIsInstalling(false);
      // Still close the modal on click as requested.
      handleClose();
    }
  };

  if (!visible) return null;

  // 根据当前状态判断按钮文字、点击回调、alt文字、是否禁用
  const getButtonState = () => {
    // 当正在未接收到延迟的消息时，禁用按钮
    const isButtonDisabled = isChecking || isDownloading || isInstalling;
    
    if (downloadStatus === DOWNLOAD_FINISH) {
      // 下载完成，显示"确认安装"
      return {
        text: t('check_update.confirm_install'),
        onClick: handleConfirmInstall,
        alt: t('check_update.confirm_install'),
        disabled: isButtonDisabled,
      };
    } else if (hasNewVersion) {
      // 有新版本，显示"立即更新"
      return {
        text: t('check_update.update_now'),
        onClick: handleUpdateNow,
        alt: t('check_update.update_now'),
        disabled: isButtonDisabled,
      };
    } else {
      // 默认状态，显示"检查更新"
      return {
        text: t('check_update.check_update'),
        onClick: handleCheckUpdate,
        alt: t('check_update.check_update'),
        disabled: isButtonDisabled,
      };
    }
  };

  const buttonState = getButtonState();

  return (
    <div className="check-update-modal-backdrop">
      <div className="check-update-modal-container" onClick={(e) => e.stopPropagation()}>
        <div className="check-update-modal-title-text">{t('check_update.title')}</div>
        <button className="check-update-modal-close-btn" onClick={handleClose}>
          <img src={cancelIcon} alt={t('check_update.close')} className="close-btn-icon" />
        </button>
        <img src={magictaskuiIcon} alt="Magic Task UI" className="check-update-modal-icon" />
        <div className="check-update-status-text">
          {(isChecking || isDownloading || isInstalling) && <div className="check-update-loading-spinner"></div>}
          {isChecking ? t('check_update.checking') : (isDownloading ? getDownloadStatusText(downloadStatus, isInstalling, displayProgress) : (downloadStatus !== null ? getDownloadStatusText(downloadStatus, isInstalling, displayProgress) : (hasNewVersion ? t('check_update.new_version_found') : t('check_update.already_latest'))))}  
        </div>
        <div className="check-update-version-text">
          {hasNewVersion ? `${t('check_update.new_version')}${newVersion}` : `${t('check_update.current_version')}${$check_update_version.value || "10.0.0.18"}`}
        </div>
        {/* 下载进度条：下载中时显示 */}
        {isDownloading && (
          <div className="check-update-progress-container">
            <div 
              className="check-update-progress-bar" 
              style={{ width: `${displayProgress}%` }}
            />
          </div>
        )}
        <button 
          className="check-update-action-btn" 
          onClick={buttonState.onClick}
          disabled={buttonState.disabled}
        >
          <img src={buttonIcon} alt={buttonState.alt} className="action-btn-icon" />
          <span className="action-btn-text">{buttonState.text}</span>
        </button>
      </div>
    </div>
  );
}

export default CheckUpdateModal;
