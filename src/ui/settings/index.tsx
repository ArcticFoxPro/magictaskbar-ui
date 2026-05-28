import { useEffect, useState, useRef } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { I18nextProvider } from "react-i18next";
import { ThirdPartyControl } from './ThirdPartyControl';
import { AppStartupControl, type AppStartupInfo } from './AppStartupControl';
import i18n, { loadTranslations } from './i18n';
import minimizeIcon from "../../static/icons/minimize.svg";
import closeIcon from "../../static/icons/close.svg";
import statusbarIcon from "../../static/icons/statusbar.svg";
import statusbarDarkIcon from "../../static/icons/statusbar_dark.svg";
import taskbarIcon from "../../static/icons/taskbar.svg";
import taskbarDarkIcon from "../../static/icons/taskbar_dark.svg";
import privacyIcon from "../../static/icons/privacy.svg";
import privacyDarkIcon from "../../static/icons/privacy_dark.svg";
import puremodeIcon from "../../static/icons/puremode.svg";
import puremodeDarkIcon from "../../static/icons/puremode_dark.svg";
import weilanWorkstationIcon from "../../static/icons/weilan_workstation.svg";
import weilanPcmanagerIcon from "../../static/icons/weilan_pcmanager.svg";
const weilanYoyoIcon = "../../static/icons/weilan_yoyo.webp";
import weilanAppmarketIcon from "../../static/icons/weilan_appmarket.svg";
import tongtouWorkstationIcon from "../../static/icons/tongtou_workstation.svg";
import tongtouPcmanagerIcon from "../../static/icons/tongtou_pcmanager.svg";
import tongtouYoyoIcon from "../../static/icons/tongtou_yoyo.svg";
import tongtouAppmarketIcon from "../../static/icons/tongtou_appmarket.svg";
import checkmarkIcon from "../../static/icons/checkmark2.svg";
import checkboxIcon from "../../static/icons/checkbox.svg";
import rectangleIcon from "../../static/icons/rectangle.svg";
import shadowIcon from "../../static/icons/shadow.svg";
import downArrowIcon from "../../static/icons/DownArrow.svg";
import backIcon from "../../static/icons/back.svg";
import rotateIcon from "../../static/icons/rotate.svg";
import rotateDarkIcon from "../../static/icons/rotate_dark.svg";

