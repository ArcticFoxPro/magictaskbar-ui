import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { invoke, FuncCommand } from "@magic-ui/lib";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import "./styles.css";
import magictaskuiIcon from "../../../../static/icons/magictaskui.svg";
import cancelIcon from "../../../../static/icons/cancel.svg";

export function AboutModal() {
  const { t } = useTranslation();
  const [version, setVersion] = useState("...");

  useEffect(() => {
    // 获取应用版本
    invoke(FuncCommand.SystemGetAppVersion, {}).then((v: any) => {
        setVersion(v || t('about.unknown'));
      })
      .catch(() => {
        setVersion(t('about.unknown'));
      });
  }, [t]);

  const handleClose = async () => {
    const webview = getCurrentWebviewWindow();
    await webview.close();
  };

  const handleOpenUrl = async () => {
    // 使用相对路径，相对于应用程序所在目录
    await (invoke as any)('system_open_file', { filePath: "config/opensource/open_source_notice.htm" });
  };

  return (
    <div className="about-modal-backdrop">
      <div className="about-modal-container" onClick={(e) => e.stopPropagation()}>
        <div className="about-modal-title-text">{t('about.title')}</div>
        <button className="about-modal-close-btn" onClick={handleClose}>
          <img src={cancelIcon} alt={t('about.close')} className="close-btn-icon" />
        </button>
        <img src={magictaskuiIcon} alt="Magic Task UI" className="about-modal-icon" />
        <div className="about-modal-title">{t('about.app_name')}</div>
        <div className="about-modal-version">{t('about.version')}: {version}</div>
        <div className="about-modal-beta">{t('about.beta')}</div>
        <div 
          className="about-modal-link" 
          onClick={handleOpenUrl}
        >
          {t('about.opensource')}
        </div>
        <div className="about-modal-copyright">
          {t('about.copyright')}
        </div>
      </div>
    </div>
  );
}

export default AboutModal;
