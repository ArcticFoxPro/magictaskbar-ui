use windows::{Devices::Bluetooth::BluetoothLEDevice, Foundation::TypedEventHandler};

use crate::{
    error::Result,
    modules::bluetooth::{
        BluetoothDevice, BluetoothDeviceType, BluetoothManager, BluetoothManagerEvent,
    },
};

pub struct BluetoothLEDeviceWrapper {
    pub(super) id: String,
    pub(super) raw: BluetoothLEDevice,
    pub(super) state: BluetoothDevice,

    name_changed_token: i64,
    connection_status_changed_token: i64,
}

impl BluetoothLEDeviceWrapper {
    pub fn create(device_id: &str) -> Result<Self> {
        let device = BluetoothLEDevice::FromIdAsync(&device_id.into())?.get()?;

        let id = device_id.to_string();
        let name_changed_token =
            device.NameChanged(&TypedEventHandler::new(move |_src, _args| {
                BluetoothManager::send(BluetoothManagerEvent::DeviceUpdated(
                    id.clone(),
                    BluetoothDeviceType::LowEnergy,
                ));
                Ok(())
            }))?;

        let id = device_id.to_string();
        let connection_status_changed_token =
            device.ConnectionStatusChanged(&TypedEventHandler::new(move |_src, _args| {
                BluetoothManager::send(BluetoothManagerEvent::DeviceUpdated(
                    id.clone(),
                    BluetoothDeviceType::LowEnergy,
                ));
                Ok(())
            }))?;

        Ok(Self {
            id: device_id.to_string(),
            state: to_serializable(device_id, &device)?,
            raw: device,
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
    pub fn refresh_state(&mut self) -> Result<()> {
        self.state = to_serializable(&self.id, &self.raw)?;
        Ok(())
    }

    pub fn close(&mut self) -> Result<()> {
        let _ = self.raw.RemoveNameChanged(self.name_changed_token);
        let _ = self
            .raw
            .RemoveConnectionStatusChanged(self.connection_status_changed_token);

        // 直接关闭设备对象即可
        let _ = self.raw.Close();

        Ok(())
    }
}

pub fn to_serializable(id: &str, device: &BluetoothLEDevice) -> Result<BluetoothDevice> {
    use windows::Devices::Bluetooth::BluetoothConnectionStatus;

    let pairing = device.DeviceInformation()?.Pairing()?;
    let connected = device.ConnectionStatus()? == BluetoothConnectionStatus::Connected;

    // 尝试获取电量信息（仅在连接时）
    let battery_percentage = if connected {
        get_battery_percentage(device).ok()
    } else {
        None
    };

    Ok(BluetoothDevice {
        id: id.to_owned(),
        name: device.Name()?.to_string(),
        address: device.BluetoothAddress()?,
        major_service_classes: Vec::new(),
        major_class: super::BluetoothMajorClass::Uncategorized,
        minor_class: super::BluetoothMinorClass::Uncategorized { unused: 0 },
        appearance: device.Appearance().ok().and_then(|a| a.RawValue().ok()),
        connected,
        paired: pairing.IsPaired()?,
        can_pair: pairing.CanPair()?,
        can_disconnect: false,
        is_low_energy: true,
        battery_percentage,
    })
}

/// 尝试从 BLE 设备获取电量信息
fn get_battery_percentage(device: &BluetoothLEDevice) -> Result<u8> {
    use windows::Devices::Bluetooth::GenericAttributeProfile::GattCommunicationStatus;

    // 电池服务 UUID: 0x180F
    let battery_service_uuid =
        windows::core::GUID::from_u128(0x0000180F_0000_1000_8000_00805F9B34FB);

    // 获取电池服务
    let services_result = device
        .GetGattServicesForUuidAsync(battery_service_uuid)?
        .get()?;
    if services_result.Status()? != GattCommunicationStatus::Success {
        return Err("Failed to get battery service".into());
    }

    let services = services_result.Services()?;
    if services.Size()? == 0 {
        return Err("No battery service found".into());
    }

    let battery_service = services.GetAt(0)?;

    // 电池电平特征 UUID: 0x2A19
    let battery_level_uuid = windows::core::GUID::from_u128(0x00002A19_0000_1000_8000_00805F9B34FB);

    // 获取电池电平特征
    let characteristics_result = battery_service
        .GetCharacteristicsForUuidAsync(battery_level_uuid)?
        .get()?;
    if characteristics_result.Status()? != GattCommunicationStatus::Success {
        return Err("Failed to get battery level characteristic".into());
    }

    let characteristics = characteristics_result.Characteristics()?;
    if characteristics.Size()? == 0 {
        return Err("No battery level characteristic found".into());
    }

    let battery_level_char = characteristics.GetAt(0)?;

    // 读取电量值
    let read_result = battery_level_char.ReadValueAsync()?.get()?;
    if read_result.Status()? != GattCommunicationStatus::Success {
        return Err("Failed to read battery level".into());
    }

    let value = read_result.Value()?;
    let data_reader = windows::Storage::Streams::DataReader::FromBuffer(&value)?;

    if data_reader.UnconsumedBufferLength()? > 0 {
        let battery_level = data_reader.ReadByte()?;
        return Ok(battery_level);
    }

    Err("No battery data".into())
}

impl Drop for BluetoothLEDeviceWrapper {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
