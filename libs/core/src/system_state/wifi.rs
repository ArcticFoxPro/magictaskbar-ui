use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WifiNetwork {
    pub ssid: String,
    /// 0 - 100 quality as reported by WLAN API
    pub signal: u32,
    /// simple security label (secured/open)
    pub security: String,
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WifiConnectArgs {
    pub ssid: String,
    /// Target AP BSSID (MAC address), e.g. "aa:bb:cc:dd:ee:ff"
    #[ts(optional = nullable)]
    pub bssid: Option<String>,
    /// If true, connect using an existing authorized/saved profile.
    pub authorized: bool,
    /// Saved profile name (required when authorized=true)
    #[ts(optional = nullable)]
    pub profile_name: Option<String>,
    /// Password for secured networks (used when authorized=false)
    #[ts(optional = nullable)]
    pub password: Option<String>,
    /// Authentication (e.g. "WPA2PSK", "WPAPSK", "open") (used when authorized=false)
    #[ts(optional = nullable)]
    pub authentication: Option<String>,
    /// Encryption (e.g. "AES", "TKIP", "none") (used when authorized=false)
    #[ts(optional = nullable)]
    pub encryption: Option<String>,
}
