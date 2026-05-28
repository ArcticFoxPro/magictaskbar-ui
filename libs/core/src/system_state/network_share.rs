use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NetworkShareDevice {
    pub device_id: String,
    pub device_name: String,
    pub connected: bool,
}
