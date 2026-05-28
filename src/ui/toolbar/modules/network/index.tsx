import { useEffect, useState, useCallback, useRef } from "react";
import { invoke, FuncCommand, FuncEvent, subscribe } from "@magic-ui/lib";
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { WifiNetwork } from "@magic-ui/lib/system_state/wifi";
import { SlPopup } from "@shared/components/SlPopup";
import { $open_popups } from "../shared/state/mod";
import { useTranslation } from "react-i18next";
import "./styles.css";

// 毛玻璃效果相关常量
const POPUP_CORNER_RADIUS = 9;
const WLAN_LOCATION_PERMISSION_REQUIRED_ERR = "WLAN_LOCATION_PERMISSION_REQUIRED";

function isWlanLocationPermissionError(error: unknown): boolean {
  if (typeof error === "string") {
    return error.includes(WLAN_LOCATION_PERMISSION_REQUIRED_ERR);
  }

  if (error && typeof error === "object") {
    const maybeMessage = (error as any).message;
    if (typeof maybeMessage === "string" && maybeMessage.includes(WLAN_LOCATION_PERMISSION_REQUIRED_ERR)) {
      return true;
    }
    try {
      return JSON.stringify(error).includes(WLAN_LOCATION_PERMISSION_REQUIRED_ERR);
    } catch {
      return false;
    }
  }

  return false;
}

// 检测文本是否被截断，只有截断时才返回 title
function useTruncatedTitle(text: string): { ref: (el: HTMLElement | null) => void; title: string | undefined } {
  const [isTruncated, setIsTruncated] = useState(false);
  const elementRef = useRef<HTMLElement | null>(null);
  
  const setRef = useCallback((el: HTMLElement | null) => {
    elementRef.current = el;
    if (el) {
      setIsTruncated(el.scrollWidth > el.clientWidth);
    }
  }, []);
  
  useEffect(() => {
    if (elementRef.current) {
      setIsTruncated(elementRef.current.scrollWidth > elementRef.current.clientWidth);
    }
  }, [text]);
  
  return {
    ref: setRef,
    title: isTruncated ? text : undefined,
  };
}

// 网络共享设备名称单元格（只有截断时才显示 title）
function NetworkShareNameCell({ deviceName, t }: { deviceName: string; t: (key: string, options?: any) => string }) {
  const displayText = t('network.streaming_from', { deviceName });
  const { ref, title } = useTruncatedTitle(displayText);
  return (
    <div ref={ref} className="network-share-name" title={title}>
      {displayText}
    </div>
  );
}

// WiFi 名称单元格（只有截断时才显示 title）
function WifiNameCell({ ssid }: { ssid: string }) {
  const { ref, title } = useTruncatedTitle(ssid);
  return (
    <div ref={ref} className="network-wifi-name" title={title}>
      {ssid}
    </div>
  );
}

interface NetworkShareDevice {
  deviceId: string;
  deviceName: string;
  connected: boolean;
}

function sortWifiNetworks(list: WifiNetwork[]): WifiNetwork[] {
  return [...list].sort((a, b) => {
    if (a.connected && !b.connected) return -1;
    if (!a.connected && b.connected) return 1;
    const signalA = Number.isFinite(Number((a as any)?.signal)) ? Number((a as any)?.signal) : 0;
    const signalB = Number.isFinite(Number((b as any)?.signal)) ? Number((b as any)?.signal) : 0;
    return signalB - signalA;
  });
}

function isConnectedOnlySnapshot(list: WifiNetwork[]): boolean {
  return list.length === 1 && Boolean(list[0]?.connected);
}

