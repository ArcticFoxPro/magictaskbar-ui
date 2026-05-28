import { useState, useEffect, useRef, useCallback } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { invoke, FuncCommand } from "@magic-ui/lib";
import "./styles.css";
import { LoadingOutlined } from "@ant-design/icons";

// 毛玻璃效果圆角
const POPUP_CORNER_RADIUS = 16;

interface SonFunction {
  num?: string;
  name: string;
  btnId?: string;
  supportMachine?: string;
}

interface AiFunction {
  num?: string;
  name: string;
  btnId?: string;
  supportMachine?: string;
  sons: SonFunction[];
}

interface ParsedScene {
  sceneType: string;
  aiFunctions: AiFunction[];
  enterMessage?: string;
  exitMessage?: string;
  id?: string;
  name?: string;
  // keep other nodes for future use
  raw?: Element;
}

const parsedScenes: Record<string, ParsedScene> = {};
// 定义中文到英文 ID 的映射关系
const TITLE_TO_ID_MAP: Record<string, string> = {
  "总结": "Summary",
  "脑图": "MindMap",
  "信息图": "InfoGraphic",
  "润色": "Polishing",
  "翻译": "Translation",
  "纠错": "Proofreading",
  "扩写": "Expand",
  "标题生成": "TitleGeneration",
  "推荐提问": "SuggestedQuestions",
  "荣耀分享": "HonorShare",
  "写评语": "Comments",
  "头脑风暴助手": "Crainstorm",
  "短视频脚本创作": "VideoScript",
  "读书笔记": "ReadingNotes",
  "文档大纲": "OutlineGenerate",
  "AI搜索": "AISearch",
  "景点识别": "LandmarkIdentify",
  "网页总结": "WebPageSummary",
  "YOYO识屏": "YOYOScreenshot",
  "截图提问": "YOYOScreenshot",
  "歌曲解读": "SongInterpretation",
  "视频解读": "VideoInterpretation",
  "歌曲相似推荐": "SongSimilarRecommend",
  "视频相似推荐": "VideoSimilarRecommend",
  "观点洞察": "OpinionInsight",
};

function logSceneDetails(s: ParsedScene) {
  console.group(
    `[AIRecommend] Scene ${s.sceneType} details (id=${s.id ?? ''}, name=${s.name ?? ''})`
  );
  if (s.enterMessage) console.info('enterMessage:', s.enterMessage);
  if (s.exitMessage) console.info('exitMessage:', s.exitMessage);
  console.info('aiFunctions count:', s.aiFunctions.length);
  s.aiFunctions.forEach((f, idx) => {
    console.group(
      `aiFunction[#${idx}] num=${f.num ?? ''} name=${f.name} supportMachine=${f.supportMachine ?? ''}`
    );
    if (f.sons && f.sons.length > 0) {
      f.sons.forEach((sn, j) => {
        console.info(
          `sonFunction[#${j}] num=${sn.num ?? ''} name=${sn.name} supportMachine=${sn.supportMachine ?? ''}`
        );
      });
    } else {
      console.info('sonFunction: none');
    }
    console.groupEnd();
  });
  console.groupEnd();
}

