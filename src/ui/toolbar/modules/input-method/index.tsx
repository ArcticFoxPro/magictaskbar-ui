import { h } from 'preact';
import { useEffect, useRef, useState, useCallback } from 'preact/hooks';
import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { listen, type Event as TauriEvent } from '@tauri-apps/api/event';
import { FuncCommand, invoke as invokeCommand } from '@magic-ui/lib';
import { SlPopup } from '@shared/components/SlPopup';
import { $open_popups } from '../shared/state/mod';
import { useTranslation } from 'react-i18next';
import './styles.css';

// 毛玻璃效果相关常量
const POPUP_CORNER_RADIUS = 9;

// TSF profile shapes
interface TsfActiveProfile { guid_profile: string; description: string }
interface TsfProfile { kind?: 'tsf'; guid_profile: string; description: string }
interface KeyboardLayoutProfile {
  kind: 'keyboardLayout';
  hkl: string;
  klid: string;
  langid: number;
  description: string;
  active?: boolean;
}
type InputMethodListItem = TsfProfile | KeyboardLayoutProfile;
type ToolbarInputMethodMode = '中' | '英' | 'A' | 'EN';

// 检测是否为搜狗输入法
function isSougouInputMethod(desc?: string): boolean {
  if (!desc) return false;
  return desc.toLowerCase().includes('sougou') || desc.includes('搜狗');
}

// 检测是否为百度输入法
function isBaiduInputMethod(desc?: string): boolean {
  if (!desc) return false;
  return desc.toLowerCase().includes('baidu') || desc.includes('百度');
}

// 检测是否为微软拼音
function isMicrosoftPinyinInputMethod(desc?: string): boolean {
  if (!desc) return false;
  return desc.toLowerCase().includes('microsoft pinyin') || desc.includes('微软拼音');
}

// 检测是否为微信输入法
function isWechatInputMethod(desc?: string): boolean {
  if (!desc) return false;
  return desc.toLowerCase().includes('weixin') || desc.toLowerCase().includes('wechat') || desc.includes('微信');
}

// 检测是否为讲飞输入法
function isXunfeiInputMethod(desc?: string): boolean {
  if (!desc) return false;
  return desc.toLowerCase().includes('讯飞') || desc.toLowerCase().includes('iflytek') || desc.includes('讲飞');
}

// 检测是否亪QQ拼音输入法
function isQQPinyinInputMethod(desc?: string): boolean {
  if (!desc) return false;
  return desc.toLowerCase().includes('QQ拼音') || desc.toLowerCase().includes('qq') || desc.includes('QQ拼音');
}


