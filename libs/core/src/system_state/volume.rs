#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct VolumeState {
    pub volume: u8,
    pub muted: bool,
}
