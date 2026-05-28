use windows::Win32::Devices::Bluetooth::{
    BluetoothFindDeviceClose, BluetoothFindFirstDevice, BluetoothFindFirstRadio,
    BluetoothFindNextDevice, BluetoothFindRadioClose, BluetoothSetServiceState,
    BLUETOOTH_DEVICE_INFO, BLUETOOTH_DEVICE_SEARCH_PARAMS, BLUETOOTH_FIND_RADIO_PARAMS,
    BLUETOOTH_SERVICE_DISABLE, BLUETOOTH_SERVICE_ENABLE,
};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::IO::DeviceIoControl;
use windows::{
    Devices::Bluetooth::BluetoothDevice as WindowsBluetoothDevice, Foundation::TypedEventHandler,
};
use windows_core::GUID;

/// IOCTL_BTH_DISCONNECT_DEVICE = CTL_CODE(FILE_DEVICE_BLUETOOTH=0x41, 0x03, METHOD_BUFFERED=0, FILE_ANY_ACCESS=0)
const IOCTL_BTH_DISCONNECT_DEVICE: u32 = (0x41_u32 << 16) | (0x03 << 2);

/// A2DP Sink Profile GUID (Audio distribution, 0x0000110B)
const GUID_A2DP: GUID = GUID::from_values(
    0x0000110B,
    0x0000,
    0x1000,
    [0x80, 0x00, 0x00, 0x80, 0x5F, 0x9B, 0x34, 0xFB],
);
/// HFP (Hands-Free Profile) GUID (0x0000111E)
const GUID_HFP: GUID = GUID::from_values(
    0x0000111E,
    0x0000,
    0x1000,
    [0x80, 0x00, 0x00, 0x80, 0x5F, 0x9B, 0x34, 0xFB],
);

use crate::{
    error::Result,
    modules::bluetooth::{
        BluetoothDevice, BluetoothDeviceType, BluetoothMajorClass, BluetoothManager,
        BluetoothManagerEvent, BluetoothMinorClass,
    },
};

pub struct BluetoothDeviceWrapper {
    pub(super) id: String,
    pub(super) raw: WindowsBluetoothDevice,
    pub(super) state: BluetoothDevice,

    name_changed_token: i64,
    connection_status_changed_token: i64,
}

impl BluetoothDeviceWrapper {
    /// 创建并注册事件
    pub fn create(device_id: &str) -> Result<Self> {
        let device = WindowsBluetoothDevice::FromIdAsync(&device_id.into())?.get()?;
        let id = device_id.to_owned();

        // 名称变化
        let name_changed_token = device.NameChanged(&TypedEventHandler::new({
            let id = id.clone();
            move |_src, _args| {
                BluetoothManager::send(BluetoothManagerEvent::DeviceUpdated(
                    id.clone(),
                    BluetoothDeviceType::Classic,
                ));
                Ok(())
            }
        }))?;

        // 连接状态变化
        let connection_status_changed_token =
            device.ConnectionStatusChanged(&TypedEventHandler::new({
                let id = id.clone();
                move |_src, _args| {
                    BluetoothManager::send(BluetoothManagerEvent::DeviceUpdated(
                        id.clone(),
                        BluetoothDeviceType::Classic,
                    ));
                    Ok(())
                }
            }))?;

        let state = to_serializable(&id, &device)?;

        Ok(Self {
            id,
            raw: device,
            state,
            name_changed_token,
            connection_status_changed_token,
        })
    }

    pub fn snapshot(&self) -> BluetoothDevice {
        use windows::Devices::Bluetooth::BluetoothConnectionStatus;

        let mut snapshot = self.state.clone();
        if let Ok(status) = self.raw.ConnectionStatus() {
            snapshot.connected = status == BluetoothConnectionStatus::Connected;
        }
        if let Ok(is_paired) = self
            .raw
            .DeviceInformation()
            .and_then(|info| info.Pairing())
            .and_then(|pairing| pairing.IsPaired())
        {
            snapshot.paired = is_paired;
        }
        snapshot
    }
    /// 重新同步当前状态
    pub fn refresh_state(&mut self) -> Result<()> {
        self.state = to_serializable(&self.id, &self.raw)?;
        Ok(())
    }

