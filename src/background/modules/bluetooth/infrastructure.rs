use super::{
    BluetoothDevice, BluetoothDeviceType, BluetoothDeviceWrapper, BluetoothLEDeviceWrapper,
    BluetoothMajorClass, BluetoothManagerEvent, BluetoothMinorClass, DevicePairingAnswer,
};
use crate::{
    app::get_app_handle,
    error::{Result, ResultLogExt},
    get_tokio_handle,
    utils::lock_free::SyncHashMap,
};
use arc_swap::ArcSwapOption;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use tokio::sync::mpsc;
use windows::Devices::Enumeration::{DevicePairingResultStatus, DeviceUnpairingResultStatus};

use crate::modules::bluetooth::DevicePairingNeededAction;
use crate::windows_api::device::{DeviceEnumerator, DeviceEvent, DeviceId};
use tauri::Emitter;
use windows::{
    Devices::{
        Bluetooth::{BluetoothDevice as WinBluetoothDevice, BluetoothLEDevice},
        Enumeration::{
            DeviceInformation, DeviceInformationCustomPairing, DevicePairingKinds,
            DevicePairingProtectionLevel, DevicePairingRequestedEventArgs, DevicePairingResult,
        },
    },
    Foundation::{Deferral, IAsyncOperation, TypedEventHandler},
};
// Pairing configuration constants
const PAIRING_REQUEST_TIMEOUT_SECS: u64 = 10;
const PAIRING_CONFIRMATION_MAX_RETRIES: u32 = 20;
const PAIRING_CONFIRMATION_RETRY_INTERVAL_MS: u64 = 500;
static BLUETOOTH_MANAGER_INSTANCE: LazyLock<BluetoothManager> = LazyLock::new(|| {
    let mut m = BluetoothManager::create();
    m.initialize().log_error();
    m
});
pub struct BluetoothManager {
    pub devices: SyncHashMap<DeviceId, Arc<Mutex<BluetoothDeviceWrapper>>>,
    pub le_devices: SyncHashMap<DeviceId, Arc<Mutex<BluetoothLEDeviceWrapper>>>,
    /// 关闭面板时保存的 BLE 已配对设备快照，下次打开面板可立即显示，避免闪烁
    le_devices_cache: SyncHashMap<DeviceId, BluetoothDevice>,
    discovery_devices: SyncHashMap<DeviceId, BluetoothDevice>,
    resolving_devices: SyncHashMap<DeviceId, ()>,
    classic_enumerator: Mutex<Option<DeviceEnumerator>>,
    le_enumerator: Mutex<Option<DeviceEnumerator>>,
    // Discovery/scanning enumerators (unpaired devices)
    discovery_classic_enumerator: ArcSwapOption<DeviceEnumerator>,
    discovery_le_enumerator: ArcSwapOption<DeviceEnumerator>,
    // Pairing state
    pending_pair_requests: SyncHashMap<DeviceId, Arc<Mutex<PendingPairRequest>>>,
    // 控制是否发送设备变化事件到前端（节省功耗）
    should_emit_events: AtomicBool,
    emit_scheduled: AtomicBool,
    last_emit_ms: AtomicU64,
    last_device_info_query: SyncHashMap<DeviceId, u64>,
    last_refresh: SyncHashMap<DeviceId, u64>,
    active_resolutions: AtomicU64,
    scan_session: AtomicU64,
}
crate::event_manager!(BluetoothManager, BluetoothManagerEvent);

