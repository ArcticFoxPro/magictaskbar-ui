use crate::app::get_app_handle;
use crate::cli::ServicePipe;
use crate::error::Result;
use libs_core::handlers::FuncEvent;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tauri::Emitter;
#[cfg(feature = "gen-binds")]
use ts_rs::TS;
use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioState};
use windows::Foundation::TypedEventHandler;

mod classic;
mod infrastructure;
mod low_energy;

pub use classic::BluetoothDeviceWrapper;
pub use infrastructure::BluetoothManager;
pub use low_energy::BluetoothLEDeviceWrapper;

use serde::{Deserialize, Serialize};
use slu_ipc::messages::SvcAction;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Devices::Enumeration::DevicePairingResultStatus;

static BT_ENABLED: AtomicBool = AtomicBool::new(false);

// 保存蓝牙 Radio 设备和事件注册 token
static BLUETOOTH_RADIO: LazyLock<Mutex<Option<(Radio, i64)>>> = LazyLock::new(|| Mutex::new(None));

pub fn set_cached_bluetooth_enabled(enabled: bool) {
    BT_ENABLED.store(enabled, Ordering::SeqCst);
}

fn service_backend_command_blocking(
    command: &str,
    args: serde_json::Value,
) -> Result<Option<serde_json::Value>> {
    let data = ServicePipe::request_with_response_blocking(
        SvcAction::ExecuteBackendCommand {
            command: command.to_string(),
            args,
        },
        Duration::from_secs(2),
    )?;

    match data {
        Some(data) if !data.trim().is_empty() => Ok(Some(serde_json::from_str(&data)?)),
        _ => Ok(None),
    }
}