    pub fn disconnect(&self) -> Result<()> {
        let bth_addr = self.raw.BluetoothAddress()?;
        disconnect_classic_by_address(bth_addr)
    }

    pub fn close(&mut self) -> Result<()> {
        let _ = self.raw.RemoveNameChanged(self.name_changed_token);
        let _ = self
            .raw
            .RemoveConnectionStatusChanged(self.connection_status_changed_token);

        // 释放 WinRT 对象
        let _ = self.raw.Close();

        Ok(())
    }
}

/// 通过 Win32 IOCTL_BTH_DISCONNECT_DEVICE 断开 Classic 蓝牙设备
/// 参考逻辑：先查找蓝牙无线电模块，再发 IOCTL 命令断开指定地址的设备
pub fn disconnect_classic_by_address(bth_addr: u64) -> Result<()> {
    unsafe {
        // 1. 获取蓝牙无线电
        let mut h_radio = windows::Win32::Foundation::HANDLE::default();
        let radio_params = BLUETOOTH_FIND_RADIO_PARAMS {
            dwSize: std::mem::size_of::<BLUETOOTH_FIND_RADIO_PARAMS>() as u32,
        };
        let h_find = BluetoothFindFirstRadio(&radio_params, &mut h_radio)
            .map_err(|e| format!("BluetoothFindFirstRadio failed: {:?}", e))?;
        if h_radio.is_invalid() {
            return Err("No Bluetooth radio found".into());
        }
        // 只使用第一个 Radio
        let _ = BluetoothFindRadioClose(h_find);

        // 2. 发送 IOCTL 断开设备
        let mut bytes_returned: u32 = 0;
        let ok = DeviceIoControl(
            h_radio,
            IOCTL_BTH_DISCONNECT_DEVICE,
            Some(&bth_addr as *const u64 as *const std::ffi::c_void),
            std::mem::size_of::<u64>() as u32,
            None,
            0,
            Some(&mut bytes_returned),
            None,
        );

        let _ = CloseHandle(h_radio);

        if let Err(e) = ok {
            log::error!(
                "[BT][Disconnect] IOCTL_BTH_DISCONNECT_DEVICE failed: {:?}",
                e
            );
            return Err(format!("Disconnect failed: {:?}", e).into());
        }

        log::info!(
            "[BT][Disconnect] IOCTL_BTH_DISCONNECT_DEVICE succeeded for addr={:#018x}",
            bth_addr
        );
        Ok(())
    }
}

/// 通过 Win32 BluetoothSetServiceState 连接 Classic 蓝牙设备
/// 将已配对设备的 A2DP / HFP 服务先禁用再启用，触发 Windows 发起实际连接
pub fn connect_classic_by_address(bth_addr: u64) -> Result<()> {
    unsafe {
        // 1. 构造搜索参数：只查已配对/已记忆设备，不发起新扫描
        let search_params = BLUETOOTH_DEVICE_SEARCH_PARAMS {
            dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
            fReturnAuthenticated: true.into(),
            fReturnRemembered: true.into(),
            fReturnConnected: true.into(),
            fReturnUnknown: false.into(),
            fIssueInquiry: false.into(),
            cTimeoutMultiplier: 0,
            hRadio: windows::Win32::Foundation::HANDLE::default(),
        };
        let mut device_info = BLUETOOTH_DEVICE_INFO {
            dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
            ..std::mem::zeroed()
        };

        // 2. 遍历已配对设备列表，找到目标地址
        let h_find = BluetoothFindFirstDevice(&search_params, &mut device_info)
            .map_err(|e| format!("BluetoothFindFirstDevice failed: {:?}", e))?;

        let mut found = false;
        loop {
            if device_info.Address.Anonymous.ullLong == bth_addr {
                found = true;
                break;
            }
            if BluetoothFindNextDevice(h_find, &mut device_info).is_err() {
                break;
            }
        }
        let _ = BluetoothFindDeviceClose(h_find);

        if !found {
            return Err(format!(
                "Device with address {:#018x} not found in paired list",
                bth_addr
            )
            .into());
        }

        // 3. 对 A2DP 和 HFP 分别执行 禁用 → 等待 → 启用
        let profiles: &[(&str, *const GUID)] = &[
            ("A2DP", &GUID_A2DP as *const GUID),
            ("HFP", &GUID_HFP as *const GUID),
        ];
        for (name, guid) in profiles {
            // 先禁用（清除假死状态）
            let r = BluetoothSetServiceState(None, &device_info, *guid, BLUETOOTH_SERVICE_DISABLE);
            log::debug!("[BT][Connect] {} disable result={}", name, r);

            std::thread::sleep(std::time::Duration::from_millis(300));

            // 再启用 —— 这会触发 Windows 底层实际连接
            let r = BluetoothSetServiceState(None, &device_info, *guid, BLUETOOTH_SERVICE_ENABLE);
            if r == 0 {
                log::info!(
                    "[BT][Connect] {} enable succeeded for addr={:#018x}",
                    name,
                    bth_addr
                );
            } else {
                log::warn!(
                    "[BT][Connect] {} enable returned error={} for addr={:#018x}",
                    name,
                    r,
                    bth_addr
                );
            }
        }

        Ok(())
    }
}