fn current_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn handle_device_event(event: DeviceEvent, device_type: BluetoothDeviceType) {
    let enabled = BluetoothManager::instance()
        .should_emit_events
        .load(Ordering::Relaxed);

    if !enabled {
        return;
    }

    let bt_event = match event {
        DeviceEvent::Added(id) => BluetoothManagerEvent::DeviceAdded(id, device_type),
        DeviceEvent::Updated(id) => BluetoothManagerEvent::DeviceUpdated(id, device_type),
        DeviceEvent::Removed(id) => BluetoothManagerEvent::DeviceRemoved(id, device_type),
    };
    BluetoothManager::send(bt_event);
}
#[allow(dead_code)]
impl BluetoothManager {
    fn create() -> Self {
        Self {
            devices: SyncHashMap::new(),
            le_devices: SyncHashMap::new(),
            le_devices_cache: SyncHashMap::new(),
            discovery_devices: SyncHashMap::new(),
            resolving_devices: SyncHashMap::new(),
            classic_enumerator: Mutex::new(None),
            le_enumerator: Mutex::new(None),
            discovery_classic_enumerator: ArcSwapOption::from(None),
            discovery_le_enumerator: ArcSwapOption::from(None),
            pending_pair_requests: SyncHashMap::new(),
            should_emit_events: AtomicBool::new(false),
            emit_scheduled: AtomicBool::new(false),
            last_emit_ms: AtomicU64::new(0),
            last_device_info_query: SyncHashMap::new(),
            last_refresh: SyncHashMap::new(),
            active_resolutions: AtomicU64::new(0),
            scan_session: AtomicU64::new(1),
        }
    }
    fn initialize(&mut self) -> Result<()> {
        Self::subscribe(|e| {
            let _ = Self::instance().on_event(&e);
        });

        Ok(())
    }
    fn start_base_watchers(&self) -> Result<()> {
        {
            let mut classic = self.classic_enumerator.lock().unwrap();
            if classic.is_none() {
                let watcher = DeviceEnumerator::new(
                    WinBluetoothDevice::GetDeviceSelector()?.to_string(),
                    |event| handle_device_event(event, BluetoothDeviceType::Classic),
                )?;
                watcher.start()?;
                *classic = Some(watcher);
            }
        }

        {
            let mut le = self.le_enumerator.lock().unwrap();
            if le.is_none() {
                let watcher = DeviceEnumerator::new(
                    BluetoothLEDevice::GetDeviceSelector()?.to_string(),
                    |event| handle_device_event(event, BluetoothDeviceType::LowEnergy),
                )?;
                watcher.start()?;
                *le = Some(watcher);
            }
        }

        Ok(())
    }
    pub fn instance() -> &'static Self {
        &BLUETOOTH_MANAGER_INSTANCE
    }

    fn schedule_resolution(&self, id: String, device_type: BluetoothDeviceType) {
        if !self.should_emit_events.load(Ordering::Relaxed) {
            return;
        }

        if self.resolving_devices.get(&id).is_some() {
            return;
        }

        const MAX_CONCURRENT: u64 = 2;

        if self.active_resolutions.load(Ordering::Relaxed) >= MAX_CONCURRENT {
            return;
        }

        let session = self.scan_session.load(Ordering::Relaxed);

        self.resolving_devices.upsert(id.clone(), ());
        self.active_resolutions.fetch_add(1, Ordering::Relaxed);

        std::thread::spawn(move || {
            let manager = BluetoothManager::instance();

            let cleanup = |id: &String| {
                manager.resolving_devices.remove(id);
                manager.active_resolutions.fetch_sub(1, Ordering::Relaxed);
            };

            if manager.scan_session.load(Ordering::Relaxed) != session {
                cleanup(&id);
                return;
            }

            const MAX_RETRIES: u32 = 3;
            const RETRY_INTERVAL_MS: u64 = 400;

            for _ in 0..MAX_RETRIES {
                if !manager.should_emit_events.load(Ordering::Relaxed) {
                    break;
                }

                if manager.scan_session.load(Ordering::Relaxed) != session {
                    break;
                }

                let result = match device_type {
                    BluetoothDeviceType::Classic => BluetoothDeviceWrapper::create(&id)
                        .map(|w| manager.devices.upsert(id.clone(), Arc::new(Mutex::new(w)))),
                    BluetoothDeviceType::LowEnergy => {
                        BluetoothLEDeviceWrapper::create(&id).map(|w| {
                            manager
                                .le_devices
                                .upsert(id.clone(), Arc::new(Mutex::new(w)))
                        })
                    }
                };

                if result.is_ok() {
                    manager.discovery_devices.remove(&id);

                    if manager.should_emit_events.load(Ordering::Relaxed) {
                        manager.request_emit_devices_snapshot(false);
                    }

                    cleanup(&id);
                    return;
                }

                std::thread::sleep(std::time::Duration::from_millis(RETRY_INTERVAL_MS));
            }

            cleanup(&id);
        });
    }

    fn should_query_device_info(&self, id: &str) -> bool {
        const MIN_INTERVAL_MS: u64 = 3000;

        let now = current_millis();
        let key = id.to_string();

        if let Some(last) = self.last_device_info_query.get(&key) {
            if now.saturating_sub(last) < MIN_INTERVAL_MS {
                return false;
            }
        }

        self.last_device_info_query.upsert(key, now);
        true
    }

    fn should_refresh_device(&self, id: &str) -> bool {
        const MIN_REFRESH_MS: u64 = 2000;

        let now = current_millis();
        let key = id.to_string();

        if let Some(last) = self.last_refresh.get(&key) {
            if now.saturating_sub(last) < MIN_REFRESH_MS {
                return false;
            }
        }

        self.last_refresh.upsert(key, now);
        true
    }

    fn on_event(&self, e: &BluetoothManagerEvent) -> Result<()> {
        //log::info!("[BT][Event] {:?}", e);
        // 如果蓝牙面板未打开，不做任何重操作
        if !self.should_emit_events.load(Ordering::Relaxed) {
            log::info!("[BT][DBG][on_event] ignored because panel closed");
            // 只做最轻量的清理，避免内存残留
            // if let BluetoothManagerEvent::DeviceRemoved(id, t) = e {
            //     match t {
            //         BluetoothDeviceType::Classic => {
            //             self.devices.remove(id);
            //             self.discovery_devices.remove(id);
            //         }
            //         BluetoothDeviceType::LowEnergy => {
            //             self.le_devices.remove(id);
            //             self.discovery_devices.remove(id);
            //         }
            //     }
            // }
            return Ok(());
        }

        let immediate_emit = matches!(e, BluetoothManagerEvent::DeviceRemoved(_, _));
        match e {
            BluetoothManagerEvent::DeviceAdded(id, t) => {
                if !self.should_emit_events.load(Ordering::Relaxed) {
                    return Ok(());
                }

                match t {
                    BluetoothDeviceType::Classic => {
                        if self.should_query_device_info(id) {
                            if let Ok(info) =
                                DeviceInformation::CreateFromIdAsync(&id.clone().into())?.get()
                            {
                                let pairing = info.Pairing()?;
                                let paired = pairing.IsPaired()?;
                                let can_pair = pairing.CanPair()?;

                                if !paired {
                                    self.discovery_devices.upsert(
                                        id.clone(),
                                        BluetoothDevice {
                                            id: id.clone(),
                                            name: info.Name()?.to_string(),
                                            address: 0,
                                            major_service_classes: Vec::new(),
                                            major_class: BluetoothMajorClass::Uncategorized,
                                            minor_class: BluetoothMinorClass::Uncategorized {
                                                unused: 0,
                                            },
                                            appearance: None,
                                            connected: false,
                                            paired,
                                            can_pair,
                                            can_disconnect: false,
                                            is_low_energy: false,
                                            battery_percentage: None,
                                        },
                                    );

                                    if self.resolving_devices.get(id).is_none() {
                                        self.schedule_resolution(
                                            id.clone(),
                                            BluetoothDeviceType::Classic,
                                        );
                                    }
                                } else if let Ok(w) = BluetoothDeviceWrapper::create(id) {
                                    self.devices.upsert(id.clone(), Arc::new(Mutex::new(w)));
                                    self.discovery_devices.remove(id);
                                }
                            }
                        }
                    }

                    BluetoothDeviceType::LowEnergy => {
                        if self.should_query_device_info(id) {
                            if let Ok(info) =
                                DeviceInformation::CreateFromIdAsync(&id.clone().into())?.get()
                            {
                                let pairing = info.Pairing()?;
                                let paired = pairing.IsPaired()?;
                                let can_pair = pairing.CanPair()?;

                                if !paired {
                                    self.discovery_devices.upsert(
                                        id.clone(),
                                        BluetoothDevice {
                                            id: id.clone(),
                                            name: info.Name()?.to_string(),
                                            address: 0,
                                            major_service_classes: Vec::new(),
                                            major_class: BluetoothMajorClass::Uncategorized,
                                            minor_class: BluetoothMinorClass::Uncategorized {
                                                unused: 0,
                                            },
                                            appearance: None,
                                            connected: false,
                                            paired,
                                            can_pair,
                                            can_disconnect: false,
                                            is_low_energy: true,
                                            battery_percentage: None,
                                        },
                                    );
                                    self.schedule_resolution(
                                        id.clone(),
                                        BluetoothDeviceType::LowEnergy,
                                    );
                                } else if let Ok(w) = BluetoothLEDeviceWrapper::create(id) {
                                    self.le_devices.upsert(id.clone(), Arc::new(Mutex::new(w)));
                                    self.discovery_devices.remove(id);
                                } else {
                                    self.discovery_devices.upsert(
                                        id.clone(),
                                        BluetoothDevice {
                                            id: id.clone(),
                                            name: info.Name()?.to_string(),
                                            address: 0,
                                            major_service_classes: Vec::new(),
                                            major_class: BluetoothMajorClass::Uncategorized,
                                            minor_class: BluetoothMinorClass::Uncategorized {
                                                unused: 0,
                                            },
                                            appearance: None,
                                            connected: false,
                                            paired,
                                            can_pair,
                                            can_disconnect: false,
                                            is_low_energy: true,
                                            battery_percentage: None,
                                        },
                                    );
                                    self.schedule_resolution(
                                        id.clone(),
                                        BluetoothDeviceType::LowEnergy,
                                    );
                                }
                            } else if let Ok(w) = BluetoothLEDeviceWrapper::create(id) {
                                self.le_devices.upsert(id.clone(), Arc::new(Mutex::new(w)));
                                self.discovery_devices.remove(id);
                            }
                        }
                    }
                }
            }
            BluetoothManagerEvent::DeviceUpdated(id, t) => match t {
                BluetoothDeviceType::Classic => {
                    if let Some(d) = self.devices.get(id) {
                        if self.should_emit_events.load(Ordering::Relaxed)
                            && self.should_refresh_device(id)
                        {
                            if let Ok(mut d) = d.lock() {
                                d.refresh_state().log_error();
                            }
                        }
                        self.discovery_devices.remove(id);
                    } else if self.discovery_devices.get(id).is_some()
                        && self.resolving_devices.get(id).is_none()
                    {
                        self.schedule_resolution(id.clone(), BluetoothDeviceType::Classic);
                    }
                }

                BluetoothDeviceType::LowEnergy => {
                    if let Some(d) = self.le_devices.get(id) {
                        if self.should_emit_events.load(Ordering::Relaxed)
                            && self.should_refresh_device(id)
                        {
                            if let Ok(mut d) = d.lock() {
                                d.refresh_state().log_error();
                            }
                        }
                        self.discovery_devices.remove(id);
                    } else if self.discovery_devices.get(id).is_some()
                        && self.resolving_devices.get(id).is_none()
                    {
                        self.schedule_resolution(id.clone(), BluetoothDeviceType::LowEnergy);
                    }
                }
            },
            BluetoothManagerEvent::DeviceRemoved(id, t) => match t {
                BluetoothDeviceType::Classic => {
                    self.devices.remove(id);
                    self.discovery_devices.remove(id);
                    self.last_device_info_query.remove(id);
                    self.last_refresh.remove(id);
                }
                BluetoothDeviceType::LowEnergy => {
                    self.le_devices.remove(id);
                    self.discovery_devices.remove(id);
                    self.last_device_info_query.remove(id);
                    self.last_refresh.remove(id);
                }
            },
        }
        // log::info!(
        //     "[BT][State] Classic={}, LE={}",
        //     self.devices.len(),
        //     self.le_devices.len()
        // );

        self.request_emit_devices_snapshot(immediate_emit);
        Ok(())
    }

    fn request_emit_devices_snapshot(&self, immediate: bool) {
        if !self.should_emit_events.load(Ordering::Relaxed) {
            return;
        }

        const MIN_INTERVAL_MS: u64 = 250;
        let now = current_millis();
        let last = self.last_emit_ms.load(Ordering::Relaxed);

        if immediate || now.saturating_sub(last) >= MIN_INTERVAL_MS {
            self.last_emit_ms.store(now, Ordering::Relaxed);
            self.emit_devices_snapshot();
            return;
        }

        if self.emit_scheduled.swap(true, Ordering::AcqRel) {
            return;
        }

        let delay = MIN_INTERVAL_MS.saturating_sub(now.saturating_sub(last));
        let _ = std::thread::Builder::new()
            .name("bt_emit_delay".into())
            .spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(delay));
                let manager = BluetoothManager::instance();

                if manager.should_emit_events.load(Ordering::Relaxed) {
                    let now = current_millis();
                    let last = manager.last_emit_ms.load(Ordering::Relaxed);
                    if now.saturating_sub(last) >= MIN_INTERVAL_MS {
                        manager.last_emit_ms.store(now, Ordering::Relaxed);
                        manager.emit_devices_snapshot();
                    }
                }

                manager.emit_scheduled.store(false, Ordering::Release);
            });
    }
    fn emit_devices_snapshot(&self) {
        let devices = self.get_all_devices();
        let _ = get_app_handle().emit(
            libs_core::handlers::FuncEvent::SystemBluetoothDevicesChanged,
            devices,
        );
    }
    pub fn get_all_devices(&self) -> Vec<BluetoothDevice> {
        let mut candidates = Vec::new();

        for d in self.devices.values() {
            if let Ok(d) = d.lock() {
                candidates.push(d.snapshot());
            }
        }

        for d in self.le_devices.values() {
            if let Ok(d) = d.lock() {
                candidates.push(d.snapshot());
            }
        }
        // le_devices 被 close+clear 后，使用缓存的快照数据（避免面板重新打开时闪烁）
        for d in self.le_devices_cache.values() {
            candidates.push(d.clone());
        }
        for d in self.discovery_devices.values() {
            candidates.push(d.clone());
        }

        fn score(device: &BluetoothDevice) -> i32 {
            let mut s = 0;
            if device.paired {
                s += 100;
            }
            if device.connected {
                s += 50;
            }
            if device.appearance.is_some() {
                s += 10;
            }
            if !matches!(device.major_class, BluetoothMajorClass::Uncategorized) {
                s += 10;
            }
            if !device.name.is_empty() {
                s += 1;
            }
            s
        }

        let mut device_map: HashMap<String, BluetoothDevice> = HashMap::new();
        for device in candidates {
            let id = device.id.clone();
            if let Some(existing) = device_map.get(&id) {
                if score(&device) > score(existing) {
                    device_map.insert(id, device);
                }
            } else {
                device_map.insert(id, device);
            }
        }

        device_map.into_values().collect()
    }

    pub fn start_scanning(&self) -> Result<()> {
        log::info!("[BT][Scan] start_scanning called");
        self.scan_session.fetch_add(1, Ordering::Relaxed);

        // 开启事件发送
        self.should_emit_events.store(true, Ordering::Relaxed);
        log::debug!("[BT][Scan] Event emission enabled");
        self.start_base_watchers()?;
        // 立即发送当前设备快照（已配对设备），确保打开面板时立刻显示
        self.emit_devices_snapshot();
        // 注意：不在此处清空 le_devices_cache
        // get_all_devices 的去重评分会自动优先使用 le_devices 中的实时数据
        // 缓存会在下次 stop_scanning 时被刷新
        // If already scanning, do nothing
        if self.discovery_classic_enumerator.load().is_some()
            || self.discovery_le_enumerator.load().is_some()
        {
            log::debug!("[BT][Scan] already scanning");
            return Ok(());
        }
        log::debug!("[BT][Scan] starting discovery...");

        // Start scanning for unpaired classic Bluetooth devices
        // 注意：GetDeviceSelectorFromPairingState(false) 只返回系统缓存的未配对设备
        // Windows 会通过 DeviceWatcher 事件持续推送新发现的设备
        let classic_selector = WinBluetoothDevice::GetDeviceSelectorFromPairingState(false)?;
        let discovery_classic_enumerator =
            DeviceEnumerator::new(classic_selector.to_string(), |event| {
                handle_device_event(event, BluetoothDeviceType::Classic)
            })?;
        discovery_classic_enumerator.start()?;
        self.discovery_classic_enumerator
            .store(Some(Arc::new(discovery_classic_enumerator)));

        // Start scanning for unpaired Bluetooth LE devices
        let le_selector = BluetoothLEDevice::GetDeviceSelectorFromPairingState(false)?;
        let discovery_le_enumerator = DeviceEnumerator::new(le_selector.to_string(), |event| {
            handle_device_event(event, BluetoothDeviceType::LowEnergy)
        })?;
        discovery_le_enumerator.start()?;
        self.discovery_le_enumerator
            .store(Some(Arc::new(discovery_le_enumerator)));

        Ok(())
    }

    fn prepare_pair_device(
        &self,
        device_id: &str,
    ) -> Result<(
        mpsc::Receiver<Result<DevicePairingNeededAction>>,
        DeviceInformationCustomPairing,
        DevicePairingProtectionLevel,
    )> {
        log::info!("Preparing to pair device {}", device_id);
        let device = DeviceInformation::CreateFromIdAsync(&device_id.into())?.get()?;
        let pairing = device.Pairing()?;
        if pairing.IsPaired()? {
            return Err("Device is already paired".into());
        }
        if !pairing.CanPair()? {
            return Err("Device cannot be paired".into());
        }
        let protection_level = pairing.ProtectionLevel()?;
        let pair_handler = pairing.Custom()?;
        let (tx, rx) = mpsc::channel::<Result<DevicePairingNeededAction>>(1);
        // Setup the pairing requested event handler
        let device_id_clone = device_id.to_string();
        let event_token =
            pair_handler.PairingRequested(&TypedEventHandler::new(move |_sender, args| {
                log::trace!("Pairing requested for device {}", device_id_clone);
                let result = Self::on_pair_request(args, device_id_clone.clone());
                let tx = tx.clone();
                get_tokio_handle().spawn(async move {
                    if let Err(e) = tx.send(result).await {
                        log::error!("Failed to send pairing result: {:?}", e);
                    }
                });
                Ok(())
            }))?;

        // Create initial pending request (will be updated by callback and after pairing)
        self.pending_pair_requests.upsert(
            device_id.to_string(),
            Arc::new(std::sync::Mutex::new(PendingPairRequest {
                handler: pair_handler.clone(),
                event_token,
                action: DevicePairingNeededAction::None,
                async_operation: None,
                request: None,
                deferral: None,
            })),
        );
        Ok((rx, pair_handler, protection_level))
    }

    pub async fn request_pair_device(&self, device_id: &str) -> Result<DevicePairingNeededAction> {
        log::info!("Requesting pairing for device {}", device_id);
        // Prepare the device for pairing
        let (rx, pair_handler, protection_level) = self.prepare_pair_device(device_id)?;
        // Start pairing and handle cleanup on failure
        match self
            .start_pairing(device_id, rx, pair_handler, protection_level)
            .await
        {
            Ok(action) => Ok(action),
            Err(e) => {
                // Clean up pending request if pairing failed
                self.pending_pair_requests.remove(&device_id.to_string());
                Err(e)
            }
        }
    }
    async fn start_pairing(
        &self,
        device_id: &str,
        mut rx: mpsc::Receiver<Result<DevicePairingNeededAction>>,
        pair_handler: DeviceInformationCustomPairing,
        protection_level: DevicePairingProtectionLevel,
    ) -> Result<DevicePairingNeededAction> {
        log::trace!("Starting pairing for device {}", device_id);
        // Start the pairing async operation (but don't await it yet to avoid deadlock)
        // The operation will call the PairingRequested callback, which will send us the action
        let pair_async_op = pair_handler.PairWithProtectionLevelAsync(
            DevicePairingKinds::ConfirmOnly
                | DevicePairingKinds::DisplayPin
                | DevicePairingKinds::ProvidePin
                | DevicePairingKinds::ConfirmPinMatch
                | DevicePairingKinds::ProvidePasswordCredential
                | DevicePairingKinds::ProvideAddress,
            protection_level,
        )?;
        // Wait for the callback to determine what action is needed
        // This must happen BEFORE we await the pairing result to avoid deadlock
        // (the callback creates a Deferral that pauses the pairing operation)
        let action = tokio::time::timeout(
            std::time::Duration::from_secs(PAIRING_REQUEST_TIMEOUT_SECS),
            rx.recv(),
        )
        .await
        .map_err(|_| {
            format!(
                "Pairing request timed out after {} seconds",
                PAIRING_REQUEST_TIMEOUT_SECS
            )
        })?
        .ok_or("Pairing channel closed unexpectedly")??;
        // If no valid action is needed, pairing cannot proceed
        if action == DevicePairingNeededAction::None {
            return Err("Device pairing requires unsupported action".into());
        }
        // Store the async operation for later (will be awaited after user confirmation)
        let key = device_id.to_string();
        if let Some(pending) = self.pending_pair_requests.get(&key) {
            let mut pending = pending.lock().unwrap();
            pending.async_operation = Some(pair_async_op);
        }
        Ok(action)
    }
    pub fn stop_scanning(&self) -> Result<()> {
        log::debug!("[BT][Scan] stop_scanning called");
        self.scan_session.fetch_add(1, Ordering::Relaxed);
        // 1 关闭事件
        self.should_emit_events.store(false, Ordering::Relaxed);
        // 2 停止 watcher
        if let Some(w) = self.discovery_classic_enumerator.swap(None) {
            let _ = w.stop();
            log::info!("[BT][DBG] stopping discovery classic watcher");
        }
        if let Some(w) = self.discovery_le_enumerator.swap(None) {
            let _ = w.stop();
            log::info!("[BT][DBG] stopping discovery le watcher");
        }

        {
            let mut classic = self.classic_enumerator.lock().unwrap();
            if let Some(w) = classic.take() {
                let _ = w.stop();
            }
        }
        {
            let mut le = self.le_enumerator.lock().unwrap();
            if let Some(w) = le.take() {
                let _ = w.stop();
            }
        }
        self.should_emit_events.store(false, Ordering::Relaxed);
        // 不要 clear devices / le_devices
        // self.le_devices.clear();
        // self.devices.clear();

        // 3 清 discovery 缓存即可
        self.discovery_devices.clear();
        self.resolving_devices.clear();
        // 释放 BLE 设备的 WinRT 底层资源（否则 dasHost 会持续跟踪）
        // 先将已配对 LE 设备的快照保存到缓存，下次打开面板时可立即显示
        self.le_devices_cache.clear();
        for dev in self.le_devices.values() {
            if let Ok(mut d) = dev.lock() {
                if d.state.paired {
                    self.le_devices_cache.upsert(d.id.clone(), d.snapshot());
                }
                let _ = d.close();
            }
        }
        self.le_devices.clear();
        self.last_device_info_query.clear();
        self.last_refresh.clear();
        self.active_resolutions.store(0, Ordering::Relaxed);
        log::info!("[BT][Scan] watchers stopped");

        Ok(())
    }

    fn on_pair_request(
        request: &Option<DevicePairingRequestedEventArgs>,
        device_id: String,
    ) -> Result<DevicePairingNeededAction> {
        let Some(request) = request else {
            return Err(format!("Pairing args are null for device {}", device_id).into());
        };
        let kind = request.PairingKind()?;
        log::trace!("Pairing kind for device {}: {:?}", device_id, kind);
        // Determine what action is needed from the user based on pairing kind
        let action = match kind {
            DevicePairingKinds::None => DevicePairingNeededAction::None,
            DevicePairingKinds::ConfirmOnly => DevicePairingNeededAction::ConfirmOnly,
            DevicePairingKinds::DisplayPin => {
                let pin = request.Pin()?.to_string();
                DevicePairingNeededAction::DisplayPin { pin }
            }

            DevicePairingKinds::ProvidePin => DevicePairingNeededAction::ProvidePin,
            DevicePairingKinds::ConfirmPinMatch => {
                let pin = request.Pin()?.to_string();
                DevicePairingNeededAction::ConfirmPinMatch { pin }
            }
            DevicePairingKinds::ProvidePasswordCredential => {
                DevicePairingNeededAction::ProvidePasswordCredential
            }
            DevicePairingKinds::ProvideAddress => DevicePairingNeededAction::ProvideAddress,
            _ => {
                log::warn!("Unsupported pairing kind for device {device_id}: {kind:?}");
                DevicePairingNeededAction::None
            }
        };
        let _key = device_id.to_string();
        if action != DevicePairingNeededAction::None {
            let key = device_id.to_string();
            if let Some(pending) = Self::instance().pending_pair_requests.get(&key) {
                let mut pending = pending.lock().unwrap();
                pending.action = action.clone();
                pending.request = Some(request.clone());
                // The deferral makes the pairing operation wait until user confirmation
                pending.deferral = request.GetDeferral().ok();
            }
        }
        Ok(action)
    }

    async fn is_device_paired(&self, device_id: &String) -> bool {
        // LE 设备
        if self.le_devices.get(device_id).is_some() {
            return true;
        }

        // Classic 设备（统一 devices 容器）
        if self.devices.get(device_id).is_some() {
            return true;
        }

        false
    }

    fn on_device_paired(&self, device_id: &String) {
        log::info!(
            "[BT][Pairing] Device {} paired successfully, triggering state refresh",
            device_id
        );

        let device_type = if self.le_devices.get(device_id).is_some() {
            BluetoothDeviceType::LowEnergy
        } else {
            BluetoothDeviceType::Classic
        };

        Self::send(BluetoothManagerEvent::DeviceUpdated(
            device_id.clone(),
            device_type,
        ));
    }

    pub async fn confirm_device_pairing(
        &self,
        device_id: &String,
        answer: DevicePairingAnswer,
    ) -> Result<DevicePairingResultStatus> {
        // 取出 pending pairing 请求
        let Some(pending) = self.pending_pair_requests.remove(device_id) else {
            return Err(format!("No pending pairing request for device {device_id}").into());
        };

        // 提取 event args（WinRT 对象）
        let event_args = {
            let guard = pending
                .lock()
                .map_err(|_| "Pending pairing request mutex poisoned")?;
            guard
                .request
                .as_ref()
                .ok_or("Pairing args are null")?
                .clone()
        };

        // 提取 async operation（WinRT 对象）
        let async_op = {
            let mut guard = pending
                .lock()
                .map_err(|_| "Pending pairing request mutex poisoned")?;
            guard
                .async_operation
                .take()
                .ok_or("Pairing async operation is null")?
        };

        // 应用用户的配对确认
        if answer.accept {
            if let Some(pin) = answer.pin {
                event_args.AcceptWithPin(&pin.into())?;
            } else if let Some(address) = answer.address {
                event_args.AcceptWithAddress(&address.into())?;
            } else {
                event_args.Accept()?;
            }
        }

        // 关键：立即释放 pending（完成 deferral）
        drop(pending);

        // 等待配对操作完成
        let pairing_result = async_op.await?;

        // 只调用一次 Status()
        let initial_status = pairing_result.Status()?;
        log::info!(
            "[BT][Pairing] Initial pairing result for {}: {:?}",
            device_id,
            initial_status
        );

        // 如果用户拒绝，直接返回
        if !answer.accept {
            return Ok(initial_status);
        }

        // 如果已经是 Paired，直接成功
        if initial_status == DevicePairingResultStatus::Paired {
            self.on_device_paired(device_id);
            return Ok(initial_status);
        }

        // 等待设备真正进入 Paired 状态（不再访问 WinRT pairing_result）
        let mut paired = false;
        for i in 0..PAIRING_CONFIRMATION_MAX_RETRIES {
            if self.is_device_paired(device_id).await {
                log::info!(
                    "[BT][Pairing] Device {} paired after {} attempts",
                    device_id,
                    i + 1
                );
                paired = true;
                break;
            }

            tokio::time::sleep(std::time::Duration::from_millis(
                PAIRING_CONFIRMATION_RETRY_INTERVAL_MS,
            ))
            .await;
        }

        if paired {
            self.on_device_paired(device_id);
            Ok(DevicePairingResultStatus::Paired)
        } else {
            log::warn!(
                "[BT][Pairing] Device {} did not reach paired state, initial status: {:?}",
                device_id,
                initial_status
            );
            Ok(initial_status)
        }
    }

    pub fn disconnect_device(&self, device_id: &str) -> Result<()> {
        let key = device_id.to_string();

        // Classic 设备：使用 IOCTL_BTH_DISCONNECT_DEVICE 强制断开
        if let Some(device) = self.devices.get(&key) {
            let bth_addr = {
                let guard = device
                    .lock()
                    .map_err(|_| "Bluetooth device mutex poisoned")?;
                guard.raw.BluetoothAddress()?
            };
            crate::modules::bluetooth::classic::disconnect_classic_by_address(bth_addr)?;
            // 断开后触发状态更新
            Self::send(BluetoothManagerEvent::DeviceUpdated(
                key.clone(),
                BluetoothDeviceType::Classic,
            ));
            return Ok(());
        }

        // LE 设备：关闭 WinRT 对象触发断开
        if let Some(device) = self.le_devices.get(&key) {
            let mut guard = device
                .lock()
                .map_err(|_| "Bluetooth LE device mutex poisoned")?;
            guard.close()?;
            drop(guard);
            // 断开后触发状态更新
            Self::send(BluetoothManagerEvent::DeviceUpdated(
                key.clone(),
                BluetoothDeviceType::LowEnergy,
            ));
            return Ok(());
        }

        Err(format!("Device not found: {}", device_id).into())
    }

    pub fn connect_device(&self, device_id: &str) -> Result<()> {
        let key = device_id.to_string();

        // Classic 设备：通过 BluetoothSetServiceState 启用 A2DP/HFP 服务触发实际连接
        if let Some(classic) = self.devices.get(&key) {
            let bth_addr = {
                let guard = classic
                    .lock()
                    .map_err(|_| "Bluetooth device mutex poisoned")?;
                guard.raw.BluetoothAddress()?
            };
            log::info!(
                "[BT][Connect] Connecting Classic device via BluetoothSetServiceState: {}",
                device_id
            );
            crate::modules::bluetooth::classic::connect_classic_by_address(bth_addr)?;
            // 连接命令发出后触发状态更新
            Self::send(BluetoothManagerEvent::DeviceUpdated(
                key.clone(),
                BluetoothDeviceType::Classic,
            ));
            return Ok(());
        }

        // LE 设备：重新创建 WinRT 对象触发连接
        if let Some(le) = self.le_devices.get(&key) {
            let le = le.lock().unwrap();
            log::info!(
                "[BT][Connect] Attempting to connect LE device: {}",
                device_id
            );
            drop(le);
            Self::send(BluetoothManagerEvent::DeviceUpdated(
                key.clone(),
                BluetoothDeviceType::LowEnergy,
            ));
            return Ok(());
        }

        if let Ok(op) = WinBluetoothDevice::FromIdAsync(&device_id.into()) {
            if let Ok(bt_device) = op.get() {
                if let Ok(bth_addr) = bt_device.BluetoothAddress() {
                    log::info!(
                        "[BT][Connect] Fallback connect Classic by id: {}",
                        device_id
                    );
                    crate::modules::bluetooth::classic::connect_classic_by_address(bth_addr)?;
                    Self::send(BluetoothManagerEvent::DeviceUpdated(
                        key.clone(),
                        BluetoothDeviceType::Classic,
                    ));
                    return Ok(());
                }
            }
        }

        if let Ok(op) = BluetoothLEDevice::FromIdAsync(&device_id.into()) {
            if let Ok(_le_device) = op.get() {
                log::info!("[BT][Connect] Fallback connect LE by id: {}", device_id);
                Self::send(BluetoothManagerEvent::DeviceUpdated(
                    key.clone(),
                    BluetoothDeviceType::LowEnergy,
                ));
                return Ok(());
            }
        }

        Err(format!("Device not found: {}", device_id).into())
    }

    pub fn forget_device(&self, device_id: &str) -> Result<()> {
        let key = device_id.to_string();
        log::info!("[BT][Unpair] Starting to unpair device: {}", device_id);

        // 确定设备类型（用于后续的事件通知）
        let device_type = if self.devices.get(&key).is_some() {
            Some(BluetoothDeviceType::Classic)
        } else if self.le_devices.get(&key).is_some() {
            Some(BluetoothDeviceType::LowEnergy)
        } else {
            None
        };

        // 直接通过设备ID创建 DeviceInformation，不依赖内存中的设备
        // 这样即使扫描停止、设备从内存中移除，也能取消配对
        let device = DeviceInformation::CreateFromIdAsync(&device_id.into())?.get()?;

        let unpair_op = device.Pairing()?.UnpairAsync()?;
        // 在同步上下文中等待异步操作完成
        let status = unpair_op.get()?.Status()?;
        if status == DeviceUnpairingResultStatus::AccessDenied
            || status == DeviceUnpairingResultStatus::Failed
        {
            return Err("Unpair was not successful!".into());
        }

        // 取消配对成功后，从内存中移除设备（如果存在）
        log::info!(
            "[BT][Unpair] Device {} unpaired successfully, removing from memory",
            device_id
        );
        self.devices.remove(&key);
        self.le_devices.remove(&key);

        // 触发设备移除事件，立即通知前端（如果事件发送已开启）
        if let Some(device_type) = device_type {
            Self::send(BluetoothManagerEvent::DeviceRemoved(
                key.clone(),
                device_type,
            ));
        }

        Ok(())
    }
}

pub struct PendingPairRequest {
    /// Custom pairing interface for the device
    pub handler: DeviceInformationCustomPairing,
    /// Event handler token to remove on cleanup
    pub event_token: i64,
    /// Action required from the user
    pub action: DevicePairingNeededAction,
    /// Pairing async operation (to be awaited after user confirmation)
    pub async_operation: Option<IAsyncOperation<DevicePairingResult>>,
    /// Event arguments from the pairing callback
    pub request: Option<DevicePairingRequestedEventArgs>,
    /// Deferral to control async pairing flow (present if user input is needed)
    pub deferral: Option<Deferral>,
}

impl Drop for PendingPairRequest {
    fn drop(&mut self) {
        if let Some(deferral) = &self.deferral {
            let _ = deferral.Complete();
        }
        let _ = self.handler.RemovePairingRequested(self.event_token);
    }
}
