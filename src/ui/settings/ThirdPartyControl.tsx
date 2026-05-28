import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import helpIcon from "../../static/icons/help.svg";

// 辅助函数：上报设置界面组件点击事件（669000022）
const reportComponentClick = (componentName: string, action: string, value?: string | boolean) => {
  const content = value !== undefined ? `${componentName}:${action}:${value}` : `${componentName}:${action}`;
  (invoke as any)('report_settings_click', { content })
    .catch((e: any) => console.warn('[ThirdPartyControl] 打点上报失败:', e));
};

interface ThirdPartyApp {
  category: string;
  appName: string;
  status: string;
  tipInfo?: string;
}

interface ThirdPartyControlProps {
  onBack: () => void;
  groupedApps: Map<string, ThirdPartyApp[]>;
  setGroupedApps: (groupedApps: Map<string, ThirdPartyApp[]>) => void;
  isDark?: boolean;
}

export function ThirdPartyControl({ onBack, groupedApps, setGroupedApps, isDark = false }: ThirdPartyControlProps) {
  const [thirdPartyAppList, setThirdPartyAppList] = useState<ThirdPartyApp[]>([]);
  const [hoveredTooltip, setHoveredTooltip] = useState<{
    category: string;
    tipInfo: string;
    left: number;
    top: number;
  } | null>(null);
  const { t, i18n } = useTranslation();
  const currentLanguage = (i18n.resolvedLanguage || i18n.language || 'en').toLowerCase();
  const isChineseLayout = currentLanguage.startsWith('zh');
  const detailPaneLeft = isChineseLayout ? '200px' : '240px';
  const detailPaneWidth = isChineseLayout ? '600px' : '560px';

  const getCategoryLabel = (category: string): string => {
    const categoryMap: Record<string, string> = {
      '划词管理': t('third_party.categories.word_selection'),
      '广告弹窗': t('third_party.categories.ad_popup'),
      '捆绑安装': t('third_party.categories.bundle_install'),
      '隐私保护': t('third_party.categories.privacy'),
    };
    return categoryMap[category] || category;
  };

  const C = {
    pageBg:      isDark ? '#1C1C1E'                 : '#F7F8FB',
    cardBg:      isDark ? '#2C2C2E'                 : '#FFFFFF',
    border:      isDark ? 'rgba(255,255,255,0.08)'  : 'rgba(0,0,0,0.08)',
    divider:     isDark ? 'rgba(255,255,255,0.08)'  : 'rgba(0,0,0,0.05)',
    textPrimary: isDark ? 'rgba(255,255,255,0.86)'  : 'rgba(0,0,0,0.9)',
    textSecond:  isDark ? 'rgba(255,255,255,0.6)'   : 'rgba(0,0,0,0.6)',
    toggleOff:   isDark ? 'rgba(255,255,255,0.2)'   : 'rgba(0,0,0,0.2)',
    tooltipBg:   isDark ? 'rgba(44,44,46,0.96)'     : 'rgba(255,255,255,0.98)',
    tooltipText: isDark ? 'rgba(255,255,255,0.86)'  : 'rgba(0,0,0,0.75)',
    tooltipShadow: isDark ? '0 10px 24px rgba(0,0,0,0.35)' : '0 8px 20px rgba(0,0,0,0.12)',
    iconFilter:  isDark ? 'invert(1) opacity(0.7)'  : 'none',
  };

  const getCategoryTipInfo = (apps: ThirdPartyApp[]): string | null => {
    const matchedApp = apps.find((app) => app.tipInfo?.trim());
    return matchedApp?.tipInfo?.trim() || null;
  };

  useEffect(() => {
    // 从 groupedApps 中提取所有应用到 thirdPartyAppList
    const allApps: ThirdPartyApp[] = [];
    groupedApps.forEach((apps) => {
      allApps.push(...apps);
    });
    setThirdPartyAppList(allApps);
  }, [groupedApps]);

  const showCategoryTip = (category: string, tipInfo: string, target: HTMLElement) => {
    const rect = target.getBoundingClientRect();
    setHoveredTooltip({
      category,
      tipInfo,
      left: rect.right + 8,
      top: rect.bottom,
    });
  };

  const hideCategoryTip = (category: string) => {
    setHoveredTooltip((current) => current?.category === category ? null : current);
  };

  // 切换应用状态
  const handleToggleApp = async (appName: string, category: string, enabled: boolean) => {
    try {
      // 1. 先更新本地状态（让 UI 立即变化，无延迟）
      const newGroupedApps = new Map(groupedApps);
      const apps = newGroupedApps.get(category);
      if (apps) {
        const updatedApps = apps.map(app =>
          app.appName === appName ? { ...app, status: enabled ? '1' : '0' } : app
        );
        newGroupedApps.set(category, updatedApps);
        setGroupedApps(newGroupedApps);
      }

      // 同时更新本地状态
      const updatedList: ThirdPartyApp[] = thirdPartyAppList.map(item =>
        item.appName === appName ? { ...item, status: enabled ? '1' : '0' } : item
      );
      setThirdPartyAppList(updatedList);

      // 2. 打点
      reportComponentClick('ThirdPartyApp', enabled ? 'enable' : 'disable', `${category}_${appName}`);

      // 3. 再调用后端 API（异步发送，不需要等待结果）
      await invoke('send_third_party_app_status', {
        category: category,
        appName: appName,
        status: enabled ? '1' : '0'
      });
    } catch (e) {
      console.error('[ThirdPartyControl] Failed to toggle app status:', e);
      console.error('[ThirdPartyControl] Error stack:', e);
    }
  };

  return (
    <div className="third-party-control-content" style={{ position: 'absolute', left: detailPaneLeft, top: '36px', width: detailPaneWidth, height: '464px', background: C.pageBg, overflowY: 'auto', zIndex: 100 }}>
      {/* 内容区域 */}
      <div style={{
        padding: '24px 36px',
        height: '100%',
        overflowY: 'auto',
        position: 'relative',
        zIndex: 1
      }}>
        {/* 分组应用列表 */}
        <div style={{
          display: 'flex',
          flexDirection: 'column',
          gap: '24px'
        }}>
          {Array.from(groupedApps.entries()).map(([category, apps]) => {
            const categoryTipInfo = getCategoryTipInfo(apps);

            return (
              <div key={category}>
              {/* 组标题 */}
              <div style={{
                position: 'relative',
                height: '18px',
                fontFamily: 'HONOR Sans Design',
                fontSize: '14px',
                fontWeight: 500,
                lineHeight: 'normal',
                letterSpacing: '0px',
                color: C.textSecond,
                marginBottom: '12px',
                display: 'flex',
                alignItems: 'center',
                gap: '4px'
              }}>
                <span>{getCategoryLabel(category)}</span>
                {categoryTipInfo && (
                  <div style={{ position: 'relative', display: 'inline-flex', alignItems: 'center' }}>
                    <button
                      type="button"
                      onMouseEnter={(event: any) => showCategoryTip(category, categoryTipInfo, event.currentTarget)}
                      onMouseLeave={() => hideCategoryTip(category)}
                      onFocus={(event: any) => showCategoryTip(category, categoryTipInfo, event.currentTarget)}
                      onBlur={() => hideCategoryTip(category)}
                      style={{
                        width: '16px',
                        height: '16px',
                        display: 'inline-flex',
                        alignItems: 'center',
                        justifyContent: 'center',
                        padding: 0,
                        border: 'none',
                        background: 'transparent',
                        cursor: 'pointer'
                      }}
                    >
                      <img
                        src={helpIcon}
                        alt="help"
                        style={{
                          width: '16px',
                          height: '16px',
                          filter: C.iconFilter,
                          opacity: 0.72
                        }}
                      />
                    </button>
                  </div>
                )}
              </div>

              {/* 应用列表 */}
              <div style={{
                display: 'flex',
                flexDirection: 'column',
                gap: '0px',
                borderRadius: '12px',
                overflow: 'hidden',
                border: `1px solid ${C.border}`,
                background: C.cardBg
              }}>
                {apps.map((app, appIndex) => (
                  <div
                    key={appIndex}
                    style={{
                      padding: '14px 16px',
                      display: 'flex',
                      alignItems: 'center',
                      justifyContent: 'space-between',
                      borderBottom: appIndex < apps.length - 1 ? `1px solid ${C.divider}` : 'none',
                      pointerEvents: 'auto'
                    }}
                  >
                    {/* 应用名称 */}
                    <div style={{
                      flex: 1,
                      fontSize: '14px',
                      fontFamily: 'HONOR Sans Design',
                      color: C.textPrimary,
                      whiteSpace: 'nowrap',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      marginRight: '16px'
                    }}>
                      {app.appName}
                    </div>

                    {/* 开关 */}
                    <button
                      onClick={() => {
                        handleToggleApp(app.appName, app.category, app.status !== '1');
                      }}
                      style={{
                        width: '44px',
                        height: '24px',
                        borderRadius: '12px',
                        border: 'none',
                        cursor: 'pointer',
                        background: app.status === '1' ? '#256FFF' : C.toggleOff,
                        transition: 'background 0.3s',
                        position: 'relative',
                        zIndex: 10,
                        flexShrink: 0
                      }}
                    >
                      <div
                        style={{
                          width: '20px',
                          height: '20px',
                          borderRadius: '50%',
                          background: 'white',
                          position: 'absolute',
                          top: '2px',
                          left: app.status === '1' ? '22px' : '2px',
                          transition: 'left 0.3s'
                        }}
                      />
                    </button>
                  </div>
                ))}
              </div>
              </div>
          )})}
        </div>
      </div>
      {hoveredTooltip && typeof document !== 'undefined' && createPortal(
        <div
          style={{
            position: 'fixed',
            left: `${hoveredTooltip.left}px`,
            top: `${hoveredTooltip.top}px`,
            transform: 'translateY(-100%)',
            width: '280px',
            padding: '12px 16px',
            borderRadius: '16px',
            background: C.tooltipBg,
            boxShadow: C.tooltipShadow,
            color: C.tooltipText,
            fontFamily: 'HONOR Sans Design',
            fontSize: '12px',
            fontWeight: 400,
            lineHeight: '18px',
            whiteSpace: 'normal',
            wordBreak: 'break-word',
            zIndex: 1000,
            pointerEvents: 'none'
          }}
        >
          {hoveredTooltip.tipInfo}
        </div>,
        document.body
      )}
    </div>
  );
}
