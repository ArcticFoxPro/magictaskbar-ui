import { useEffect, useState, useCallback, useMemo, useRef } from "react";
import { invoke, FuncCommand, FuncEvent, subscribe } from "@magic-ui/lib";
import { SlPopup } from "@shared/components/SlPopup";
import { useTranslation } from "react-i18next";
import "./styles.css";

const BT_UI_DEBUG = false;

// 毛玻璃效果相关常量
const POPUP_CORNER_RADIUS = 9;

function btLogError(...args: any[]) {
  if (!BT_UI_DEBUG) return;
  console.error(...args);
}

/* ==================== 类型定义 ==================== */

type PairingNeeds =
  | "DisplayPin"
  | "ConfirmPinMatch"
  | "ConfirmOnly"
  | "ProvidePin"
  | "ProvidePasswordCredential";

interface PairingAction {
  deviceId: string;
  needs: PairingNeeds;
  pin?: string;
}

/**
 * SDK 返回的真实联合类型（使用 serde tag 模式）
 */
type RawPairingAction =
  | { needs: "None" }
  | { needs: "ConfirmOnly" }
  | { needs: "ProvidePin" }
  | { needs: "ProvidePasswordCredential" }
  | { needs: "DisplayPin"; pin: string }
  | { needs: "ConfirmPinMatch"; pin: string };

/* ==================== 设备图标获取 ==================== */

/**
 * 根据设备类型和外观返回对应的图标路径
 */
function getDeviceIcon(device: any): string {
  const { major_class, minor_class, appearance, major_service_classes } = device;
  
  // 对于 LE 设备，优先使用 appearance 值
  if (device.is_low_energy && appearance) {
    // Appearance 的高 10 位是 category，低 6 位是 subcategory
    const category = appearance >> 6;
    const subCategory = appearance & 0x3F; // 低 6 位
    
    // 根据 BLE Appearance 分类（Bluetooth SIG Assigned Numbers）
    switch (category) {
      case 0x01: // Phone
        return "/static/icons/phone.svg";
      case 0x02: // Computer
        return "/static/icons/matebook.svg";
      case 0x03: // Watch
        return "/static/icons/double_watch.svg";
      case 0x05: // Display
        return "/static/icons/matebook.svg";
      case 0x07: // Tablet (Remote Control 在规范中是 0x07，Tablet 没有独立 category)
        return "/static/icons/pad.svg";
      case 0x0F: // HID (Human Interface Device)
        // subCategory: 1=Keyboard, 2=Mouse, 3=Joystick, 4=Gamepad
        if (subCategory === 1) {
          return "/static/icons/keyboard.svg";
        }
        if (subCategory === 2) {
          return "/static/icons/mouse.svg";
        }
        // Joystick/Gamepad/其他 HID 设备使用鼠标图标
        return "/static/icons/mouse.svg";
      case 0x10: // Headset/Headphones (0x10 = Audio/Video)
        return "/static/icons/earphone.svg";
      default:
        break;
    }
  }
  
  // 对于 Classic 设备，使用 major_class 和 minor_class
  if (major_class === "Computer") {
    return "/static/icons/matebook.svg";
  }
  
  if (major_class === "Phone") {
    return "/static/icons/phone.svg";
  }
  
  if (major_class === "AudioVideo") {
    // 检查 minor_class 或 major_service_classes
    if (minor_class?.AudioVideo) {
      const audioType = minor_class.AudioVideo;
      if (["Headset", "HandsFree", "Headphones", "Loudspeaker", "PortableAudio"].includes(audioType)) {
        return "/static/icons/earphone.svg";
      }
      if (audioType === "Microphone") {
        return "/static/icons/voice.svg";
      }
    }
    // 默认音频设备使用耳机图标
    return "/static/icons/earphone.svg";
  }
  
  if (major_class === "Peripheral") {
    if (minor_class?.Peripheral) {
      const peripheralType = minor_class.Peripheral[0]; // [type, subtype]
      if (peripheralType === "Keyboard" || peripheralType === "ComboKeyboardPointing") {
        return "/static/icons/keyboard.svg";
      }
      if (peripheralType === "Pointing") {
        return "/static/icons/mouse.svg";
      }
    }
    // 默认外设使用鼠标图标
    return "/static/icons/mouse.svg";
  }
  
  if (major_class === "Imaging") {
    // 检查是否包含打印机
    if (minor_class?.Imaging && Array.isArray(minor_class.Imaging[0])) {
      const imagingTypes = minor_class.Imaging[0];
      if (imagingTypes.includes("Printer")) {
        return "/static/icons/printer.svg";
      }
    }
  }
  
  if (major_class === "Wearable") {
    return "/static/icons/double_watch.svg";
  }
  
  // 通过 major_service_classes 判断
  if (Array.isArray(major_service_classes)) {
    if (major_service_classes.includes("Audio")) {
      return "/static/icons/earphone.svg";
    }
    if (major_service_classes.includes("Capturing")) {
      return "/static/icons/voice.svg";
    }
    if (major_service_classes.includes("Rendering")) {
      return "/static/icons/printer.svg";
    }
  }
  
  // 默认返回蓝牙图标
  return "/static/icons/Bluetooth.svg";
}

function isHeadphoneDevice(device: any): boolean {
  if (!device) return false;

  const appearance = device.appearance;
  if (device.is_low_energy && typeof appearance === "number") {
    const category = appearance >> 6;
    // 0x10 = Audio/Video category (headset, headphones, speaker, etc.)
    if (category === 0x10) return true;
    return false;
  }

  const majorClass = device.major_class;
  if (majorClass !== "AudioVideo") return false;

  const minorClass = device.minor_class;
  const audioType = minorClass?.AudioVideo;
  if (typeof audioType === "string") {
    return ["Headset", "HandsFree", "Headphones", "PortableAudio", "Loudspeaker"].includes(audioType);
  }

  return false;
}

