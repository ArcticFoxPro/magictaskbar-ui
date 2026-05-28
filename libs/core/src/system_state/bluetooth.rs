use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "gen-binds", ts(export))]
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
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "gen-binds", ts(export))]
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

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "gen-binds", ts(export))]
pub enum BluetoothMinorClass {
    Uncategorized { unused: u8 },
    Computer(BluetoothComputerMinor),
    Phone(BluetoothPhoneMinor),
    AudioVideo(BluetoothAudioVideoMinor),
    Peripheral(BluetoothPeripheralMinor, BluetoothPeripheralSubMinor),
    Wearable(BluetoothWearableMinor),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "gen-binds", ts(export))]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "gen-binds", ts(export))]
pub enum BluetoothPhoneMinor {
    Uncategorized,
    Cellular,
    Cordless,
    Smartphone,
    WiredModemOrVoiceGateway,
    CommonIsdnAccess,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "gen-binds", ts(export))]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "gen-binds", ts(export))]
pub enum BluetoothPeripheralMinor {
    Uncategorized,
    Keyboard,
    Pointing,
    ComboKeyboardPointing,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "gen-binds", ts(export))]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "gen-binds", ts(export))]
pub enum BluetoothWearableMinor {
    Uncategorized,
    Wristwatch,
    Pager,
    Jacket,
    Helmet,
    Glasses,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "gen-binds", ts(export))]
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

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "gen-binds", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct DevicePairingAnswer {
    pub accept: bool,
    pub pin: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub address: Option<String>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize, TS)]
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