async function loadConfig() {
  try {
    // Prefer relative to the toolbar app folder (dist/toolbar -> ../static)
    const tryPaths = [
      '../static/AIRecommend.xml',
      '/static/AIRecommend.xml',
    ];
    let txt = '';
    for (const p of tryPaths) {
      try {
        const res = await fetch(p);
        if (res.ok) { txt = await res.text(); break; }
      } catch {}
    }
    if (!txt) throw new Error('AIRecommend.xml not found');
    const doc = new DOMParser().parseFromString(txt, 'application/xml');
    const scenes = Array.from(doc.querySelectorAll('Scene'));
    scenes.forEach((s) => {
      const sceneType = s.getAttribute('sceneType') || '';
      const id = s.getAttribute('id') || '';
      const name = s.getAttribute('name') || '';
      const enterMsgNode = s.querySelector('enterMessage');
      const enterMsgAttr = s.getAttribute('enterMessage');
      const exitMsgNode = s.querySelector('exitMessage');
      const exitMsgAttr = s.getAttribute('exitMessage');
      const enterMessage = (enterMsgNode?.textContent || enterMsgAttr || '').trim() || undefined;
      const exitMessage = (exitMsgNode?.textContent || exitMsgAttr || '').trim() || undefined;

      const aiFuncs: AiFunction[] = Array.from(s.querySelectorAll('aiFunction')).map((n) => {
        const num = n.getAttribute('num') || undefined;
        const name = n.getAttribute('name') || '';
        const btnId = n.getAttribute('btnId') || undefined;
        const supportMachine = n.getAttribute('supportMachine') || undefined;
        const sons: SonFunction[] = Array.from(n.querySelectorAll('sonFunction')).map((sn) => ({
          num: sn.getAttribute('num') || undefined,
          name: sn.getAttribute('name') || '',
          btnId: sn.getAttribute('btnId') || undefined,
          supportMachine: sn.getAttribute('supportMachine') || undefined,
        })).filter(sf => sf.name);
        return { num, name, btnId, supportMachine, sons };
      }).filter(f => f.name);

      parsedScenes[sceneType] = { sceneType, aiFunctions: aiFuncs, enterMessage, exitMessage, id, name, raw: s };
    });

    // Print a concise summary for verification
    const summary: Record<string, { name?: string; id?: string; enterMessage?: string; exitMessage?: string; aiFunctions: string[]; sonsCount: number }>
      = {};
    Object.keys(parsedScenes).forEach((k) => {
      const s = parsedScenes[k];
      const aiNames = s ? s.aiFunctions.map(f => f.name) : [];
      const sonsCount = s ? s.aiFunctions.reduce((acc, f) => acc + (f.sons?.length || 0), 0) : 0;
      summary[k] = s ? { name: s.name, id: s.id, enterMessage: s.enterMessage, exitMessage: s.exitMessage, aiFunctions: aiNames, sonsCount } : { aiFunctions: [], sonsCount: 0 };
    });
    console.group('[AIRecommend] Parsed config summary (messages + nested)');
    console.table(summary);
    // Print full details for verification (includes sonFunction entries)
    Object.values(parsedScenes).forEach((scene) => logSceneDetails(scene));
    console.groupEnd();
  } catch (e) {
    console.warn('[AIRecommend] failed to load config', e);
  }
}

