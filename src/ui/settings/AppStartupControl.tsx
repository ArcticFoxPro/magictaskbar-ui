import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

// 辅助函数：上报设置界面组件点击事件（669000022）
const reportComponentClick = (componentName: string, action: string, value?: string | boolean) => {
  const content = value !== undefined ? `${componentName}:${action}:${value}` : `${componentName}:${action}`;
  (invoke as any)('report_settings_click', { content })
    .catch((e: any) => console.warn('[AppStartupControl] 打点上报失败:', e));
};

export interface AppStartupInfo {
  name: string;
  displayName: string;
  displayNameUtf8: string;
  description: string;
  descriptionUtf8: string;
  status: boolean; // true: open, false: close
}

interface AppStartupControlProps {
  onBack: () => void;
  appList: AppStartupInfo[];
  setAppList: (list: AppStartupInfo[]) => void;
  isDark?: boolean;
}

export function AppStartupControl({ onBack, appList, setAppList, isDark = false }: AppStartupControlProps) {
  const { t, i18n } = useTranslation();
  const currentLanguage = (i18n.resolvedLanguage || i18n.language || 'en').toLowerCase();
  const isChineseLayout = currentLanguage.startsWith('zh');
  const detailPaneLeft = isChineseLayout ? '200px' : '240px';
  const detailPaneWidth = isChineseLayout ? '600px' : '560px';
  const listCardWidth = isChineseLayout ? '528px' : '488px';
  const listRowWidth = isChineseLayout ? '496px' : '456px';
  const textRightInset = isChineseLayout ? '70px' : '52px';
  const toggleLeft = isChineseLayout ? '454px' : '412px';
  const C = {
    pageBg:      isDark ? '#1C1C1E'                : '#F7F8FB',
    cardBg:      isDark ? '#2C2C2E'                : '#FFFFFF',
    border:      isDark ? 'rgba(255,255,255,0.08)' : 'rgba(0,0,0,0.08)',
    divider:     isDark ? 'rgba(255,255,255,0.08)' : 'rgba(0,0,0,0.05)',
    textPrimary: isDark ? 'rgba(255,255,255,0.86)' : 'rgba(0,0,0,0.9)',
    textSecond:  isDark ? 'rgba(255,255,255,0.6)'  : 'rgba(0,0,0,0.6)',
    toggleOff:   isDark ? 'rgba(255,255,255,0.2)'  : 'rgba(0,0,0,0.2)',
  };

  const handleToggle = async (app: AppStartupInfo, enabled: boolean) => {
    try {
      // 1. 立即更新本地状态
      const updated = appList.map(item =>
        item.name === app.name ? { ...item, status: enabled } : item
      );
      setAppList(updated);

      // 2. 打点
      reportComponentClick('AppStartup', enabled ? 'enable' : 'disable', app.name);

      // 3. 通知后端发送 WM_COPYDATA(dwData=0x03) 给 MagicSpaceTurbo（传完整字段）
      await (invoke as any)('send_app_startup_status', {
        name: app.name,
        displayName: app.displayName,
        displayNameUtf8: app.displayNameUtf8,
        description: app.description,
        descriptionUtf8: app.descriptionUtf8,
        status: enabled,
      });
    } catch (e) {
      console.error('[AppStartupControl] Failed to toggle app status:', e);
    }
  };

  return (
    <div style={{
      position: 'absolute', left: detailPaneLeft, top: '36px',
      width: detailPaneWidth, height: '464px',
      background: C.pageBg, overflowY: 'auto', zIndex: 100
    }}>
      {/* 标题 */}
      <div style={{
        position: 'absolute',
        left: '52px',
        top: '16px',
        width: '480px',
        height: '18px',
        fontFamily: 'HONOR Sans Design',
        fontSize: '14px',
        fontWeight: 500,
        lineHeight: 'normal',
        letterSpacing: '0px',
        color: C.textPrimary,
      }}>{t('app_startup.title')}</div>
      <div style={{ padding: '52px 36px 24px', height: '100%', overflowY: 'auto', position: 'relative', zIndex: 1 }}>
        {appList.length === 0 ? (
          <div style={{
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            height: '100%', color: C.textSecond,
            fontFamily: 'HONOR Sans Design', fontSize: '14px'
          }}>
            {t('app_startup.empty')}
          </div>
        ) : (
          <div style={{
            position: 'absolute',
            left: '36px',
            top: '44px',
            width: listCardWidth,
            borderRadius: '12px',
            display: 'flex', flexDirection: 'column',
            justifyContent: 'center',
            padding: '0px 16px',
            alignSelf: 'stretch',
            border: `1px solid ${C.border}`,
            background: C.cardBg,
            zIndex: 1,
          }}>
            {appList.map((app, index) => (
              <div
                key={app.name}
                style={{
                  width: listRowWidth,
                  height: '64px',
                  position: 'relative',
                }}
              >
                {/* 分隔线 */}
                {index < appList.length - 1 && (
                  <div style={{
                    position: 'absolute',
                    left: '0',
                    bottom: 0,
                    width: listRowWidth,
                    height: '1px',
                    background: C.divider,
                    zIndex: 3,
                  }} />
                )}
                {/* 左侧：名称 + 描述 */}
                <div style={{ position: 'absolute', left: '0', top: '13px', right: textRightInset, overflow: 'hidden' }}>
                  <div style={{
                    fontFamily: 'HONOR Sans Design',
                    fontSize: '14px',
                    fontWeight: 'normal',
                    lineHeight: 'normal',
                    letterSpacing: '0px',
                    color: C.textPrimary,
                    whiteSpace: 'nowrap',
                    overflow: 'hidden', textOverflow: 'ellipsis',
                    zIndex: 0,
                  }}>
                    {app.displayNameUtf8 || app.displayName || app.name}
                  </div>
                  {(app.descriptionUtf8 || app.description) && (
                    <div style={{
                      fontFamily: 'HONOR Sans Design',
                      fontSize: '12px',
                      fontWeight: 'normal',
                      lineHeight: 'normal',
                      letterSpacing: '0px',
                      color: C.textSecond,
                      marginTop: '2px',
                      whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
                      zIndex: 1,
                    }}>
                      {app.descriptionUtf8 || app.description}
                    </div>
                  )}
                </div>

                {/* 右侧：开关 */}
                <button
                  onClick={() => handleToggle(app, !app.status)}
                  style={{
                    position: 'absolute', left: toggleLeft, top: '20px',
                    width: '44px', height: '24px', borderRadius: '12px',
                    border: 'none', cursor: 'pointer',
                    background: app.status ? '#256FFF' : C.toggleOff,
                    transition: 'background 0.3s',
                  }}
                >
                  <div style={{
                    width: '20px', height: '20px', borderRadius: '50%',
                    background: 'white', position: 'absolute', top: '2px',
                    left: app.status ? '22px' : '2px',
                    transition: 'left 0.3s'
                  }} />
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