/// 将 WinRT 设备转换为可序列化状态
pub fn to_serializable(id: &str, device: &WindowsBluetoothDevice) -> Result<BluetoothDevice> {
    use windows::Devices::Bluetooth::BluetoothConnectionStatus;

    let pairing = device.DeviceInformation()?.Pairing()?;
    let connected = device.ConnectionStatus()? == BluetoothConnectionStatus::Connected;

    // 尝试获取 Class of Device (CoD) 来解析设备类型
    let (major_service_classes, major_class, minor_class) =
        if let Ok(class_of_device) = device.ClassOfDevice() {
            if let Ok(raw_value) = class_of_device.RawValue() {
                parse_class_of_device(raw_value)
            } else {
                default_device_classes()
            }
        } else {
            default_device_classes()
        };

    Ok(BluetoothDevice {
        id: id.to_owned(),
        name: device.Name()?.to_string(),
        address: device.BluetoothAddress()?,
        major_service_classes,
        major_class,
        minor_class,
        appearance: None, // Classic 蓝牙不使用 appearance
        connected,
        paired: pairing.IsPaired()?,
        can_pair: pairing.CanPair()?,
        can_disconnect: false,
        is_low_energy: false,
        battery_percentage: None, // Classic 蓝牙通常无法直接获取电量
    })
}

