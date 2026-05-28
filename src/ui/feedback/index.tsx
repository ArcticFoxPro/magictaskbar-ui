import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { applyTextScaleCompensation, getRootContainer } from "@shared";
import { removeDefaultWebviewActions } from "@shared/setup";
import { createRoot } from "react-dom/client";
import { I18nextProvider } from "react-i18next";
import { Provider } from "react-redux";
import { store } from "../toolbar/modules/shared/store/infra";
import i18n, { loadTranslations } from "../taskbar/i18n";
import { Settings, invoke, FuncCommand } from "@magic-ui/lib";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import "@shared/styles/colors.css";
import "../toolbar/styles/variables.css";
import "@shared/styles/reset.css";
import "../toolbar/styles/global.css";
import cancelIcon from "../../static/icons/cancel.svg";
import downArrowIcon from "../../static/icons/DownArrow.svg";
import "./styles.css";

removeDefaultWebviewActions();
await applyTextScaleCompensation();
await loadTranslations();

try {
  const settings = await Settings.getAsync();
  const lang = settings.inner.language || "en";
  await i18n.changeLanguage(lang);
} catch (e) {
  console.warn("[Feedback] Failed to load language from settings");
}

function FeedbackApp() {
  const { t } = useTranslation();
  const [opinionType, setOpinionType] = useState<string>("");
  const [selectedTypes, setSelectedTypes] = useState<Set<string>>(new Set());
  const [description, setDescription] = useState("");
  const [uploadLogs, setUploadLogs] = useState(true);
  const [contactMethod, setContactMethod] = useState("wechat");
  const [contactValue, setContactValue] = useState("");
  const [showContactDropdown, setShowContactDropdown] = useState(false);

  useEffect(() => {
    console.log('[Feedback] Component mounted, will show window in 100ms');
    import("@tauri-apps/api/webviewWindow").then(({ getCurrentWebviewWindow }) => {
      const win = getCurrentWebviewWindow();
      console.log('[Feedback] Got webview window, showing...');
      setTimeout(() => {
        win.show();
        console.log('[Feedback] Window shown');
      }, 100);
    }).catch((e) => {
      console.error('[Feedback] Failed to show window:', e);
    });
  }, []);

  const handleClose = async () => {
    const webview = getCurrentWebviewWindow();
    await webview.close();
  };

  const toggleType = (id: string) => {
    setSelectedTypes(prev => {
      const newSet = new Set(prev);
      if (newSet.has(id)) {
        newSet.delete(id);
      } else {
        newSet.add(id);
      }
      return newSet;
    });
  };

  const handleSubmit = async () => {
    console.log('[Feedback] handleSubmit 被调用');
    console.log('[Feedback] selectedTypes:', selectedTypes);
    console.log('[Feedback] selectedTypes.size:', selectedTypes.size);
    console.log('[Feedback] description:', description);
    
    // 验证反馈类型和详细描述是否为空
    if (selectedTypes.size === 0) {
      console.log('[Feedback] 反馈类型为空，不提交');
      return;
    }
    if (!description.trim()) {
      console.log('[Feedback] 详细描述为空，不提交');
      return;
    }

    // 准备参数
    const feedbackTypes = Array.from(selectedTypes).join(',');
    const contactInfo = contactValue ? `${contactMethod}:${contactValue}` : '';

    // 打印反馈信息
    console.log('[Feedback] 意见类型:', opinionType);
    console.log('[Feedback] 反馈类型:', feedbackTypes);
    console.log('[Feedback] 详细描述:', description);
    console.log('[Feedback] 联系方式:', contactInfo);
    console.log('[Feedback] 上传日志:', uploadLogs);

    // 调用后端打点接口（后端会立即返回，在后台执行）
    try {
      await invoke(FuncCommand.ReportFeedback, {
        feedbackTypes: feedbackTypes,
        description: description,
        contactInfo: contactInfo,
        uploadLogs: uploadLogs,
      });
      console.log('[Feedback] 打点请求已发送');
    } catch (e) {
      console.warn('[Feedback] 打点上报失败:', e);
    }

    // 关闭窗口
    try {
      await getCurrentWebviewWindow().close();
    } catch (e) {
      console.warn('[Feedback] 关闭窗口失败:', e);
    }
  };

  const opinionTypes = [
    { id: "problem", label: t('feedback.problem') },
    { id: "suggestion", label: t('feedback.suggestion') },
  ];

  const feedbackTypes = [
    { id: "upgrade_install_uninstall", label: t('feedback.feedback_types.upgrade_install_uninstall') },
    { id: "animation", label: t('feedback.feedback_types.animation') },
    { id: "desktop", label: t('feedback.feedback_types.desktop') },
    { id: "desktop_widget", label: t('feedback.feedback_types.desktop_widget') },
    { id: "desktop_wallpaper", label: t('feedback.feedback_types.desktop_wallpaper') },
    { id: "bottom_taskbar", label: t('feedback.feedback_types.bottom_taskbar') },
    { id: "top_taskbar", label: t('feedback.feedback_types.top_taskbar') },
    { id: "control_center", label: t('feedback.feedback_types.control_center') },
    { id: "calendar_notification", label: t('feedback.feedback_types.calendar_notification') },
    { id: "ai_recommendation", label: t('feedback.feedback_types.ai_recommendation') },
    { id: "hummingbird_engine", label: t('feedback.feedback_types.hummingbird_engine') },
    { id: "ui_design", label: t('feedback.feedback_types.ui_design') },
    { id: "account_settings", label: t('feedback.feedback_types.account_settings') },
    { id: "other", label: t('feedback.feedback_types.other') },
  ];

  const contactMethods = [
    { id: "wechat", label: t('feedback.contact_methods.wechat') },
    { id: "qq", label: t('feedback.contact_methods.qq') },
    { id: "email", label: t('feedback.contact_methods.email') },
    { id: "phone", label: t('feedback.contact_methods.phone') },
  ];

  const selectedContactLabel = contactMethods.find(m => m.id === contactMethod)?.label || t('feedback.contact_methods.wechat');

  return (
    <div className="feedback-modal-backdrop">
      <div className="feedback-modal-container">
        {/* 标题栏区域 - 可拖拽 */}
        <div className="feedback-title-bar">
          <div className="feedback-modal-title-text">{t('feedback.title')}</div>
          <button className="feedback-modal-close-btn" onClick={handleClose}>
            <img src={cancelIcon} alt={t('feedback.close')} className="close-btn-icon" />
          </button>
        </div>

        {/* 意见类型 */}
        <div className="feedback-section-inline">
          <div className="feedback-label-fixed">{t('feedback.opinion_type')}</div>
          <div className="feedback-content">
            <div className="opinion-types-row">
              {opinionTypes.map((type) => (
                <label key={type.id} className="opinion-type-item">
                  <input
                    type="radio"
                    name="opinionType"
                    checked={opinionType === type.id}
                    onChange={() => setOpinionType(type.id)}
                  />
                  <span className="opinion-radio-custom"></span>
                  <span className="opinion-label">{type.label}</span>
                </label>
              ))}
            </div>
          </div>
        </div>

        {/* 反馈类型 */}
        <div className="feedback-section-inline">
          <div className="feedback-label-fixed">{t('feedback.feedback_type')}<span className="required">*</span></div>
          <div className="feedback-content">
            <div className="feedback-types-grid">
              {feedbackTypes.map((type) => (
                <label key={type.id} className="feedback-type-item">
                  <input
                    type="checkbox"
                    checked={selectedTypes.has(type.id)}
                    onChange={() => toggleType(type.id)}
                  />
                  <span className="type-checkbox-custom"></span>
                  <span className="type-label">{type.label}</span>
                </label>
              ))}
            </div>
          </div>
        </div>

        {/* 详细描述 */}
        <div className="feedback-section-inline">
          <div className="feedback-label-fixed">{t('feedback.description')}<span className="required">*</span></div>
          <div className="feedback-content">
            <textarea
              className="feedback-textarea"
              placeholder={t('feedback.description_placeholder')}
              value={description}
              onChange={(e) => setDescription(e.currentTarget.value)}
            />
          </div>
        </div>

        {/* 上传日志信息 */}
        <label className="feedback-checkbox-row">
          <input
            type="checkbox"
            checked={uploadLogs}
            onChange={(e) => setUploadLogs(e.currentTarget.checked)}
          />
          <span className="checkbox-custom"></span>
          <span className="checkbox-label">{t('feedback.upload_logs')}</span>
        </label>

        {/* 联系方式 */}
        <div className="feedback-contact-row">
          <span className="feedback-label">{t('feedback.contact_method')}</span>
          <div className="feedback-dropdown-wrapper">
            <button
              className="feedback-dropdown"
              onClick={() => setShowContactDropdown(!showContactDropdown)}
            >
              <span>{selectedContactLabel}</span>
              <img src={downArrowIcon} alt={t('feedback.expand')} className="dropdown-arrow" />
            </button>
            {showContactDropdown && (
              <div className="feedback-dropdown-menu">
                {contactMethods.map((method) => (
                  <div
                    key={method.id}
                    className="feedback-dropdown-item"
                    onClick={() => {
                      setContactMethod(method.id);
                      setShowContactDropdown(false);
                    }}
                  >
                    {method.label}
                  </div>
                ))}
              </div>
            )}
          </div>
          <input
            type="text"
            className="feedback-contact-input"
            placeholder={t('feedback.contact_placeholder', { method: selectedContactLabel })}
            value={contactValue}
            onChange={(e) => {
              // 只允许输入数字、字母、符号（禁止中文）
              const value = e.currentTarget.value.replace(/[\u4e00-\u9fa5]/g, '');
              setContactValue(value);
            }}
          />
        </div>

        {/* 官方联系信息 */}
        <div className="feedback-official-contact">
          {t('feedback.official_contact')} {t('feedback.qrcode_hint')}
        </div>

        {/* QQ 群二维码 */}
        <div className="feedback-qrcode-area">
          <div className="feedback-qrcode-bg">
            <img src="/static/icons/suggestion.jpg" alt={t('feedback.qrcode_alt')} className="feedback-qrcode-img" />
          </div>
        </div>

        {/* 提交按钮 */}
        <button className="feedback-submit-btn" onClick={handleSubmit}>
          {t('feedback.submit')}
        </button>
      </div>
    </div>
  );
}

const container = getRootContainer();
createRoot(container).render(
  <Provider store={store}>
    <I18nextProvider i18n={i18n}>
      <FeedbackApp />
    </I18nextProvider>
  </Provider>,
);