fn service_backend_bool_command_blocking(command: &str, default: bool) -> Result<bool> {
    Ok(
        service_backend_command_blocking(command, serde_json::json!({}))?
            .and_then(|value| value.get("value").and_then(|value| value.as_bool()))
            .unwrap_or(default),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BluetoothDeviceType {
    Classic,
    LowEnergy,
}

#[derive(Debug, Clone)]
pub enum BluetoothManagerEvent {
    DeviceAdded(String, BluetoothDeviceType),
    DeviceUpdated(String, BluetoothDeviceType),
    DeviceRemoved(String, BluetoothDeviceType),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BluetoothDevice {
    pub id: String,
    pub name: String,
    pub address: u64,
    pub major_service_classes: Vec<BluetoothMajorServiceClass>,
    pub major_class: BluetoothMajorClass,
    pub minor_class: BluetoothMinorClass,
    pub appearance: Option<u16>,
    pub connected: bool,
    pub paired: bool,
    pub can_pair: bool,
    pub can_disconnect: bool,
    pub is_low_energy: bool,
    pub battery_percentage: Option<u8>, // 电量百分比
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "gen-binds", derive(TS))]
#[cfg_attr(feature = "gen-binds", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct DevicePairingAnswer {
    pub accept: bool,
    pub pin: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BluetoothMajorClass {
    Uncategorized,
    Miscellaneous,
    Computer,
    Phone,
    NetworkAccessPoint,
    AudioVideo,
    Peripheral,
    Imaging,
    Wearable,
    Toy,
    Health,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BluetoothMinorClass {
    Uncategorized { unused: u8 },
    Computer(BluetoothComputerMinor),
    Phone(BluetoothPhoneMinor),
    AudioVideo(BluetoothAudioVideoMinor),
    Peripheral(BluetoothPeripheralMinor, BluetoothPeripheralSubMinor),
    Wearable(BluetoothWearableMinor),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum BluetoothComputerMinor {
    Uncategorized,
    DesktopWorkstation,
    ServerClassComputer,
    Laptop,
    HandheldPcPda,
    PalmSizePcPda,
    WearableComputer,
    Tablet,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum BluetoothPhoneMinor {
    Uncategorized,
    Cellular,
    Cordless,
    Smartphone,
    WiredModemOrVoiceGateway,
    CommonIsdnAccess,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum BluetoothAudioVideoMinor {
    Uncategorized,
    Headset,
    HandsFree,
    Microphone,
    Loudspeaker,
    Headphones,
    PortableAudio,
    CarAudio,
    SetTopBox,
    HiFiAudioDevice,
    Vcr,
    VideoCamera,
    Camcorder,
    VideoMonitor,
    VideoDisplayAndLoudspeaker,
    VideoConferencing,
    GamingToy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum BluetoothPeripheralMinor {
    Uncategorized,
    Keyboard,
    Pointing,
    ComboKeyboardPointing,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum BluetoothPeripheralSubMinor {
    Uncategorized,
    Joystick,
    Gamepad,
    RemoteControl,
    SensingDevice,
    DigitizerTablet,
    CardReader,
    DigitalPen,
    HandheldScanner,
    HandheldGesture,
}

impl From<u8> for BluetoothPeripheralSubMinor {
    fn from(value: u8) -> Self {
        match value {
            1 => Self::Joystick,
            2 => Self::Gamepad,
            3 => Self::RemoteControl,
            4 => Self::SensingDevice,
            5 => Self::DigitizerTablet,
            6 => Self::CardReader,
            7 => Self::DigitalPen,
            8 => Self::HandheldScanner,
            9 => Self::HandheldGesture,
            _ => Self::Uncategorized,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum BluetoothWearableMinor {
    Uncategorized,
    Wristwatch,
    Pager,
    Jacket,
    Helmet,
    Glasses,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[repr(u32)]
pub enum BluetoothMajorServiceClass {
    LimitedDiscoverableMode = 0x0001,
    Positioning = 0x0008,
    Networking = 0x0010,
    Rendering = 0x0020,
    Capturing = 0x0040,
    ObjectTransfer = 0x0080,
    Audio = 0x0100,
    Telephony = 0x0200,
    Information = 0x0400,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "gen-binds", derive(TS))]
#[cfg_attr(feature = "gen-binds", ts(export))]
#[serde(tag = "needs")]
pub enum DevicePairingNeededAction {
    None,
    ConfirmOnly,
    DisplayPin { pin: String },
    ProvidePin,
    ConfirmPinMatch { pin: String },
    ProvidePasswordCredential,
    ProvideAddress,
}

fn get_bluetooth_manager() -> &'static BluetoothManager {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        // 初始化蓝牙 Radio 状态监听
        if let Err(e) = initialize_bluetooth_radio_listener() {
            log::warn!("[蓝牙] 初始化蓝牙状态监听失败: {:?}", e);
        }
    });
    BluetoothManager::instance()
}

/// 初始化蓝牙 Radio 状态监听
fn initialize_bluetooth_radio_listener() -> Result<()> {
    log::info!("[Bluetooth] radio state events are forwarded by magictaskbar-srv");
    Ok(())
}

#[allow(dead_code)]
fn initialize_bluetooth_radio_listener_ui_legacy_disabled() -> Result<()> {
    use windows::Foundation::IAsyncOperation;

    // 获取所有 Radio 设备
    let radios_async: IAsyncOperation<windows::Foundation::Collections::IVectorView<Radio>> =
        Radio::GetRadiosAsync()?;
    let radios = radios_async.get()?;

    // 查找蓝牙 Radio
    for i in 0..radios.Size()? {
        let radio = radios.GetAt(i)?;
        let kind = radio.Kind()?;

        if kind == windows::Devices::Radios::RadioKind::Bluetooth {
            // 注册状态变化事件
            let state_changed_token = radio.StateChanged(&TypedEventHandler::new(
                move |sender: &Option<Radio>, _args: &Option<windows_core::IInspectable>| {
                    if let Some(sender) = sender {
                        if let Ok(state) = sender.State() {
                            let enabled = state == RadioState::On;
                            log::info!("[蓝牙] 系统蓝牙状态变化: {}", enabled);

                            // 更新缓存状态
                            BT_ENABLED.store(enabled, Ordering::SeqCst);

                            // 发送状态变化事件到前端
                            let _ = get_app_handle().emit(
                                FuncEvent::SystemBluetoothStateChanged,
                                serde_json::json!({ "enabled": enabled }),
                            );

                            // 管理扫描状态
                            let manager = BluetoothManager::instance();
                            if enabled {
                                let _ = manager.start_scanning();
                            } else {
                                let _ = manager.stop_scanning();
                            }
                        }
                    }
                    Ok(())
                },
            ))?;

            // 保存 Radio 和 token
            if let Ok(mut guard) = BLUETOOTH_RADIO.lock() {
                *guard = Some((radio, state_changed_token));
            }

            log::info!("[蓝牙] 蓝牙状态监听初始化成功");
            return Ok(());
        }
    }

    log::warn!("[蓝牙] 未找到蓝牙适配器");
    Ok(())
}

#[tauri::command]
pub fn system_get_bluetooth_devices() -> Vec<BluetoothDevice> {
    get_bluetooth_manager().get_all_devices()
}

#[tauri::command]
pub fn system_get_bluetooth_enabled() -> bool {
    match service_backend_bool_command_blocking("system_get_bluetooth_enabled", false) {
        Ok(enabled) => {
            BT_ENABLED.store(enabled, Ordering::SeqCst);
            return enabled;
        }
        Err(e) => {
            log::warn!(
                "[Bluetooth] srv get bluetooth enabled failed, fallback to UI Radio API: {:?}",
                e
            );
        }
    }

    // 尝试从 Windows Radio API 获取真实的蓝牙状态
    match get_bluetooth_radio_state() {
        Ok(enabled) => {
            // 同步更新缓存状态
            BT_ENABLED.store(enabled, Ordering::SeqCst);
            enabled
        }
        Err(e) => {
            log::warn!("[蓝牙] 无法获取蓝牙适配器状态: {:?}", e);
            // 如果无法获取，返回缓存值
            BT_ENABLED.load(Ordering::SeqCst)
        }
    }
}

/// 从 Windows Radio API 获取蓝牙适配器状态
fn get_bluetooth_radio_state() -> Result<bool> {
    use windows::Foundation::IAsyncOperation;

    if let Ok(guard) = BLUETOOTH_RADIO.lock() {
        if let Some((radio, _token)) = guard.as_ref() {
            let state = radio.State()?;
            return Ok(state == RadioState::On);
        }
    }

    // 获取所有 Radio 设备
    let radios_async: IAsyncOperation<windows::Foundation::Collections::IVectorView<Radio>> =
        Radio::GetRadiosAsync()?;
    let radios = radios_async.get()?;

    // 查找蓝牙 Radio
    for i in 0..radios.Size()? {
        let radio = radios.GetAt(i)?;
        let kind = radio.Kind()?;

        // 检查是否是蓝牙设备
        if kind == windows::Devices::Radios::RadioKind::Bluetooth {
            let state = radio.State()?;
            return Ok(state == RadioState::On);
        }
    }

    // 没有找到蓝牙适配器
    Ok(false)
}

#[tauri::command]
pub fn system_set_bluetooth_enabled(enabled: bool) -> Result<()> {
    let radio_updated_by_srv = match service_backend_command_blocking(
        "system_set_bluetooth_enabled",
        serde_json::json!({ "enabled": enabled }),
    ) {
        Ok(_) => true,
        Err(e) => {
            log::warn!(
                "[Bluetooth] srv set bluetooth enabled failed, fallback to UI Radio API: {:?}",
                e
            );
            false
        }
    };

    // 先更新缓存状态
    BT_ENABLED.store(enabled, Ordering::SeqCst);

    // 尝试设置真实的蓝牙状态
    if !radio_updated_by_srv {
        if let Err(e) = set_bluetooth_radio_state(enabled) {
            log::error!("[蓝牙] 设置蓝牙状态失败: {:?}", e);
        }
    }

    // 管理扫描状态
    let manager = BluetoothManager::instance();
    if enabled {
        manager.start_scanning()?;
    } else {
        manager.stop_scanning()?;
    }

    // 发送状态变化事件
    let _ = get_app_handle().emit(FuncEvent::SystemBluetoothStateChanged, enabled);
    Ok(())
}

/// 设置蓝牙适配器状态
fn set_bluetooth_radio_state(enabled: bool) -> Result<()> {
    use windows::Foundation::IAsyncOperation;

    // 检查权限
    if Radio::RequestAccessAsync()?.get()? != RadioAccessStatus::Allowed {
        log::warn!("[蓝牙] 没有权限控制蓝牙适配器");
        return Ok(());
    }

    // 获取所有 Radio 设备
    let radios_async: IAsyncOperation<windows::Foundation::Collections::IVectorView<Radio>> =
        Radio::GetRadiosAsync()?;
    let radios = radios_async.get()?;

    let state = if enabled {
        RadioState::On
    } else {
        RadioState::Off
    };

    // 设置所有蓝牙 Radio 的状态
    for i in 0..radios.Size()? {
        let radio = radios.GetAt(i)?;
        let kind = radio.Kind()?;

        if kind == windows::Devices::Radios::RadioKind::Bluetooth {
            log::info!("[蓝牙] 设置蓝牙状态: {}", enabled);
            radio.SetStateAsync(state)?.get()?;
        }
    }

    Ok(())
}

#[tauri::command]
pub fn start_bluetooth_scanning() -> Result<()> {
    BluetoothManager::instance().start_scanning()
}

#[tauri::command]
pub fn stop_bluetooth_scanning() -> Result<()> {
    BluetoothManager::instance().stop_scanning()
}

#[tauri::command(async)]
pub async fn request_pair_bluetooth_device(id: String) -> Result<DevicePairingNeededAction> {
    log::info!("Requesting pairing for device {}", id);
    let manager = get_bluetooth_manager();
    manager.request_pair_device(&id).await
}

#[tauri::command(async)]
pub async fn confirm_bluetooth_device_pairing(
    id: String,
    answer: DevicePairingAnswer,
) -> Result<()> {
    let expected_status = if answer.accept {
        DevicePairingResultStatus::Paired
    } else {
        DevicePairingResultStatus::RejectedByHandler
    };

    let manager = get_bluetooth_manager();
    let status = manager.confirm_device_pairing(&id, answer).await?;

    if status != expected_status {
        return Err(
            format!("Pairing action was not successful! Current status: {status:?}").into(),
        );
    }
    Ok(())
}
#[tauri::command(async)]
pub fn disconnect_bluetooth_device(id: String) -> Result<()> {
    // 在独立线程中执行阻塞操作，避免阻塞 tokio 运行时
    let id_clone = id.clone();
    std::thread::spawn(move || {
        let manager = BluetoothManager::instance();
        if let Err(e) = manager.disconnect_device(&id_clone) {
            log::error!("[BT][Disconnect] 断开设备失败: {:?}", e);
        }
    });
    Ok(())
}

#[tauri::command(async)]
pub fn connect_bluetooth_device(id: String) -> Result<()> {
    // 在独立线程中执行阻塞操作（含 sleep），避免阻塞 tokio 运行时
    let id_clone = id.clone();
    std::thread::spawn(move || {
        let manager = BluetoothManager::instance();
        if let Err(e) = manager.connect_device(&id_clone) {
            log::error!("[BT][Connect] 连接设备失败: {:?}", e);
        }
    });
    Ok(())
}

#[tauri::command]
pub fn forget_bluetooth_device(id: String) -> Result<()> {
    std::thread::spawn(move || {
        let manager = get_bluetooth_manager();
        if let Err(e) = manager.forget_device(&id) {
            log::error!("[BT][Unpair] 取消配对失败: {:?}", e);
        }
    });
    Ok(())
}
