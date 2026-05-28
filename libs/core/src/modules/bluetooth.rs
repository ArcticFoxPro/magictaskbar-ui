use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BluetoothDevice {
    pub name: String,
    pub address: String,
    pub connected: bool,
    pub signal_strength: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum DevicePairingNeededAction {
    /// No extra action is needed
    None,
    /// The user only needs to confirm the pairing
    ConfirmOnly,
    /// Should be displayed to the user to be inserted in the other device
    DisplayPin { pin: String },
    /// An input pin should be provided
    ProvidePin,
    /// Pin should be displayed to the user and confirm that is the same as the other device
    ConfirmPinMatch { pin: String },
}

impl BluetoothDevice {
    pub fn new(name: String, address: String) -> Self {
        Self {
            name,
            address,
            connected: false,
            signal_strength: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct DevicePairingAnswer {
    pub accept: bool,
    pub pin: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub address: Option<String>,
}