export function AIRecommendModule() {
  const [items, setItems] = useState<string[]>([]);
  const [currentScene, setCurrentScene] = useState<ParsedScene | null>(null);
  const [processData, setProcessData] = useState<Record<string, number> | null>(null);
  // Per aiFunction index, cache filtered sons according to processData
  const [filteredSonsByIndex, setFilteredSonsByIndex] = useState<Record<number, SonFunction[]>>({});
  // Per aiFunction index, mark selected when a sonFunction with PD value 1 determines the label
  const [selectedIndices, setSelectedIndices] = useState<Record<number, boolean>>({});
  // Override displayed labels when user clicks a sonFunction; persists until scene changes/clears
  const [overrideLabelsByIndex, setOverrideLabelsByIndex] = useState<Record<number, string>>({});
  // Track whether latest ProcessData is valid; if invalid, hide aiFunctions with sons
  const [processDataValid, setProcessDataValid] = useState<boolean>(true);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const popupGlassRef = useRef<HTMLDivElement | null>(null);
  const [popup, setPopup] = useState<{
    openIndex: number | null;
    sons: SonFunction[];
    left: number;
    top: number;
  }>({ openIndex: null, sons: [], left: 0, top: 0 });
  const [recommendedItems, setRecommendedItems] = useState<string[]>([]);
  const currentSceneKeyRef = useRef<string | null>(null);
  const lastNotifyRef = useRef<string>("");
  const lastRawProcessDataRef = useRef<string>("");
  const [isMoreLoading, setIsMoreLoading] = useState<boolean>(false);
  const isMoreLoadingRef = useRef<boolean>(false);
  const moreLoadingTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const debounceTimeoutRef = useRef<NodeJS.Timeout | null>(null);
  // 毛玻璃效果：显示弹窗模糊
  const showPopupGlass = useCallback(() => {
    const el = popupGlassRef.current;
    if (!el) return;
    
    const rect = el.getBoundingClientRect();
    if (rect.left <= -9999 || rect.top <= -9999 || rect.width <= 0 || rect.height <= 0) return;
    
    requestAnimationFrame(() => {
      (invoke as any)('popup_glass_show', {
        id: 'ai-recommend-popup',
        x: Math.round(rect.left),
        y: Math.round(rect.top),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
        cornerRadius: POPUP_CORNER_RADIUS
      }).catch((e: any) => {
        console.warn('[AIRecommend] Failed to show glass effect:', e);
      });
    });
  }, []);
  
  // 毛玻璃效果：隐藏弹窗模糊
  const hidePopupGlass = useCallback(() => {
    (invoke as any)('popup_glass_hide', { id: 'ai-recommend-popup' }).catch((e: any) => {
      console.warn('[AIRecommend] Failed to hide glass effect:', e);
    });
  }, []);

  
  useEffect(() => {
    let mounted = true;
    let unlisten: (() => void) | null = null;
    let unlistenClear: (() => void) | null = null;
    let unlistenNotify: (() => void) | null = null;
    let unlistenRecog: (() => void) | null = null;
    let undisposerProcess: (() => void) | null = null;

    (async () => {
      await loadConfig(); // 仅初始化时加载一次到内存
      try {
        const win = getCurrentWebviewWindow();
        const disposer = await win.listen<number | string | { type?: string; sceneType?: string | number }>('ai-recommend:scene', (event) => {
          const key = String(event.payload);
          setIsMoreLoading(false); // 切换场景时重置加载状态
          isMoreLoadingRef.current = false; // 同步更新，这是拦截的关键！
          currentSceneKeyRef.current = key; 
          const prev = currentSceneKeyRef.current;
          if (prev && prev !== key) {
            const ps = parsedScenes[prev];
            if (ps?.exitMessage) {
              console.info('[AIRecommend] exitMessage:', ps.exitMessage);
            }
          }
          currentSceneKeyRef.current = key;

          const s = parsedScenes[key];
          if (mounted) {
            setCurrentScene(s || null);
            // 这里可以根据逻辑决定是否重置推荐项
            setRecommendedItems([]); 
            setIsMoreLoading(false); // 切换场景时重置加载状态
            isMoreLoadingRef.current = false; // 同步更新，这是拦截的关键！
          }
          // Build list based on current processData: if a function has sons and PD specifies keys,
          // label shows sonFunction with value 1; otherwise show aiFunction name. Track selection.
          const nextSelected: Record<number, boolean> = {};
          const list = s ? s.aiFunctions.map((f, idx) => {
            if (!f.sons || f.sons.length === 0) { nextSelected[idx] = false; return f.name; }
            // Hide aiFunctions with sons when ProcessData is invalid
            if (!processDataValid) { nextSelected[idx] = false; return ''; }
            const pd = processData;
            // 当未收到 process_data 时，隐藏包含 sonFunction 的 aiFunction
            if (!pd) { nextSelected[idx] = false; return ''; }
            const candidates = f.sons.filter(sn => !!sn.btnId && Object.prototype.hasOwnProperty.call(pd, sn.btnId!));
            // Requirement: if PD has no corresponding sonFunction keys, hide this aiFunction
            if (candidates.length === 0) { nextSelected[idx] = false; return ''; }
            const chosen = candidates.find(sn => pd[sn.btnId!] === 1);
            nextSelected[idx] = !!chosen;
            return (chosen?.name) || f.name;
          }) : [];
          // Also refresh filtered son lists if needed
          if (s && processData) {
            const next: Record<number, SonFunction[]> = {};
            s.aiFunctions.forEach((f, idx) => {
              const sons = (f.sons || []).filter(sn => !!sn.btnId && Object.prototype.hasOwnProperty.call(processData, sn.btnId!));
              if (sons.length > 0) next[idx] = sons;
            });
            setFilteredSonsByIndex(next);
          } else {
            setFilteredSonsByIndex({});
          }
          if (s?.enterMessage) {
            console.info('[AIRecommend] enterMessage:', s.enterMessage);
          }
          // Also print full details of the active scene, including sonFunction
          if (s) {
            logSceneDetails(s);
          }
          console.info('[AIRecommend] set scene (via backend message)', { sceneType: key, items: list });
          if (mounted) {
            setItems(list);
            setCurrentScene(s || null);
            setSelectedIndices(nextSelected);
            // Reset any label overrides on scene change
            setOverrideLabelsByIndex({});
            setRecommendedItems([]);
            setIsMoreLoading(false); // 切换场景时重置加载状态
            isMoreLoadingRef.current = false; // 同步更新，这是拦截的关键！
          }
        });
        unlisten = disposer;

        const disposerClear = await win.listen<string | number>('ai-recommend:clear', (event) => {
          const sceneKey = String(event.payload);
          console.info('[AIRecommend] clear scene items due to exitMessage match', { sceneType: sceneKey });
           currentSceneKeyRef.current = null;
          if (mounted) {
            setItems([]);
            setFilteredSonsByIndex({});
            setSelectedIndices({});
            setOverrideLabelsByIndex({});
            setRecommendedItems([]); 
            setIsMoreLoading(false); // 切换场景时重置加载状态
            isMoreLoadingRef.current = false; // 同步更新，这是拦截的关键！
          }
        });
        unlistenClear = disposerClear;

        // New: react to notify events by matching enter/exitMessage
        const disposerNotify = await win.listen<string>('ai-recommend:notify', (event) => {
          const notify = String(event.payload).trim();
          
          if (!notify) return;
            // 场景发生变化时，如果不是 startRead，可能需要清空推荐
          if (notify !== lastNotifyRef.current) {
            setRecommendedItems([]);
            setIsMoreLoading(false); // 切换场景时重置加载状态
            isMoreLoadingRef.current = false; // 同步更新，这是拦截的关键！
          }
          
          lastNotifyRef.current = notify;
          // Try to find a scene whose enterMessage matches
          const keys = Object.keys(parsedScenes);
          let matched: ParsedScene | null = null;
          for (const k of keys) {
            const s = parsedScenes[k];
            if (s?.enterMessage && s.enterMessage === notify) { matched = s; break; }
          }
          if (matched) {
            currentSceneKeyRef.current = matched.sceneType;
            // Use current processData if present to compute labels
            const nextSelected: Record<number, boolean> = {};
            const list = matched.aiFunctions.map((f, idx) => {
              if (!f.sons || f.sons.length === 0) { nextSelected[idx] = false; return f.name; }
              if (!processDataValid) { nextSelected[idx] = false; return ''; }
              // 当 process_data 为空时，隐藏包含 sonFunction 的 aiFunction
              if (!processData) { nextSelected[idx] = false; return ''; }
              const candidates = f.sons.filter(sn => !!sn.btnId && Object.prototype.hasOwnProperty.call(processData, sn.btnId!));
              if (candidates.length === 0) { nextSelected[idx] = false; return ''; }
              const chosen = candidates.find(sn => processData[sn.btnId!] === 1);
              nextSelected[idx] = !!chosen;
              return (chosen?.name) || f.name;
            });
            // Update filteredSonsByIndex for popup rendering
            if (processData) {
              const next: Record<number, SonFunction[]> = {};
              matched.aiFunctions.forEach((f, idx) => {
                const sons = (f.sons || []).filter(sn => !!sn.btnId && Object.prototype.hasOwnProperty.call(processData, sn.btnId!));
                if (sons.length > 0) next[idx] = sons;
              });
              setFilteredSonsByIndex(next);
            } else {
              setFilteredSonsByIndex({});
            }
            console.info('[AIRecommend] notify-enter matched; set items', { sceneType: matched.sceneType, items: list });
            currentSceneKeyRef.current = matched.sceneType;

            if (mounted) {
              setItems(list);
              setCurrentScene(matched);
              setSelectedIndices(nextSelected);
              // New scene matched by notify: clear overrides
              setOverrideLabelsByIndex({});
            }
            return;
          } else {
          // 如果匹配到 exitMessage，也得清空 Ref
          for (const k of keys) {
            const s = parsedScenes[k];
            if (s?.exitMessage && s.exitMessage === notify) {
              currentSceneKeyRef.current = null;
              break;
            }
          }
        }
          // Else, check exitMessage and clear
          for (const k of keys) {
            const s = parsedScenes[k];
            if (s?.exitMessage && s.exitMessage === notify) {
              console.info('[AIRecommend] notify-exit matched; clear items', { sceneType: s.sceneType });
                  currentSceneKeyRef.current = null;
              if (mounted) {
                setItems([]);
                setCurrentScene(null);
              }
              return;
            }
          }
          console.info('[AIRecommend] notify did not match any enter/exitMessage:', notify);
        });
        unlistenNotify = disposerNotify;

        // New: receive process data JSON as string and update filtering
        const disposerProcess = await win.listen<string>('ai-recommend:process', (event) => {
          const raw = String(event.payload || '').trim();
          if (!raw) return;
          if (lastRawProcessDataRef.current !== raw) {
            // 1. 立即清空推荐项，这会强制 displayItems 渲染原始 items
            setRecommendedItems([]); 
            setIsMoreLoading(false); // 切换场景时重置加载状态
            isMoreLoadingRef.current = false; // 同步更新，这是拦截的关键！
            // 2. 强制重置覆盖标签（防止上一个文档的选择影响当前文档）
            setOverrideLabelsByIndex({});
            // 3. 更新记录值
            lastRawProcessDataRef.current = raw;
            
          }

          try {
            const obj = JSON.parse(raw) as Record<string, number>;
            if (!mounted) return;
            // 1. 获取当前场景（从 Ref 获取最新值，避免闭包陷阱）
            const activeKey = currentSceneKeyRef.current; 
            // 更新记录，供下次对比
            setProcessData(obj);
            setProcessDataValid(true);
            // Recompute labels and filtered sons for current scene
            if (activeKey) {
              const s = parsedScenes[activeKey];
              if (s) {
                const nextSelected: Record<number, boolean> = {};
                const list = s.aiFunctions.map((f, idx) => {
                  if (!f.sons || f.sons.length === 0) { nextSelected[idx] = false; return f.name; }
                  if (!processDataValid) { nextSelected[idx] = false; return ''; }
                  const candidates = f.sons.filter(sn => !!sn.btnId && Object.prototype.hasOwnProperty.call(obj, sn.btnId!));
                  // If PD doesn't include any son keys for this function, hide it regardless of overrides
                  if (candidates.length === 0) { nextSelected[idx] = false; return ''; }
                  const chosen = candidates.find(sn => obj[sn.btnId!] === 1);
                  nextSelected[idx] = !!chosen;
                  // Apply override if user selected a son previously; else use chosen or fallback
                  return overrideLabelsByIndex[idx] ?? (chosen?.name) ?? f.name;
                });
                setItems(list);
                setSelectedIndices(nextSelected);
                const next: Record<number, SonFunction[]> = {};
                s.aiFunctions.forEach((f, idx) => {
                  const sons = (f.sons || []).filter(sn => !!sn.btnId && Object.prototype.hasOwnProperty.call(obj, sn.btnId!));
                  if (sons.length > 0) next[idx] = sons;
                });
                setFilteredSonsByIndex(next);
              }
            }
          } catch (e) {
            console.warn('[AIRecommend] invalid process_data JSON', e);
            setProcessDataValid(false);
            setProcessData(null);
            const activeKey = currentSceneKeyRef.current;
            if (activeKey) {
              const s = parsedScenes[activeKey];
              if (s) {
                // 重新生成不含子功能过滤的原始列表
                const list = s.aiFunctions.map(f => {
                  return (!f.sons || f.sons.length === 0) ? f.name : ""; 
                });
                setItems(list);
      }
    }

          }
        });
        undisposerProcess = disposerProcess

        const disposerRecog = await win.listen<string>('ai-recommend:recognition-data', (event) => {
        if (!isMoreLoadingRef.current) {
          console.info('[AIRecommend] 拦截到过期请求：页面已切换，放弃渲染推荐项');
          return; 
        }
          try {
         const data = JSON.parse(event.payload);
        if (data.recommendFunctions && Array.isArray(data.recommendFunctions)) {
          // 过滤逻辑：仅保留在 TITLE_TO_ID_MAP 中存在的键名
          const filtered = data.recommendFunctions.filter((name: string) =>
            Object.prototype.hasOwnProperty.call(TITLE_TO_ID_MAP, name)
          );
          setRecommendedItems(filtered);
          setIsMoreLoading(false); // 关键：数据返回，停止加载
          isMoreLoadingRef.current = false; // 同步更新，这是拦截的关键！
          // 数据返回时清除定时器
          if (moreLoadingTimeoutRef.current) {
            clearTimeout(moreLoadingTimeoutRef.current);
            moreLoadingTimeoutRef.current = null;
          }
        }
          } catch (e) {
            console.error('[AIRecommend] Failed to parse recognition data', e);
          }
        });
        unlistenRecog = disposerRecog
        // Note: no need to keep an extra handle; window.listen returns a function to unlisten
      } catch (e) {
        console.warn('[AIRecommend] listen failed', e);
      }
    })();

    return () => {
      mounted = false;
      if (unlisten) try { unlisten(); } catch {}
      if (unlistenClear) try { unlistenClear(); } catch {}
      if (unlistenNotify) try { unlistenNotify(); } catch {}
      if (unlistenRecog) try { unlistenRecog(); } catch {}
      if (undisposerProcess) try { undisposerProcess(); } catch {}
      // 组件卸载时清除定时器
      if (moreLoadingTimeoutRef.current) {
        clearTimeout(moreLoadingTimeoutRef.current);
        moreLoadingTimeoutRef.current = null;
      }
    };
  }, []);

  // 关闭弹窗时立即隐藏毛玻璃
  const closePopup = useCallback(() => {
    setPopup({ openIndex: null, sons: [], left: 0, top: 0 });
  }, []);

  // 失去焦点（点击到非 toolbar 窗口）时关闭弹窗
  useEffect(() => {
    let unlistenBlur: (() => void) | null = null;
    (async () => {
      try {
        const win = getCurrentWebviewWindow();
        unlistenBlur = await win.listen('tauri://blur', () => {
          closePopup();
        });
      } catch {}
    })();

    const onWindowBlur = () => {
      closePopup();
    };
    window.addEventListener('blur', onWindowBlur);

    return () => {
      if (unlistenBlur) try { unlistenBlur(); } catch {}
      window.removeEventListener('blur', onWindowBlur);
    };
  }, [closePopup]);

  // 点击外部区域时关闭弹窗
  useEffect(() => {
    const onDocClick = (e: MouseEvent) => {
      const root = containerRef.current;
      if (!root) return;
      if (popup.openIndex === null) return;
      const target = e.target as HTMLElement | null;
      if (target && root.contains(target)) {
        // 如果点击在弹窗或容器内部，但不在弹窗内元素之外则不关闭
        const popupEl = root.querySelector('.ai-recommend-popup');
        if (popupEl && popupEl.contains(target)) return;
      }
      closePopup();
    };
    document.addEventListener('click', onDocClick, true);
    return () => document.removeEventListener('click', onDocClick, true);
  }, [popup.openIndex, closePopup]);

  // 毛玻璃效果：监听弹窗状态变化
  const prevPopupOpenIndexRef = useRef<number | null>(null);
  
  useEffect(() => {
    // 当从有值变为 null 时，立即隐藏毛玻璃
    if (prevPopupOpenIndexRef.current !== null && popup.openIndex === null) {
      hidePopupGlass();
    }
    prevPopupOpenIndexRef.current = popup.openIndex;
    
    if (popup.openIndex === null) return;
    showPopupGlass();
  }, [popup.openIndex, showPopupGlass, hidePopupGlass]);

  // 将事件类型指定为 React.MouseEvent<HTMLElement>
  const handleItemClick = (e: React.MouseEvent<HTMLElement>, t: string, i: number) => {
    e.stopPropagation();
    const fn = currentScene?.aiFunctions[i];
    const sons = (filteredSonsByIndex[i] ?? fn?.sons ?? []);
    if (sons.length > 0) {
      // 展示弹窗列表
              const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
              const parentRect = containerRef.current?.getBoundingClientRect();
              const left = parentRect ? rect.left - parentRect.left : 0;
              const top = parentRect ? rect.bottom - parentRect.top + 6 : rect.bottom + 6;
              setPopup({ openIndex: i, sons, left, top });
    } else {
      // 无子功能：发送配置中的 btnId，标题使用 name（后端会用 name 作为 window_title）
      const meta = fn;
      const btnId = meta?.btnId || meta?.name || t;
      const windowTitle = meta?.name || t;
      console.info('[AIRecommend] item clicked -> send HeartbeatClick', { btnID: btnId, windowTitle, sceneType: currentScene?.sceneType });
      // 特殊处理：AvailableNow 触发屏幕识别 gRPC 调用
      if (t === '更多功能') {
        // 上报点击功能
        (invoke as any)(FuncCommand.ReportAiRecommendFunction, { content: `更多功能` });
        
        // 开启加载状态
        setIsMoreLoading(true);
        isMoreLoadingRef.current = true; // 同步更新
        
        // 清除之前的定时器，重新开始10秒计时
        if (moreLoadingTimeoutRef.current) {
          clearTimeout(moreLoadingTimeoutRef.current);
        }
        moreLoadingTimeoutRef.current = setTimeout(() => {
          console.info('[AIRecommend] Loading timeout after 10s, reset loading state');
          setIsMoreLoading(false);
          isMoreLoadingRef.current = false;
          moreLoadingTimeoutRef.current = null;
        }, 10000);
        
        // 获取当前展示的aiFunction names，用逗号拼接
        const aiFunctionNames = items.filter(name => name && name.trim()).join('#');
        console.info('[AIRecommend] AvailableNow clicked, aiFunctions:', aiFunctionNames);
        invoke(FuncCommand.AiRecommendSendScreenRecognition as any, { btnId, aiFunctionNames } as any).catch((err: any) => {
          console.error('[AIRecommend] invoke AiRecommendSendScreenRecognition failed', err);
        });
      } else {
        // 普通功能点击逻辑
            const windowTitle = t || ""; 
            const CombtnId = TITLE_TO_ID_MAP[windowTitle] || btnId;
        
        // 上报点击功能
        (invoke as any)(FuncCommand.ReportAiRecommendFunction, { content: `${windowTitle}` });
        
        // 清除之前设置的防抖定时器
        if (debounceTimeoutRef.current) {
          clearTimeout(debounceTimeoutRef.current);
        }
        // 设置一个新的防抖定时器
        debounceTimeoutRef.current = setTimeout(() => {
            invoke(FuncCommand.AiRecommendIconClicked as any,  {  btnId: CombtnId,  windowTitle: windowTitle } as any).catch((err: any) => {
                    console.error('点击调用失败', err);
            });
          // 执行完成后，将定时器引用置空
          debounceTimeoutRef.current = null;
        }, 800); // 设置防抖延迟时间
      }
    }
  };

  // 在组件的 useEffect 清理函数中，清除防抖定时器以防止内存泄漏
  useEffect(() => {
    return () => {
      // 清理“更多功能”的超时定时器
      if (moreLoadingTimeoutRef.current) {
        clearTimeout(moreLoadingTimeoutRef.current);
      }
      // 清理防抖定时器
      if (debounceTimeoutRef.current) {
        clearTimeout(debounceTimeoutRef.current);
      }
    };
  }, []);

  const renderItems = () => {
    const elements: JSX.Element[] = [];
    items.forEach((item, idx) => {
      const fnMeta = currentScene?.aiFunctions[idx];
      const isAvailableNow = fnMeta?.btnId === "AvailableNow" || fnMeta?.name === "更多功能";
      if (isAvailableNow) {
        const validRecommended = recommendedItems.filter(name => 
          Object.prototype.hasOwnProperty.call(TITLE_TO_ID_MAP, name)
        );
        // 场景 A: 推荐内容已返回 -> 隐藏“更多功能”和 Loading，显示推荐项
        if (validRecommended.length > 0) {
          validRecommended.forEach((recName, recIdx) => {
            elements.push(
              <div
                key={`rec-${recIdx}`}
                className="ai-recommend-item ai-recommend-item-animate"
                onClick={(e) => handleItemClick(e, recName, idx)}
              >
                <span className="ai-recommend-text">{recName}</span>
              </div>
            );
          });
        } else {
          // 场景 B: 推荐内容未返回 -> 始终显示“更多功能”按钮
          elements.push(
            <div
              key={`more-${idx}`}
              className="ai-recommend-item"
              style={{ display: 'flex', alignItems: 'center' }}
              onClick={(e) => handleItemClick(e, item || "更多功能", idx)}
            >
              <span className="ai-recommend-text">{item || "更多功能"}</span>
              {/* 关键修改：如果正在加载，在按钮内部或右侧显示 Loading */}
              {isMoreLoading && (
                <div className="ai-recommend-loading-inline">
                  <LoadingOutlined style={{ fontSize: 14, marginLeft: 6 }} />
                </div>
              )}
            </div>
          );
        }
      } else if (item && item.trim() !== '') {
        // 普通项渲染保持不变...
        elements.push(
          <div
            key={`normal-${idx}`}
            className={`ai-recommend-item ${selectedIndices[idx] ? 'ai-recommend-item--selected' : ''}`}
            onClick={(e) => handleItemClick(e, item, idx)}
          >
            <span className="ai-recommend-text">{item}</span>
            {fnMeta?.sons && fnMeta.sons.length > 0 && (
              <img className="ai-recommend-updown" src="/static/icons/updown.svg" alt="" />
            )}
          </div>
        );
      }
    });
    return elements;
  };

  return (
    <div
      className="taskbar-item taskbar-module ai-recommend-module"
      ref={containerRef}
    >
      {renderItems()}
      {popup.openIndex !== null && (
        <div
          ref={popupGlassRef}
          className="ai-recommend-popup"
          style={{ left: popup.left, top: popup.top }}
          onClick={(e) => e.stopPropagation()}
        >
          {popup.sons.map((sn, idx) => (
            <div
              key={idx}
              className="ai-recommend-son-item"
              onClick={(e) => {
                e.stopPropagation();
                const btnId = sn.btnId || sn.name;
                const windowTitle = sn.name;
                
                // 上报点击功能
                (invoke as any)(FuncCommand.ReportAiRecommendFunction, { content: `${windowTitle}` });
                
                console.info('[AIRecommend] son item clicked -> send HeartbeatClick', { btnID: btnId, windowTitle, sceneType: currentScene?.sceneType });
                invoke(FuncCommand.AiRecommendIconClicked as any, { btnId, windowTitle } as any).catch((err: any) => {
                  console.error('[AIRecommend] invoke AiRecommendIconClicked failed', err);
                });
                // 点击后关闭弹窗
                // Persist selection as the default label for the item
                if (popup.openIndex !== null) {
                  setOverrideLabelsByIndex((prev) => ({ ...prev, [popup.openIndex!]: sn.name }));
                  setSelectedIndices((prev) => ({ ...prev, [popup.openIndex!]: true }));
                  // Update items immediately
                  setItems((prev) => {
                    const next = [...prev];
                    if (popup.openIndex !== null) next[popup.openIndex!] = sn.name;
                    return next;
                  });
                }
                closePopup();
              }}
            >
              <div className="ai-recommend-son-text">{sn.name}</div>
              {(() => {
                const idxOpen = popup.openIndex;
                const hasOverride = idxOpen !== null && overrideLabelsByIndex[idxOpen!] !== undefined;
                const overrideActive = hasOverride && idxOpen !== null && overrideLabelsByIndex[idxOpen!] === sn.name;
                const pdActive = !hasOverride && !!processData && !!sn.btnId && processData[sn.btnId!] === 1;
                const isActive = overrideActive || pdActive;
                return isActive ? (
                  <img src="/static/icons/confirm.svg" alt="当前" className="ai-recommend-badge" />
                ) : null;
              })()}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export default AIRecommendModule;