async function getActiveTsf(): Promise<TsfActiveProfile | null> {
  try {
    const live = await tauriInvoke('get_active_input_profile');
    console.debug('[InputMethod] getActiveTsf response:', live);
    if (live && typeof live === 'object' && live !== null) {
      const obj = live as TsfActiveProfile;
      if (obj.description) {
        console.debug('[InputMethod] getActiveTsf success:', obj);
        return obj;
      }
    }
  } catch (e) {
    console.debug('[InputMethod] getActiveTsf error:', e);
  }
  return null;
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

// 带重试的获取已安装输入法列表
async function getInstalledProfilesWithRetry(maxRetries = 3, delayMs = 300): Promise<TsfProfile[]> {
  for (let i = 0; i < maxRetries; i++) {
    try {
      const profs = (await tauriInvoke('get_installed_input_profiles')) as TsfProfile[];
      if (Array.isArray(profs) && profs.length > 0) {
        console.debug('[InputMethod] getInstalledProfiles success', { attempt: i + 1, count: profs.length });
        return profs;
      }
      console.debug('[InputMethod] getInstalledProfiles empty', { attempt: i + 1 });
    } catch (e) {
      console.debug('[InputMethod] getInstalledProfiles failed', { attempt: i + 1, error: String(e) });
    }
    // 除了最后一次，其他失败都等待后重试
    if (i < maxRetries - 1) {
      await sleep(delayMs);
    }
  }
  return [];
}

async function getInstalledKeyboardLayoutsWithRetry(maxRetries = 2, delayMs = 200): Promise<KeyboardLayoutProfile[]> {
  for (let i = 0; i < maxRetries; i++) {
    try {
      const layouts = (await tauriInvoke('get_installed_keyboard_layouts')) as KeyboardLayoutProfile[];
      if (Array.isArray(layouts)) {
        console.debug('[InputMethod] getInstalledKeyboardLayouts success', { attempt: i + 1, count: layouts.length });
        return layouts.map(layout => ({
          kind: 'keyboardLayout',
          hkl: layout.hkl ?? '',
          klid: layout.klid ?? '',
          langid: layout.langid ?? 0,
          description: layout.description ?? '',
          active: !!layout.active,
        }));
      }
    } catch (e) {
      console.debug('[InputMethod] getInstalledKeyboardLayouts failed', { attempt: i + 1, error: String(e) });
    }
    if (i < maxRetries - 1) {
      await sleep(delayMs);
    }
  }
  return [];
}

export default function InputMethodModule() {
  const { t } = useTranslation();
  const [toolbarMode, setToolbarMode] = useState<ToolbarInputMethodMode>('中');
  const [popupOpen, setPopupOpen] = useState<boolean>(false);
  const popupOpenRef = useRef(false);
  // Popup data state
  const [inputMethodItems, setInputMethodItems] = useState<InputMethodListItem[]>([]);
  const [activeTipGuid, setActiveTipGuid] = useState<string | null>(null);
  const activeTipGuidRef = useRef<string | null>(null);
  const programmaticSwitchSuppressUntilRef = useRef(0);
  const programmaticRefreshTimeoutRef = useRef<number | null>(null);
  // 毛玻璃效果相关 ref
  const popupGlassRef = useRef<HTMLDivElement | null>(null);
  const glassUpdateSeqRef = useRef(0);
  // 仅过滤"微软五笔"和"快捷入口"，其他正常显示
  const filteredInputMethodItems = inputMethodItems.filter((p) => {
    if (p.kind === 'keyboardLayout') return true;
    const d = (p.description ?? '').toLowerCase();
    return !d.includes('微软五笔') && !d.includes('快捷入口');
  });
  // 毛玻璃效果：显示弹窗模糊
  const showPopupGlass = useCallback(() => {
    if (!popupOpenRef.current) return;
    const scheduledSeq = glassUpdateSeqRef.current;

    const el = popupGlassRef.current;
    if (!el) return;
    
    const rect = el.getBoundingClientRect();
    // 必须同时检查坐标和尺寸，确保元素已正确渲染
    if (rect.left <= -9999 || rect.top <= -9999 || rect.width <= 0 || rect.height <= 0) return;
    
    requestAnimationFrame(() => {
      if (!popupOpenRef.current || scheduledSeq !== glassUpdateSeqRef.current) return;
      
      (invokeCommand as any)('popup_glass_show', {
        id: 'input-method-primary',
        x: Math.round(rect.left),
        y: Math.round(rect.top),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
        cornerRadius: POPUP_CORNER_RADIUS
      }).catch((e: any) => {
        console.warn('[InputMethod] Failed to show glass effect:', e);
      });
    });
  }, []);
  
  // 毛玻璃效果：隐藏弹窗模糊
  const hidePopupGlass = useCallback(() => {
    glassUpdateSeqRef.current++;
    (invokeCommand as any)('popup_glass_hide', { id: 'input-method-primary' }).catch((e: any) => {
      console.warn('[InputMethod] Failed to hide glass effect:', e);
    });
  }, []);

  const closeInputMethodPopup = useCallback(() => {
    popupOpenRef.current = false;
    hidePopupGlass();
    setPopupOpen(false);
  }, [hidePopupGlass]);

  const applyActiveInputMethodProfile = useCallback((profile: TsfActiveProfile | null) => {
    const nextGuid = profile?.guid_profile ? profile.guid_profile.toLowerCase() : null;
    activeTipGuidRef.current = nextGuid;
    setActiveTipGuid(nextGuid);
    if (nextGuid) {
      setInputMethodItems(items => items.map(item => (
        item.kind === 'keyboardLayout'
          ? { ...item, active: false }
          : item
      )));
    }
  }, []);

  const applyActiveKeyboardLayout = useCallback((klid: string) => {
    const normalizedKlid = klid.toLowerCase();
    activeTipGuidRef.current = null;
    setActiveTipGuid(null);
    setToolbarMode('EN');
    setInputMethodItems(items => items.map(item => (
      item.kind === 'keyboardLayout'
        ? { ...item, active: item.klid.toLowerCase() === normalizedKlid }
        : item
    )));
  }, []);

  const buildInputMethodItems = useCallback((profs: TsfProfile[], layouts: KeyboardLayoutProfile[]): InputMethodListItem[] => {
    const tsfItems: TsfProfile[] = profs.map(p => ({
      kind: 'tsf',
      guid_profile: (p.guid_profile ?? '').toLowerCase(),
      description: p.description ?? '',
    }));
    const normalizedActiveGuid = activeTipGuidRef.current;
    const normalizedLayouts = normalizedActiveGuid
      ? layouts.map(layout => ({ ...layout, active: false }))
      : layouts;
    if (!normalizedActiveGuid && normalizedLayouts.some(layout => layout.active)) {
      setToolbarMode('EN');
    }
    return [...tsfItems, ...normalizedLayouts];
  }, []);

  const refreshInputMethodState = useCallback(async (refreshProfiles = false) => {
    try {
      const active = await getActiveTsf();
      applyActiveInputMethodProfile(active);

      if (refreshProfiles) {
        const [profs, layouts] = await Promise.all([
          getInstalledProfilesWithRetry(2, 200),
          getInstalledKeyboardLayoutsWithRetry(2, 200),
        ]);
        setInputMethodItems(buildInputMethodItems(profs, layouts));
      }
    } catch (e) {
      console.debug('[InputMethod] refreshInputMethodState error', e);
    }
  }, [applyActiveInputMethodProfile, buildInputMethodItems]);

  useEffect(() => {
    popupOpenRef.current = popupOpen;
    if (!popupOpen) {
      glassUpdateSeqRef.current++;
    }
    $open_popups.value = { ...$open_popups.value, inputMethodPopup: popupOpen };
  }, [popupOpen]);

  // 毛玻璃效果：监听弹窗打开（显示毛玻璃）
  useEffect(() => {
    if (!popupOpen) return;
    let rafId1: number | null = null;
    let rafId2: number | null = null;
    let timeoutId: number | null = null;

    const checkAndShowGlass = () => {
      const el = popupGlassRef.current;
      if (!el) return false;
      const rect = el.getBoundingClientRect();
      if (rect.left > -9999 && rect.top > -9999 && rect.width > 0 && rect.height > 0) {
        showPopupGlass();
        return true;
      }
      return false;
    };
    
    if (!checkAndShowGlass()) {
      rafId1 = requestAnimationFrame(() => {
        rafId1 = null;
        if (!checkAndShowGlass()) {
          rafId2 = requestAnimationFrame(() => {
            rafId2 = null;
            if (!checkAndShowGlass()) {
              timeoutId = window.setTimeout(() => {
                timeoutId = null;
                checkAndShowGlass();
              }, 50);
            }
          });
        }
      });
    }

    return () => {
      if (rafId1 !== null) cancelAnimationFrame(rafId1);
      if (rafId2 !== null) cancelAnimationFrame(rafId2);
      if (timeoutId !== null) clearTimeout(timeoutId);
    };
  }, [popupOpen, showPopupGlass]);

  // 毛玻璃效果：监听弹窗尺寸变化
  useEffect(() => {
    if (!popupOpen) return;
    
    const el = popupGlassRef.current;
    if (!el) return;
    
    let rafId: number | null = null;
    const resizeObserver = new ResizeObserver(() => {
      if (rafId) return;
      rafId = requestAnimationFrame(() => {
        rafId = null;
        showPopupGlass();
      });
    });
    
    resizeObserver.observe(el);
    return () => {
      if (rafId) cancelAnimationFrame(rafId);
      resizeObserver.disconnect();
    };
  }, [popupOpen, showPopupGlass]);

  // 弹窗打开时，重新获取一次已安装输入法列表，保证安装/卸载后列表最新
  useEffect(() => {
    if (!popupOpen) return;
    let cancelled = false;
    (async () => {
      try {
        const active = await getActiveTsf();
        if (cancelled) return;
        applyActiveInputMethodProfile(active);
        console.debug('[InputMethod] popup-open refresh active', {
          guid: active?.guid_profile,
          description: active?.description,
        });
      } catch (e) {
        console.debug('[InputMethod] popup-open refresh active error', e);
      }
    })();

    (async () => {
      try {
        const [profs, layouts] = await Promise.all([
          getInstalledProfilesWithRetry(3, 300),
          getInstalledKeyboardLayoutsWithRetry(2, 200),
        ]);
        if (cancelled) return;
        setInputMethodItems(buildInputMethodItems(profs, layouts));
        console.debug('[InputMethod] popup-open refresh list', {
          tsfCount: profs.length,
          keyboardLayoutCount: layouts.length,
        });
      } catch (e) {
        console.debug('[InputMethod] popup-open refresh list error', e);
      }
    })();

    return () => { cancelled = true; };
  }, [popupOpen, applyActiveInputMethodProfile, buildInputMethodItems]);

  useEffect(() => {
    let mounted = true;
    let unlistenTsf: (() => void) | undefined;
    let unlistenToolbarMode: (() => void) | undefined;
    // 初始化：获取当前 TSF 状态和已安装输入法列表
    (async () => {
      try {
        console.debug('[InputMethod] init start');
        
        // 获取 TSF（带简单重试，因为后端可能在初始化中）
        let tsf = await getActiveTsf();
        if (!tsf) {
          console.debug('[InputMethod] first TSF query returned null, retrying after delay');
          await sleep(500);
          tsf = await getActiveTsf();
        }
        
        // 获取已安装输入法列表（带重试）
        const [profs, layouts] = await Promise.all([
          getInstalledProfilesWithRetry(3, 300),
          getInstalledKeyboardLayoutsWithRetry(2, 200),
        ]);
        
        if (!mounted) return;
        
        // 保存所有已安装的输入法
        setInputMethodItems(buildInputMethodItems(profs, layouts));
        if (Array.isArray(profs) && profs.length > 0) {
          console.debug('[InputMethod] init: profiles loaded', { tsfCount: profs.length, keyboardLayoutCount: layouts.length });
        } else {
          console.debug('[InputMethod] init: no profiles found after retries');
        }
        
        console.debug('[InputMethod] init data', { tsf, profilesCount: Array.isArray(profs) ? profs.length : 0 });
        console.debug('[InputMethod] init TSF details', { guid: tsf?.guid_profile, description: tsf?.description });
        
        if (tsf && tsf.guid_profile && tsf.description) {
          applyActiveInputMethodProfile(tsf);
        } else {
          console.debug('[InputMethod] init: TSF not available');
        }
      } catch (e) {
        console.debug('[InputMethod] init error', e);
      }
    })();

    // TSF watcher event：所有 TSF 变化都进行输入法更新
    listen<TsfActiveProfile>('tsf_active_profile_changed', (ev: TauriEvent<TsfActiveProfile>) => {
      if (!mounted) return;
      const p = ev.payload;
      console.debug('[InputMethod] TSF event', { payload: p });
      const suppressUntil = programmaticSwitchSuppressUntilRef.current;
      if (suppressUntil !== 0 && Date.now() <= suppressUntil) {
        console.debug('[InputMethod] TSF event suppressed during programmatic switch', {
          guid: p?.guid_profile,
          suppressUntil,
        });
        return;
      }
      if (p && p.guid_profile && p.description) {
        applyActiveInputMethodProfile(p);
      }
    }).then((un) => {
      if (!mounted) {
        un();
        return;
      }
      unlistenTsf = un;
    }).catch(() => {});

    listen<ToolbarInputMethodMode>('input_method_toolbar_mode_changed', (ev: TauriEvent<ToolbarInputMethodMode>) => {
      if (!mounted) return;
      const nextMode = ev.payload;
      if (nextMode === '中' || nextMode === '英' || nextMode === 'A' || nextMode === 'EN') {
        console.debug('[InputMethod] toolbar mode event', { mode: nextMode });
        setToolbarMode(nextMode);
      }
    }).then((un) => {
      if (!mounted) {
        un();
        return;
      }
      unlistenToolbarMode = un;
    }).catch(() => {});

    return () => {
      mounted = false;
      if (programmaticRefreshTimeoutRef.current !== null) {
        clearTimeout(programmaticRefreshTimeoutRef.current);
        programmaticRefreshTimeoutRef.current = null;
      }
      if (unlistenTsf) unlistenTsf();
      if (unlistenToolbarMode) unlistenToolbarMode();
    };
  }, [applyActiveInputMethodProfile, buildInputMethodItems]);

  // 构建弹窗内容：只显示非中文(简体)的键盘布局和 TSF 输入法
  const PopupContent = (
    <div className="ime-popup" ref={popupGlassRef}>
      <div className="ime-popup-title">{t('input_method.title')}</div>
      <ul className="ime-list">
        {/* TSF 输入法 */}
        {filteredInputMethodItems.map(p => {
          const isKeyboardLayout = p.kind === 'keyboardLayout';
          const isActive = isKeyboardLayout ? !!p.active : activeTipGuid === (p.guid_profile ?? '').toLowerCase();
          // 移除输入法描述中的"中文" 前缀，保留"-"之后的名称
          const cleanDescription = (p.description ?? '').split(/-|-|\u2013|\u2014/)[1]?.trim() || (p.description ?? '');
          const onClick = async () => {
            try {
              if (isKeyboardLayout) {
                console.debug('[InputMethod][KeyboardLayout] click switch start', {
                  hkl: p.hkl,
                  klid: p.klid,
                  langid: p.langid,
                  description: p.description,
                });
                closeInputMethodPopup();
                try {
                  await tauriInvoke('activate_keyboard_layout', { args: { klid: p.klid } });
                  applyActiveKeyboardLayout(p.klid);
                  await refreshInputMethodState(true);
                } catch (e) {
                  console.error('[InputMethod] keyboard layout activation failed', e);
                  await refreshInputMethodState(true);
                }
              } else if (!isActive && p.guid_profile) {
                const targetGuid = (p.guid_profile ?? '').toLowerCase();
                console.debug('[InputMethod][TSF] click switch start (by hotkey)', { targetGuid, description: p.description });
                closeInputMethodPopup();
                programmaticSwitchSuppressUntilRef.current = Date.now() + 10000;
                
                try {
                  const switchedProfile = await tauriInvoke('activate_input_profile', { args: { guidProfile: targetGuid } }) as TsfActiveProfile | null;
                  if (switchedProfile?.guid_profile && switchedProfile.guid_profile.toLowerCase() === targetGuid) {
                    applyActiveInputMethodProfile(switchedProfile);
                  } else {
                    await refreshInputMethodState(true);
                  }
                } catch (e) {
                  console.error('[InputMethod] activation by hotkey failed', e);
                  await refreshInputMethodState(true);
                }

                if (programmaticRefreshTimeoutRef.current !== null) {
                  clearTimeout(programmaticRefreshTimeoutRef.current);
                }
                programmaticRefreshTimeoutRef.current = window.setTimeout(() => {
                  programmaticRefreshTimeoutRef.current = null;
                  void refreshInputMethodState(true).finally(() => {
                    programmaticSwitchSuppressUntilRef.current = 0;
                  });
                }, 260);
              } else {
                closeInputMethodPopup();
              }
              console.debug('[InputMethod][TSF] popup closed after click');
            } catch (e) {
              console.debug('[InputMethod] click handler error', e);
            }
          };
          return (
            <div key={isKeyboardLayout ? `keyboard-${p.klid}-${p.hkl}` : p.guid_profile} className="ime-item-container">
              <li className={isActive ? 'active' : ''} role="button" tabIndex={0} onClick={onClick}>
                {isKeyboardLayout ? (
                  <span className="ime-icon-text">EN</span>
                ) : isSougouInputMethod(p.description) ? (
                  <img src="/static/icons/sougou.ico" alt={t('input_method.sougou')} className="ime-icon" />
                ) : isBaiduInputMethod(p.description) ? (
                  <img src="/static/icons/baidu.ico" alt={t('input_method.baidu')} className="ime-icon" />
                ) : isWechatInputMethod(p.description) ? (
                  <img src="/static/icons/weixin.ico" alt={t('input_method.wechat')} className="ime-icon" />
                ) : isXunfeiInputMethod(p.description) ? (
                  <img src="/static/icons/xunfei.ico" alt={t('input_method.xunfei')} className="ime-icon" />
                ) : isQQPinyinInputMethod(p.description) ? (
                  <img src="/static/icons/qq.ico" alt={t('input_method.qq_pinyin')} className="ime-icon" />
                ) : isMicrosoftPinyinInputMethod(p.description) ? (
                  <svg className="ime-icon" xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 20 20" fill="none">
                    <path d="M7.4554815703125,11.3816404L7.4554815703125,14.95136Q7.4554815703125,15.695,7.1408407703125,16.038280999999998Q6.8288409703125,16.36836,6.1347627703125,16.425641Q5.5574815703125005,16.472360000000002,5.0504814703125,16.433359L4.8191220803125,15.219281Q5.2948407703125,15.284281,5.7214033703125,15.268641Q6.0254814703125,15.255641,6.1634032703125,15.117719Q6.3011221703124995,14.967,6.3011221703124995,14.621282L6.3011221703124995,11.8366404Q5.6874815703124995,12.0576401,4.7748408303125,12.4007187L4.4914814503125005,11.1736412Q5.4874033703125,10.8459997,6.3011221703124995,10.5782814L6.3011221703124995,8.0146403L4.7671220903125,8.0146403L4.7671220903125,6.9096403L6.3011221703124995,6.9096403L6.3011221703124995,4.5903586999999995L7.4554815703125,4.5903586999999995L7.4554815703125,6.9096403L8.5968408703125,6.9096403L8.5968408703125,8.0146403L7.4554815703125,8.0146403L7.4554815703125,10.172640300000001Q8.0067629703125,9.9593592,8.5344815703125,9.7513595L8.734762670312499,10.8407192Q8.1054816703125,11.1189995,7.4554815703125,11.3816404ZM14.6237630703125,11.8963594L14.6237630703125,16.506281L13.4094819703125,16.506281L13.4094819703125,11.8963594L11.4647626703125,11.8963594Q11.360763070312501,13.422641,10.8797626703125,14.47036Q10.3831219703125,15.539,9.2624816703125,16.641359L8.2537627703125,15.822359Q9.272840970312501,14.894281,9.7071223703125,14.059641Q10.1544031703125,13.198999,10.2504815703125,11.8963594L8.6594033703125,11.8963594L8.6594033703125,10.7732811L10.2921223703125,10.7732811Q10.2921223703125,10.651,10.2921223703125,10.5157185L10.2921223703125,8.019718600000001L9.1038408703125,8.019718600000001L9.1038408703125,6.9329996L12.9104032703125,6.9329996Q13.5291223703125,5.6927185,13.9764032703125,4.5279999L15.1021220703125,4.907640499999999Q14.5664820703125,6.1373596,14.1504810703125,6.9329996L16.1264820703125,6.9329996L16.1264820703125,8.019718600000001L14.6237630703125,8.019718600000001L14.6237630703125,10.7732811L16.4334040703125,10.7732811L16.4334040703125,11.8963594L14.6237630703125,11.8963594ZM13.4094819703125,10.7732811L13.4094819703125,8.019718600000001L11.5037627703125,8.019718600000001L11.5037627703125,10.5522814Q11.5037627703125,10.6639996,11.5037627703125,10.7732811L13.4094819703125,10.7732811ZM10.6301221703125,4.5852813999999995Q11.4154033703125,5.9346409,12.0394038703125,7.3412809L10.9344033703125,7.7467184Q10.3128409703125,6.295999500000001,9.519840670312501,4.9543591L10.6301221703125,4.5852813999999995Z" fill="currentColor" fillOpacity="0.9"/>
                  </svg>
                ) : (
                  <span className="ime-icon-text">{t('input_method.input')}</span>
                )}
                <div className="ime-content">
                  <div className="ime-title">{isKeyboardLayout ? 'Keyboard' : t('input_method.simplified_chinese')}</div>
                  <div className="ime-description">{cleanDescription}</div>
                </div>
                {isActive && <img src="/static/icons/confirm.svg" alt={t('input_method.current')} className="ime-badge" />}
              </li>
            </div>
          );
        })}
        {/* 当没有TSF输入法时昷示提示 */}
        {filteredInputMethodItems.length === 0 && (
          <li className="empty">{t('input_method.not_detected')}</li>
        )}
        {/* 分隔线 */}
        <div className="ime-divider"></div>
        {/* 更多键盘设置 */}
        <div className="ime-settings-container">
          <div className="ime-settings-inner" onClick={async () => {
          // 关闭弹窗
          closeInputMethodPopup();
          try {
              await invokeCommand((window as any).FuncCommand?.SystemOpenLanguageSettings ?? ("system_open_language_settings" as any));
            } catch (e) {
              try {
                // fallback: use tauri invoke directly
                await (window as any).__TAURI_INVOKE_?.("system_open_language_settings");
              } catch {}
            }
          }}>
            <div className="ime-more-settings">{t('input_method.more_settings')}</div>
          </div>
        </div>
      </ul>
    </div>
  );

  // Debug: Always log the state to see why it's falling back to "中"
  console.debug('[InputMethod] render-state', { 
    toolbarMode,
    activeTipGuid: activeTipGuidRef.current,
    installedCount: filteredInputMethodItems.length,
  });

  // 毛玻璃效果：监听弹窗状态变化
  const prevPopupOpenRef = useRef(false);
  
  useEffect(() => {
    // 当从 true 变为 false 时，立即隐藏毛玻璃
    if (prevPopupOpenRef.current && !popupOpen) {
      hidePopupGlass();
    }
    prevPopupOpenRef.current = popupOpen;
    
    if (!popupOpen) return;
    showPopupGlass();
  }, [popupOpen, showPopupGlass, hidePopupGlass]);

  // 处理弹窗打开事件
  const handleOpenChange = useCallback((open: boolean) => {
    popupOpenRef.current = open;
    if (open) {
      // 上报输入法按钮点击
      (tauriInvoke as any)(FuncCommand.ReportClickComponent, { content: "InputMethod" });
      
      setPopupOpen(true);
    } else {
      closeInputMethodPopup();
    }
  }, [closeInputMethodPopup]);

  return (
    <SlPopup placement="top" offset={4} align="start" content={PopupContent} open={popupOpen} onOpenChange={handleOpenChange}>
      <div className={`taskbar-item taskbar-module input-method-module${popupOpen ? ' selected' : ''}`}>
        <span className="input-method-label">{toolbarMode}</span>
      </div>
    </SlPopup>
  );
}