function SettingsApp() {
  const { t, i18n } = useTranslation();
  const currentLanguage = (i18n.resolvedLanguage || i18n.language || 'en').toLowerCase();
  const isChineseLayout = currentLanguage.startsWith('zh');
  const sidebarWidth = isChineseLayout ? 200 : 240;
  const detailPageBackButtonLeft = sidebarWidth + 16;
  // 深浅色模式检测
  const [isDark, setIsDark] = useState(() =>
    window.matchMedia('(prefers-color-scheme: dark)').matches
  );
  useEffect(() => {
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const handler = (e: MediaQueryListEvent) => setIsDark(e.matches);
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  }, []);

  // 颜色变量（随深浅色模式切换）
  const C = {
    pageBg:       isDark ? '#1C1C1E'                   : '#F7F8FB',
    sidebarBg:    isDark ? '#2C2C2E'                   : '#EDEEF1',
    cardBg:       isDark ? '#2C2C2E'                   : '#FFFFFF',
    cardBgHover:  isDark ? '#3A3A3C'                   : '#fafafa',
    subBg:        isDark ? '#3A3A3C'                   : '#F3F3F5',
    border:       isDark ? 'rgba(255,255,255,0.08)'    : 'rgba(0,0,0,0.08)',
    borderHover:  isDark ? 'rgba(255,255,255,0.15)'    : 'rgba(0,0,0,0.12)',
    divider:      isDark ? 'rgba(255,255,255,0.08)'    : 'rgba(0,0,0,0.08)',
    textPrimary:  isDark ? 'rgba(255,255,255,0.86)'    : 'rgba(0,0,0,0.9)',
    textSecond:   isDark ? 'rgba(255,255,255,0.6)'     : 'rgba(0,0,0,0.6)',
    textDisable:  isDark ? 'rgba(255,255,255,0.4)'     : 'rgba(0,0,0,0.4)',
    textMenu:     isDark ? 'rgba(255,255,255,0.86)'    : '#333',
    hover:        isDark ? 'rgba(255,255,255,0.08)'    : 'rgba(0,0,0,0.05)',
    active:       isDark ? 'rgba(255,255,255,0.15)'    : 'rgba(0,0,0,0.1)',
    menuActive:   isDark ? 'rgba(37,111,255,0.2)'      : 'rgba(31,115,231,0.1)',
    dropdownBg:   isDark ? '#3A3A3C'                   : '#FFFFFF',
    toggleOff:    isDark ? 'rgba(255,255,255,0.2)'     : 'rgba(0,0,0,0.2)',
    iconFilter:   isDark ? 'invert(1)'                : 'none',
    btnColor:     isDark ? 'rgba(255,255,255,0.6)'     : '#666',
  };

  // 从 URL 参数读取初始标签页，如果没有则默认为 'notification'
  const [activeTab, setActiveTab] = useState(() => {
    if (typeof window !== 'undefined') {
      const params = new URLSearchParams(window.location.search);
      const tab = params.get('tab');
      return tab || 'notification';
    }
    return 'notification';
  });
  const [toolbarVisibility, setToolbarVisibility] = useState('always');
  const [showToolbarMenu, setShowToolbarMenu] = useState(false);

  const [taskbarZoomEnabled, setTaskbarZoomEnabled] = useState(true);
  const [minimizeAnimationEnabled, setMinimizeAnimationEnabled] = useState(true);
  const [userExperienceEnabled, setUserExperienceEnabled] = useState(true);
  const [pureModeEnabled, setPureModeEnabled] = useState(false);
  const [defenderDisabled, setDefenderDisabled] = useState(false); // 初始值为 false，表示启用 Defender
  const [stopWuEnabled, setStopWuEnabled] = useState(false); // 服务体验优化开关状态
  const [browserEnhanceEnabled, setBrowserEnhanceEnabled] = useState(false); // 浏览器搜索体验增强开关状态
  const [upgradeEnabled, setUpgradeEnabled] = useState(false); // 升级管理开关状态
  const [thirdPartyAppList, setThirdPartyAppList] = useState<any[]>([]); // 三方软件管控应用列表
  const [showThirdPartyControlPage, setShowThirdPartyControlPage] = useState(false); // 是否显示三方软件管控二级页面
  const [groupedApps, setGroupedApps] = useState<Map<string, any[]>>(new Map()); // 按 category 分组的应用列表
  const [showAppStartupPage, setShowAppStartupPage] = useState(false); // 是否显示开机自启二级页面
  const [appStartupList, setAppStartupList] = useState<AppStartupInfo[]>([]); // 开机自启应用列表
  const [showPrivacyPopup, setShowPrivacyPopup] = useState(false);
  const [showStopServicePopup, setShowStopServicePopup] = useState(false);
  const [selectedTheme, setSelectedTheme] = useState('default');
const [zoomEffectType, setZoomEffectType] = useState<'wave' | 'singleIcon'>('wave');
  const [waveHover, setWaveHover] = useState(false);
  const [singleIconHover, setSingleIconHover] = useState(false);
const [blueThemeHover, setBlueThemeHover] = useState(false);
const [defaultThemeHover, setDefaultThemeHover] = useState(false);

  // Settings 缓存，避免重复读取
  const settingsCache = useRef<any>(null);
  const zoomEffectWriteQueueRef = useRef<Promise<void>>(Promise.resolve());

  // 辅助函数：上报设置界面组件点击事件（669000022）
  const reportComponentClick = (componentName: string, action: string, value?: string | boolean) => {
    const content = value !== undefined ? `${componentName}:${action}:${value}` : `${componentName}:${action}`;
    (invoke as any)('report_settings_click', { content })
      .catch((e: any) => console.warn('[Settings] 打点上报失败:', e));
  };

  useEffect(() => {
    (async () => {
      try {
        const { getCurrentWebviewWindow } = await import("@tauri-apps/api/webviewWindow");
        const win = getCurrentWebviewWindow();

        // 读取系统文字缩放比例，对页面做反向补偿，修复辅助功能文字大小非100%时布局溢出
        try {
          const textScale = await (invoke as any)('get_text_scale_factor') as number;
          if (textScale && textScale > 1.0) {
            const compensation = (1 / textScale) * 100;
            document.documentElement.style.zoom = `${compensation}%`;
          }
        } catch (e) {
          // 获取失败不影响主流程
        }

        // 使用缓存，避免重复读取 settings.json
        let settings = settingsCache.current;
        if (!settings) {
          settings = await invoke('state_get_settings');
          settingsCache.current = settings;
        }

        // 加载工具栏 visiblity
        const hideMode = settings?.byWidget?.fancyToolbar?.hideMode || 'Never';
        setToolbarVisibility(hideMode === 'Never' ? 'always' : 'smart');

        // 仅加载当前标签页需要的状态
        if (activeTab === 'taskbar') {
          // 任务栏标签页：加载放大效果，最小化动画、图标主题
          if (settings?.taskbar) {
            setTaskbarZoomEnabled(settings.taskbar.enableZoomEffect ?? true);

            // 加载放大效果类型
            const savedZoomEffectType = settings.taskbar.zoomEffectType;
            if (savedZoomEffectType) {
              setZoomEffectType(savedZoomEffectType.toLowerCase() === 'singleicon' ? 'singleIcon' : 'wave');
            }

            // 并行读取注册表状态
            const [registryMinimizeEnabled, registryThemeType] = await Promise.all([
              (invoke as any)('get_minimize_animation_from_registry'),
              (invoke as any)('get_icon_theme_from_registry')
            ]);

            setMinimizeAnimationEnabled(registryMinimizeEnabled);
            setSelectedTheme(registryThemeType === 1 ? 'blue' : 'default');
          }
        } else if (activeTab === 'puremode') {
          // 纯净系统标签页：加载纯净系统、服务体验优化、闲时更新、浏览器搜索体验增强
          const [registryPureModeEnabled, registryStopWuEnabled, registryUpgradeEnabled, registryBrowserEnhanceEnabled] = await Promise.all([
            (invoke as any)('get_clean_mode_from_registry'),
            (invoke as any)('get_stop_wu_from_registry'),
            (invoke as any)('get_upgrade_mode_from_registry'),
            (invoke as any)('get_browser_enhance_from_registry')
          ]);

          setPureModeEnabled(registryPureModeEnabled);
          setStopWuEnabled(registryStopWuEnabled);
          setUpgradeEnabled(registryUpgradeEnabled);
          setBrowserEnhanceEnabled(registryBrowserEnhanceEnabled);
        } else if (activeTab === 'upgrade') {
          // 闲时更新标签页
          const registryUpgradeEnabled = await (invoke as any)('get_upgrade_mode_from_registry');
          setUpgradeEnabled(registryUpgradeEnabled);
        } else if (activeTab === 'privacy') {
          // 隐私权限标签页：加载用户体验计划
          const registryUserExperienceEnabled = await (invoke as any)('get_user_experience_plan_from_registry');
          setUserExperienceEnabled(registryUserExperienceEnabled);
        }

        // 通知 MagicSpaceTurbo 初始化三方软件管控列表（异步，不阻塞）
        (invoke as any)('send_message_to_magic_space_turbo')
          .catch((e: any) => console.warn('[Settings] 发送消息到 MagicSpaceTurbo 失败:', e));

        // 通知进程发送开机自启列表（异步，不阻塞）
        (invoke as any)('send_message_to_app_startup')
          .then(() => console.log('[AppStartup] send_message_to_app_startup 发送成功'))
          .catch((e: any) => console.warn('[Settings] 发送消息到 AppStartup 失败:', e));

      } catch (e) {
        console.error('[Settings] Error in useEffect:', e);
      }
    })();
  }, [activeTab]); // 依赖 activeTab，每次切换标签都会触发

  // 监听工具栏设置变化事件，自动更新设置界面
  useEffect(() => {
    (async () => {
      try {
        const { getCurrentWebviewWindow } = await import("@tauri-apps/api/webviewWindow");
        const { listen } = await import("@tauri-apps/api/event");
        const win = getCurrentWebviewWindow();

        const unlisten = await listen('settings-changed', async (event: any) => {

          try {
            // 重新加载设置
            const settings: any = event.payload;
            settingsCache.current = settings;

            // 更新工具栏显示模式
            const hideMode = settings?.byWidget?.fancyToolbar?.hideMode || 'Never';
            const newToolbarVisibility = hideMode === 'Never' ? 'always' : 'smart';
            if (newToolbarVisibility !== toolbarVisibility) {
              setToolbarVisibility(newToolbarVisibility);
            }
          } catch (e) {
            console.error('[Settings] Error handling settings change event:', e);
          }
        });

        // 监听跳转到蜂鸟引擎的消息
        const unlistenPureMode = await listen('navigate-to-pure-mode', async () => {
          // 立即切换标签页
          setActiveTab('puremode');
          // 确保窗口显示和恢复
          try {
            const { getCurrentWebviewWindow } = await import("@tauri-apps/api/webviewWindow");
            const currentWin = getCurrentWebviewWindow();
            await currentWin.unminimize();
            await currentWin.setFocus();
          } catch (e) {
            console.error('[Settings] Error in navigate-to-pure-mode:', e);
          }
        });

        // 监听 MagicSpaceTurbo 发送的三方软件管控列表
        const unlistenThirdPartyList = await listen('third-party-control-list', async (event: any) => {
          try {
            const list = JSON.parse(event.payload);
            setThirdPartyAppList(list);

            // 按 category 分组
            const grouped = new Map<string, any[]>();
            list.forEach((app: any) => {
              const category = app.category;
              if (!grouped.has(category)) {
                grouped.set(category, []);
              }
              grouped.get(category)?.push(app);
            });
            setGroupedApps(grouped);

            // 如果列表不为空，则显示三方软件管控
            if (list && list.length > 0) {
            }
          } catch (e) {
            console.error('[Settings] Failed to parse third party app list:', e);
          }
        });

        // 监听开机自启列表
        const unlistenAppStartupList = await listen('app-startup-list', async (event: any) => {
          try {
            console.log('[AppStartup] 收到 app-startup-list 事件, payload:', event.payload);
            const list: AppStartupInfo[] = JSON.parse(event.payload);
            console.log('[AppStartup] 解析后列表长度:', list.length, '内容:', list);
            setAppStartupList(list);
          } catch (e) {
            console.error('[Settings] Failed to parse app startup list:', e);
          }
        });

        // 所有初始化完成后，显示窗口
        await win.show();

        // 返回清理函数以取消监听
        return () => {
          unlisten();
          unlistenPureMode();
          unlistenThirdPartyList();
          unlistenAppStartupList();
        };
      } catch (e) {
        console.error('[Settings] Error setting up event listener:', e);
      }
    })();
  }, []);

  const handleMinimize = async () => {
    const { getCurrentWebviewWindow } = await import("@tauri-apps/api/webviewWindow");
    getCurrentWebviewWindow().minimize();
  };

  const handleThemeChange = async (themeType: 0 | 1, backplateStyle: 'Transparent' | 'White') => {
    try {
      // 1. 更新UI状态
      const themeName = themeType === 0 ? 'default' : 'blue';
      setSelectedTheme(themeName);
      // 2. 打点
      reportComponentClick('IconTheme', 'switch', `${themeName}_${backplateStyle}`);

      // 发送主题切换消息并等待完成（会写入注册表）
      const result = await (invoke as any)('system_change_theme', { themeType: themeType });

      // 保存设置到 settings.json（这会触发 StateSettingsChanged 事件）
      const settings = settingsCache.current;
      const newSettings = {
        ...settings,
        taskbar: {
          ...(settings?.taskbar ?? {}),
          iconBackplateStyle: backplateStyle,
        },
      };
      await invoke('state_write_settings', { settings: newSettings });
      settingsCache.current = newSettings;

      // 调用后端命令改变背板样式（发送自定义事件给所有窗口通知背板样式变更）
      await (invoke as any)('system_switch_icon_backplate_style', { style: backplateStyle });

    } catch (e) {
      console.error('[Settings] Error changing theme:', e);
      console.error('[Settings] 错误详情:', JSON.stringify(e));
    }
  };

  const handleClose = async () => {
    const { getCurrentWebviewWindow } = await import("@tauri-apps/api/webviewWindow");
    getCurrentWebviewWindow().close();
  };

  const menuItems = [
    { id: 'notification', label: t('menu.notification'), icon: isDark ? statusbarDarkIcon : statusbarIcon },
    { id: 'taskbar', label: t('menu.taskbar'), icon: isDark ? taskbarDarkIcon : taskbarIcon },
    { id: 'puremode', label: t('menu.puremode'), icon: isDark ? puremodeDarkIcon : puremodeIcon },
    { id: 'upgrade', label: t('menu.upgrade'), icon: isDark ? rotateDarkIcon : rotateIcon },
    { id: 'privacy', label: t('menu.privacy'), icon: isDark ? privacyDarkIcon : privacyIcon },
  ];

  const toolbarOptions = [
    { value: 'always', label: t('toolbar.always_show') },
    { value: 'smart', label: t('toolbar.smart_hide') },
  ];

  const getToolbarLabel = () => {
    return toolbarOptions.find(opt => opt.value === toolbarVisibility)?.label || t('toolbar.always_show');
  };

  const handleToolbarModeChange = async (mode: string) => {
    try {
      const hideMode = mode === 'always' ? 'Never' : 'OnOverlap';
      // 1. 更新UI状态
      setToolbarVisibility(mode);
      setShowToolbarMenu(false);
      // 2. 打点
      reportComponentClick('ToolbarMode', 'switch', mode);

      const settings = settingsCache.current;
      const newSettings = {
        ...settings,
        byWidget: {
          ...(settings?.byWidget ?? {}),
          fancyToolbar: {
            ...(settings?.byWidget?.fancyToolbar ?? {}),
            hideMode: hideMode,
          },
        },
      };
      await invoke('state_write_settings', { settings: newSettings });
      settingsCache.current = newSettings;
    } catch (e) {
      console.error('[Settings] Failed to change toolbar mode:', e);
    }
  };

  const handleTaskbarZoomToggle = async (enabled: boolean) => {
    try {
      const toggleAction = enabled ? 'enable' : 'disable';
      // 1. 先更新UI 状态
      setTaskbarZoomEnabled(enabled);
      // 2. 打点
      reportComponentClick('TaskbarZoom', toggleAction, enabled);
      const settings = settingsCache.current;

      const newSettings = {
        ...settings,
        taskbar: {
          ...(settings?.taskbar ?? {}),
          enableZoomEffect: enabled,
        },
      };

      settingsCache.current = newSettings;
      const writeTask = zoomEffectWriteQueueRef.current
        .catch(() => undefined)
        .then(async () => {
          await invoke('state_write_settings', { settings: newSettings });
        });
      zoomEffectWriteQueueRef.current = writeTask;
      await writeTask;
    } catch (e) {
      console.error('[Settings] 更新任务栏放大效果失败', e);
      console.error('[Settings] 错误详情:', (e as any)?.message || String(e));
    }
  };

  const handleOpenCleanModeDeclaration = async () => {
    try {
      // 获取系统语言环境
      const lang = navigator.language || 'zh-CN';

      // 根据语言选择对应的 htm 文件
      let htmFile = 'hummingbird_engine_statement_en-us.htm';
      if (lang.startsWith('zh')) {
        htmFile = 'hummingbird_engine_statement_zh-cn.htm';
      }

      const filePath = `config\\hummingbird\\${htmFile}`;

      // 调用后端函数打开文件
      await (invoke as any)('system_open_file', { filePath });
    } catch (e) {
      console.error('[Settings] Failed to open clean mode declaration:', e);
    }
  };

  const handleOpenUserAgreement = async () => {
    try {
      const lang = navigator.language || 'zh-CN';

      let htmFile = 'usr_agreement_en-us.htm';
      if (lang.startsWith('zh')) {
        htmFile = 'usr_agreement_zh-cn.htm';
      }

      // 使用相对路径，相对于应用程序所在目录
      const filePath = `config/usragreement/${htmFile}`;

      await (invoke as any)('system_open_file', { filePath });
    } catch (e) {
      console.error('[Settings] Failed to open user agreement:', e);
    }
  };

  const handleOpenPrivacyStatement = async () => {
    try {
      const lang = navigator.language || 'zh-CN';

      let htmFile = 'privacy_statement_en-us.htm';
      if (lang.startsWith('zh')) {
        htmFile = 'privacy_statement_zh-cn.htm';
      }

      // 使用相对路径，相对于应用程序所在目录
      const filePath = `config/privacy/${htmFile}`;

      await (invoke as any)('system_open_file', { filePath });
    } catch (e) {
      console.error('[Settings] Failed to open privacy statement:', e);
    }
  };

  const handleUpgradeToggle = async (enabled: boolean) => {
      try {
        const toggleAction = enabled ? 'enable' : 'disable';
        // 1. 先更新 UI 状态
        setUpgradeEnabled(enabled);
        // 2. 打点
        reportComponentClick('Upgrade', toggleAction, enabled);
        (invoke as any)('system_toggle_upgrade_mode', { enabled })
          .then(() => {
          })
          .catch((e: any) => {
            console.warn('[Settings] 2. 调用后端函数执行失败:', e);
          });

        // 保存设置到 settings.json
        const settings = settingsCache.current;
        const newSettings = {
          ...settings,
          upgrade: {
            ...(settings?.upgrade ?? {}),
            enabled: enabled,
          },
        };
        await invoke('state_write_settings', { settings: newSettings });
        settingsCache.current = newSettings;
      } catch (e) {
        console.error('[Settings] 更新升级管理失败', e);
      }
    };

  const handlePureModeToggle = async (enabled: boolean) => {
    try {
      const toggleAction = enabled ? 'enable' : 'disable';
      // 1. 先更新 UI 状态
      setPureModeEnabled(enabled);
      // 2. 打点
      reportComponentClick('PureMode', toggleAction, enabled);
      (invoke as any)('system_toggle_clean_mode', { enabled })
        .then(() => {
        })
        .catch((e: any) => {
          console.warn('[Settings] 2. 调用后端函数执行失败:', e);
        });

      // 保存设置到 settings.json
      const settings = settingsCache.current;
      const newSettings = {
        ...settings,
        pureModeEnabled: enabled,
      };
      await invoke('state_write_settings', { settings: newSettings });
      settingsCache.current = newSettings;
    } catch (e) {
      console.error('[Settings] 更新纯净系统失败', e);
      console.error('[Settings] 错误详情:', (e as any)?.message || String(e));
    }
  };

  const handleDefenderToggle = async (disabled: boolean) => {
    try {
      const toggleAction = disabled ? 'disable' : 'enable';
      // 1. 先更新 UI 状态（注意：defenderDisabled 表示停用状态）
      setDefenderDisabled(disabled);
      // 2. 打点
      reportComponentClick('Defender', toggleAction, !disabled);

      (invoke as any)('system_toggle_defender', { disabled })
        .then(() => {
        })
        .catch((e: any) => {
          console.warn('[Settings] 2. 调用后端函数执行失败:', e);
        });

      // 保存设置到 settings.json
      const settings = settingsCache.current;
      const newSettings = {
        ...settings,
        defenderDisabled: disabled,
      };
      await invoke('state_write_settings', { settings: newSettings });
      settingsCache.current = newSettings;
    } catch (e) {
      console.error('[Settings] 更新 Defender 失败', e);
      console.error('[Settings] 错误详情:', (e as any)?.message || String(e));
    }
  };

  const handleStopWuToggle = async (enabled: boolean) => {
    try {
      const toggleAction = enabled ? 'enable' : 'disable';
      // 1. 先更新 UI 状态
      setStopWuEnabled(enabled);
      // 2. 打点
      reportComponentClick('StopWu', toggleAction, enabled);

      (invoke as any)('system_toggle_stop_wu', { enabled })
        .then(() => {
        })
        .catch((e: any) => {
          console.warn('[Settings] 2. 调用后端函数执行失败:', e);
        });

      // 保存设置到 settings.json
      const settings = settingsCache.current;
      const newSettings = {
        ...settings,
        stopWuEnabled: enabled,
      };
      await invoke('state_write_settings', { settings: newSettings });
      settingsCache.current = newSettings;
    } catch (e) {
      console.error('[Settings] 更新服务体验优化失败', e);
      console.error('[Settings] 错误详情:', (e as any)?.message || String(e));
    }
  };

  const handleZoomEffectTypeChange = async (type: 'wave' | 'singleIcon') => {
    try {
      // 1. 先更新UI状态
      setZoomEffectType(type);
      // 2. 打点
      reportComponentClick('ZoomEffectType', 'switch', type);
      const settings = settingsCache.current ?? await invoke('state_get_settings');
      const newSettings = {
        ...settings,
        taskbar: {
          ...(settings?.taskbar ?? {}),
          zoomEffectType: type === 'wave' ? 'Wave' : 'SingleIcon',
        },
      };
      // 先更新缓存，避免快速连续切换时后续写操作仍基于旧快照构造 settings。
      settingsCache.current = newSettings;
      const writeTask = zoomEffectWriteQueueRef.current
        .catch(() => undefined)
        .then(async () => {
          await invoke('state_write_settings', { settings: newSettings });
        });
      zoomEffectWriteQueueRef.current = writeTask;
      await writeTask;
    } catch (e) {
      console.error('[Settings] 更新放大效果类型失败', e);
    }
  };

  const handleBrowserEnhanceToggle = async (enabled: boolean) => {
    if (enabled) {
      try {
        // 1. 更新UI状态
        setBrowserEnhanceEnabled(true);
        // 2. 打点
        reportComponentClick('BrowserEnhance', 'enable', true);
        (invoke as any)('system_toggle_browser_enhance', { enabled: true })
          .catch((e: any) => console.warn('[Settings] 调用后端函数执行失败:', e));
        const settings = settingsCache.current;
        const newSettings = { ...settings, browserEnhanceEnabled: true };
        await invoke('state_write_settings', { settings: newSettings });
        settingsCache.current = newSettings;
      } catch (e) {
        console.error('[Settings] 开启浏览器搜索体验增强失败', e);
      }
      return;
    }
    try {
      const toggleAction = '关闭';
      // 1. 更新UI状态
      setBrowserEnhanceEnabled(false);
      // 2. 打点
      reportComponentClick('BrowserEnhance', 'disable', false);
      (invoke as any)('system_toggle_browser_enhance', { enabled: false })
        .catch((e: any) => console.warn('[Settings] 2. 调用后端函数执行失败:', e));
      const settings = settingsCache.current;
      const newSettings = { ...settings, browserEnhanceEnabled: false };
      await invoke('state_write_settings', { settings: newSettings });
      settingsCache.current = newSettings;
    } catch (e) {
      console.error('[Settings] 更新浏览器搜索体验增强失败', e);
    }
  };

  const handleMinimizeAnimationToggle = async (enabled: boolean) => {
    try {
      const toggleAction = enabled ? 'enable' : 'disable';
      // 1. 先更新UI 状态
      setMinimizeAnimationEnabled(enabled);
      // 2. 打点
      reportComponentClick('MinimizeAnimation', toggleAction, enabled);
      // 如果 MagicVisuals 窗口不存在，不影响前端UI
      (invoke as any)('system_toggle_minimize_animation', { enabled })
        .then(() => {
        })
        .catch((e: any) => {
          console.warn('[Settings] 2. 调用后端函数执行失败:', e);
          console.warn('[Settings] 原因:', e?.message || String(e));
          // 不影响，继续运行（UI 已经更新）
        });

      // 保存设置到 settings.json
      const settings = settingsCache.current;
      const newSettings = {
        ...settings,
        taskbar: {
          ...(settings?.taskbar ?? {}),
          enableMinimizeAnimation: enabled,
        },
      };
      await invoke('state_write_settings', { settings: newSettings });
      settingsCache.current = newSettings;

    } catch (e) {
      console.error('[Settings] 更新 minimize animation UI 失败:', e);
    }
  };

  const handleUserExperienceToggle = async (enabled: boolean) => {
    try {
      const toggleAction = enabled ? 'agree' : 'cancel';
      // 1. 先更新UI 状态
      setUserExperienceEnabled(enabled);
      // 2. 打点
      reportComponentClick('UserExperience', toggleAction, enabled);

      // 然后尝试调用后端发送Windows 消息
      (invoke as any)('system_toggle_user_experience_plan', { enabled })
        .then(() => {
        })
        .catch((e: any) => {
          console.warn('[Settings] 2. 调用后端函数执行失败:', e);
          console.warn('[Settings] 原因:', e?.message || String(e));
        });

      // 保存设置到 settings.json
      const settings = settingsCache.current;
      const newSettings = {
        ...settings,
        taskbar: {
          ...(settings?.taskbar ?? {}),
          enableUserExperiencePlan: enabled,
        },
      };
      await invoke('state_write_settings', { settings: newSettings });
      settingsCache.current = newSettings;

    } catch (e) {
      console.error('[Settings] 更新用户体验计划 UI 失败:', e);
    }
  };

  return (
    <>
      <style>{`
        * {
          margin: 0;
          padding: 0;
          box-sizing: border-box;
        }
        html, body, #root {
          width: 100%;
          height: 100%;
        }
        body {
          font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
          background: ${C.pageBg};
        }
        .settings-container {
          width: 100%;
          height: 100%;
          display: flex;
          position: relative;
        }
        .settings-window {
          display: flex;
          width: 100%;
          height: 100%;
          background: ${C.pageBg};
          position: relative;
        }
        .sidebar {
          width: ${sidebarWidth}px;
          height: 100%;
          background: ${C.sidebarBg};
          display: flex;
          flex-direction: column;
          overflow: hidden;
        }
        .sidebar-header {
          height: 40px;
          padding: 0 12px;
          display: flex;
          align-items: center;
          flex-shrink: 0;
          -webkit-app-region: drag;
          user-select: none;
        }
        .sidebar-title {
          font-size: 14px;
          font-weight: 500;
          color: ${C.textMenu};
        }
        .sidebar-menu {
          flex: 1;
          overflow-y: auto;
          padding: 0 12px 12px 12px;
        }
        .menu-item {
          padding: 12px 16px;
          margin: 4px 0;
          background: transparent;
          border: none;
          border-left: none;
          cursor: pointer;
          color: ${C.textMenu};
          font-size: 14px;
          transition: all 0.2s;
          text-align: left;
          width: 100%;
          border-radius: 6px;
          display: flex;
          align-items: center;
          gap: 8px;
          white-space: nowrap;
          overflow: hidden;
          text-overflow: ellipsis;
        }
        .menu-icon {
          width: 20px;
          height: 20px;
          flex-shrink: 0;
          filter: ${C.iconFilter};
        }
        .menu-icon-theme {
          filter: none;
        }
        .menu-item:hover {
          background: ${C.hover};
        }
        .menu-item.active {
          background: ${C.menuActive};
          color: #1f73e7;
          font-weight: 500;
        }
        .content-wrapper {
          flex: 1;
          height: 100%;
          background: ${C.pageBg};
          display: flex;
          flex-direction: column;
        }
        .title-bar {
          height: 36px;
          background: ${C.pageBg};
          border-bottom: none;
          display: flex;
          align-items: center;
          justify-content: space-between;
          padding: 0 12px;
          -webkit-app-region: drag;
          user-select: none;
          flex-shrink: 0;
        }
        .title-bar-buttons {
          display: flex;
          gap: 8px;
          -webkit-app-region: no-drag;
        }
        .title-bar-btn {
          width: 24px;
          height: 24px;
          border: none;
          background: transparent;
          cursor: pointer;
          display: flex;
          align-items: center;
          justify-content: center;
          font-size: 18px;
          color: ${C.btnColor};
          transition: all 0.2s;
          border-radius: 4px;
          position: relative;
        }
        .title-bar-btn:first-child {
          position: absolute;
          left: 724px;
          top: 6px;
        }
        .title-bar-btn:hover {
          background: ${C.hover};
          color: ${C.textPrimary};
        }
        .title-bar-btn:active {
          background: ${C.active};
        }
        .title-bar-btn img {
          position: absolute;
          left: 4px;
          top: 4.06px;
          width: 16px;
          height: 16px;
          opacity: 1;
        }
        .title-bar-btn:first-child::before {
          content: '';
          position: absolute;
          width: 13px;
          height: 1px;
          background: currentColor;
          left: 50%;
          top: 50%;
          transform: translate(-50%, -50%);
        }
        .title-bar-btn:first-child img {
          display: none;
        }
        .title-bar-btn.close-btn {
          position: absolute;
          left: 764px;
          top: 6px;
        }
        .title-bar-btn.close-btn img {
          position: absolute;
          left: 0px;
          top: 0px;
          width: 24px;
          height: 24px;
          opacity: 1;
          filter: ${C.iconFilter};
        }
        .title-bar-text {
          position: absolute;
          left: 16px;
          top: 10px;
          width: 172px;
          height: 16px;
          opacity: 1;
          font-family: HONOR Sans Design;
          font-size: 12px;
          font-weight: normal;
          line-height: normal;
          letter-spacing: 0px;
          color: ${C.textSecond};
          z-index: 10;
        }
        .content {
          flex: 1;
          overflow: visible;
          padding: 24px 20px;
          max-width: 600px;
          position: relative;
        }
        .content h2 {
          font-size: 14px;
          color: ${C.textSecond};
          font-weight: 500;
          position: absolute;
          left: 52px;
          top: 15px;
          width: 496px;
          height: 18px;
          margin: 0;
          z-index: 100;
          font-family: HONOR Sans Design;
          line-height: normal;
          letter-spacing: 0px;
          opacity: 1;
        }
        .setting-item {
          background: ${C.cardBg};
          padding: 16px 24px;
          margin-bottom: 12px;
          border-radius: 8px;
          border: 1px solid ${C.border};
          display: flex;
          align-items: center;
          justify-content: space-between;
          cursor: pointer;
          transition: all 0.2s;
          position: relative;
        }
        .taskbar-settings-container .setting-item:first-child {
          position: absolute;
          left: 0px;
          top: 0px;
          width: 93.94%;
          height: 50%;
          opacity: 1;
          padding: 0;
          margin-bottom: 0;
          background: transparent;
          border: none;
        }
        .taskbar-settings-container .setting-item:nth-child(3) {
          position: absolute;
          left: 0px;
          top: 48px;
          width: 93.94%;
          height: 50%;
          opacity: 1;
          padding: 0;
          margin-bottom: 0;
          background: transparent;
          border: none;
        }
        .setting-item-first {
          position: absolute;
          left: 36px;
          top: 43px;
          width: 528px;
          height: 48px;
        }
        .setting-item:hover {
          background: ${C.cardBgHover};
          border-color: ${C.borderHover};
        }
        .setting-item-left {
          display: flex;
          flex-direction: column;
          gap: 4px;
        }
        .setting-item-title {
          position: absolute;
          left: 16px;
          top: 14.5px;
          width: auto;
          height: 18px;
          right: 180px;
          opacity: 1;
          font-family: HONOR Sans Design;
          font-size: 14px;
          font-weight: normal;
          line-height: normal;
          letter-spacing: 0px;
          color: ${C.textPrimary};
        }
        .setting-item-desc {
          font-size: 12px;
          color: #999;
        }
        .setting-group-title {
          font-size: 12px;
          color: ${C.textDisable};
          padding: 16px 16px 8px 16px;
        }
        .privacy-title {
          position: absolute;
          left: 52px;
          top: 15px;
          width: 496px;
          height: 18px;
          opacity: 1;
          font-family: HONOR Sans Design;
          font-size: 14px;
          font-weight: 500;
          line-height: normal;
          letter-spacing: 0px;
          color: ${C.textSecond};
        }
        .privacy-container {
          width: 528px;
        }
        .privacy-setting-item {
          background: ${C.cardBg};
          border-radius: 12px;
          display: flex;
          align-items: center;
          justify-content: space-between;
          padding: 14px 16px;
          cursor: pointer;
          border: 1px solid ${C.border};
          margin-bottom: 12px;
        }
        .privacy-item-title {
          font-size: 14px;
          color: ${C.textPrimary};
        }
        .privacy-link {
          color: #2563eb;
          font-size: 14px;
          text-decoration: none;
          cursor: pointer;
        }
        .privacy-link:hover {
          text-decoration: underline;
        }
        .stop-service-btn {
          background: none;
          border: none;
          color: #dc2626;
          font-size: 14px;
          cursor: pointer;
          padding: 8px 16px;
        }
        .stop-service-btn:hover {
          opacity: 0.8;
        }
        .setting-dropdown {
          position: absolute;
          right: 8px;
          top: 8px;
          width: auto;
          min-width: 180px;
          max-width: 260px;
          height: 32px;
        }
        .dropdown-trigger {
          display: flex;
          align-items: center;
          justify-content: space-between;
          gap: 8px;
          background: transparent;
          border: none;
          cursor: pointer;
          color: ${C.textMenu};
          font-size: 14px;
          -webkit-app-region: no-drag;
          width: 100%;
          height: 100%;
          padding: 0 12px;
          box-sizing: border-box;
        }
        .dropdown-trigger-box {
          position: absolute;
          right: 8px;
          top: 8px;
          width: auto;
          min-width: 180px;
          max-width: 260px;
          height: 32px;
          border-radius: 8px;
          opacity: 1;
          background: ${C.cardBg};
          border: 1px solid ${C.border};
          display: flex;
          align-items: center;
          justify-content: flex-start;
          padding: 0 12px;
          box-sizing: border-box;
        }
        .dropdown-trigger:hover {
          color: #1f73e7;
        }
        .dropdown-menu {
          position: absolute;
          left: 0;
          top: 100%;
          min-width: 174px;
          width: auto;
          height: auto;
          border-radius: 12px;
          opacity: 1;
          display: flex;
          flex-direction: column;
          padding: 4px 4px 0px 4px;
          gap: 8px;
          background: ${C.dropdownBg};
          border: 1px solid ${C.border};
          box-shadow: 0px 8px 16px 0px rgba(0, 0, 0, 0.14);
          z-index: 1000;
          margin-top: 8px;
        }
        .dropdown-item {
          padding: 0;
          border: none;
          background: transparent;
          cursor: pointer;
          color: ${C.textMenu};
          font-size: 14px;
          min-width: 166px;
          width: auto;
          height: 32px;
          text-align: left;
          transition: all 0.2s;
          display: flex;
          flex-direction: row;
          align-items: center;
          box-sizing: border-box;
          position: relative;
          gap: 8px;
          white-space: nowrap;
        }
        .dropdown-item:first-child {
          position: static;
          left: 4px;
          top: 0px;
          min-width: 166px;
          width: auto;
          height: 34px;
          border-radius: 8px;
          opacity: 1;
          display: flex;
          flex-direction: row;
          align-items: center;
          padding: 7px 12px;
          gap: 8px;
          z-index: 0;
        }
        .dropdown-divider {
          position: absolute;
          left: 36px;
          top: 42px;
          width: 118px;
          height: 0px;
          opacity: 1;
          border-top: 1px solid rgba(0, 0, 0, 0.08);
          z-index: 0;
        }
        .dropdown-item:nth-child(3) {
          position: static;
          left: 4px;
          top: 0px;
          width: 166px;
          height: 32px;
          border-radius: 8px;
          opacity: 1;
          display: flex;
          flex-direction: row;
          align-items: center;
          padding: 7px 12px;
          gap: 8px;
          background: rgba(0, 0, 0, 0.05);
          z-index: 2;
        }
        .dropdown-item:hover {
          background: ${C.hover};
          color: #1f73e7;
        }
        .dropdown-item.active {
          color: #1f73e7;
          font-weight: 500;
          background: ${C.hover};
        }
        .dropdown-item-check {
          position: absolute;
          left: 12px;
          top: 9px;
          width: 16px;
          height: 16px;
          opacity: 1;
          z-index: 0;
        }
        .dropdown-item-text {
          position: absolute;
          left: 36px;
          top: 7px;
          width: 118px;
          height: 20px;
          opacity: 1;
          fontFamily: 'Source Han Sans';
          fontSize: '14px';
          fontWeight: 'normal';
          lineHeight: 'normal';
          letterSpacing: '0px';
          color: 'rgba(0, 0, 0, 0.9)';
          zIndex: 1;
        }
        .dropdown-item:nth-child(3) .dropdown-item-text {
          position: absolute;
          left: 36px;
          top: 7px;
          width: 118px;
          height: 18px;
          opacity: 1;
          fontFamily: 'HONOR Sans Design';
          fontSize: '14px';
          fontWeight: 'normal';
          lineHeight: 'normal';
          letterSpacing: '0px';
          fontFeatureSettings: '"kern" on';
          color: 'rgba(0, 0, 0, 0.9)';
          zIndex: 1;
        }
        .toggle-switch {
          position: relative;
          width: 44px;
          height: 24px;
          background: ${C.toggleOff};
          border-radius: 12px;
          cursor: pointer;
          transition: background 0.3s;
          border: none;
          padding: 0;
          -webkit-app-region: no-drag;
          pointer-events: auto;
          z-index: 10;
          flex-shrink: 0;
        }
        .toggle-switch.enabled {
          background: #1f73e7;
        }
        .toggle-switch::after {
          content: '';
          position: absolute;
          width: 20px;
          height: 20px;
          background: white;
          border-radius: 50%;
          top: 2px;
          left: 2px;
          transition: left 0.3s;
        }
        .toggle-switch.enabled::after {
          left: 22px;
        }
        .taskbar-settings-container .toggle-switch {
          margin-left: 16px;
          position: absolute;
          left: 470px;
          top: 12px;
          margin-left: 0;
        }
        .section-title {
          font-size: 14px;
          font-weight: 600;
          color: ${C.textMenu};
          position: absolute;
          left: 36px;
          top: 163px;
          width: 528px;
          height: 19px;
          opacity: 1;
        }
        .desktop-title {
          position: absolute;
          left: 16px;
          top: -1px;
          width: 496px;
          height: 18px;
          opacity: 1;
          font-family: HONOR Sans Design;
          font-size: 14px;
          font-weight: 500;
          line-height: normal;
          letter-spacing: 0px;
          color: ${C.textSecond};
        }
        .section-title:first-of-type {
          margin-top: 0;
        }
        .taskbar-settings-container {
          position: absolute;
          left: 36px;
          top: 43px;
          width: 528px;
          height: 96px;
          opacity: 1;
          background: ${C.cardBg};
          border: 1px solid ${C.border};
          border-radius: 8px;
        }
        .taskbar-settings-divider {
          position: absolute;
          left: 16px;
          top: 47px;
          width: 496px;
          height: 1px;
          opacity: 1;
          background: ${C.divider};
        }
        .theme-selector {
          background: ${C.cardBg};
          padding: 12px 16px;
          border-radius: 12px;
          border: 1px solid ${C.border};
          width: 528px;
          height: 138px;
          display: flex;
          flex-direction: column;
          position: absolute;
          top: 190px;
          left: 36px;
          opacity: 1;
        }
        .theme-selector-title {
          position: absolute;
          left: 16px;
          top: 14px;
          width: 438px;
          height: 18px;
          opacity: 1;
          font-family: HONOR Sans Design;
          font-size: 14px;
          font-weight: normal;
          line-height: normal;
          letter-spacing: 0px;
          color: ${C.textPrimary};
        }
        .theme-columns {
          display: grid;
          grid-template-columns: 242px 242px;
          gap: 24px;
          justify-items: center;
        }
        .theme-column:first-child {
          position: absolute;
          left: 16px;
          top: 44px;
          width: 240px;
          height: 56px;
          border-radius: 6px;
          opacity: 1;
          background: url('../../static/icons/zhutibeijing.svg') center/cover no-repeat;
        }
        .theme-column:nth-of-type(2) {
          position: absolute;
          left: 272px;
          top: 44px;
          width: 240px;
          height: 56px;
          border-radius: 6px;
          opacity: 1;
          background: url('../../static/icons/zhutibeijing.svg') center/cover no-repeat;
        }
        .theme-column {
          display: flex;
          flex-direction: column;
          gap: 12px;
        }
        .theme-column-label {
          font-size: 12px;
          color: ${C.btnColor};
          font-weight: 500;
          text-align: center;
        }
        .theme-icons {
          position: relative;
          background-size: contain;
          background-repeat: no-repeat;
          background-position: center;
          width: 242px;
          height: 44px;
        }
        .theme-column .theme-icon-item.selected {
          border: none;
          background: #1f73e7;
        }
        .theme-column:nth-of-type(2) .theme-icon-item:nth-child(1) {
          position: absolute;
          left: 38px;
          top: 12px;
          width: 32px;
          height: 32px;
          opacity: 1;
        }
        .theme-column:nth-of-type(2) .theme-icon-item:nth-child(2) {
          position: absolute;
          left: 82px;
          top: 12px;
          width: 32px;
          height: 32px;
          opacity: 1;
        }
        .theme-column:nth-of-type(2) .theme-icon-item:nth-child(3) {
          position: absolute;
          left: 126px;
          top: 12px;
          width: 32px;
          height: 32px;
          opacity: 1;
        }
        .theme-column:nth-of-type(2) .theme-icon-item:nth-child(4) {
          position: absolute;
          left: 170px;
          top: 12px;
          width: 32px;
          height: 32px;
          opacity: 1;
        }
        .theme-column:nth-of-type(2) .theme-icon-item:nth-child(5) {
          position: absolute;
          left: 220px;
          width: 20px;
          height: 20px;
          opacity: 1;
        }
        .theme-column:first-child .theme-icon-item {
          border: none;
          background: transparent;
        }
        .theme-column:first-child .theme-icon-item:nth-child(1) {
          position: absolute;
          left: 38px;
          top: 12px;
          width: 32px;
          height: 32px;
          opacity: 1;
        }
        .theme-column:first-child .theme-icon-item:nth-child(2) {
          position: absolute;
          left: 82px;
          top: 12px;
          width: 32px;
          height: 32px;
          opacity: 1;
        }
        .theme-column:first-child .theme-icon-item:nth-child(3) {
          position: absolute;
          left: 126px;
          top: 12px;
          width: 32px;
          height: 32px;
          opacity: 1;
        }
        .theme-column:first-child .theme-icon-item:nth-child(4) {
          position: absolute;
          left: 170px;
          top: 12px;
          width: 32px;
          height: 32px;
          opacity: 1;
        }
        .theme-column:first-child .theme-icon-item:nth-child(5) {
          position: absolute;
          left: 220px;
          top: 24px;
          width: 20px;
          height: 20px;
          opacity: 1;
        }
        .theme-column:nth-of-type(2) .theme-icon-item {
          border: none;
          background: transparent;
        }
        .theme-icon-item {
          aspect-ratio: 1;
          border: 2px solid ${C.border};
          border-radius: 6px;
          display: flex;
          align-items: center;
          justify-content: center;
          cursor: pointer;
          font-size: 24px;
          background: ${C.cardBg};
          transition: all 0.2s;
          width: 24px;
          height: 24px;
          flex-shrink: 0;
        }
        .theme-icon-item:hover {
          border-color: #1f73e7;
          background: rgba(31, 115, 231, 0.05);
        }
        .theme-icon-item.selected {
          border-color: #1f73e7;
          background: #1f73e7;
          border-radius: 50%;
        }
      `}</style>
      <div className="settings-container">
        <div className="settings-window">
          <div className="sidebar">
            <div className="sidebar-header">
            </div>
            <div className="sidebar-menu">
              {menuItems.map(item => (
                <button
                  key={item.id}
                  className={`menu-item ${activeTab === item.id ? 'active' : ''}`}
                  onClick={() => {
                    setActiveTab(item.id);
                    setShowThirdPartyControlPage(false); // 切换标签时关闭二级菜单
                    setShowAppStartupPage(false); // 切换标签时关闭开机自启二级页面
                  }}
                >
                  <img src={item.icon} alt={item.label} className="menu-icon menu-icon-theme" />
                  <span>{item.label}</span>
                </button>
              ))}
            </div>
          </div>
          <div className="content-wrapper">
            <div className="title-bar">
              {/* 返回箭头 - 仅在三级菜单显示时显示 */}
              {(showThirdPartyControlPage && thirdPartyAppList.length > 0 || showAppStartupPage) && (
                <button
                  onClick={() => {
                    setShowThirdPartyControlPage(false);
                    setShowAppStartupPage(false);
                  }}
                  style={{
                    position: 'absolute',
                    left: `${detailPageBackButtonLeft}px`,
                    top: '10px',
                    width: '24px',
                    height: '24px',
                    opacity: 1,
                    background: 'none',
                    border: 'none',
                    cursor: 'pointer',
                    padding: '0',
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center'
                  }}
                >
                  <img src={backIcon} alt="back" style={{width: '16px', height: '16px', filter: C.iconFilter}} />
                </button>
              )}
              <div className="title-bar-text">{t('window.title')}</div>
              <div className="title-bar-buttons">
                <button className="title-bar-btn" onClick={handleMinimize} title={t('window.minimize')}>
                  <img src={minimizeIcon} alt="minimize" />
                </button>
                <button className="title-bar-btn close-btn" onClick={handleClose} title={t('window.close')}>
                  <img src={closeIcon} alt="close" />
                </button>
              </div>
            </div>
            <div className="content">
              {activeTab === 'notification' && (
                <div>
                  <h2>{t('notification.title')}</h2>
                  <div className="setting-item setting-item-first">
                    <div className="setting-item-left">
                      <div className="setting-item-title">{t('notification.toolbar_display')}</div>
                    </div>
                    <div className="dropdown-trigger-box"></div>
                    <div className="setting-dropdown">
                      <button
                        className="dropdown-trigger"
                        onClick={() => setShowToolbarMenu(!showToolbarMenu)}
                      >
                        <span>{getToolbarLabel()}</span>
                        <img src={downArrowIcon} alt="dropdown" style={{width: '24px', height: '24px', filter: C.iconFilter}} />
                      </button>
                      {showToolbarMenu && (
                        <div className="dropdown-menu">
                          {toolbarOptions.map(option => (
                            <button
                              key={option.value}
                              className={`dropdown-item ${toolbarVisibility === option.value ? 'active' : ''}`}
                              onClick={() => handleToolbarModeChange(option.value)}
                            >
                              {toolbarVisibility === option.value && <img src={checkmarkIcon} alt="checkmark" className="dropdown-item-check" />}
                              {toolbarVisibility !== option.value && <img src={rectangleIcon} alt="unchecked" className="dropdown-item-check" />}
                              <span className="dropdown-item-text">{option.label}</span>
                            </button>
                          ))}
                          <div className="dropdown-divider"></div>
                        </div>
                      )}
                    </div>
                  </div>
                </div>
              )}
              {activeTab === 'taskbar' && (
                <div>
                  <h2>{t('taskbar.section_taskbar')}</h2>

                  <div className="taskbar-settings-container" style={taskbarZoomEnabled ? {
                    position: 'absolute',
                    left: '36px',
                    top: '43px',
                    width: '528px',
                    height: '218px',
                    borderRadius: '12px',
                    background: C.cardBg,
                  } : {}}>
                    <div className="setting-item" style={{position: 'absolute', left: '0px', top: '0px', width: '496px', height: '48px'}}>
                      <div className="setting-item-left">
                        <div className="setting-item-title">{t('taskbar.zoom_effect')}</div>
                      </div>
                      <button
                        className={`toggle-switch ${taskbarZoomEnabled ? 'enabled' : ''}`}
                        onClick={(e) => {
                          e.stopPropagation();
                          handleTaskbarZoomToggle(!taskbarZoomEnabled);
                        }}
                        title={taskbarZoomEnabled ? t('taskbar.zoom_effect_desc') : t('taskbar.zoom_effect_desc')}
                      />
                    </div>

                    {taskbarZoomEnabled && (
                      <>
                        {/* 灵动波浪外框 */}
                        {(zoomEffectType === 'wave' || waveHover) && <div style={{
                          position: 'absolute',
                          left: '14px',
                          top: '60px',
                          width: '244px',
                          height: '74px',
                          borderRadius: '9px',
                          opacity: 1,
                          border: zoomEffectType === 'wave' ? '1px solid #256FFF' : '1px solid rgba(0, 0, 0, 0.1)',
                          pointerEvents: 'none',
                        }} />}
                        <div style={{
                          position: 'absolute',
                          left: '16px',
                          top: '62px',
                          width: '240px',
                          height: '70px',
                          opacity: 1,
                          border: '0px solid',
                          borderImage: 'linear-gradient(106deg, rgba(255, 255, 255, 0.4) 13%, rgba(255, 255, 255, 0.36) 31%, rgba(255, 255, 255, 0.25) 51%, rgba(255, 255, 255, 0.2) 68%, rgba(255, 255, 255, 0.4) 87%) 0',
                          borderRadius: '6px',
                          background: C.subBg,
                          cursor: 'pointer',
                          display: 'flex',
                          flexDirection: 'column',
                          alignItems: 'center',
                          justifyContent: 'center',
                          gap: '8px',
                        }} onClick={() => handleZoomEffectTypeChange('wave')} onMouseEnter={() => setWaveHover(true)} onMouseLeave={() => setWaveHover(false)}>
                          <img src="../../static/icons/lingdongbolang.webp" alt={t('taskbar.zoom_wave')} style={{width: '100%', height: '100%', objectFit: 'cover', borderRadius: '6px'}} />
                        </div>
                        <div style={{
                          position: 'absolute',
                          left: '88px',
                          top: '140px',
                          width: '96px',
                          height: '16px',
                          opacity: 1,
                          fontFamily: 'HONOR Sans Design',
                          fontSize: '12px',
                          fontWeight: 'normal',
                          lineHeight: 'normal',
                          letterSpacing: '0px',
                          fontFeatureSettings: '"kern" on',
                          whiteSpace: 'nowrap',
                          color: zoomEffectType === 'wave' ? C.textPrimary : C.textSecond,
                        }}>{t('taskbar.zoom_wave')}</div>
                        {/* 单点聚焦外框 */}
                        {(zoomEffectType === 'singleIcon' || singleIconHover) && <div style={{
                          position: 'absolute',
                          left: '270px',
                          top: '60px',
                          width: '244px',
                          height: '74px',
                          borderRadius: '9px',
                          opacity: 1,
                          border: zoomEffectType === 'singleIcon' ? '1px solid #256FFF' : '1px solid rgba(0, 0, 0, 0.1)',
                          pointerEvents: 'none',
                        }} />}
                        <div style={{
                          position: 'absolute',
                          left: '272px',
                          top: '62px',
                          width: '240px',
                          height: '70px',
                          opacity: 1,
                          border: '0px solid',
                          borderImage: 'linear-gradient(106deg, rgba(255, 255, 255, 0.4) 13%, rgba(255, 255, 255, 0.36) 31%, rgba(255, 255, 255, 0.25) 51%, rgba(255, 255, 255, 0.2) 68%, rgba(255, 255, 255, 0.4) 87%) 0',
                          borderRadius: '6px',
                          background: C.subBg,
                          cursor: 'pointer',
                          display: 'flex',
                          flexDirection: 'column',
                          alignItems: 'center',
                          justifyContent: 'center',
                          gap: '8px',
                        }} onClick={() => handleZoomEffectTypeChange('singleIcon')} onMouseEnter={() => setSingleIconHover(true)} onMouseLeave={() => setSingleIconHover(false)}>
                          <img src="../../static/icons/dandianjujiao.webp" alt={t('taskbar.zoom_single')} style={{width: '100%', height: '100%', objectFit: 'cover', borderRadius: '6px'}} />
                        </div>
                        <div style={{
                          position: 'absolute',
                          left: '344px',
                          top: '140px',
                          width: '96px',
                          height: '16px',
                          opacity: 1,
                          fontFamily: 'HONOR Sans Design',
                          fontSize: '12px',
                          fontWeight: 'normal',
                          lineHeight: 'normal',
                          letterSpacing: '0px',
                          fontFeatureSettings: '"kern" on',
                          whiteSpace: 'nowrap',
                          color: zoomEffectType === 'singleIcon' ? C.textPrimary : C.textSecond,
                        }}>{t('taskbar.zoom_single')}</div>
                        {/* 灵动波浪选中框 */}
                        <div style={{
                          position: 'absolute',
                          left: '240.44px',
                          top: '116.45px',
                          width: '11.11px',
                          height: '11.11px',
                          borderRadius: '5.56px',
                          opacity: 1,
                          background: zoomEffectType === 'wave' ? '#256FFF' : 'transparent',
                          cursor: 'pointer',
                          zIndex: 0,
                        }} onClick={() => handleZoomEffectTypeChange('wave')}>
                          {zoomEffectType === 'wave' && <img src={checkboxIcon} alt="checkmark" style={{position: 'absolute', width: '11.11px', height: '11.11px', top: '50%', left: '50%', transform: 'translate(-50%, -50%)', objectFit: 'contain'}} />}
                        </div>
                        {/* 单点聚焦选中框 */}
                        <div style={{
                          position: 'absolute',
                          left: '496.44px',
                          top: '116.45px',
                          width: '11.11px',
                          height: '11.11px',
                          borderRadius: '5.56px',
                          opacity: 1,
                          background: zoomEffectType === 'singleIcon' ? '#256FFF' : 'transparent',
                          cursor: 'pointer',
                          zIndex: 0,
                        }} onClick={() => handleZoomEffectTypeChange('singleIcon')}>
                          {zoomEffectType === 'singleIcon' && <img src={checkboxIcon} alt="checkmark" style={{position: 'absolute', width: '11.11px', height: '11.11px', top: '50%', left: '50%', transform: 'translate(-50%, -50%)', objectFit: 'contain'}} />}
                        </div>
                      </>
                    )}

                    {!taskbarZoomEnabled && <div className="taskbar-settings-divider"></div>}

                    {taskbarZoomEnabled && (
                      <>
                        <div className="taskbar-settings-divider" style={{left: '16px', top: '48px'}}></div>
                        <div className="taskbar-settings-divider" style={{left: '16px', top: '170px'}}></div>
                      </>
                    )}

                    <div className="setting-item" style={taskbarZoomEnabled ? {position: 'absolute', left: '0px', top: '170px', width: '496px', height: '48px', border: 'none', background: 'transparent', margin: 0, padding: 0} : {}}>
                      <div className="setting-item-left">
                        <div className="setting-item-title">{t('taskbar.minimize_animation')}</div>
                      </div>
                      <button
                        className={`toggle-switch ${minimizeAnimationEnabled ? 'enabled' : ''}`}
                        onClick={(e) => {
                          e.stopPropagation();
                          handleMinimizeAnimationToggle(!minimizeAnimationEnabled);
                        }}
                        title={minimizeAnimationEnabled ? t('taskbar.disable_animation') : t('taskbar.enable_animation')}
                      />
                    </div>
                  </div>

                  <div className="section-title" style={taskbarZoomEnabled ? {top: '285px'} : {}}><div className="desktop-title">{t('taskbar.section_desktop')}</div></div>

                  <div className="theme-selector" style={taskbarZoomEnabled ? {top: '309px'} : {}}>
                    <div className="theme-selector-title">{t('taskbar.icon_theme')}</div>
                    {(selectedTheme === 'blue' || blueThemeHover) && <div style={{
                      position: 'absolute', left: '14px', top: '42px',
                      width: '244px', height: '60px', borderRadius: '8px', opacity: 1,
                      border: selectedTheme === 'blue' ? '1px solid #256FFF' : '1px solid rgba(0, 0, 0, 0.1)',
                      pointerEvents: 'none',
                    }} />}
                    {(selectedTheme === 'default' || defaultThemeHover) && <div style={{
                      position: 'absolute', left: '270px', top: '42px',
                      width: '244px', height: '60px', borderRadius: '8px', opacity: 1,
                      border: selectedTheme === 'default' ? '1px solid #256FFF' : '1px solid rgba(0, 0, 0, 0.1)',
                      pointerEvents: 'none',
                    }} />}
                    <div className="theme-columns">
                      <div className="theme-column" onClick={() => { setSelectedTheme('blue'); handleThemeChange(1, 'White'); }} onMouseEnter={() => setBlueThemeHover(true)} onMouseLeave={() => setBlueThemeHover(false)}>
                        <div className="theme-icons" style={{position: 'relative'}}>
                          <div className="theme-icon-item">
                            <img src={weilanWorkstationIcon} alt="workstation" style={{width: '100%', height: '100%', objectFit: 'contain'}} />
                          </div>
                          <div className="theme-icon-item">
                            <img src={weilanPcmanagerIcon} alt="pcmanager" style={{width: '100%', height: '100%', objectFit: 'contain'}} />
                          </div>
                          <div className="theme-icon-item">
                            <img src={weilanYoyoIcon} alt="yoyo" style={{width: '100%', height: '100%', objectFit: 'contain'}} />
                          </div>
                          <div className="theme-icon-item">
                            <img src={weilanAppmarketIcon} alt="appmarket" style={{width: '100%', height: '100%', objectFit: 'contain'}} />
                          </div>
                          <div className={`theme-icon-item ${selectedTheme === 'blue' ? 'selected' : ''}`} onClick={() => { setSelectedTheme('blue'); handleThemeChange(1, 'White'); }} style={{position: 'absolute', width: '11.11px', height: '11.11px', borderRadius: '50%', left: '224.44px', top: '40.45px', zIndex: 10, background: selectedTheme === 'blue' ? '#1f73e7' : 'transparent'}}>
                            {selectedTheme === 'blue' && <img src={checkboxIcon} alt="checkmark" style={{position: 'absolute', width: '11.11px', height: '11.11px', top: '50%', left: '50%', transform: 'translate(-50%, -50%)', objectFit: 'contain'}} />}
                          </div>
                        </div>
                        <div className="theme-column-label" style={{position: 'absolute', left: '72px', top: '64px', width: '96px', height: '16px', opacity: 1, fontFamily: 'HONOR Sans Design', fontSize: '12px', fontWeight: 'normal', lineHeight: 'normal', letterSpacing: '0px', fontFeatureSettings: '"kern" on', whiteSpace: 'nowrap', color: selectedTheme === 'blue' ? C.textPrimary : C.textSecond}}>{t('taskbar.icon_theme_blue')}</div>
                      </div>
                      <div className="theme-column" onClick={() => { setSelectedTheme('default'); handleThemeChange(0, 'Transparent'); }} onMouseEnter={() => setDefaultThemeHover(true)} onMouseLeave={() => setDefaultThemeHover(false)}>
                        <div className="theme-icons" style={{position: 'relative'}}>
                          <div className="theme-icon-item">
                            <img src={tongtouWorkstationIcon} alt="workstation" style={{width: '100%', height: '100%', objectFit: 'contain'}} />
                          </div>
                          <div className="theme-icon-item">
                            <img src={tongtouPcmanagerIcon} alt="pcmanager" style={{width: '100%', height: '100%', objectFit: 'contain'}} />
                          </div>
                          <div className="theme-icon-item">
                            <img src={tongtouYoyoIcon} alt="yoyo" style={{width: '100%', height: '100%', objectFit: 'contain'}} />
                          </div>
                          <div className="theme-icon-item">
                            <img src={tongtouAppmarketIcon} alt="appmarket" style={{width: '100%', height: '100%', objectFit: 'contain'}} />
                          </div>
                          <div className={`theme-icon-item ${selectedTheme === 'default' ? 'selected' : ''}`} onClick={() => { setSelectedTheme('default'); handleThemeChange(0, 'Transparent'); }} style={{position: 'absolute', width: '11.11px', height: '11.11px', borderRadius: '50%', left: '224.44px', top: '40.45px', background: selectedTheme === 'default' ? '#1f73e7' : 'transparent'}}>
                            {selectedTheme === 'default' && <img src={checkboxIcon} alt="checkmark" style={{position: 'absolute', width: '11.11px', height: '11.11px', top: '50%', left: '50%', transform: 'translate(-50%, -50%)', objectFit: 'contain'}} />}
                          </div>
                        </div>
                        <div className="theme-column-label" style={{position: 'absolute', left: '72px', top: '64px', width: '96px', height: '16px', opacity: 1, fontFamily: 'HONOR Sans Design', fontSize: '12px', fontWeight: 'normal', lineHeight: 'normal', letterSpacing: '0px', fontFeatureSettings: '"kern" on', whiteSpace: 'nowrap', color: selectedTheme === 'default' ? C.textPrimary : C.textSecond}}>{t('taskbar.icon_theme_default')}</div>
                      </div>
                    </div>
                  </div>
                </div>
              )}
              {activeTab === 'puremode' && (
                <div className="privacy-container" style={{padding: 0, margin: 0}}>
                  <div className="privacy-title">{t('puremode.title')}</div>
                  <div className="setting-item" style={{position: 'absolute', left: '36px', top: '42px', width: '528px', height: '48px', borderRadius: '12px', opacity: 1, border: `1px solid ${C.border}`, background: C.cardBg}}>
                    <div className="setting-item-left">
                      <div className="setting-item-title" style={{position: 'absolute', left: '16px', top: '15px', width: '438px', height: '18px', opacity: 1, fontFamily: 'HONOR Sans Design', fontSize: '14px', fontWeight: 'normal', lineHeight: 'normal', letterSpacing: '0px', color: C.textPrimary}}>{t('puremode.turn_on_pure_mode')}</div>
                    </div>
                    <button
                      className={`toggle-switch ${pureModeEnabled ? 'enabled' : ''}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        handlePureModeToggle(!pureModeEnabled);
                      }}
                      title={pureModeEnabled ? t('puremode.turn_off_pure_mode') : t('puremode.turn_on_pure_mode')}
                      style={{position: 'absolute', left: '470px', top: '12px'}}
                    />
                  </div>
                  <a
                    href="#"
                    style={{
                      position: 'absolute',
                      left: '52px',
                      top: '98px',
                      width: '496px',
                      height: '16px',
                      opacity: 1,
                      fontFamily: 'HONOR Sans Design',
                      fontSize: '12px',
                      fontWeight: 'normal',
                      lineHeight: 'normal',
                      letterSpacing: '0px',
                      fontVariationSettings: '"opsz" auto',
                      fontFeatureSettings: '"kern" on',
                      color: '#256FFF',
                      textDecoration: 'none',
                      cursor: 'pointer'
                    }}
                    onClick={(e) => {
                      e.preventDefault();
                      handleOpenCleanModeDeclaration();
                    }}
                  >
                    {t('puremode.clean_mode_declaration')}
                  </a>

                  {/* 服务体验优化 - 仅在纯净系统开启时显示  */}
                  {pureModeEnabled && (
                    <>
                      <div style={{position: 'absolute', left: '36px', top: '138px', width: '528px', height: '18px'}}>
                        <div className="setting-item-title" style={{position: 'absolute', left: '16px', top: 0, opacity: 1, fontFamily: 'HONOR Sans Design', fontSize: '14px', fontWeight: 500, lineHeight: 'normal', letterSpacing: '0px', color: C.textSecond, zIndex: 0}}>{t('puremode.service_optimization')}</div>
                      </div>

                      {/* 更新服务优化 + 浏览器搜索体验增强 */}
                      <div className="taskbar-settings-container" style={{position: 'absolute', left: '36px', top: '164px', width: '528px'}}>
                        <div className="setting-item">
                          <div className="setting-item-left">
                            <div className="setting-item-title">{t('puremode.update_service_optimization')}</div>
                          </div>
                          <button
                            className={`toggle-switch ${stopWuEnabled ? 'enabled' : ''}`}
                            onClick={(e) => {
                              e.stopPropagation();
                              handleStopWuToggle(!stopWuEnabled);
                            }}
                            title={stopWuEnabled ? t('puremode.turn_off_service') : t('puremode.turn_on_service')}
                          />
                        </div>

                        <div className="taskbar-settings-divider"></div>

                        <div className="setting-item">
                          <div className="setting-item-left">
                            <div className="setting-item-title">{t('puremode.browser_enhance')}</div>
                          </div>
                          <button
                            className={`toggle-switch ${browserEnhanceEnabled ? 'enabled' : ''}`}
                            onClick={(e) => {
                              e.stopPropagation();
                              handleBrowserEnhanceToggle(!browserEnhanceEnabled);
                            }}
                            title={browserEnhanceEnabled ? t('puremode.turn_off_browser_enhance') : t('puremode.turn_on_browser_enhance')}
                          />
                        </div>
                      </div>

                    </>
                  )}

                  {/* 软件管控 - 仅在纯净系统开启且至少有一个应用列表时显示 */}
{pureModeEnabled && !showThirdPartyControlPage && !showAppStartupPage && (thirdPartyAppList.length > 0 || appStartupList.length > 0) && (
  <>
    <div style={{position: 'absolute', left: '36px', top: '285px', width: '528px', height: '18px'}}>
      <div className="setting-item-title" style={{position: 'absolute', left: '16px', top: 0, opacity: 1, fontFamily: 'HONOR Sans Design', fontSize: '14px', fontWeight: 500, lineHeight: 'normal', letterSpacing: '0px', color: C.textSecond, zIndex: 3}}>{t('puremode.software_control')}</div>
    </div>

    {/* 软件管控容器 */}
    <div className="taskbar-settings-container" style={{position: 'absolute', left: '36px', top: '313px', width: '528px', height: (thirdPartyAppList.length > 0 && appStartupList.length > 0) ? '97px' : '48px'}}>
      {thirdPartyAppList.length > 0 && (
        <div className="setting-item" style={{cursor: 'pointer', position: 'relative', width: 'auto', height: '48px', padding: '0 16px', top: 'auto', left: 'auto'}}
          onClick={(e) => {
            e.preventDefault();
            e.stopPropagation();
            setShowThirdPartyControlPage(true);
          }}
        >
          <div className="setting-item-left">
            <div className="setting-item-title" style={{whiteSpace: 'nowrap'}}>{t('puremode.third_party_desc')}</div>
          </div>
          <div style={{width: '20px', height: '20px', display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0}}>
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
              <path d="M9 18L15 12L9 6" stroke={C.textSecond} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"/>
            </svg>
          </div>
        </div>
      )}

      {thirdPartyAppList.length > 0 && appStartupList.length > 0 && (
        <div className="taskbar-settings-divider" style={{position: 'relative', top: 'auto', left: 'auto', width: '100%'}}></div>
      )}

      {appStartupList.length > 0 && (
        <div className="setting-item" style={{cursor: 'pointer', position: 'relative', width: 'auto', height: '48px', padding: '0 16px', top: 'auto', left: 'auto'}}
          onClick={(e) => {
            e.preventDefault();
            e.stopPropagation();
            setShowAppStartupPage(true);
          }}
        >
          <div className="setting-item-left">
            <div className="setting-item-title" style={{whiteSpace: 'nowrap'}}>{t('puremode.startup_desc')}</div>
          </div>
          <div style={{width: '20px', height: '20px', display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0}}>
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
              <path d="M9 18L15 12L9 6" stroke={C.textSecond} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"/>
            </svg>
          </div>
        </div>
      )}
    </div>
  </>
)}
                </div>
              )}
              {activeTab === 'upgrade' && (
                <div className="privacy-container" style={{padding: 0, margin: 0}}>
                  <div className="privacy-title">{t('puremode.idle_update')}</div>
                  <div className="setting-item" style={{position: 'absolute', left: '36px', top: '42px', width: '528px', height: '48px', borderRadius: '12px', opacity: 1, border: `1px solid ${C.border}`, background: C.cardBg}}>
                    <div className="setting-item-left">
                      <div className="setting-item-title" style={{position: 'absolute', left: '16px', top: '15px', width: '438px', height: '18px', opacity: 1, fontFamily: 'HONOR Sans Design', fontSize: '14px', fontWeight: 'normal', lineHeight: 'normal', letterSpacing: '0px', color: C.textPrimary}}>{t('puremode.idle_update_magic')}</div>
                    </div>
                    <button
                      className={`toggle-switch ${upgradeEnabled ? 'enabled' : ''}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        handleUpgradeToggle(!upgradeEnabled);
                      }}
                      title={upgradeEnabled ? t('upgrade.turn_off') : t('upgrade.turn_on')}
                      style={{position: 'absolute', left: '470px', top: '12px'}}
                    />
                  </div>
                </div>
              )}
              {activeTab === 'privacy' && (
                <div className="privacy-container">
                  <div className="privacy-title">{t('privacy.title')}</div>
                  <div className="privacy-setting-item" style={{position: 'absolute', left: '36px', top: '43px', width: '528px', height: '48px'}}>
                    <span className="privacy-item-title">{t('privacy.user_experience')}</span>
                    <button className={`toggle-switch ${userExperienceEnabled ? 'enabled' : ''}`} onClick={() => handleUserExperienceToggle(!userExperienceEnabled)}></button>
                  </div>
                  <div className="setting-group-title" style={{position: 'absolute', left: '52px', top: '115px', width: '496px', height: '18px', padding: '0', fontFamily: 'HONOR Sans Design', fontSize: '14px', fontWeight: '500', color: C.textSecond}}>{t('privacy.other')}</div>
                  <div style={{position: 'absolute', left: '36px', top: '140px', width: '528px', height: '96px', borderRadius: '12px', background: C.cardBg, border: `1px solid ${C.border}`}}>
                  <a className="privacy-link" href="#" style={{position: 'absolute', left: '16px', top: '14.5px', width: '496px', height: '18px', fontFamily: 'HONOR Sans Design', fontSize: '14px', fontWeight: 'normal', color: '#256FFF'}} onClick={(e) => { e.preventDefault(); setShowPrivacyPopup(true); }}>{t('privacy.permission_statement')}</a>
                  <div style={{position: 'absolute', left: '16px', top: '47px', width: '496px', height: '1px', backgroundColor: C.divider}}></div>
                  <button className="stop-service-btn" style={{position: 'absolute', left: '16px', top: '63.5px', width: '496px', height: '18px', color: '#FA2A2D', textAlign: 'left', padding: '0'}} onClick={() => setShowStopServicePopup(true)}>{t('privacy.stop_service')}</button>
                </div>
                {showPrivacyPopup && (
                  <div style={{position: 'absolute', left: '0px', top: '8px', width: '388px', height: '422px', borderRadius: '12px', background: C.pageBg, border: `1px solid ${C.border}`, zIndex: 100}}>
                    <div style={{position: 'absolute', left: '24px', top: '32px', width: '200px', height: '21px', fontFamily: 'HONOR Sans Design', fontSize: '16px', fontWeight: '600', lineHeight: 'normal', letterSpacing: '0em', fontVariationSettings: '"opsz" auto', fontFeatureSettings: '"kern" on', color: C.textPrimary, whiteSpace: 'nowrap'}}>{t('privacy.agreement_declaration')}</div>
                    <button style={{position: 'absolute', left: '352px', top: '12px', width: '24px', height: '24px', opacity: 1, background: 'none', border: 'none', cursor: 'pointer'}} onClick={() => setShowPrivacyPopup(false)}>
                                          <img src={closeIcon} alt="close" style={{width: '24px', height: '24px', filter: C.iconFilter}} />
                                        </button>
                    <div style={{position: 'absolute', left: '24px', top: '73px', width: '340px', height: '317px', fontFamily: 'HONOR Sans Design', fontSize: '14px', fontWeight: 'normal', color: C.textSecond, lineHeight: '1.6', overflow: 'auto', border: `2px dashed ${C.border}`, padding: '14px 13px', boxSizing: 'border-box'}}>
                      <div style={{fontWeight: '500', color: C.textPrimary, marginBottom: '12px'}}>{t('privacy.permission_statement')}</div>
                      <div>{t('permission_content.intro')}</div>
                      <div style={{marginTop: '12px'}}>{t('permission_content.info_device')}</div>
                      <div>{t('permission_content.info_network')}</div>
                      <div>{t('permission_content.info_other')}</div>
                      <div style={{marginTop: '12px'}}>{t('permission_content.user_experience')}</div>
                      <div style={{marginTop: '12px'}}>{t('permission_content.service')}</div>
                      <div style={{marginTop: '12px'}}>{t('permission_content.retention')}</div>
                      <div style={{marginTop: '12px'}}>{t('permission_content.uninstall_prefix')}<a href="#" style={{color: '#256FFF', textDecoration: 'none'}} onClick={(e) => { e.preventDefault(); handleOpenUserAgreement(); }}>{t('privacy.user_agreement')}</a>{t('privacy.and')}<a href="#" style={{color: '#256FFF', textDecoration: 'none'}} onClick={(e) => { e.preventDefault(); handleOpenPrivacyStatement(); }}>{t('privacy.privacy_statement')}</a>{t('permission_content.uninstall_suffix')}</div>
                    </div>
                  </div>
                )}
                </div>
              )}
              {showStopServicePopup && (
                <div style={{position: 'absolute', left: '-2px', top: '102px', width: '400px', height: '241px', borderRadius: '12px', background: C.cardBg, border: `1px solid ${C.border}`, zIndex: 100}}>
                  <div style={{position: 'absolute', left: '24px', top: '24px', width: '352px', height: '24px', fontFamily: 'HONOR Sans Design', fontSize: '18px', fontWeight: '500', lineHeight: 'normal', letterSpacing: '0px', fontVariationSettings: '"opsz" auto', color: C.textPrimary}}>{t('popup.stop_service_title')}</div>
                  <div style={{position: 'absolute', left: '24px', top: '60px', width: '352px', height: '54px', fontFamily: 'HONOR Sans Design', fontSize: '14px', fontWeight: 'normal', lineHeight: 'normal', letterSpacing: '0px', color: C.textPrimary}}>{t('popup.stop_service_desc')}</div>
                  <div style={{position: 'absolute', left: '24px', top: '185px', width: '352px', height: '32px'}}>
                    <button style={{position: 'absolute', left: '0px', top: '0px', width: '168px', height: '32px', borderRadius: '16px', border: 'none', background: C.hover, cursor: 'pointer'}} onClick={async () => { reportComponentClick('StopService', 'confirm'); await (invoke as any)('system_stop_service'); setShowStopServicePopup(false); }}>
                      <span style={{position: 'absolute', left: '70px', top: '6.5px', width: '28px', height: '18px', fontFamily: 'HONOR Sans Design', fontSize: '14px', fontWeight: 'normal', lineHeight: 'normal', textAlign: 'center', letterSpacing: '0px', color: '#FA2A2D'}}>{t('popup.confirm')}</span>
                    </button>
                  </div>
                  <button style={{position: 'absolute', left: '208px', top: '185px', width: '168px', height: '32px', borderRadius: '16px', border: 'none', background: C.hover, fontFamily: 'HONOR Sans Design', fontSize: '14px', fontWeight: '500', color: C.textSecond, cursor: 'pointer'}} onClick={() => setShowStopServicePopup(false)}>{t('popup.cancel')}</button>
                </div>
              )}
            </div>
          </div>
        </div>
      </div>

      {/* 三方软件管控二级菜单 */}
      {showThirdPartyControlPage && thirdPartyAppList.length > 0 && (
        <ThirdPartyControl
          onBack={() => setShowThirdPartyControlPage(false)}
          groupedApps={groupedApps}
          setGroupedApps={setGroupedApps}
          isDark={isDark}
        />
      )}

      {/* 开机自启管控二级菜单 */}
      {showAppStartupPage && appStartupList.length > 0 && (
        <AppStartupControl
          onBack={() => setShowAppStartupPage(false)}
          appList={appStartupList}
          setAppList={setAppStartupList}
          isDark={isDark}
        />
      )}
    </>
  );
}

async function bootstrap() {
  await loadTranslations();

  try {
    const { Settings } = await import("@magic-ui/lib");
    const settings = await Settings.getAsync();
    const lang = settings.inner.language || "en";
    await i18n.changeLanguage(lang);
  } catch (e) {
    console.warn("[Settings] Failed to load language from settings");
  }

  const root = document.getElementById('root');
  if (root) {
    const reactRoot = createRoot(root);
    reactRoot.render(
      <I18nextProvider i18n={i18n}>
        <SettingsApp />
      </I18nextProvider>
    );
  } else {
    console.error('[Settings] root element not found!');
  }
}

bootstrap();