/// 解析 Class of Device 值
fn parse_class_of_device(
    class: u32,
) -> (
    Vec<super::BluetoothMajorServiceClass>,
    BluetoothMajorClass,
    BluetoothMinorClass,
) {
    // Major Service Classes (bits 13-23)
    let major_service_classes = {
        use super::BluetoothMajorServiceClass::*;
        let services = class >> 13;
        [
            LimitedDiscoverableMode,
            Positioning,
            Networking,
            Rendering,
            Capturing,
            ObjectTransfer,
            Audio,
            Telephony,
            Information,
        ]
        .into_iter()
        .filter(|&service| services & service as u32 != 0)
        .collect()
    };

    // Major Class (bits 8-12, 5 bits)
    let major_class = match (class >> 8) & 0b11111 {
        0 => BluetoothMajorClass::Miscellaneous,
        1 => BluetoothMajorClass::Computer,
        2 => BluetoothMajorClass::Phone,
        3 => BluetoothMajorClass::NetworkAccessPoint,
        4 => BluetoothMajorClass::AudioVideo,
        5 => BluetoothMajorClass::Peripheral,
        6 => BluetoothMajorClass::Imaging,
        7 => BluetoothMajorClass::Wearable,
        8 => BluetoothMajorClass::Toy,
        9 => BluetoothMajorClass::Health,
        _ => BluetoothMajorClass::Uncategorized,
    };

    // Minor Class (bits 2-7, 6 bits)
    let minor_class_value = ((class >> 2) & 0b111111) as u8;
    let minor_class = match major_class {
        BluetoothMajorClass::Computer => BluetoothMinorClass::Computer(match minor_class_value {
            0 => super::BluetoothComputerMinor::Uncategorized,
            1 => super::BluetoothComputerMinor::DesktopWorkstation,
            2 => super::BluetoothComputerMinor::ServerClassComputer,
            3 => super::BluetoothComputerMinor::Laptop,
            4 => super::BluetoothComputerMinor::HandheldPcPda,
            5 => super::BluetoothComputerMinor::PalmSizePcPda,
            6 => super::BluetoothComputerMinor::WearableComputer,
            7 => super::BluetoothComputerMinor::Tablet,
            _ => super::BluetoothComputerMinor::Uncategorized,
        }),
        BluetoothMajorClass::Phone => BluetoothMinorClass::Phone(match minor_class_value {
            0 => super::BluetoothPhoneMinor::Uncategorized,
            1 => super::BluetoothPhoneMinor::Cellular,
            2 => super::BluetoothPhoneMinor::Cordless,
            3 => super::BluetoothPhoneMinor::Smartphone,
            4 => super::BluetoothPhoneMinor::WiredModemOrVoiceGateway,
            5 => super::BluetoothPhoneMinor::CommonIsdnAccess,
            _ => super::BluetoothPhoneMinor::Uncategorized,
        }),
        BluetoothMajorClass::AudioVideo => {
            BluetoothMinorClass::AudioVideo(match minor_class_value {
                0 => super::BluetoothAudioVideoMinor::Uncategorized,
                1 => super::BluetoothAudioVideoMinor::Headset,
                2 => super::BluetoothAudioVideoMinor::HandsFree,
                4 => super::BluetoothAudioVideoMinor::Microphone,
                5 => super::BluetoothAudioVideoMinor::Loudspeaker,
                6 => super::BluetoothAudioVideoMinor::Headphones,
                7 => super::BluetoothAudioVideoMinor::PortableAudio,
                8 => super::BluetoothAudioVideoMinor::CarAudio,
                9 => super::BluetoothAudioVideoMinor::SetTopBox,
                10 => super::BluetoothAudioVideoMinor::HiFiAudioDevice,
                11 => super::BluetoothAudioVideoMinor::Vcr,
                12 => super::BluetoothAudioVideoMinor::VideoCamera,
                13 => super::BluetoothAudioVideoMinor::Camcorder,
                14 => super::BluetoothAudioVideoMinor::VideoMonitor,
                15 => super::BluetoothAudioVideoMinor::VideoDisplayAndLoudspeaker,
                16 => super::BluetoothAudioVideoMinor::VideoConferencing,
                18 => super::BluetoothAudioVideoMinor::GamingToy,
                _ => super::BluetoothAudioVideoMinor::Uncategorized,
            })
        }
        BluetoothMajorClass::Peripheral => {
            let device_type = minor_class_value >> 4;
            let sub_type = minor_class_value & 0b1111;
            BluetoothMinorClass::Peripheral(
                match device_type {
                    0 => super::BluetoothPeripheralMinor::Uncategorized,
                    1 => super::BluetoothPeripheralMinor::Keyboard,
                    2 => super::BluetoothPeripheralMinor::Pointing,
                    3 => super::BluetoothPeripheralMinor::ComboKeyboardPointing,
                    _ => super::BluetoothPeripheralMinor::Uncategorized,
                },
                super::BluetoothPeripheralSubMinor::from(sub_type),
            )
        }
        BluetoothMajorClass::Wearable => BluetoothMinorClass::Wearable(match minor_class_value {
            1 => super::BluetoothWearableMinor::Wristwatch,
            2 => super::BluetoothWearableMinor::Pager,
            3 => super::BluetoothWearableMinor::Jacket,
            4 => super::BluetoothWearableMinor::Helmet,
            5 => super::BluetoothWearableMinor::Glasses,
            _ => super::BluetoothWearableMinor::Uncategorized,
        }),
        _ => BluetoothMinorClass::Uncategorized {
            unused: minor_class_value,
        },
    };

    (major_service_classes, major_class, minor_class)
}

/// 默认设备类型（当无法获取 ClassOfDevice 时使用）
fn default_device_classes() -> (
    Vec<super::BluetoothMajorServiceClass>,
    BluetoothMajorClass,
    BluetoothMinorClass,
) {
    (
        Vec::new(),
        BluetoothMajorClass::Uncategorized,
        BluetoothMinorClass::Uncategorized { unused: 0 },
    )
}

impl Drop for BluetoothDeviceWrapper {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