export function NetworkModule() {
  const { t } = useTranslation();
  const [networks, setNetworks] = useState<WifiNetwork[]>([]);
  const [networkShareDevices, setNetworkShareDevices] = useState<NetworkShareDevice[]>([]);
  const [popupOpen, setPopupOpen] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [connectingSsid, setConnectingSsid] = useState<string | null>(null);
  const [connectingNetworkShareDeviceId, setConnectingNetworkShareDeviceId] = useState<string | null>(null);
  const [mutatingSsid, setMutatingSsid] = useState<string | null>(null);
  const [captivePortalSsid, setCaptivePortalSsid] = useState<string | null>(null);
  const captiveCheckSeq = useRef(0);
  const captiveCheckedSsid = useRef<string | null>(null);
  const [wlanEnabled, setWlanEnabled] = useState<boolean | null>(null);
  const [wlanSetting, setWlanSetting] = useState(false);
  const [connectedDetailsOpen, setConnectedDetailsOpen] = useState(false);
  const [connectedDetailsSsid, setConnectedDetailsSsid] = useState<string | null>(null);
  const [connectedNetworkShareDetailsOpen, setConnectedNetworkShareDetailsOpen] = useState(false);
  const [connectedNetworkShareDevice, setConnectedNetworkShareDevice] = useState<NetworkShareDevice | null>(null);
  const [autoConnect, setAutoConnect] = useState<boolean | null>(null);
  const [autoConnectSetting, setAutoConnectSetting] = useState(false);
  const [connectDialogOpen, setConnectDialogOpen] = useState(false);
  const [connectTarget, setConnectTarget] = useState<WifiNetwork | null>(null);
  const [connectPassword, setConnectPassword] = useState("");
  const [connectShowPassword, setConnectShowPassword] = useState(false);
  const [connectAutoConnect, setConnectAutoConnect] = useState(true);
  const [connectSubmitting, setConnectSubmitting] = useState(false);
  const [connectError, setConnectError] = useState<string | null>(null);
  const [locationPermissionRestricted, setLocationPermissionRestricted] = useState(false);
  const [awaitingStableWifiList, setAwaitingStableWifiList] = useState(false);
  const latestFetchRequestId = useRef(0);
  const latestWlanStateRequestId = useRef(0);
  const popupOpenRef = useRef(false);
  const networksRef = useRef<WifiNetwork[]>([]);
  const networkShareDevicesRef = useRef<NetworkShareDevice[]>([]);
  const wlanEnabledRef = useRef<boolean | null>(null);
  const awaitingStableWifiListRef = useRef(false);
  const wlanToggleVersionRef = useRef(0);
  const pendingWlanTargetRef = useRef<boolean | null>(null);
  const wlanFollowupTimerRef = useRef<number | null>(null);
  const stableWifiRetryTimerRef = useRef<number | null>(null);
  const stableWifiDeadlineRef = useRef(0);
  const fetchNetworksRef = useRef<(() => Promise<void>) | null>(null);
  const networkListRef = useRef<HTMLDivElement | null>(null);
  const networkShareConnectTimerRef = useRef<number | null>(null);
  const hadConnectedNetworkShareDeviceRef = useRef(false);
  const pendingNetworkShareDisconnectDeviceIdRef = useRef<string | null>(null);
  const pendingNetworkShareDisconnectResolveRef = useRef<(() => void) | null>(null);
  const pendingNetworkShareDisconnectRejectRef = useRef<((error: Error) => void) | null>(null);
  const pendingNetworkShareDisconnectTimerRef = useRef<number | null>(null);
  const popupGlassRef = useRef<HTMLDivElement | null>(null);
  const glassUpdateSeqRef = useRef(0);
  
  // 二级窗口毛玻璃效果 refs
  const passwordDialogGlassRef = useRef<HTMLDivElement | null>(null);
  const connectedDetailsGlassRef = useRef<HTMLDivElement | null>(null);
  const networkShareDetailsGlassRef = useRef<HTMLDivElement | null>(null);
  // 二级窗口状态追踪 refs（用于检测关闭时自动隐藏毛玻璃）
  const prevConnectDialogOpenRef = useRef(false);
  const prevConnectedDetailsOpenRef = useRef(false);
  const prevConnectedNetworkShareDetailsOpenRef = useRef(false);

  const applyNetworkShareDevices = useCallback((list: NetworkShareDevice[]) => {
    const next = Array.isArray(list)
      ? list.filter((device) => typeof device?.deviceName === "string" && device.deviceName.trim().length > 0)
      : [];

    networkShareDevicesRef.current = next;
    setNetworkShareDevices(next);
  }, []);

  const fetchNetworkShareDevices = useCallback(async () => {
    try {
      const list = await tauriInvoke<NetworkShareDevice[]>("system_get_network_share_devices");
      if (Array.isArray(list)) {
        applyNetworkShareDevices(list);
      }
    } catch {
    }
  }, [applyNetworkShareDevices]);

  const clearNetworkShareConnectTimer = useCallback(() => {
    if (networkShareConnectTimerRef.current != null) {
      window.clearTimeout(networkShareConnectTimerRef.current);
      networkShareConnectTimerRef.current = null;
    }
  }, []);

  // 毛玻璃效果：显示弹窗模糊
  // 使用防抖避免频繁调用
  const showPopupGlass = useCallback(() => {
    if (!popupOpenRef.current) return;
    const scheduledSeq = glassUpdateSeqRef.current;

    const el = popupGlassRef.current;
    if (!el) return;
    
    const rect = el.getBoundingClientRect();
    // 必须同时检查坐标和尺寸，确保元素已正确渲染
    if (rect.left <= -9999 || rect.top <= -9999 || rect.width <= 0 || rect.height <= 0) return;
    
    // 使用 requestAnimationFrame 确保在下一帧执行
    requestAnimationFrame(() => {
      if (!popupOpenRef.current || scheduledSeq !== glassUpdateSeqRef.current) return;
      
      (invoke as any)('popup_glass_show', {
        id: 'network-primary',
        x: Math.round(rect.left),
        y: Math.round(rect.top),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
        cornerRadius: POPUP_CORNER_RADIUS
      }).catch((e: any) => {
        console.warn('[Network] Failed to show glass effect:', e);
      });
    });
  }, []);
  
  // 毛玻璃效果：隐藏弹窗模糊
  const hidePopupGlass = useCallback(() => {
    glassUpdateSeqRef.current++; // 使任何进行中的更新失效
    (invoke as any)('popup_glass_hide', { id: 'network-primary' }).catch((e: any) => {
      console.warn('[Network] Failed to hide glass effect:', e);
    });
  }, []);
  
  // 二级窗口毛玻璃效果：显示
  const showSecondaryGlass = useCallback((ref: React.RefObject<HTMLDivElement | null>, id: string, cornerRadius: number = 12) => {
    if (!popupOpenRef.current) return;
    const scheduledSeq = glassUpdateSeqRef.current;

    const el = ref.current;
    if (!el) return;
    
    const rect = el.getBoundingClientRect();
    // 必须同时检查坐标和尺寸，确保元素已正确渲染
    if (rect.width <= 0 || rect.height <= 0) return;
    
    requestAnimationFrame(() => {
      if (!popupOpenRef.current || scheduledSeq !== glassUpdateSeqRef.current) return;

      (invoke as any)('popup_glass_show', {
        id,
        x: Math.round(rect.left),
        y: Math.round(rect.top),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
        cornerRadius
      }).catch((e: any) => {
        console.warn('[Network] Failed to show secondary glass effect:', e);
      });
    });
  }, []);
  
  // 二级窗口毛玻璃效果：隐藏
  const hideSecondaryGlass = useCallback((id: string) => {
    glassUpdateSeqRef.current++;
    (invoke as any)('popup_glass_hide', { id }).catch((e: any) => {
      console.warn('[Network] Failed to hide secondary glass effect:', e);
    });
  }, []);

  const closeNetworkPopup = useCallback(() => {
    popupOpenRef.current = false;
    hidePopupGlass();
    hideSecondaryGlass('network-password');
    hideSecondaryGlass('network-wifi-details');
    hideSecondaryGlass('network-share-details');
    setPopupOpen(false);
  }, [hidePopupGlass, hideSecondaryGlass]);
  
  // 二级窗口毛玻璃效果：监听密码输入弹窗
  useEffect(() => {
    // 当从 true 变为 false 时，立即隐藏毛玻璃
    if (prevConnectDialogOpenRef.current && !connectDialogOpen) {
      hideSecondaryGlass('network-password');
    }
    prevConnectDialogOpenRef.current = connectDialogOpen;
    
    if (!connectDialogOpen) return;
    
    const checkAndShow = () => {
      const el = passwordDialogGlassRef.current;
      if (!el) return false;
      const rect = el.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) {
        showSecondaryGlass(passwordDialogGlassRef, 'network-password', 12);
        return true;
      }
      return false;
    };
    
    if (!checkAndShow()) {
      requestAnimationFrame(() => {
        if (!checkAndShow()) {
          setTimeout(checkAndShow, 50);
        }
      });
    }
  }, [connectDialogOpen, showSecondaryGlass, hideSecondaryGlass]);
  
  // 二级窗口毛玻璃效果：监听 WiFi 已连接详情弹窗
  useEffect(() => {
    // 当从 true 变为 false 时，立即隐藏毛玻璃
    if (prevConnectedDetailsOpenRef.current && !connectedDetailsOpen) {
      hideSecondaryGlass('network-wifi-details');
    }
    prevConnectedDetailsOpenRef.current = connectedDetailsOpen;
    
    if (!connectedDetailsOpen || !connectedDetailsSsid) return;
    
    const checkAndShow = () => {
      const el = connectedDetailsGlassRef.current;
      if (!el) return false;
      const rect = el.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) {
        showSecondaryGlass(connectedDetailsGlassRef, 'network-wifi-details', 12);
        return true;
      }
      return false;
    };
    
    if (!checkAndShow()) {
      requestAnimationFrame(() => {
        if (!checkAndShow()) {
          setTimeout(checkAndShow, 50);
        }
      });
    }
  }, [connectedDetailsOpen, connectedDetailsSsid, showSecondaryGlass, hideSecondaryGlass]);
  
  // 二级窗口毛玻璃效果：监听网络共享详情弹窗
  useEffect(() => {
    // 当从 true 变为 false 时，立即隐藏毛玻璃
    if (prevConnectedNetworkShareDetailsOpenRef.current && !connectedNetworkShareDetailsOpen) {
      hideSecondaryGlass('network-share-details');
    }
    prevConnectedNetworkShareDetailsOpenRef.current = connectedNetworkShareDetailsOpen;
    
    if (!connectedNetworkShareDetailsOpen || !connectedNetworkShareDevice) return;
    
    const checkAndShow = () => {
      const el = networkShareDetailsGlassRef.current;
      if (!el) return false;
      const rect = el.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) {
        showSecondaryGlass(networkShareDetailsGlassRef, 'network-share-details', 12);
        return true;
      }
      return false;
    };
    
    if (!checkAndShow()) {
      requestAnimationFrame(() => {
        if (!checkAndShow()) {
          setTimeout(checkAndShow, 50);
        }
      });
    }
  }, [connectedNetworkShareDetailsOpen, connectedNetworkShareDevice, showSecondaryGlass, hideSecondaryGlass]);

  const settlePendingNetworkShareDisconnectWait = useCallback((error?: Error) => {
    if (pendingNetworkShareDisconnectTimerRef.current != null) {
      window.clearTimeout(pendingNetworkShareDisconnectTimerRef.current);
      pendingNetworkShareDisconnectTimerRef.current = null;
    }

    const resolve = pendingNetworkShareDisconnectResolveRef.current;
    const reject = pendingNetworkShareDisconnectRejectRef.current;

    pendingNetworkShareDisconnectDeviceIdRef.current = null;
    pendingNetworkShareDisconnectResolveRef.current = null;
    pendingNetworkShareDisconnectRejectRef.current = null;

    if (error) {
      reject?.(error);
      return;
    }

    resolve?.();
  }, []);

  const requestDisconnectNetworkShareDevice = useCallback(async (device: NetworkShareDevice) => {
    if (!device?.deviceId) return;

    try {
      await tauriInvoke("system_disconnect_network_share_device", {
        deviceId: device.deviceId,
        deviceName: device.deviceName,
      });
    } catch {
    }
  }, []);

  const waitForNetworkShareDeviceDisconnected = useCallback((deviceId: string) => {
    const target = networkShareDevicesRef.current.find((device) => device.deviceId === deviceId);
    if (!target || !target.connected) {
      return Promise.resolve();
    }

    settlePendingNetworkShareDisconnectWait(new Error("Superseded network share disconnect wait"));

    return new Promise<void>((resolve, reject) => {
      pendingNetworkShareDisconnectDeviceIdRef.current = deviceId;
      pendingNetworkShareDisconnectResolveRef.current = resolve;
      pendingNetworkShareDisconnectRejectRef.current = reject;
      pendingNetworkShareDisconnectTimerRef.current = window.setTimeout(() => {
        settlePendingNetworkShareDisconnectWait(new Error("Timed out waiting for network share device to disconnect"));
      }, 15000);
    });
  }, [settlePendingNetworkShareDisconnectWait]);

  const connectNetworkShareDevice = useCallback(async (device: NetworkShareDevice) => {
    if (!device?.deviceId || connectingNetworkShareDeviceId) return;

    const connectedDevice = networkShareDevices.find(
      (item) => item.connected && item.deviceId !== device.deviceId
    );

    try {
      clearNetworkShareConnectTimer();
      setConnectingNetworkShareDeviceId(device.deviceId);

      if (connectedDevice) {
        await requestDisconnectNetworkShareDevice(connectedDevice);
        await waitForNetworkShareDeviceDisconnected(connectedDevice.deviceId);
      }

      await tauriInvoke("system_connect_network_share_device", {
        deviceId: device.deviceId,
        deviceName: device.deviceName,
      });
      void fetchNetworkShareDevices();
      networkShareConnectTimerRef.current = window.setTimeout(() => {
        networkShareConnectTimerRef.current = null;
        setConnectingNetworkShareDeviceId(null);
      }, 15000);
    } catch {
      clearNetworkShareConnectTimer();
      setConnectingNetworkShareDeviceId(null);
    }
  }, [clearNetworkShareConnectTimer, connectingNetworkShareDeviceId, fetchNetworkShareDevices, networkShareDevices, requestDisconnectNetworkShareDevice, waitForNetworkShareDeviceDisconnected]);

  const openConnectedNetworkShareDetails = useCallback((device: NetworkShareDevice) => {
    if (!device?.deviceId) return;
    setConnectedNetworkShareDevice(device);
    setConnectedNetworkShareDetailsOpen(true);
  }, []);

  const disconnectNetworkShareDevice = useCallback(async () => {
    const device = connectedNetworkShareDevice;
    if (!device?.deviceId) return;

    setConnectedNetworkShareDetailsOpen(false);
    setConnectedNetworkShareDevice(null);

    await requestDisconnectNetworkShareDevice(device);
  }, [connectedNetworkShareDevice, requestDisconnectNetworkShareDevice]);

  const disconnectConnectedNetworkShareForWifi = useCallback(async () => {
    const connectedDevice = networkShareDevicesRef.current.find((device) => device.connected);
    if (!connectedDevice) return;

    await requestDisconnectNetworkShareDevice(connectedDevice);
    await waitForNetworkShareDeviceDisconnected(connectedDevice.deviceId);
  }, [requestDisconnectNetworkShareDevice, waitForNetworkShareDeviceDisconnected]);

  useEffect(() => {
    if (!connectingNetworkShareDeviceId) return;

    const target = networkShareDevices.find((device) => device.deviceId === connectingNetworkShareDeviceId);
    if (target?.connected) {
      clearNetworkShareConnectTimer();
      setConnectingNetworkShareDeviceId(null);
    }
  }, [clearNetworkShareConnectTimer, connectingNetworkShareDeviceId, networkShareDevices]);

  useEffect(() => {
    const pendingDeviceId = pendingNetworkShareDisconnectDeviceIdRef.current;
    if (!pendingDeviceId) return;

    const target = networkShareDevices.find((device) => device.deviceId === pendingDeviceId);
    if (!target || !target.connected) {
      settlePendingNetworkShareDisconnectWait();
    }
  }, [networkShareDevices, settlePendingNetworkShareDisconnectWait]);

  const clearWlanFollowupTimer = useCallback(() => {
    if (wlanFollowupTimerRef.current != null) {
      window.clearTimeout(wlanFollowupTimerRef.current);
      wlanFollowupTimerRef.current = null;
    }
  }, []);

  const clearStableWifiRetryTimer = useCallback(() => {
    if (stableWifiRetryTimerRef.current != null) {
      window.clearTimeout(stableWifiRetryTimerRef.current);
      stableWifiRetryTimerRef.current = null;
    }
  }, []);

  const setWlanEnabledValue = useCallback((value: boolean | null) => {
    wlanEnabledRef.current = value;
    setWlanEnabled(value);
  }, []);

  const setAwaitingStableWifiListValue = useCallback((value: boolean) => {
    awaitingStableWifiListRef.current = value;
    setAwaitingStableWifiList(value);
  }, []);

  const setNetworksValue = useCallback((value: WifiNetwork[]) => {
    networksRef.current = value;
    setNetworks(value);
  }, []);

  const applyNetworkList = useCallback((list: WifiNetwork[]) => {
    const sorted = sortWifiNetworks(list);
    const previous = networksRef.current;

    const shouldIgnoreTransientConnectedOnly =
      popupOpenRef.current
      && previous.length > 1
      && !isConnectedOnlySnapshot(previous)
      && isConnectedOnlySnapshot(sorted);

    if (awaitingStableWifiListRef.current && wlanEnabledRef.current) {
      const waitingForFullList =
        isConnectedOnlySnapshot(sorted) && Date.now() < stableWifiDeadlineRef.current;
      if (waitingForFullList) {
        if (stableWifiRetryTimerRef.current == null) {
          stableWifiRetryTimerRef.current = window.setTimeout(() => {
            stableWifiRetryTimerRef.current = null;
            void fetchNetworksRef.current?.();
          }, 250);
        }
        return;
      }

      clearStableWifiRetryTimer();
      setAwaitingStableWifiListValue(false);
    }

    if (shouldIgnoreTransientConnectedOnly) {
      if (stableWifiRetryTimerRef.current == null) {
        stableWifiRetryTimerRef.current = window.setTimeout(() => {
          stableWifiRetryTimerRef.current = null;
          void fetchNetworksRef.current?.();
        }, 250);
      }
      return;
    }

    setNetworksValue(sorted);
  }, [clearStableWifiRetryTimer, setAwaitingStableWifiListValue, setNetworksValue]);

  const fetchNetworks = useCallback(async () => {
    const requestId = ++latestFetchRequestId.current;
    setRefreshing(true);
    try {
      const list = await invoke(FuncCommand.SystemGetWifiNetworks, undefined);
      if (requestId !== latestFetchRequestId.current) {
        return;
      }
      if (Array.isArray(list)) {
        setLocationPermissionRestricted(false);
        applyNetworkList(list as WifiNetwork[]);
      }
    } catch (error) {
      if (requestId !== latestFetchRequestId.current) {
        return;
      }
      if (isWlanLocationPermissionError(error)) {
        setLocationPermissionRestricted(true);
        setNetworksValue([]);
      } else {
        setLocationPermissionRestricted(false);
      }
    } finally {
      if (requestId === latestFetchRequestId.current) {
        setRefreshing(false);
      }
    }
  }, [applyNetworkList, setNetworksValue]);

  const openLocationSettings = useCallback(async () => {
    try {
      await invoke(FuncCommand.SystemOpenLocationSettings as any, undefined as any);
    } catch {
      console.warn("[Network] Failed to open location settings");
    }
  }, []);

  useEffect(() => {
    const hasConnectedNetworkShareDevice = networkShareDevices.some((device) => device.connected);
    if (hadConnectedNetworkShareDeviceRef.current && !hasConnectedNetworkShareDevice) {
      void fetchNetworks();
    }

    hadConnectedNetworkShareDeviceRef.current = hasConnectedNetworkShareDevice;
  }, [fetchNetworks, networkShareDevices]);

  fetchNetworksRef.current = fetchNetworks;

  const fetchWlanState = useCallback(async (toggleVersion?: number) => {
    const requestId = ++latestWlanStateRequestId.current;
    try {
      const enabled = await invoke(FuncCommand.SystemGetWlanEnabled, undefined as any);
      if (requestId !== latestWlanStateRequestId.current) {
        return;
      }
      if (toggleVersion != null && toggleVersion !== wlanToggleVersionRef.current) {
        return;
      }

      const nextEnabled = Boolean(enabled);
      const pendingTarget = pendingWlanTargetRef.current;
      if (pendingTarget != null && nextEnabled !== pendingTarget) {
        return;
      }

      if (pendingTarget != null && nextEnabled === pendingTarget) {
        pendingWlanTargetRef.current = null;
      }

      setWlanEnabledValue(nextEnabled);
    } catch {
      if (requestId !== latestWlanStateRequestId.current) {
        return;
      }
      if (toggleVersion != null && toggleVersion !== wlanToggleVersionRef.current) {
        return;
      }
      setWlanEnabledValue(null);
    }
  }, [setWlanEnabledValue]);

  const setWlanState = useCallback(
    async (nextEnabled: boolean) => {
      if (wlanSetting || wlanEnabled == null) return;

      const toggleVersion = ++wlanToggleVersionRef.current;
      const prev = wlanEnabled;
      pendingWlanTargetRef.current = nextEnabled;
      clearWlanFollowupTimer();
      clearStableWifiRetryTimer();
      if (nextEnabled) {
        stableWifiDeadlineRef.current = Date.now() + 1800;
        setAwaitingStableWifiListValue(true);
        setNetworksValue([]);
      } else {
        stableWifiDeadlineRef.current = 0;
        setAwaitingStableWifiListValue(false);
        setNetworksValue([]);
      }
      setWlanSetting(true);
      setWlanEnabledValue(nextEnabled);
      try {
        await invoke(FuncCommand.SystemSetWlanEnabled, { enabled: nextEnabled } as any);
        if (toggleVersion !== wlanToggleVersionRef.current) {
          return;
        }
        void fetchNetworks();
        // OS radio/scan transitions are async; delay the follow-up refresh to avoid
        // overwriting a fuller list with an intermediate connected-only snapshot.
        wlanFollowupTimerRef.current = window.setTimeout(() => {
          if (toggleVersion !== wlanToggleVersionRef.current) {
            return;
          }
          void fetchNetworks();
          void fetchWlanState(toggleVersion);
          wlanFollowupTimerRef.current = null;
        }, nextEnabled ? 900 : 350);
      } catch {
        if (toggleVersion === wlanToggleVersionRef.current) {
          pendingWlanTargetRef.current = null;
          clearStableWifiRetryTimer();
          setAwaitingStableWifiListValue(false);
          setWlanEnabledValue(prev);
        }
      } finally {
        if (toggleVersion === wlanToggleVersionRef.current) {
          setWlanSetting(false);
        }
      }
    },
    [
      clearStableWifiRetryTimer,
      clearWlanFollowupTimer,
      fetchNetworks,
      fetchWlanState,
      setAwaitingStableWifiListValue,
      setNetworksValue,
      setWlanEnabledValue,
      wlanEnabled,
      wlanSetting,
    ]
  );

  useEffect(() => {
    popupOpenRef.current = popupOpen;
    if (!popupOpen) {
      glassUpdateSeqRef.current++;
    }
  }, [popupOpen]);

  useEffect(() => {
    networksRef.current = networks;
  }, [networks]);

  useEffect(() => {
    return () => {
      clearWlanFollowupTimer();
      clearStableWifiRetryTimer();
      clearNetworkShareConnectTimer();
      settlePendingNetworkShareDisconnectWait(new Error("Network share disconnect wait cancelled"));
    };
  }, [clearNetworkShareConnectTimer, clearStableWifiRetryTimer, clearWlanFollowupTimer, settlePendingNetworkShareDisconnectWait]);

  useEffect(() => {
    void fetchNetworks();
    let unsub: (() => void) | null = null;
    let disposed = false;
    subscribe(FuncEvent.SystemNetworksChanged, (e) => {
      try {
        const payload = (e as any).payload;
        if (Array.isArray(payload)) {
          applyNetworkList(payload as WifiNetwork[]);
        }
      } catch {}
    })
      .then((u) => {
        if (disposed) {
          try { u(); } catch {}
          return;
        }
        unsub = u;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      if (unsub) {
        try { unsub(); } catch {}
      }
    };
  }, [applyNetworkList, fetchNetworks]);

  useEffect(() => {
    void fetchNetworkShareDevices();

    let unlisten: (() => void) | null = null;
    let disposed = false;

    listen<NetworkShareDevice[]>("system::network-share-devices-changed", (event) => {
      if (Array.isArray(event.payload)) {
        applyNetworkShareDevices(event.payload);
      }
    })
      .then((unsubscribe) => {
        if (disposed) {
          try { unsubscribe(); } catch {}
          return;
        }
        unlisten = unsubscribe;
      })
      .catch(() => {});

    return () => {
      disposed = true;
      if (unlisten) {
        try { unlisten(); } catch {}
      }
    };
  }, [applyNetworkShareDevices, fetchNetworkShareDevices]);

  useEffect(() => {
    $open_popups.value = { ...$open_popups.value, networkPopup: popupOpen };
  }, [popupOpen]);

  // 毛玻璃效果：监听弹窗打开（显示毛玻璃）
  useEffect(() => {
    if (!popupOpen) return;
    
    // 使用多次 requestAnimationFrame 确保定位完成
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
      requestAnimationFrame(() => {
        if (!checkAndShowGlass()) {
          requestAnimationFrame(() => {
            if (!checkAndShowGlass()) {
              setTimeout(checkAndShowGlass, 50);
            }
          });
        }
      });
    }
  }, [popupOpen, showPopupGlass]);

  // 毛玻璃效果：监听弹窗尺寸变化，动态更新模糊区域
  useEffect(() => {
    if (!popupOpen) return;
    
    const el = popupGlassRef.current;
    if (!el) return;
    
    let rafId: number | null = null;
    const resizeObserver = new ResizeObserver(() => {
      // 使用 requestAnimationFrame 节流
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

  // 毛玻璃效果：当关键状态变化时主动更新模糊区域
  // 这些状态变化会导致弹窗高度改变
  useEffect(() => {
    if (!popupOpen) return;
    // 使用 requestAnimationFrame 确保 DOM 已更新
    requestAnimationFrame(() => {
      showPopupGlass();
    });
  }, [
    popupOpen,
    wlanEnabled,           // WLAN 开关状态
    networks,              // WiFi 列表
    networkShareDevices,   // 网络共享设备列表
    awaitingStableWifiList, // 加载状态
    showPopupGlass
  ]);

  useEffect(() => {
    if (!popupOpen) return;

    // Immediate refresh on open, then poll every 5s while open.
    void fetchNetworks();
    void fetchNetworkShareDevices();

    const timer = window.setInterval(() => {
      void fetchNetworks();
    }, 5000);

    return () => {
      window.clearInterval(timer);
    };
  }, [popupOpen, fetchNetworkShareDevices, fetchNetworks]);

  const connectToNetwork = useCallback(
    async (n: WifiNetwork) => {
      if (!n?.ssid || connectingSsid || mutatingSsid) return;

      const hasConnectedNetworkShareDevice = networkShareDevicesRef.current.some((device) => device.connected);
      if (n.connected && !hasConnectedNetworkShareDevice) return;

      const ssid = n.ssid;
      const security = String((n as any).security ?? "").toLowerCase();
      const isOpen = security.includes("open") || security.includes("none");
      const isEnterprise = security.includes("enterprise") || security.includes("802.1x") || security.includes("8021x") || security.includes("eap") || security.includes("企业");

      // Scheme A: enterprise/802.1X networks are configured via system settings.
      if (isEnterprise) {
        // Keep behavior consistent with the "更多WLAN设置" entry.
        invoke(FuncCommand.SystemOpenWifiSettings as any, undefined as any).catch(() => {});
        closeNetworkPopup();
        setConnectingSsid(null);
        return;
      }

      // First try connecting using an existing/saved profile. If it works, don't ask for password.
      setConnectingSsid(ssid);

      if (hasConnectedNetworkShareDevice) {
        try {
          await disconnectConnectedNetworkShareForWifi();
        } catch {
          setConnectingSsid(null);
          return;
        }

        if (n.connected) {
          void fetchNetworks();
          setConnectingSsid(null);
          return;
        }
      }

      try {
        await invoke(FuncCommand.SystemConnectWifi, {
          args: {
            ssid,
            authorized: true,
          },
        } as any);

        void fetchNetworks();
        window.setTimeout(() => {
          void fetchNetworks();
        }, 800);
        return;
      } catch {
        // If it's an open network, we can also connect without prompting.
        if (isOpen) {
          try {
            await invoke(FuncCommand.SystemConnectWifi, {
              args: {
                ssid,
                authorized: false,
                authentication: "open",
                encryption: "none",
              },
            } as any);

            void fetchNetworks();
            window.setTimeout(() => {
              void fetchNetworks();
            }, 800);
            return;
          } catch {
            // fall through to dialog
          }
        }

        // Otherwise prompt for password / settings.
        setConnectingSsid(null);
        setConnectTarget(n);
        setConnectPassword("");
        setConnectShowPassword(false);
        setConnectAutoConnect(true);
        setConnectError(null);
        setConnectDialogOpen(true);
      }
    },
    [closeNetworkPopup, connectingSsid, disconnectConnectedNetworkShareForWifi, fetchNetworks, mutatingSsid]
  );

  const closeConnectDialog = useCallback(() => {
    setConnectDialogOpen(false);
    setConnectTarget(null);
    setConnectPassword("");
    setConnectShowPassword(false);
    setConnectAutoConnect(true);
    setConnectError(null);
    setConnectSubmitting(false);
  }, []);

  const submitConnectDialog = useCallback(async () => {
    const n = connectTarget;
    if (!n?.ssid || connectSubmitting) return;

    const ssid = n.ssid;
    setConnectSubmitting(true);
    setConnectError(null);
    setConnectingSsid(ssid);

    const security = String((n as any).security ?? "").toLowerCase();
    const isOpen = security.includes("open") || security.includes("none");

    if (networkShareDevicesRef.current.some((device) => device.connected)) {
      try {
        await disconnectConnectedNetworkShareForWifi();
      } catch {
        setConnectingSsid(null);
        setConnectSubmitting(false);
        return;
      }
    }

    try {
      // Always try saved/authorized profile first.
      try {
        await invoke(FuncCommand.SystemConnectWifi, {
          args: {
            ssid,
            authorized: true,
          },
        } as any);
      } catch {
        if (isOpen) {
          await invoke(FuncCommand.SystemConnectWifi, {
            args: {
              ssid,
              authorized: false,
              authentication: "open",
              encryption: "none",
            },
          } as any);
        } else {
          if (!connectPassword.trim()) {
            setConnectError(t('network.enter_password'));
            setConnectingSsid(null);
            return;
          }
          await invoke(FuncCommand.SystemConnectWifi, {
            args: {
              ssid,
              authorized: false,
              password: connectPassword,
              // Let backend infer auth/encryption when possible.
            },
          } as any);
        }
      }

      // Apply autoconnect preference (best-effort).
      try {
        await invoke(FuncCommand.SystemSetWifiAutoconnect, {
          profileName: ssid,
          enabled: connectAutoConnect,
        } as any);
      } catch {
        // ignore
      }

      void fetchNetworks();
      closeConnectDialog();
      // OS state transitions can be async; refresh again shortly.
      window.setTimeout(() => {
        void fetchNetworks();
      }, 800);
    } catch {
      // As requested previously: if connection doesn't succeed, show "密码错误".
      setConnectError(t('network.password_error'));
      setConnectingSsid(null);
    } finally {
      setConnectSubmitting(false);
    }
  }, [closeConnectDialog, connectAutoConnect, connectPassword, connectSubmitting, connectTarget, disconnectConnectedNetworkShareForWifi, fetchNetworks]);

  const openConnectedDetails = useCallback(
    async (n: WifiNetwork) => {
      if (!n?.ssid) return;
      setConnectedDetailsSsid(n.ssid);
      setConnectedDetailsOpen(true);
      setAutoConnect(null);
      try {
        const enabled = await invoke(FuncCommand.SystemGetWifiAutoconnect, { profileName: n.ssid } as any);
        setAutoConnect(Boolean(enabled));
      } catch {
        setAutoConnect(null);
      }
    },
    []
  );

  const setAutoConnectState = useCallback(
    async (next: boolean) => {
      const ssid = connectedDetailsSsid;
      if (!ssid || autoConnectSetting) return;

      const prev = autoConnect;
      setAutoConnectSetting(true);
      setAutoConnect(next);
      try {
        await invoke(FuncCommand.SystemSetWifiAutoconnect, { profileName: ssid, enabled: next } as any);
      } catch {
        setAutoConnect(prev ?? null);
      } finally {
        setAutoConnectSetting(false);
      }
    },
    [autoConnect, autoConnectSetting, connectedDetailsSsid]
  );

  const disconnectCurrent = useCallback(async () => {
    const ssid = connectedDetailsSsid;
    if (!ssid || mutatingSsid || connectingSsid) return;

    // UX: close immediately + switch row into spinner state.
    setMutatingSsid(ssid);
    setConnectedDetailsOpen(false);
    setConnectedDetailsSsid(null);
    setAutoConnect(null);

    try {
      await invoke(FuncCommand.SystemDisconnectWifi, undefined as any);
    } finally {
      await fetchNetworks();
      window.setTimeout(() => {
        void fetchNetworks();
      }, 800);
      setMutatingSsid(null);
    }
  }, [connectedDetailsSsid, connectingSsid, fetchNetworks, mutatingSsid]);

  const forgetCurrent = useCallback(async () => {
    const ssid = connectedDetailsSsid;
    if (!ssid || mutatingSsid || connectingSsid) return;

    // UX: close immediately + switch row into spinner state.
    setMutatingSsid(ssid);
    setConnectedDetailsOpen(false);
    setConnectedDetailsSsid(null);
    setAutoConnect(null);

    try {
      await invoke(FuncCommand.SystemForgetWifi, { profileName: ssid } as any);
    } finally {
      await fetchNetworks();
      window.setTimeout(() => {
        void fetchNetworks();
      }, 800);
      setMutatingSsid(null);
    }
  }, [connectedDetailsSsid, connectingSsid, fetchNetworks, mutatingSsid]);

  const current = networks.find(n => n.connected);
  const title = current ? current.ssid : t('network.not_connected');

  useEffect(() => {
    const ssid = current?.ssid ?? null;
    if (!ssid) {
      setCaptivePortalSsid(null);
      captiveCheckedSsid.current = null;
      return;
    }

    // Only check once per connected SSID.
    if (captiveCheckedSsid.current === ssid) return;
    captiveCheckedSsid.current = ssid;
    setCaptivePortalSsid(null);

    const seq = ++captiveCheckSeq.current;
    const timer = window.setTimeout(() => {
      (async () => {
        try {
          const isCaptive = await invoke(FuncCommand.SystemCheckCaptivePortal, undefined as any);
          if (captiveCheckSeq.current !== seq) return;
          setCaptivePortalSsid(isCaptive ? ssid : null);
        } catch {
          if (captiveCheckSeq.current !== seq) return;
          setCaptivePortalSsid(null);
        }
      })();
    }, 1200);

    return () => {
      window.clearTimeout(timer);
    };
  }, [current?.ssid]);

  // While captive portal is active, re-check periodically so the warning clears after user logs in.
  useEffect(() => {
    const ssid = current?.ssid ?? null;
    if (!ssid) return;
    if (captivePortalSsid !== ssid) return;

    let disposed = false;
    const interval = window.setInterval(() => {
      const seq = ++captiveCheckSeq.current;
      (async () => {
        try {
          const isCaptive = await invoke(FuncCommand.SystemCheckCaptivePortal, undefined as any);
          if (disposed) return;
          if (captiveCheckSeq.current !== seq) return;
          setCaptivePortalSsid(isCaptive ? ssid : null);
        } catch {
          // ignore
        }
      })();
    }, 8000);

    return () => {
      disposed = true;
      window.clearInterval(interval);
    };
  }, [current?.ssid, captivePortalSsid]);

  // 任务栏图标使用 img 标签（固定颜色）
  const getWifiIconSrc = (strength: number): string => {
    if (strength >= 71) return "/static/icons/WiFi3.svg";
    if (strength >= 31) return "/static/icons/WiFi2.svg";
    return "/static/icons/WiFi1.svg";
  };
  
  // 弹窗内列表图标需要支持深浅色模式，使用内联 SVG + currentColor
  const renderWifiListIcon = (strength: number) => {
    // 信号强度 >= 71: 三格全亮 (WiFi3)
    if (strength >= 71) {
      return (
        <svg className="wifi-icon" xmlns="http://www.w3.org/2000/svg" version="1.1" width="16" height="16" viewBox="0 0 20 20">
          <path d="M16.728940767547606,7.563206703417968C17.132524767547608,7.966791103417968,17.51312476754761,8.427314503417968,17.844464767547606,8.907396303417968C18.079746767547608,9.24829960341797,17.994123767547606,9.715390703417969,17.65322076754761,9.95067310341797C17.312317767547608,10.185955503417969,16.845226767547608,10.100332303417968,16.60994476754761,9.759429003417969C16.329020767547608,9.352395103417969,16.00620976754761,8.96179630341797,15.668279767547608,8.623867003417969C12.556661767547608,5.512249233417969,7.511733567547608,5.512249233417969,4.400115967547608,8.623867003417969C4.079130567547607,8.944852303417969,3.7595782675476075,9.335590803417968,3.4677425675476075,9.756262803417968C3.2316393675476074,10.096598103417968,2.7643436175476075,10.181094203417969,2.4240082775476073,9.94499110341797C2.0836729105476075,9.70888810341797,1.9991760875476074,9.241592903417969,2.2352792475476075,8.90125750341797C2.5755379175476074,8.41078520341797,2.9510878275476076,7.951574603417969,3.3394557675476073,7.563206703417968C7.036859967547607,3.865802523417969,13.031535767547608,3.865802523417969,16.728940767547606,7.563206703417968ZM14.589910767547607,9.45134070341797C15.062508767547607,9.923938703417969,15.470796767547608,10.496004603417969,15.781866767547607,11.104184103417968C15.970485767547608,11.472959503417968,15.824441767547608,11.924817603417969,15.455665767547607,12.11343760341797C15.086890767547608,12.302057703417969,14.635032767547607,12.156013503417968,14.446412767547608,11.78723810341797C14.204670767547608,11.31460280341797,13.887613767547608,10.870364203417969,13.529250767547607,10.512001003417968C11.598986167547608,8.581737003417969,8.469409967547607,8.581737003417969,6.539145967547608,10.512001003417968C6.164062967547608,10.88708350341797,5.859581967547607,11.308522203417969,5.625088167547608,11.77212050341797C5.438129467547608,12.141740803417969,4.986932967547608,12.289818303417968,4.617312467547608,12.10285950341797C4.247691867547608,11.915900703417968,4.0996150675476075,11.46470360341797,4.286573867547608,11.095083203417968C4.592513767547608,10.490235303417968,4.9906229675476075,9.939203303417969,5.478485567547608,9.45134070341797C7.994535967547607,6.935290103417969,12.073860667547608,6.935290103417969,14.589910767547607,9.45134070341797ZM12.954810767547608,11.84055420341797C13.318285767547607,12.20402910341797,13.616534767547607,12.649276703417968,13.824168767547608,13.123107903417969C13.990416767547607,13.502494803417969,13.817633767547607,13.944819403417968,13.438246767547607,14.111067803417969C13.058860767547607,14.277316103417968,12.616535767547607,14.104533203417969,12.450287767547607,13.725146303417969C12.315858767547608,13.418372203417968,12.122197767547608,13.129262003417969,11.894150267547607,12.90121460341797C10.866926667547608,11.873991003417968,9.201468967547608,11.873991003417968,8.174245867547608,12.90121460341797C7.9475989675476075,13.12786100341797,7.762874567547607,13.40496640341797,7.627239667547608,13.714946703417969C7.461195967547607,14.094423303417969,7.018964767547607,14.26744460341797,6.639488267547607,14.101401303417969C6.260011667547608,13.935358003417969,6.086990167547608,13.493126903417968,6.253033667547608,13.113650303417968C6.461537867547608,12.637134103417969,6.750544067547607,12.203596103417969,7.113585967547608,11.84055420341797C8.726595367547606,10.227544803417969,11.341801167547608,10.227544803417969,12.954810767547608,11.84055420341797ZM10.921279467547608,13.881700503417969C11.406425967547607,14.366847003417968,11.406425967547607,15.153424503417968,10.921279467547608,15.638570503417968C10.436132867547608,16.12371750341797,9.649555167547607,16.12371750341797,9.164408667547608,15.638570503417968C8.679262167547607,15.153424503417968,8.679262167547607,14.366847003417968,9.164408667547608,13.881700503417969C9.649555167547607,13.396554003417968,10.436132867547608,13.396554003417968,10.921279467547608,13.881700503417969Z" fill="currentColor" fillOpacity="0.9"/>
        </svg>
      );
    }
    // 信号强度 >= 31: 两格亮 (WiFi2)
    if (strength >= 31) {
      return (
        <svg className="wifi-icon" xmlns="http://www.w3.org/2000/svg" version="1.1" width="16" height="16" viewBox="0 0 20 20">
          <path d="M14.589910403076171,9.451340444458008C15.062509403076172,9.923938544458007,15.470796403076172,10.496004344458008,15.781866403076172,11.104183944458008C15.970485403076172,11.472959244458007,15.824442403076171,11.924817044458008,15.455665403076171,12.113437644458008C15.086891403076171,12.302057244458009,14.635032403076172,12.156013444458008,14.446413403076171,11.787238144458009C14.204669903076171,11.314602644458008,13.887613303076172,10.870363944458008,13.529251103076172,10.512000844458008C11.598986103076172,8.581736844458007,8.469409903076173,8.581736844458007,6.539145903076172,10.512000844458008C6.164063003076172,10.887083244458008,5.8595819030761715,11.308521944458008,5.625088203076172,11.772120444458007C5.4381294030761715,12.141740844458008,4.986932993076172,12.289817844458007,4.617312433076172,12.102859544458008C4.247691870076172,11.915900244458008,4.099615093076172,11.464703344458009,4.286573887076172,11.095083044458008C4.592513803076172,10.490235044458007,4.990622993076172,9.939203044458008,5.478485603076172,9.451340444458008C7.994535903076172,6.935289864458007,12.073860603076172,6.935289864458007,14.589910403076171,9.451340444458008ZM12.954811103076171,11.840554244458009C13.318285003076172,12.204029044458007,13.616535203076172,12.649276744458007,13.824169203076172,13.123107944458008C13.990417503076172,13.502494844458008,13.817634603076172,13.944819444458009,13.438247703076172,14.111067744458008C13.058860803076172,14.277316044458008,12.616535203076172,14.104533244458008,12.450286903076172,13.725146244458008C12.315858803076171,13.418372144458008,12.122197603076172,13.129261944458008,11.894150303076172,12.901214644458008C10.866926703076171,11.873991044458009,9.201468903076172,11.873991044458009,8.174245803076172,12.901214644458008C7.947598903076171,13.127861044458008,7.762874603076172,13.404966344458007,7.627239703076172,13.714946744458008C7.461195903076172,14.094423244458007,7.018964803076171,14.267444644458008,6.639488203076172,14.101401344458008C6.260011703076172,13.935358044458008,6.086990103076172,13.493126844458008,6.253033603076172,13.113650344458009C6.4615378030761725,12.637133644458007,6.750544103076171,12.203596144458007,7.113585903076172,11.840554244458009C8.726595403076171,10.227544544458008,11.341801203076173,10.227544544458008,12.954811103076171,11.840554244458009ZM10.921279403076172,13.881700544458008C11.406425903076173,14.366847044458009,11.406425903076173,15.153424244458009,10.921279403076172,15.638570744458008C10.436132903076171,16.123717344458008,9.649555203076172,16.123717344458008,9.164408703076173,15.638570744458008C8.67926220307617,15.153424244458009,8.67926220307617,14.366847044458009,9.164408703076173,13.881700544458008C9.649555203076172,13.396553944458008,10.436132903076171,13.396553944458008,10.921279403076172,13.881700544458008Z" fill="currentColor" fillOpacity="0.9"/>
          <path d="M2.23539798,8.901143062500001C1.99929482,9.2414784625,2.083791643,9.7087736625,2.42412701,9.9448766625C2.76446235,10.1809797625,3.2317581,10.096483662499999,3.4678613,9.7561483625C3.759697,9.3354763625,4.0792493,8.9447378625,4.4002347,8.6237525625C7.5118523,5.5121347925,12.5567805,5.5121347925,15.6683985,8.6237525625C16.006328500000002,8.9616818625,16.3291395,9.3522806625,16.610063500000003,9.7593145625C16.8453455,10.1002178625,17.3124365,10.1858410625,17.6533395,9.9505586625C17.9942425,9.7152762625,18.0798655,9.2481851625,17.8445835,8.9072818625C17.5132435,8.427200062499999,17.1326435,7.966676662499999,16.729059499999998,7.5630922625C13.0316545,3.8656880825,7.0369787,3.8656880825,3.3395745,7.5630922625C2.95120656,7.9514601625,2.57565665,8.4106707625,2.23539798,8.901143062500001Z" fill="currentColor" fillOpacity="0.3"/>
        </svg>
      );
    }
    // 信号强度 < 31: 只有一格亮 (WiFi1)
    return (
      <svg className="wifi-icon" xmlns="http://www.w3.org/2000/svg" version="1.1" width="16" height="16" viewBox="0 0 20 20">
        <path d="M12.95481065145874,11.840554286169434C13.31828545145874,12.204029086169434,13.61653475145874,12.649276686169433,13.824168651458741,13.123107886169434C13.99041705145874,13.502494786169434,13.81763415145874,13.944819486169433,13.43824725145874,14.111067786169434C13.05886035145874,14.277316086169433,12.61653565145874,14.104533186169434,12.45028735145874,13.725146286169434C12.31585835145874,13.418372186169433,12.122197651458741,13.129261986169434,11.89415025145874,12.901214586169434C10.86692665145874,11.873990986169433,9.20146895145874,11.873990986169433,8.17424585145874,12.901214586169434C7.94759895145874,13.127860986169434,7.76287465145874,13.404966386169434,7.62723975145874,13.714946786169433C7.46119595145874,14.094423286169434,7.01896477145874,14.267444586169434,6.63948822145874,14.101401286169434C6.26001167345874,13.935358086169433,6.08699012145874,13.493126886169433,6.25303363845874,13.113650286169435C6.46153784145874,12.637134086169434,6.75054407145874,12.203596086169433,7.11358595145874,11.840554286169434C8.72659545145874,10.227544786169434,11.34180115145874,10.227544786169434,12.95481065145874,11.840554286169434ZM10.92127945145874,13.881700486169434C11.40642595145874,14.366847086169434,11.40642595145874,15.153424286169432,10.92127945145874,15.638570786169433C10.43613295145874,16.123717286169434,9.64955525145874,16.123717286169434,9.16440865145874,15.638570786169433C8.67926215145874,15.153424286169432,8.67926215145874,14.366847086169434,9.16440865145874,13.881700486169434C9.64955525145874,13.396553986169433,10.43613295145874,13.396553986169433,10.92127945145874,13.881700486169434Z" fill="currentColor" fillOpacity="0.9"/>
        <path d="M2.23539798,8.901143062500001C1.99929482,9.2414784625,2.083791643,9.7087736625,2.42412701,9.9448766625C2.76446235,10.1809797625,3.2317581,10.096483662499999,3.4678613,9.7561483625C3.759697,9.3354763625,4.0792493,8.9447378625,4.4002347,8.6237525625C7.5118523,5.5121347925,12.5567805,5.5121347925,15.6683985,8.6237525625C16.006328500000002,8.9616818625,16.3291395,9.3522806625,16.610063500000003,9.7593145625C16.8453455,10.1002178625,17.3124365,10.1858410625,17.6533395,9.9505586625C17.9942425,9.7152762625,18.0798655,9.2481851625,17.8445835,8.9072818625C17.5132435,8.427200062499999,17.1326435,7.966676662499999,16.729059499999998,7.5630922625C13.0316545,3.8656880825,7.0369787,3.8656880825,3.3395745,7.5630922625C2.95120656,7.9514601625,2.57565665,8.4106707625,2.23539798,8.901143062500001ZM4.2866926,11.094968762499999C4.0997338,11.464589162500001,4.2478106,11.9157862625,4.6174312,12.1027450625C4.9870517,12.2897038625,5.4382482,12.1416263625,5.6252069,11.772006062500001C5.859700699999999,11.3084077625,6.1641817,10.8869690625,6.5392647,10.511886562499999C8.4695287,8.5816225625,11.5991049,8.5816225625,13.5293695,10.511886562499999C13.8877325,10.8702497625,14.2047895,11.3144883625,14.4465315,11.7871236625C14.6351515,12.1558990625,15.0870095,12.3019432625,15.4557845,12.1133231625C15.8245605,11.9247031625,15.9706045,11.4728450625,15.7819855,11.104069662499999C15.4709155,10.4958901625,15.0626275,9.9238242625,14.5900295,9.4512262625C12.0739794,6.9351756625,7.9946547,6.9351756625,5.478604300000001,9.4512262625C4.9907417,9.9390888625,4.592632500000001,10.4901208625,4.2866926,11.094968762499999Z" fill="currentColor" fillOpacity="0.3"/>
      </svg>
    );
  };
  
  // 弹窗头部 WiFi 图标（支持深浅色模式，使用 WiFi3 的 path）
  const renderHeaderWifiIcon = () => (
    <svg className="wifi-icon connected" xmlns="http://www.w3.org/2000/svg" version="1.1" width="24" height="24" viewBox="0 0 20 20">
      <path d="M16.728940767547606,7.563206703417968C17.132524767547608,7.966791103417968,17.51312476754761,8.427314503417968,17.844464767547606,8.907396303417968C18.079746767547608,9.24829960341797,17.994123767547606,9.715390703417969,17.65322076754761,9.95067310341797C17.312317767547608,10.185955503417969,16.845226767547608,10.100332303417968,16.60994476754761,9.759429003417969C16.329020767547608,9.352395103417969,16.00620976754761,8.96179630341797,15.668279767547608,8.623867003417969C12.556661767547608,5.512249233417969,7.511733567547608,5.512249233417969,4.400115967547608,8.623867003417969C4.079130567547607,8.944852303417969,3.7595782675476075,9.335590803417968,3.4677425675476075,9.756262803417968C3.2316393675476074,10.096598103417968,2.7643436175476075,10.181094203417969,2.4240082775476073,9.94499110341797C2.0836729105476075,9.70888810341797,1.9991760875476074,9.241592903417969,2.2352792475476075,8.90125750341797C2.5755379175476074,8.41078520341797,2.9510878275476076,7.951574603417969,3.3394557675476073,7.563206703417968C7.036859967547607,3.865802523417969,13.031535767547608,3.865802523417969,16.728940767547606,7.563206703417968ZM14.589910767547607,9.45134070341797C15.062508767547607,9.923938703417969,15.470796767547608,10.496004603417969,15.781866767547607,11.104184103417968C15.970485767547608,11.472959503417968,15.824441767547608,11.924817603417969,15.455665767547607,12.11343760341797C15.086890767547608,12.302057703417969,14.635032767547607,12.156013503417968,14.446412767547608,11.78723810341797C14.204670767547608,11.31460280341797,13.887613767547608,10.870364203417969,13.529250767547607,10.512001003417968C11.598986167547608,8.581737003417969,8.469409967547607,8.581737003417969,6.539145967547608,10.512001003417968C6.164062967547608,10.88708350341797,5.859581967547607,11.308522203417969,5.625088167547608,11.77212050341797C5.438129467547608,12.141740803417969,4.986932967547608,12.289818303417968,4.617312467547608,12.10285950341797C4.247691867547608,11.915900703417968,4.0996150675476075,11.46470360341797,4.286573867547608,11.095083203417968C4.592513767547608,10.490235303417968,4.9906229675476075,9.939203303417969,5.478485567547608,9.45134070341797C7.994535967547607,6.935290103417969,12.073860667547608,6.935290103417969,14.589910767547607,9.45134070341797ZM12.954810767547608,11.84055420341797C13.318285767547607,12.20402910341797,13.616534767547607,12.649276703417968,13.824168767547608,13.123107903417969C13.990416767547607,13.502494803417969,13.817633767547607,13.944819403417968,13.438246767547607,14.111067803417969C13.058860767547607,14.277316103417968,12.616535767547607,14.104533203417969,12.450287767547607,13.725146303417969C12.315858767547608,13.418372203417968,12.122197767547608,13.129262003417969,11.894150267547607,12.90121460341797C10.866926667547608,11.873991003417968,9.201468967547608,11.873991003417968,8.174245867547608,12.90121460341797C7.9475989675476075,13.12786100341797,7.762874567547607,13.40496640341797,7.627239667547608,13.714946703417969C7.461195967547607,14.094423303417969,7.018964767547607,14.26744460341797,6.639488267547607,14.101401303417969C6.260011667547608,13.935358003417969,6.086990167547608,13.493126903417968,6.253033667547608,13.113650303417968C6.461537867547608,12.637134103417969,6.750544067547607,12.203596103417969,7.113585967547608,11.84055420341797C8.726595367547606,10.227544803417969,11.341801167547608,10.227544803417969,12.954810767547608,11.84055420341797ZM10.921279467547608,13.881700503417969C11.406425967547607,14.366847003417968,11.406425967547607,15.153424503417968,10.921279467547608,15.638570503417968C10.436132867547608,16.12371750341797,9.649555167547607,16.12371750341797,9.164408667547608,15.638570503417968C8.679262167547607,15.153424503417968,8.679262167547607,14.366847003417968,9.164408667547608,13.881700503417969C9.649555167547607,13.396554003417968,10.436132867547608,13.396554003417968,10.921279467547608,13.881700503417969Z" fill="currentColor" fillOpacity="0.9"/>
    </svg>
  );

  const renderHeaderNetworkShareIcon = () => (
    <img
      src="/static/icons/superterminal.svg"
      alt={t('network.network_share')}
      className="network-connected-share-icon"
      draggable={false}
    />
  );
  
  const renderIcon = () => {
    if (networkShareDevices.some((device) => device.connected)) {
      return (
        <img
          src="/static/icons/NetShare.svg"
          className="wifi-icon connected"
          width="20"
          height="20"
          draggable={false}
        />
      );
    }

    if (!current) {
      return (
        <img
          src="/static/icons/WiFiOff.svg"
          alt={t('network.not_connected')}
          className="wifi-icon disconnected"
          width="20"
          height="20"
          draggable={false}
        />
      );
    }

    if (captivePortalSsid && captivePortalSsid === current.ssid) {
      return (
        <img
          src="/static/icons/WiFiWarning.svg"
          alt={t('network.login_required')}
          className="network-toolbar-warning-icon"
          draggable={false}
        />
      );
    }

    const strengthRaw = Number((current as any)?.signal);
    const strength = Number.isFinite(strengthRaw) ? Math.max(0, Math.min(100, strengthRaw)) : 0;
    const iconSrc = getWifiIconSrc(strength);

    return (
      <img
        src={iconSrc}
        alt={`WiFi signal ${strength}%`}
        className="wifi-icon connected"
        width="20"
        height="20"
        draggable={false}
      />
    );
  };

  const hasNetworkShareDevices = networkShareDevices.length > 0;
  const hasVisibleNetworkShareDevices = Boolean(wlanEnabled) && hasNetworkShareDevices;
  const hasConnectedNetworkShareDevice = networkShareDevices.some((device) => device.connected);
  const showLocationSettingsNotice = Boolean(wlanEnabled) && locationPermissionRestricted && networks.length === 0;
  const hasVisibleWifiSection = Boolean(wlanEnabled)
    && (showLocationSettingsNotice || (networks.length > 0 && (!awaitingStableWifiList || !isConnectedOnlySnapshot(networks))));
  const hasVisibleContentSection = hasVisibleNetworkShareDevices || hasVisibleWifiSection;

  const PopupContent = (
    <>
      {/* 主面板 */}
      <div className="network-popup" ref={popupGlassRef}>
        <div className="network-wlan-label">
          <div className="ssid" title="WLAN">WLAN</div>
        </div>
        <div className="network-wlan-switch-container">
          <label
            className={`network-wlan-switch ${wlanEnabled ? "on" : ""} ${wlanEnabled == null ? "disabled" : ""}`}
          >
            <input
              type="checkbox"
              checked={Boolean(wlanEnabled)}
              disabled={wlanEnabled == null || wlanSetting}
              onChange={(e) => {
                void setWlanState(e.currentTarget.checked);
              }}
            />
            <img
              className="network-wlan-switch-img"
              src={wlanEnabled ? "/static/icons/Switch.svg" : "/static/icons/SwitchBase.svg"}
              alt={t('network.wlan_switch')}
              draggable={false}
            />
          </label>
        </div>

        <div className="network-divider" />

        <div className="network-popup-header">
        </div>

        {hasVisibleNetworkShareDevices && (
          <div className="network-share-section">
            <div className="network-list-header">
              <div className="network-list-header-text">{t('network.network_share')}</div>
            </div>
            <div className="network-share-container">
              {networkShareDevices.map((device) => (
                (() => {
                  const isBusy = connectingNetworkShareDeviceId === device.deviceId;
                  return (
                <div
                  key={device.deviceId}
                  className={`network-share-row ${device.connected && !isBusy ? "is-connected" : ""}`}
                  onClick={() => {
                    if (isBusy) return;
                    if (device.connected) {
                      openConnectedNetworkShareDetails(device);
                      return;
                    }
                    void connectNetworkShareDevice(device);
                  }}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      if (isBusy) return;
                      if (device.connected) {
                        openConnectedNetworkShareDetails(device);
                        return;
                      }
                      void connectNetworkShareDevice(device);
                    }
                  }}
                >
                  <img
                    className="network-share-icon"
                    src="/static/icons/superterminal.svg"
                    draggable={false}
                  />
                  <NetworkShareNameCell deviceName={device.deviceName} t={t} />
                  <div className="network-share-status">
                    {isBusy && (
                      <span className="network-wifi-spinner" aria-label={t('network.connecting')} role="img">
                        <span className="network-wifi-spinner-dot dot-1" aria-hidden="true" />
                        <span className="network-wifi-spinner-dot dot-2" aria-hidden="true" />
                        <span className="network-wifi-spinner-dot dot-3" aria-hidden="true" />
                        <span className="network-wifi-spinner-dot dot-4" aria-hidden="true" />
                        <span className="network-wifi-spinner-dot dot-5" aria-hidden="true" />
                        <span className="network-wifi-spinner-dot dot-6" aria-hidden="true" />
                        <span className="network-wifi-spinner-dot dot-7" aria-hidden="true" />
                        <span className="network-wifi-spinner-dot dot-8" aria-hidden="true" />
                        <span className="network-wifi-spinner-dot dot-9" aria-hidden="true" />
                        <span className="network-wifi-spinner-dot dot-10" aria-hidden="true" />
                        <span className="network-wifi-spinner-dot dot-11" aria-hidden="true" />
                        <span className="network-wifi-spinner-dot dot-12" aria-hidden="true" />
                      </span>
                    )}
                    {device.connected && !isBusy && (
                      <img src="/static/icons/confirm.svg" alt="connected" style={{ width: "16px", height: "16px" }} />
                    )}
                  </div>
                </div>
                  );
                })()
              ))}
            </div>
          </div>
        )}

        {hasVisibleWifiSection && (
          <>
            {hasVisibleNetworkShareDevices && <div className="network-section-divider" />}
            <div className="network-list-header">
              <div className="network-list-header-text">{t('network.available_devices')}</div>
            </div>
            <div className="network-section">
              {showLocationSettingsNotice ? (
                <div className="network-location-notice-row">
                  <span className="network-location-notice-text">{t('network.location_notice')}</span>
                  <button
                    type="button"
                    className="network-location-notice-link"
                    onClick={() => {
                      void openLocationSettings();
                    }}
                  >
                    {t('network.go_settings')}
                  </button>
                </div>
              ) : (
              <div className="network-list" ref={networkListRef}>
                {networks.map((n, idx) => {
                  const strengthRaw = Number((n as any)?.signal);
                  const strength = Number.isFinite(strengthRaw) ? Math.max(0, Math.min(100, strengthRaw)) : 0;
                  const isBusy = connectingSsid === n.ssid || mutatingSsid === n.ssid;
                  const isConnectedVisible = n.connected && !isBusy && !hasConnectedNetworkShareDevice;
                  
                  return (
                    <div
                      key={`${n.ssid}-${idx}`}
                      className={`network-wifi-row ${isConnectedVisible ? "is-connected" : ""}`}
                      onClick={() => {
                        if (isConnectedVisible) {
                          void openConnectedDetails(n);
                          return;
                        }
                        void connectToNetwork(n);
                      }}
                      role="button"
                      tabIndex={0}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          if (isConnectedVisible) {
                            void openConnectedDetails(n);
                            return;
                          }
                          void connectToNetwork(n);
                        }
                      }}
                      aria-disabled={!!connectingSsid || !!mutatingSsid}
                    >
                      <div className="network-wifi-icon">
                        {renderWifiListIcon(strength)}
                      </div>
                      <WifiNameCell ssid={n.ssid} />
                      <div className="network-wifi-status">
                        {isBusy && (
                          <span className="network-wifi-spinner" aria-label={t('network.connecting')} role="img">
                            <span className="network-wifi-spinner-dot dot-1" aria-hidden="true" />
                            <span className="network-wifi-spinner-dot dot-2" aria-hidden="true" />
                            <span className="network-wifi-spinner-dot dot-3" aria-hidden="true" />
                            <span className="network-wifi-spinner-dot dot-4" aria-hidden="true" />
                            <span className="network-wifi-spinner-dot dot-5" aria-hidden="true" />
                            <span className="network-wifi-spinner-dot dot-6" aria-hidden="true" />
                            <span className="network-wifi-spinner-dot dot-7" aria-hidden="true" />
                            <span className="network-wifi-spinner-dot dot-8" aria-hidden="true" />
                            <span className="network-wifi-spinner-dot dot-9" aria-hidden="true" />
                            <span className="network-wifi-spinner-dot dot-10" aria-hidden="true" />
                            <span className="network-wifi-spinner-dot dot-11" aria-hidden="true" />
                            <span className="network-wifi-spinner-dot dot-12" aria-hidden="true" />
                          </span>
                        )}
                        {isConnectedVisible && (
                          <img src="/static/icons/confirm.svg" alt="connected" style={{ width: '16px', height: '16px' }} />
                        )}
                      </div>
                    </div>
                  );
                })}
            </div>
              )}
          </div>
          </>
        )}

        {hasVisibleContentSection && <div className="network-more-settings-divider" />}

        <div className="network-more-settings-row" role="button" tabIndex={0} onClick={(e) => { e.preventDefault(); e.stopPropagation(); invoke(FuncCommand.SystemOpenWifiSettings, undefined as any).catch(() => {}); closeNetworkPopup(); }} onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); e.stopPropagation(); invoke(FuncCommand.SystemOpenWifiSettings, undefined as any).catch(() => {}); closeNetworkPopup(); } }}>
          <div className="network-more-settings-text">{t('network.more_settings')}</div>
        </div>
      </div>

      {/* 已连接网络详情弹窗 */}
      {connectedDetailsOpen && connectedDetailsSsid && (
        <div className="network-status-overlay">
          <div className="network-connected-dialog" ref={connectedDetailsGlassRef}>
            <div className="network-connected-info">
              <div className="network-connected-icon">{renderHeaderWifiIcon()}</div>
              <div className="network-connected-name" title={connectedDetailsSsid ?? undefined}>{connectedDetailsSsid}</div>
            </div>

            <div className="network-connected-autoconnect">
              <div className="network-connected-autoconnect-text">{t('network.auto_connect')}</div>
              <button
                className="network-connected-autoconnect-switch"
                onClick={() => void setAutoConnectState(!autoConnect)}
                disabled={autoConnectSetting}
                title={autoConnect ? t('network.enabled') : t('network.disabled')}
              >
                {autoConnect ? (
                  <img src="/static/icons/Switch.svg" alt={t('network.auto_connect')} style={{ width: '40px', height: '20px' }} />
                ) : (
                  <img src="/static/icons/SwitchBase.svg" alt={t('network.auto_connect')} style={{ width: '40px', height: '20px' }} />
                )}
              </button>
            </div>

            <div className="network-connected-divider" />

            <div className="network-connected-status">
              <div className="network-connected-status-label">{t('network.status_message')}</div>
              {captivePortalSsid && captivePortalSsid === connectedDetailsSsid ? (
                <button
                  type="button"
                  className="network-connected-portal-link"
                  onClick={() => {
                    // Use a plain HTTP URL so portals can intercept.
                    invoke(FuncCommand.OpenUrl, { url: "http://www.msftconnecttest.com/redirect" } as any).catch(() => {});
                  }}
                >
                  {t('network.open_browser')}
                </button>
              ) : (
                <div className="network-connected-status-value">{t('network.connected')}</div>
              )}
            </div>

            <div className="network-detail-actions">
              <button className="network-connected-btn" onClick={() => {
                setConnectedDetailsOpen(false);
                setConnectedDetailsSsid(null);
                setAutoConnect(null);
              }}><span className="network-connected-btn-text">{t('network.cancel')}</span></button>
              <button className="network-connected-btn" onClick={() => void forgetCurrent()}><span className="network-connected-btn-text">{t('network.forget')}</span></button>
              <button className="network-connected-btn" onClick={() => void disconnectCurrent()}><span className="network-connected-btn-text">{t('network.disconnect')}</span></button>
            </div>
          </div>
        </div>
      )}

      {connectedNetworkShareDetailsOpen && connectedNetworkShareDevice && (
        <div className="network-status-overlay">
          <div className="network-connected-dialog network-share-connected-dialog" ref={networkShareDetailsGlassRef}>
            <div className="network-connected-info">
              <div className="network-connected-icon">{renderHeaderNetworkShareIcon()}</div>
              <div className="network-connected-name" title={connectedNetworkShareDevice.deviceName}>{connectedNetworkShareDevice.deviceName}</div>
            </div>

            <div className="network-connected-status">
              <div className="network-connected-status-label">{t('network.status_message')}</div>
              <div className="network-connected-status-value">{t('network.connected')}</div>
            </div>

            <div className="network-detail-actions">
              <button className="network-connected-btn" onClick={() => {
                setConnectedNetworkShareDetailsOpen(false);
                setConnectedNetworkShareDevice(null);
              }}><span className="network-connected-btn-text">{t('network.cancel')}</span></button>
              <button className="network-connected-btn" onClick={() => void disconnectNetworkShareDevice()}><span className="network-connected-btn-text">{t('network.disconnect')}</span></button>
            </div>
          </div>
        </div>
      )}

      {/* 密码框居中 Overlay */}
      {connectDialogOpen && connectTarget && (
        <div className="network-password-overlay">
          <div className="network-password-dialog" ref={passwordDialogGlassRef}>
            {/* WiFi 图标和名称 */}
            <div className="network-password-header">
              <div className="network-password-icon">{renderHeaderWifiIcon()}</div>
              <div className="network-password-title" title={connectTarget.ssid}>{connectTarget.ssid}</div>
            </div>

            {/* 密码框 */}
            {(() => {
              const security = String((connectTarget as any).security ?? "").toLowerCase();
              const isOpen = security.includes("open") || security.includes("none");
              if (isOpen) return null;
              return (
                <div className="network-password-input-wrapper">
                  <div className="network-password-input-inner">
                    <input
                      className="network-password-input"
                      type={connectShowPassword ? "text" : "password"}
                      placeholder={t('network.password')}
                      value={connectPassword}
                      onChange={(e) => {
                        setConnectPassword(e.currentTarget.value);
                        if (connectError) setConnectError(null);
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          void submitConnectDialog();
                        }
                      }}
                    />
                  </div>
                  <button
                    className="network-password-eye-icon"
                    type="button"
                    onClick={() => setConnectShowPassword(v => !v)}
                    aria-label={connectShowPassword ? t('network.hide_password') : t('network.show_password')}
                  >
                    <img src={connectShowPassword ? "/static/icons/eyeopen.svg" : "/static/icons/eyeconfortclosed.svg"} alt="eye" />
                  </button>
                </div>
              );
            })()}

            {/* 自动连接 */}
            <div className="network-autoconnect-wrapper">
              <div 
                className="network-autoconnect-checkbox"
                onClick={() => setConnectAutoConnect(v => !v)}
              >
                {connectAutoConnect ? (
                  <img src="/static/icons/Select.svg" alt={t('network.auto_connect')} style={{ width: '16px', height: '16px' }} />
                ) : (
                  <div className="network-autoconnect-unchecked" />
                )}
              </div>
              <label style={{ cursor: 'pointer' }} onClick={() => setConnectAutoConnect(v => !v)} className="network-autoconnect-label">{t('network.auto_connect')}</label>
              {connectError ? (
                <div className="network-autoconnect-error" aria-live="polite">{connectError}</div>
              ) : null}
            </div>

            {/* 按钮 */}
            <div className="network-password-buttons">
              <button className="network-password-cancel-btn" onClick={closeConnectDialog} disabled={connectSubmitting}>
                {t('network.cancel')}
              </button>
              <button className="network-password-connect-btn" onClick={() => void submitConnectDialog()} disabled={connectSubmitting}>
                {connectSubmitting ? t('network.connecting') : t('network.connect')}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );

  useEffect(() => {
    if (!connectingSsid) return;
    const matched = networks.find(n => n.ssid === connectingSsid);
    if (matched?.connected) {
      setConnectingSsid(null);
      return;
    }
    const t = window.setTimeout(() => setConnectingSsid(null), 6000);
    return () => window.clearTimeout(t);
  }, [connectingSsid, networks]);

  // 处理弹窗打开/关闭事件 - 直接绑定毛玻璃生命周期
  const handleOpenChange = useCallback((open: boolean) => {
    popupOpenRef.current = open;

    if (open) {
      setPopupOpen(true);
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          const el = networkListRef.current;
          if (el) el.scrollTop = 0;
        });
      });
      fetchWlanState();
      (invoke as any)(FuncCommand.ReportClickComponent, { content: "WIFI" })
        .catch((err: any) => console.error('report click failed', err));
    } else {
      // 先隐藏所有毛玻璃，再关闭弹窗
      closeNetworkPopup();
      // 关闭所有二级窗口
      setConnectedDetailsOpen(false);
      setConnectedDetailsSsid(null);
      setAutoConnect(null);
      setConnectDialogOpen(false);
      setConnectTarget(null);
      setConnectPassword("");
      setConnectShowPassword(false);
      setConnectAutoConnect(true);
      setConnectError(null);
      setConnectSubmitting(false);
      setConnectedNetworkShareDetailsOpen(false);
      setConnectedNetworkShareDevice(null);
    }
  }, [closeNetworkPopup, fetchWlanState]);

  return (
    <SlPopup
      placement="top"
      offset={4}
      align="start"
      content={PopupContent}
      open={popupOpen}
      onOpenChange={handleOpenChange}
    >
      <div className={`taskbar-item taskbar-module network-status${popupOpen ? ' selected' : ''}`}>
        {renderIcon()}
      </div>
    </SlPopup>
  );
}