function getDisplayName(device: any, t?: (key: string) => string): string {
  const unknownDevice = t ? t('bluetooth.unknown_device') : "未知设备";
  if (!device) return unknownDevice;

  const raw = String(device.name ?? "").trim();
  if (!raw) return unknownDevice;

  const id = String(device.id ?? "").trim().toLowerCase();
  const address = String(device.address ?? "").trim().toLowerCase();
  const lower = raw.toLowerCase();

  // 各种 MAC 地址格式检测
  const macPattern = /^([0-9a-f]{2}[:-]){5}[0-9a-f]{2}$/i;  // AA:BB:CC:DD:EE:FF
  const hex12 = /^[0-9a-f]{12}$/i;  // AABBCCDDEEFF
  const hex16 = /^[0-9a-f]{16}$/i;  // 16位十六进制（某些设备）
  const macSubstringPattern = /([0-9a-f]{2}[:-]){5}[0-9a-f]{2}/i;

  // 纯 MAC 地址格式
  if (macPattern.test(raw) || hex12.test(raw) || hex16.test(raw)) return unknownDevice;

  // 名称等于设备 ID 或地址
  if (lower === id) return unknownDevice;
  if (address && (lower === address || lower.includes(address))) return unknownDevice;

  // Bluetooth#... 或 BluetoothDevice 等无意义名称
  if (/^bluetooth[#\s\-_]/i.test(raw)) return unknownDevice;

  // 名称只是 MAC 地址的组合（检查是否全是十六进制字符和分隔符）
  const cleanName = raw.replace(/[:\-\s]/g, "");
  if (/^[0-9a-f]{12,}$/i.test(cleanName)) return unknownDevice;

  // 名称包含 MAC 地址，但其余部分无意义
  const m = raw.match(macSubstringPattern);
  if (m) {
    const rest = raw
      .replace(m[0], "")
      .replace(/[\s\-\(\)\[\]\{\}_:]/g, "");
    if (rest.length === 0) return unknownDevice;
  }

  return raw;
}

function mergeDevices(prev: any[], incoming: any[]): any[] {
  const incomingMap = new Map<string, any>();
  for (const d of incoming) {
    if (d && d.id) {
      incomingMap.set(d.id, d);
    }
  }

  const next: any[] = [];
  for (const d of prev) {
    if (!d || !d.id) continue;
    const updated = incomingMap.get(d.id);
    if (updated) {
      next.push(updated);
      incomingMap.delete(d.id);
    }
  }

  for (const d of incomingMap.values()) {
    next.push(d);
  }

  return next;
}

function getDevicePriority(device: any, t?: (key: string) => string): number {
  if (!device) return 100;

  const displayName = getDisplayName(device, t);
  const hasName = displayName !== (t ? t('bluetooth.unknown_device') : "未知设备");
  const hasTypeInfo = getDeviceIcon(device) !== "/static/icons/Bluetooth.svg";

  const connectable =
    device.connected ||
    device.paired ||
    device.can_pair;

  const base = hasTypeInfo ? 0 : 10;

  if (hasTypeInfo) {
    if (hasName && connectable) return base + 0;
    if (hasName && !connectable) return base + 1;
    if (!hasName && connectable) return base + 2;
    return base + 3;
  }

  if (hasName && connectable) return base + 0;
  if (hasName && !connectable) return base + 1;
  if (!hasName && connectable) return base + 2;
  return base + 3;
}

/* ==================== Type Guards ==================== */

function isDisplayPinAction(
  action: RawPairingAction
): action is { needs: "DisplayPin"; pin: string } {
  return (
    typeof action === "object" &&
    action !== null &&
    "needs" in action &&
    action.needs === "DisplayPin" &&
    "pin" in action
  );
}

function isConfirmPinMatchAction(
  action: RawPairingAction
): action is { needs: "ConfirmPinMatch"; pin: string } {
  return (
    typeof action === "object" &&
    action !== null &&
    "needs" in action &&
    action.needs === "ConfirmPinMatch" &&
    "pin" in action
  );
}

/* ==================== 组件 ==================== */

export function BluetoothModule() {
  const { t } = useTranslation();
  const [bluetoothEnabled, setBluetoothEnabled] = useState(false);
  const [devices, setDevices] = useState<any[]>([]);
  const [popupVisible, setPopupVisible] = useState(false);
  const [selectedDevice, setSelectedDevice] = useState<string | null>(null);
  const [pairingModalVisible, setPairingModalVisible] = useState(false);
  const [pairingDevice, setPairingDevice] = useState<string | null>(null);
  const [pairingAction, setPairingAction] = useState<PairingAction | null>(null);
  const [usernameInput, setUsernameInput] = useState("");
  const [passwordInput, setPasswordInput] = useState("");
  const [connectingDevices, setConnectingDevices] = useState<Set<string>>(new Set());
  const [pairingFailed, setPairingFailed] = useState<{
    visible: boolean;
    deviceName: string;
  }>({
    visible: false,
    deviceName: "",
  });
  const [disconnectModalVisible, setDisconnectModalVisible] = useState(false);
  const [disconnectDevice, setDisconnectDevice] = useState<{id: string, ids: string[], name: string, connected: boolean, isAudio: boolean} | null>(null);
  const popupVisibleRef = useRef(false);
  const popupBodyRef = useRef<HTMLDivElement | null>(null);
  const pendingDevicesPayloadRef = useRef<any[] | null>(null);
  const devicesUpdateTimerRef = useRef<number | null>(null);
  const knownDeviceIdsRef = useRef<Set<string>>(new Set());
  // 毛玻璃效果相关 ref
  const popupGlassRef = useRef<HTMLDivElement | null>(null);
  const glassUpdateSeqRef = useRef(0);
    
  // 二级窗口毛玻璃效果 refs
  const pairingModalGlassRef = useRef<HTMLDivElement | null>(null);
  const disconnectModalGlassRef = useRef<HTMLDivElement | null>(null);
  const pairingFailedGlassRef = useRef<HTMLDivElement | null>(null);
  // 二级窗口状态追踪 refs（用于检测关闭时自动隐藏毛玻璃）
  const prevPairingModalVisibleRef = useRef(false);
  const prevDisconnectModalVisibleRef = useRef(false);
  const prevPairingFailedVisibleRef = useRef(false);
  
  /* ---------- 初始化 ---------- */

  const fetchBluetoothStatus = useCallback(async () => {
    try {
      const enabled = await invoke(
        FuncCommand.SystemGetBluetoothEnabled,
        undefined
      );
      if (typeof enabled === "boolean") {
        setBluetoothEnabled(enabled);
      }
    } catch {}
  }, []);

  const applyDevicesList = useCallback((list: any[]) => {
    if (!Array.isArray(list)) return;

    const normalizeName = (device: any) => {
      const displayName = getDisplayName(device, t);
      if (displayName === t('bluetooth.unknown_device')) return "";
      return displayName.trim().toLowerCase();
    };

    const nameStats = new Map<
      string,
      { count: number; anyPaired: boolean; stableAddress: string | null }
    >();
    for (const device of list as any[]) {
      const normalizedName = normalizeName(device);
      if (!normalizedName) continue;
      const entry = nameStats.get(normalizedName) ?? {
        count: 0,
        anyPaired: false,
        stableAddress: null,
      };
      entry.count += 1;
      if (device?.paired === true || device?.connected === true) {
        entry.anyPaired = true;
      }
      const address = device?.address;
      if (entry.stableAddress == null && address && address !== 0) {
        entry.stableAddress = String(address);
      }
      nameStats.set(normalizedName, entry);
    }

    const deviceMap = new Map();
    list.forEach((device: any) => {
      const address = device?.address;
      const normalizedName = normalizeName(device);
      const stats = normalizedName ? nameStats.get(normalizedName) : undefined;

      const key =
        stats && stats.count > 1 && stats.anyPaired
          ? `paired-name:${normalizedName}`
          : address && address !== 0
            ? `addr:${String(address)}`
            : stats?.stableAddress
              ? `addr:${stats.stableAddress}`
              : normalizedName
                ? `name:${normalizedName}`
                : typeof device?.id === "string"
                  ? `id:${device.id}`
                  : `name:unknown`;
      const existing = deviceMap.get(key);
      if (!existing) {
        deviceMap.set(key, {
          ...device,
          connected: device?.connected === true,
          paired: device?.paired === true,
          merged_ids: typeof device?.id === "string" ? [device.id] : [],
        });
        return;
      }

      const preferIncoming = existing.is_low_energy && !device.is_low_energy;
      const preferred = preferIncoming ? device : existing;
      const other = preferIncoming ? existing : device;

      const merged = {
        ...preferred,
        merged_ids: Array.from(
          new Set([
            ...(Array.isArray(existing.merged_ids) ? existing.merged_ids : []),
            ...(typeof existing?.id === "string" ? [existing.id] : []),
            ...(typeof device?.id === "string" ? [device.id] : []),
          ])
        ),
        battery_percentage:
          preferred.battery_percentage == null && other.battery_percentage != null
            ? other.battery_percentage
            : preferred.battery_percentage,
        appearance:
          preferred.appearance == null && other.appearance != null
            ? other.appearance
            : preferred.appearance,
        major_class:
          preferred.major_class === "Uncategorized" &&
          other.major_class != null &&
          other.major_class !== "Uncategorized"
            ? other.major_class
            : preferred.major_class,
        minor_class:
          preferred.minor_class == null && other.minor_class != null
            ? other.minor_class
            : preferred.minor_class,
        major_service_classes:
          Array.isArray(preferred.major_service_classes) &&
          preferred.major_service_classes.length === 0 &&
          Array.isArray(other.major_service_classes) &&
          other.major_service_classes.length > 0
            ? other.major_service_classes
            : preferred.major_service_classes,
        connected:
          preferred.connected == null
            ? other.connected === true
            : preferred.connected === true,
        paired: preferred.paired === true || other.paired === true,
      };

      deviceMap.set(key, merged);
    });

    const uniqueDevices = Array.from(deviceMap.values());
    const visibleDevices = (uniqueDevices as any[]).filter((d: any) => {
      if (!d) return false;
      if (d?.paired === true || d?.connected === true) return true;
      const displayName = getDisplayName(d, t);
      const hasName = displayName !== t('bluetooth.unknown_device');
      const hasTypeInfo = getDeviceIcon(d) !== "/static/icons/Bluetooth.svg";
      if (!hasName && !hasTypeInfo) return false;
      return true;
    });
    const allKnownIds: string[] = [];
    for (const d of visibleDevices as any[]) {
      if (Array.isArray(d?.merged_ids) && d.merged_ids.length > 0) {
        for (const id of d.merged_ids) {
          if (typeof id === "string" && id.length > 0) allKnownIds.push(id);
        }
      } else if (typeof d?.id === "string" && d.id.length > 0) {
        allKnownIds.push(d.id);
      }
    }
    knownDeviceIdsRef.current = new Set(allKnownIds);

    setDevices((prev) => {
      const merged = mergeDevices(prev, visibleDevices);
      const prevIndex = new Map<string, number>();
      merged.forEach((d: any, idx: number) => {
        if (d && d.id && !prevIndex.has(d.id)) {
          prevIndex.set(d.id, idx);
        }
      });

      const priorityById = new Map<string, number>();
      const getPriority = (d: any) => {
        const id = typeof d?.id === "string" ? d.id : "";
        const cached = priorityById.get(id);
        if (cached != null) return cached;
        const p = getDevicePriority(d, t);
        priorityById.set(id, p);
        return p;
      };

      merged.sort((a: any, b: any) => {
        const pa = getPriority(a);
        const pb = getPriority(b);
        if (pa !== pb) return pa - pb;
        const ia = prevIndex.get(a?.id) ?? 0;
        const ib = prevIndex.get(b?.id) ?? 0;
        return ia - ib;
      });
      return merged;
    });

    setConnectingDevices((prev) => {
      const next = new Set(prev);
      for (const d of visibleDevices as any[]) {
        if (d?.paired || d?.connected) {
          const ids =
            Array.isArray(d?.merged_ids) && d.merged_ids.length > 0
              ? d.merged_ids
              : [d.id];
          for (const id of ids) {
            next.delete(id);
          }
        }
      }
      return next;
    });
  }, []);

  const scheduleApplyDevicesPayload = useCallback(
    (payload: any[], immediate: boolean) => {
      const hasNewDevice = payload.some(
        (d: any) => typeof d?.id === "string" && !knownDeviceIdsRef.current.has(d.id)
      );
      const shouldImmediate = immediate || hasNewDevice;

      pendingDevicesPayloadRef.current = payload;

      if (devicesUpdateTimerRef.current !== null) {
        if (shouldImmediate) {
          window.clearTimeout(devicesUpdateTimerRef.current);
          devicesUpdateTimerRef.current = null;
        } else {
          return;
        }
      }

      if (shouldImmediate) {
        const nextPayload = pendingDevicesPayloadRef.current;
        pendingDevicesPayloadRef.current = null;
        if (nextPayload) applyDevicesList(nextPayload);
        return;
      }

      devicesUpdateTimerRef.current = window.setTimeout(() => {
        devicesUpdateTimerRef.current = null;
        const nextPayload = pendingDevicesPayloadRef.current;
        pendingDevicesPayloadRef.current = null;
        if (nextPayload) applyDevicesList(nextPayload);
      }, 80);
    },
    [applyDevicesList]
  );

  const fetchBluetoothDevices = useCallback(async () => {
    try {
      const list = await invoke(
        FuncCommand.SystemGetBluetoothDevices,
        undefined
      );
      if (Array.isArray(list)) applyDevicesList(list);
    } catch {}
  }, [applyDevicesList]);

  useEffect(() => {
    fetchBluetoothStatus();
    fetchBluetoothDevices();

    let unsub1: (() => void) | null = null;
    let unsub2: (() => void) | null = null;

    subscribe(FuncEvent.SystemBluetoothStateChanged, (e) => {
      const payload = (e as any).payload;
      if (typeof payload?.enabled === "boolean") {
        setBluetoothEnabled(payload.enabled);
      }
    }).then((u) => (unsub1 = u));

    subscribe(FuncEvent.SystemBluetoothDevicesChanged, (e) => {
      const payload = (e as any).payload;
      if (Array.isArray(payload)) {
        if (!popupVisibleRef.current) {
          return;
        }
        scheduleApplyDevicesPayload(payload, false);
      }
    }).then((u) => (unsub2 = u));

    return () => {
      unsub1?.();
      unsub2?.();
      if (devicesUpdateTimerRef.current !== null) {
        window.clearTimeout(devicesUpdateTimerRef.current);
        devicesUpdateTimerRef.current = null;
      }
      pendingDevicesPayloadRef.current = null;
    };
  }, [fetchBluetoothStatus, fetchBluetoothDevices, scheduleApplyDevicesPayload]);

  /* ---------- 毛玻璃效果 ---------- */

  // 毛玻璃效果：显示弹窗模糊
  const showPopupGlass = useCallback(() => {
    if (!popupVisibleRef.current) return;
    const scheduledSeq = glassUpdateSeqRef.current;

    const el = popupGlassRef.current;
    if (!el) return;
    
    const rect = el.getBoundingClientRect();
    // 必须同时检查坐标和尺寸，确保元素已正确渲染
    if (rect.left <= -9999 || rect.top <= -9999 || rect.width <= 0 || rect.height <= 0) return;
    
    requestAnimationFrame(() => {
      if (!popupVisibleRef.current || scheduledSeq !== glassUpdateSeqRef.current) return;
      (invoke as any)('popup_glass_show', {
        id: 'bluetooth-primary',
        x: Math.round(rect.left),
        y: Math.round(rect.top),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
        cornerRadius: POPUP_CORNER_RADIUS
      }).catch((e: any) => {
        btLogError('[Bluetooth] Failed to show glass effect:', e);
      });
    });
  }, []);
  
  // 毛玻璃效果：隐藏弹窗模糊
  const hidePopupGlass = useCallback(() => {
    glassUpdateSeqRef.current++;
    (invoke as any)('popup_glass_hide', { id: 'bluetooth-primary' }).catch((e: any) => {
      btLogError('[Bluetooth] Failed to hide glass effect:', e);
    });
  }, []);
    
  // 二级窗口毛玻璃效果：显示
  const showSecondaryGlass = useCallback((ref: React.RefObject<HTMLDivElement | null>, id: string, cornerRadius: number = 12) => {
    if (!popupVisibleRef.current) return;
    const scheduledSeq = glassUpdateSeqRef.current;

    const el = ref.current;
    if (!el || !document.contains(el)) return;
        
    const rect = el.getBoundingClientRect();
    // 必须检查尺寸，确保元素已正确渲染
    if (rect.width <= 0 || rect.height <= 0) return;
        
    requestAnimationFrame(() => {
      if (!popupVisibleRef.current || scheduledSeq !== glassUpdateSeqRef.current) return;
      const el2 = ref.current;
      if (!el2 || !document.contains(el2)) return;
      (invoke as any)('popup_glass_show', {
        id,
        x: Math.round(rect.left),
        y: Math.round(rect.top),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
        cornerRadius
      }).catch((e: any) => {
        btLogError('[Bluetooth] Failed to show secondary glass effect:', e);
      });
    });
  }, []);
      
  // 二级窗口毛玻璃效果：隐藏
  const hideSecondaryGlass = useCallback((id: string) => {
    glassUpdateSeqRef.current++;
    (invoke as any)('popup_glass_hide', { id }).catch((e: any) => {
      btLogError('[Bluetooth] Failed to hide secondary glass effect:', e);
    });
  }, []);
    
  // 二级窗口毛玻璃效果：监听配对弹窗
  useEffect(() => {
    // 当从 true 变为 false 时，立即隐藏毛玻璃
    if (prevPairingModalVisibleRef.current && !pairingModalVisible) {
      hideSecondaryGlass('bluetooth-pairing');
    }
    prevPairingModalVisibleRef.current = pairingModalVisible;
    
    if (!pairingModalVisible) return;
    
    const checkAndShow = () => {
      const el = pairingModalGlassRef.current;
      if (!el || !document.contains(el)) return false;
      const rect = el.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) {
        showSecondaryGlass(pairingModalGlassRef, 'bluetooth-pairing', 12);
        return true;
      }
      return false;
    };
        
    if (!checkAndShow()) {
      requestAnimationFrame(() => {
        if (!checkAndShow()) {
          requestAnimationFrame(checkAndShow);
        }
      });
    }
  }, [pairingModalVisible, showSecondaryGlass, hideSecondaryGlass]);
    
  // 二级窗口毛玻璃效果：监听断开连接弹窗
  useEffect(() => {
    // 当从 true 变为 false 时，立即隐藏毛玻璃
    if (prevDisconnectModalVisibleRef.current && !disconnectModalVisible) {
      hideSecondaryGlass('bluetooth-disconnect');
    }
    prevDisconnectModalVisibleRef.current = disconnectModalVisible;
    
    if (!disconnectModalVisible || !disconnectDevice) return;
    
    const checkAndShow = () => {
      const el = disconnectModalGlassRef.current;
      if (!el || !document.contains(el)) return false;
      const rect = el.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) {
        showSecondaryGlass(disconnectModalGlassRef, 'bluetooth-disconnect', 12);
        return true;
      }
      return false;
    };
        
    if (!checkAndShow()) {
      requestAnimationFrame(() => {
        if (!checkAndShow()) {
          requestAnimationFrame(checkAndShow);
        }
      });
    }
  }, [disconnectModalVisible, disconnectDevice, showSecondaryGlass, hideSecondaryGlass]);
    
  // 二级窗口毛玻璃效果：监听配对失败弹窗
  useEffect(() => {
    // 当从 true 变为 false 时，立即隐藏毛玻璃
    if (prevPairingFailedVisibleRef.current && !pairingFailed.visible) {
      hideSecondaryGlass('bluetooth-pairing-failed');
    }
    prevPairingFailedVisibleRef.current = pairingFailed.visible;
    
    if (!pairingFailed.visible) return;
    
    const checkAndShow = () => {
      const el = pairingFailedGlassRef.current;
      if (!el || !document.contains(el)) return false;
      const rect = el.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) {
        showSecondaryGlass(pairingFailedGlassRef, 'bluetooth-pairing-failed', 12);
        return true;
      }
      return false;
    };
        
    if (!checkAndShow()) {
      requestAnimationFrame(() => {
        if (!checkAndShow()) {
          requestAnimationFrame(checkAndShow);
        }
      });
    }
  }, [pairingFailed.visible, showSecondaryGlass, hideSecondaryGlass]);

  // 毛玻璃效果：监听弹窗打开和尺寸变化
  useEffect(() => {
    if (!popupVisible) return;
    
    let resizeObserver: ResizeObserver | null = null;
    let rafId: number | null = null;
    let timeoutId: number | null = null;
    
    // 使用多次 requestAnimationFrame 确保定位完成，并创建 ResizeObserver
    const checkAndObserve = () => {
      const el = popupGlassRef.current;
      if (!el) return false;
      const rect = el.getBoundingClientRect();
      if (rect.left > -9999 && rect.top > -9999 && rect.width > 0 && rect.height > 0) {
        showPopupGlass();
        
        // 创建 ResizeObserver 监听后续尺寸变化
        if (!resizeObserver) {
          resizeObserver = new ResizeObserver(() => {
            if (rafId) return;
            rafId = requestAnimationFrame(() => {
              rafId = null;
              showPopupGlass();
            });
          });
          resizeObserver.observe(el);
        }
        return true;
      }
      return false;
    };
    
    if (!checkAndObserve()) {
      requestAnimationFrame(() => {
        if (!checkAndObserve()) {
          requestAnimationFrame(() => {
            if (!checkAndObserve()) {
              requestAnimationFrame(checkAndObserve);
            }
          });
        }
      });
    }
    
    return () => {
      if (timeoutId) window.clearTimeout(timeoutId);
      if (rafId) cancelAnimationFrame(rafId);
      if (resizeObserver) resizeObserver.disconnect();
    };
  }, [popupVisible, bluetoothEnabled, showPopupGlass]);

  /* ---------- 行为 ---------- */

  const handlePopupOpenChange = useCallback((open: boolean) => {
    if (open) {
      // 上报蓝牙按钮点击
      (invoke as any)(FuncCommand.ReportClickComponent, { content: "Bluetooth" });
      
      popupVisibleRef.current = true;
      setPopupVisible(true);
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          const el = popupBodyRef.current;
          if (el) el.scrollTop = 0;
        });
      });
      if (devicesUpdateTimerRef.current !== null) {
        window.clearTimeout(devicesUpdateTimerRef.current);
        devicesUpdateTimerRef.current = null;
      }
      pendingDevicesPayloadRef.current = null;

      invoke(FuncCommand.StartBluetoothScanning, undefined).catch(err => {
        btLogError("[蓝牙] 开始扫描失败:", err);
      });
      fetchBluetoothDevices();
    } else {
      // 先隐藏所有毛玻璃，再关闭弹窗
      hidePopupGlass();
      hideSecondaryGlass('bluetooth-pairing');
      hideSecondaryGlass('bluetooth-disconnect');
      hideSecondaryGlass('bluetooth-pairing-failed');
      
      popupVisibleRef.current = false;
      setPopupVisible(false);
      invoke(FuncCommand.StopBluetoothScanning, undefined).catch(err => {
        btLogError("[蓝牙] 停止扫描失败:", err);
      });
      if (devicesUpdateTimerRef.current !== null) {
        window.clearTimeout(devicesUpdateTimerRef.current);
        devicesUpdateTimerRef.current = null;
      }
      pendingDevicesPayloadRef.current = null;
      // 清除未配对设备，避免下次打开面板时残留旧的可用设备导致闪烁
      setDevices(prev => prev.filter((d: any) => d?.paired));
      
      // 关闭所有二级窗口
      setPairingModalVisible(false);
      setPairingAction(null);
      setUsernameInput("");
      setPasswordInput("");
      setSelectedDevice(null);
      setDisconnectModalVisible(false);
      setDisconnectDevice(null);
      setPairingFailed({ visible: false, deviceName: "" });
      // 清除连接中状态
      setConnectingDevices(new Set());
    }
  }, [fetchBluetoothDevices, hidePopupGlass, hideSecondaryGlass]);

  const toggleBluetooth = async () => {
    const next = !bluetoothEnabled;
    setBluetoothEnabled(next);
    await invoke(FuncCommand.SystemSetBluetoothEnabled, { enabled: next });
  };

  const handleDeviceClick = async (deviceId: string) => {
    const device = devices.find((d) => d.id === deviceId);
    if (!device) return;


    // 如果已连接且已配对，显示断开连接弹框
    if (device.connected && device.paired) {
      const ids =
        Array.isArray((device as any)?.merged_ids) && (device as any).merged_ids.length > 0
          ? (device as any).merged_ids
          : [deviceId];
      setDisconnectDevice({
        id: deviceId,
        ids,
        name: getDisplayName(device, t),
        connected: true,
        isAudio: isHeadphoneDevice(device),
      });
      setDisconnectModalVisible(true);
      return;
    }

    // 如果已配对但未连接，显示取消配对选项
    if (device.paired && !device.connected) {
      // 已配对设备通常由系统自动连接，显示取消配对弹框
      const ids =
        Array.isArray((device as any)?.merged_ids) && (device as any).merged_ids.length > 0
          ? (device as any).merged_ids
          : [deviceId];
      setDisconnectDevice({
        id: deviceId,
        ids,
        name: getDisplayName(device, t),
        connected: false,
        isAudio: isHeadphoneDevice(device),
      });
      setDisconnectModalVisible(true);
      return;
    }

    // 如果正在连接中，不做处理
    if (connectingDevices.has(deviceId)) {
      return;
    }

    // 未配对设备，开始配对流程（先转圈，获取配对信息后再弹窗）
    await startPairing(deviceId);
  };

  const startPairing = async (deviceId: string) => {
    // 先设置转圈状态，不显示弹窗
    setSelectedDevice(deviceId);
    setConnectingDevices(prev => new Set(prev).add(deviceId));
    
    // 记录开始时间，确保转圈至少显示 500ms
    const startTime = Date.now();
    
    try {
      // 调用配对请求获取配对信息
      const raw = (await invoke(
        FuncCommand.RequestPairBluetoothDevice,
        { id: String(deviceId) }
      )) as unknown as RawPairingAction;

      // 确保转圈至少显示 500ms
      const elapsed = Date.now() - startTime;
      if (elapsed < 500) {
        await new Promise(resolve => setTimeout(resolve, 500 - elapsed));
      }

      // 获取到配对信息后，显示配对弹窗
      setPairingModalVisible(true);

      if (!raw || (typeof raw === "object" && raw.needs === "None")) {
        // 不需要用户交互，直接确认配对
        setPairingAction(null);
        return;
      }

      if (isDisplayPinAction(raw)) {
        setPairingAction({
          deviceId: deviceId,
          needs: "DisplayPin",
          pin: raw.pin,
        });
        return;
      }

      if (isConfirmPinMatchAction(raw)) {
        setPairingAction({
          deviceId: deviceId,
          needs: "ConfirmPinMatch",
          pin: raw.pin,
        });
        return;
      }
      if (typeof raw === "object" && "needs" in raw) {
        setPairingAction({
          deviceId: deviceId,
          needs: raw.needs as PairingNeeds,
        });
      }
    } catch (error) {
      const device = devices.find((d) => d.id === deviceId);
      setPairingFailed({
        visible: true,
        deviceName: getDisplayName(device, t) || t('bluetooth.this_device'),
      });
      setSelectedDevice(null);
      // 移除连接中状态
      setConnectingDevices(prev => {
        const next = new Set(prev);
        next.delete(deviceId);
        return next;
      });
    }
  };

  const handlePairingConfirm = async () => {
    if (!selectedDevice) return;
    
    // 直接确认配对（配对信息已在 startPairing 中获取）
    await confirmPairing(true);
  };

  


  const confirmPairing = async (accept: boolean) => {
    if (!selectedDevice) return;
      
    try {
      await invoke(FuncCommand.ConfirmBluetoothDevicePairing, {
        id: String(selectedDevice),
        answer: {
          accept,
          pin: pairingAction?.pin ?? null,
          username:
            pairingAction?.needs === "ProvidePasswordCredential"
              ? usernameInput || null
              : null,
          password:
            pairingAction?.needs === "ProvidePasswordCredential"
              ? passwordInput || null
              : null,
          address: null,
        },
      });
        
      if (accept) {
        // 配对成功
        setPairingModalVisible(false);
          
        // 等待 200ms 后第一次刷新
        setTimeout(async () => {
          await fetchBluetoothDevices();
        }, 200);
          
        // 等待 1秒后再次刷新并移除 loading
        setTimeout(async () => {
          await fetchBluetoothDevices();
          setConnectingDevices(prev => {
            const next = new Set(prev);
            next.delete(selectedDevice);
            return next;
          });
        }, 1000);
      } else {
        // 取消配对
        setPairingModalVisible(false);
        setConnectingDevices(prev => {
          const next = new Set(prev);
          next.delete(selectedDevice);
          return next;
        });
      }
    } catch (error) {
      btLogError("确认配对失败:", error);
      // 配对失败
      setPairingModalVisible(false);
      const device = devices.find((d) => d.id === selectedDevice);
      setPairingFailed({
        visible: true,
        deviceName: getDisplayName(device, t) || t('bluetooth.this_device'),
      });
      // 移除连接中状态
      setConnectingDevices(prev => {
        const next = new Set(prev);
        if (selectedDevice) {
          next.delete(selectedDevice);
        }
        return next;
      });
    } finally {
      setPairingAction(null);
      setUsernameInput("");
      setPasswordInput("");
      setSelectedDevice(null);
    }
  };
  

  const cancelPairing = async () => {
    const deviceId = selectedDevice;
    const action = pairingAction;

    setPairingModalVisible(false);
    setPairingAction(null);
    setUsernameInput("");
    setPasswordInput("");
    setSelectedDevice(null);
    
    if (deviceId) {
      // 移除连接中状态
      setConnectingDevices(prev => {
        const next = new Set(prev);
        next.delete(deviceId);
        return next;
      });

      // 后台通知后端取消配对（不阻塞 UI）
      if (action) {
        invoke(FuncCommand.ConfirmBluetoothDevicePairing, {
          id: String(deviceId),
          answer: {
            accept: false,
            pin: null,
            username: null,
            password: null,
            address: null,
          },
        }).catch(err => btLogError("[蓝牙] 取消配对通知失败:", err));
      }
    }
  };

  const disconnectBluetoothDevice = async (deviceId: string) => {
    // 使用桥接命令
    await (invoke as any)("disconnect_bluetooth_device", {
      id: String(deviceId),
    });
    setSelectedDevice(null);
    popupVisibleRef.current = false;
    hidePopupGlass();
    hideSecondaryGlass('bluetooth-pairing');
    hideSecondaryGlass('bluetooth-disconnect');
    hideSecondaryGlass('bluetooth-pairing-failed');
    setPopupVisible(false);
  };

  const handleDisconnectConfirm = async () => {
    if (!disconnectDevice) return;
    
    // 已连接设备，执行断开连接
    await disconnectBluetoothDevice(disconnectDevice.id);
    
    setDisconnectModalVisible(false);
    setDisconnectDevice(null);
  };

  const handleConnectConfirm = async () => {
    if (!disconnectDevice) return;
    
    setDisconnectModalVisible(false);
    setDisconnectDevice(null);

    (invoke as any)("connect_bluetooth_device", { id: disconnectDevice.id })
      .then(() => {
        return fetchBluetoothDevices();
      })
      .catch((error: any) => {
      });
  };

  const handleForgetConfirm = () => {
    if (!disconnectDevice) return;
    
    // 关闭弹框
    setDisconnectModalVisible(false);
    setDisconnectDevice(null);
    
    const ids =
      Array.isArray(disconnectDevice.ids) && disconnectDevice.ids.length > 0
        ? disconnectDevice.ids
        : [disconnectDevice.id];
    // 后台执行取消配对，不阻塞 UI
    Promise.allSettled(
      ids.map((id) => invoke(FuncCommand.ForgetBluetoothDevice, { id: String(id) }))
    ).catch((error) => {
      btLogError("[蓝牙] 取消配对失败:", error);
    });
  };

  const handleDisconnectCancel = () => {
    setDisconnectModalVisible(false);
    setDisconnectDevice(null);
  };

  /* ---------- UI ---------- */

  const iconSrc = bluetoothEnabled
    ? "/static/icons/Bluetooth.svg"
    : "/static/icons/BluetoothOff.svg";

  const pairedDevices = useMemo(
    () => devices.filter((d: any) => d?.paired),
    [devices]
  );
  const availableDevices = useMemo(
    () => devices.filter((d: any) => !d?.paired),
    [devices]
  );

  const PopupContent = (
    <div className="bluetooth-popup" ref={popupGlassRef}>
      {/* 第一分区 - 蓝牙 + 开关 */}
      <div className="popup-header">
        <span>{t('bluetooth.title')}</span>
        <img
          className="switch-icon"
          src={
            bluetoothEnabled
              ? "/static/icons/Switch.svg"
              : "/static/icons/SwitchBase.svg"
          }
          onClick={toggleBluetooth}
        />
      </div>

      {/* 第二、三分区 - 设备列表 */}
      <div
        ref={popupBodyRef}
        className={`popup-body ${!bluetoothEnabled ? 'popup-body-disabled' : ''}`}
      >
        {bluetoothEnabled && (
          <>
            {/* 已配对设备 - 只在有设备时显示 */}
            {pairedDevices.length > 0 && (
              <>
                <div className="device-section-title">{t('bluetooth.paired_devices')} ({pairedDevices.length})</div>
                <div className="device-list">
                  {pairedDevices.map((d: any) => (
                    <div key={d.id} className="device-item-container">
                      <div
                        className={`device-item ${
                          d.connected ? "device-connected" : ""
                        } ${
                          selectedDevice === d.id ? "device-selected" : ""
                        }`}
                        onClick={() => handleDeviceClick(d.id)}
                      >
                        <div className="device-info">
                          <img 
                            src={getDeviceIcon(d)} 
                            className="device-type-icon"
                            alt={t('bluetooth.device_type')}
                          />
                          <span className="device-name">
                            {getDisplayName(d, t)}
                          </span>
                        </div>
                        {d.connected && !connectingDevices.has(d.id) && d.battery_percentage !== undefined && d.battery_percentage !== null && (
                          <div className="device-battery-info">
                            <span className="battery-percentage">{d.battery_percentage}%</span>
                            <img 
                              src="/static/icons/battery.svg" 
                              className="battery-icon"
                              alt={t('bluetooth.battery')}
                            />
                          </div>
                        )}
                      </div>
              
                      {selectedDevice === d.id && d.connected && isHeadphoneDevice(d) && (
                        <div className="pair-button-container">
                          <button
                            className="disconnect-button"
                            onClick={() => disconnectBluetoothDevice(d.id)}
                          >
                          </button>
                        </div>
                      )}
                    </div>
                  ))}
                </div>
              </>
            )}

            {/* 可用设备 */}
            <>
              <div className="device-section-title">{t('bluetooth.available_devices')}</div>
              <div className="device-list">
                {availableDevices.map((d: any) => (
                  <div key={d.id} className="device-item-container">
                    <div
                      className={`device-item ${
                          connectingDevices.has(d.id) ? "device-selected" : ""
                      }`}
                      onClick={() => handleDeviceClick(d.id)}
                    >
                      <div className="device-info">
                        <img 
                          src={getDeviceIcon(d)} 
                          className="device-type-icon"
                          alt={t('bluetooth.device_type')}
                        />
                        <span className="device-name">
                          {getDisplayName(d, t)}
                        </span>
                      </div>
                      {connectingDevices.has(d.id) && (
                        <span className="loading-spinner" aria-label={t('bluetooth.connecting')} role="img">
                          <span className="loading-spinner-dot dot-1" aria-hidden="true" />
                          <span className="loading-spinner-dot dot-2" aria-hidden="true" />
                          <span className="loading-spinner-dot dot-3" aria-hidden="true" />
                          <span className="loading-spinner-dot dot-4" aria-hidden="true" />
                          <span className="loading-spinner-dot dot-5" aria-hidden="true" />
                          <span className="loading-spinner-dot dot-6" aria-hidden="true" />
                          <span className="loading-spinner-dot dot-7" aria-hidden="true" />
                          <span className="loading-spinner-dot dot-8" aria-hidden="true" />
                          <span className="loading-spinner-dot dot-9" aria-hidden="true" />
                          <span className="loading-spinner-dot dot-10" aria-hidden="true" />
                          <span className="loading-spinner-dot dot-11" aria-hidden="true" />
                          <span className="loading-spinner-dot dot-12" aria-hidden="true" />
                        </span>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            </>
          </>
        )}
      </div>

      {/* 第四分区 - 更多设置 */}
      <div className="popup-footer">
        <div className="popup-footer-item" onClick={() => invoke(FuncCommand.SystemOpenBluetoothSettings)}>
          {t('bluetooth.more_settings')}
        </div>
      </div>
    </div>
  );

  
  return (
    <>
      <SlPopup
        placement="top"
        offset={4}
        align="start"
        content={PopupContent}
        open={popupVisible}
        onOpenChange={handlePopupOpenChange}
      >
        <div className={`taskbar-item taskbar-module bluetooth-module${popupVisible ? ' selected' : ''}`}>
          <img className="bluetooth-icon" src={iconSrc} />
        </div>
      </SlPopup>

      {/* 配对模态弹窗 */}
      {pairingModalVisible && (
        <div className={`pairing-modal-container ${pairingAction && (pairingAction.needs === "DisplayPin" || pairingAction.needs === "ConfirmPinMatch") ? "has-pin" : ""}`} ref={pairingModalGlassRef}>
          {/* 头部 */}
          <div className="pairing-modal-header">
            <img src="/static/icons/Bluetooth.svg" className="pairing-modal-icon" />
            <span className="pairing-modal-title">{t('bluetooth.pairing_request_title')}</span>
          </div>

          {/* 主体内容 */}
          <div className="pairing-modal-body">
            {/* PIN码显示 */}
            {pairingAction && (pairingAction.needs === "DisplayPin" || pairingAction.needs === "ConfirmPinMatch") && (
              <div className="pairing-pin-display">
                <div className="pin-code-large">
                  {pairingAction.pin}
                </div>
              </div>
            )}

            {/* 账号密码输入 */}
            {pairingAction?.needs === "ProvidePasswordCredential" && (
              <div className="pairing-credential-inputs">
                <input
                  className="pairing-input"
                  value={usernameInput}
                  onChange={(e) => setUsernameInput(e.currentTarget.value)}
                  placeholder={t('bluetooth.username')}
                />
                <input
                  className="pairing-input"
                  type="password"
                  value={passwordInput}
                  onChange={(e) => setPasswordInput(e.currentTarget.value)}
                  placeholder={t('bluetooth.password')}
                />
              </div>
            )}
          </div>

          {/* 底部按钮 - 始终显示取消和配对按钮 */}
          <div className="pairing-modal-footer">
            <button className="pairing-btn pairing-btn-cancel" onClick={cancelPairing}>
              {t('bluetooth.cancel')}
            </button>
            <button 
              className="pairing-btn pairing-btn-confirm" 
              onClick={handlePairingConfirm}
              disabled={pairingAction?.needs === "ProvidePasswordCredential" && (!usernameInput || !passwordInput)}
            >
              {t('bluetooth.pair')}
            </button>
          </div>
        </div>
      )}
  
      {pairingFailed.visible && (
        <div className="pairing-failed-popup" ref={pairingFailedGlassRef}>
          <div className="pairing-failed-header">
            <img
              src="/static/icons/Bluetooth.svg"
              className="pairing-failed-icon"
            />
            <span className="pairing-failed-title">{t('bluetooth.pairing_failed_title')}</span>
          </div>
  
          <div className="pairing-failed-content">
            {t('bluetooth.pairing_failed_content', { deviceName: pairingFailed.deviceName })}
            <br />
            1. {t('bluetooth.pairing_failed_reason1')}
            <br />
            2. {t('bluetooth.pairing_failed_reason2')}
          </div>
  
          <button
            className="pairing-failed-button"
            onClick={() => {
              setPairingFailed({ visible: false, deviceName: "" });
            }}
          >
            {t('bluetooth.pairing_failed_button')}
          </button>
        </div>
      )}
      {/* 断开连接弹框 */}
      {disconnectModalVisible && disconnectDevice && (
        <div className="bluetooth-disconnect-dialog" ref={disconnectModalGlassRef}>
          {/* 头部 - 蓝牙图标和设备名称 */}
          <div className="disconnect-modal-header">
            <img src="/static/icons/Bluetooth.svg" className="disconnect-modal-icon" />
            <span className="disconnect-modal-device-name">{disconnectDevice.name}</span>
          </div>

          {/* 底部按钮 */}
          <div className="disconnect-modal-footer">
            <button className="disconnect-btn disconnect-btn-cancel" onClick={handleDisconnectCancel}>
              {t('bluetooth.cancel')}
            </button>
            <button className="disconnect-btn disconnect-btn-forget" onClick={handleForgetConfirm}>
              {t('bluetooth.unpair')}
            </button>
            {disconnectDevice.isAudio && disconnectDevice.connected && (
              <button className="disconnect-btn disconnect-btn-disconnect" onClick={handleDisconnectConfirm}>
                {t('bluetooth.disconnect')}
              </button>
            )}
            {disconnectDevice.isAudio && !disconnectDevice.connected && (
              <button className="disconnect-btn disconnect-btn-connect" onClick={handleConnectConfirm}>
                {t('bluetooth.connect')}
              </button>
            )}
          </div>
        </div>
      )}
    </>
  );
  
}