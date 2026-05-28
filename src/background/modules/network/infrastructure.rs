use crate::cli::ServicePipe;
use crate::error::Result;
use libs_core::system_state::{NetworkShareDevice, WifiConnectArgs, WifiNetwork};

use libs_core::handlers::FuncEvent;
use parking_lot::Mutex;
use regex::Regex;
use slu_ipc::messages::SvcAction;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::LazyLock;
use std::time::Duration;
use tauri::Emitter;
use tauri_plugin_shell::ShellExt;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_COPYDATA};

use crate::windows_api::WindowsApi;

const CONTROL_CENTER_AUX_WINDOW_CLASS: &str = "ControlCenterAuxBackgroundWindows";
const WM_USER_CONNECT_NETWORK_SHARE_DEVICE: u32 = 0x0400 + 301;
const WM_USER_DISCONNECT_NETWORK_SHARE_DEVICE: u32 = 0x0400 + 302;
const NETWORK_SHARE_BUSINESS_ID: &str = "HnNetworkShare";
const WLAN_LOCATION_PERMISSION_REQUIRED_ERR: &str = "WLAN_LOCATION_PERMISSION_REQUIRED";

pub static NETWORK_SHARE_DEVICES: LazyLock<Mutex<Vec<NetworkShareDevice>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

async fn service_backend_command(
    command: &str,
    args: serde_json::Value,
) -> Result<Option<serde_json::Value>> {
    let data = ServicePipe::request_with_response(SvcAction::ExecuteBackendCommand {
        command: command.to_string(),
        args,
    })
    .await?;

    match data {
        Some(data) if !data.trim().is_empty() => Ok(Some(serde_json::from_str(&data)?)),
        _ => Ok(None),
    }
}

fn value_bool(value: Option<serde_json::Value>, default: bool) -> bool {
    value
        .and_then(|value| value.get("value").and_then(|value| value.as_bool()))
        .unwrap_or(default)
}

fn emit_network_share_devices(devices: &[NetworkShareDevice]) {
    let app = crate::app::get_app_handle();
    if let Err(error) = app.emit(FuncEvent::SystemNetworkShareDevicesChanged, devices) {
        log::warn!(
            "[NET][NetworkShare] Failed to emit network share devices: {}",
            error
        );
        return;
    }

    log::info!(
        "[NET][NetworkShare] Notified network share devices changed, count={}",
        devices.len()
    );
}

pub fn update_network_share_devices(devices: Vec<NetworkShareDevice>) {
    let snapshot = {
        let mut guard = NETWORK_SHARE_DEVICES.lock();

        let merged: Vec<NetworkShareDevice> = devices
            .into_iter()
            .map(|device| {
                let connected = guard
                    .iter()
                    .find(|existing| existing.device_id == device.device_id)
                    .map(|existing| existing.connected)
                    .unwrap_or(device.connected);

                NetworkShareDevice {
                    connected,
                    ..device
                }
            })
            .collect();

        *guard = merged.clone();
        merged
    };

    emit_network_share_devices(&snapshot);
}

pub fn get_network_share_devices() -> Vec<NetworkShareDevice> {
    NETWORK_SHARE_DEVICES.lock().clone()
}

pub fn add_network_share_devices(devices: Vec<NetworkShareDevice>) {
    if devices.is_empty() {
        return;
    }

    let snapshot = {
        let mut guard = NETWORK_SHARE_DEVICES.lock();
        let mut changed = false;

        for device in devices {
            let exists = guard
                .iter()
                .any(|existing| existing.device_id == device.device_id);
            if !exists {
                guard.push(device);
                changed = true;
            }
        }

        if !changed {
            return;
        }

        guard.clone()
    };

    emit_network_share_devices(&snapshot);
}

pub fn set_network_share_device_connected(device_id: &str, connected: bool) {
    if device_id.is_empty() {
        return;
    }

    let snapshot = {
        let mut guard = NETWORK_SHARE_DEVICES.lock();
        let mut changed = false;

        for device in guard.iter_mut() {
            if device.device_id == device_id && device.connected != connected {
                device.connected = connected;
                changed = true;
                break;
            }
        }

        if !changed {
            return;
        }

        guard.clone()
    };

    emit_network_share_devices(&snapshot);
}

pub fn remove_network_share_devices_by_ids(device_ids: Vec<String>) {
    if device_ids.is_empty() {
        return;
    }

    let snapshot = {
        let mut guard = NETWORK_SHARE_DEVICES.lock();
        let before_len = guard.len();
        guard.retain(|device| {
            !device_ids
                .iter()
                .any(|device_id| device.device_id == *device_id)
        });

        if guard.len() == before_len {
            return;
        }

        guard.clone()
    };

    emit_network_share_devices(&snapshot);
}

fn decode_netsh_text(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    fn decode_utf16(bytes: &[u8], le: bool) -> Option<String> {
        if bytes.len() < 2 {
            return None;
        }
        let mut u16s = Vec::with_capacity(bytes.len() / 2);
        for chunk in bytes.chunks_exact(2) {
            let v = if le {
                u16::from_le_bytes([chunk[0], chunk[1]])
            } else {
                u16::from_be_bytes([chunk[0], chunk[1]])
            };
            u16s.push(v);
        }
        Some(String::from_utf16_lossy(&u16s))
    }

    // Some systems may emit UTF-16 text when stdout is captured; handle BOM and UTF-16-like data.
    if bytes.len() >= 2 {
        if bytes[0] == 0xFF && bytes[1] == 0xFE {
            if let Some(s) = decode_utf16(&bytes[2..], true) {
                return s;
            }
        }
        if bytes[0] == 0xFE && bytes[1] == 0xFF {
            if let Some(s) = decode_utf16(&bytes[2..], false) {
                return s;
            }
        }
    }

    // Heuristic: lots of NUL bytes usually indicates UTF-16LE/BE.
    let nul_count = bytes.iter().filter(|b| **b == 0).count();
    if nul_count > bytes.len() / 4 {
        // Try UTF-16LE first (most common on Windows)
        if let Some(s) = decode_utf16(bytes, true) {
            // If decoded string contains some expected ASCII letters, keep it.
            if s.contains("SSID") || s.contains("ssid") || s.contains("信号") || s.contains("状态")
            {
                return s;
            }
        }
        if let Some(s) = decode_utf16(bytes, false) {
            return s;
        }
    }

    // Prefer UTF-8 when possible; fallback to GBK for typical CN Windows netsh output.
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    let (cow, _used, _has_errors) = encoding_rs::GBK.decode(bytes);
    cow.to_string()
}

fn is_wlan_location_permission_restricted_output(output: &str) -> bool {
    if output.trim().is_empty() {
        return false;
    }

    let lower = output.to_ascii_lowercase();
    lower.contains("ms-settings:privacy-location")
        || lower.contains("ms-settings：privacy-location")
        || (lower.contains("wlan") && lower.contains("location permission"))
        || (lower.contains("wifi") && lower.contains("location permission"))
        || output.contains("位置权限")
        || output.contains("定位服务")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocationRestrictionProbe {
    Restricted,
    Allowed,
    Unknown,
}

#[cfg(target_os = "windows")]
fn probe_location_restriction_api() -> LocationRestrictionProbe {
    use windows::Devices::Geolocation::{GeolocationAccessStatus, Geolocator, PositionStatus};

    let access_status = match Geolocator::RequestAccessAsync().and_then(|op| op.get()) {
        Ok(status) => status,
        Err(err) => {
            log::debug!("[NET][LocationProbe] RequestAccessAsync failed: {err:?}");
            return LocationRestrictionProbe::Unknown;
        }
    };

    match access_status {
        GeolocationAccessStatus::Allowed => {
            let geolocator = match Geolocator::new() {
                Ok(g) => g,
                Err(err) => {
                    log::debug!("[NET][LocationProbe] Geolocator::new failed: {err:?}");
                    return LocationRestrictionProbe::Allowed;
                }
            };

            match geolocator.LocationStatus() {
                Ok(
                    PositionStatus::Disabled
                    | PositionStatus::NotAvailable
                    | PositionStatus::NoData,
                ) => LocationRestrictionProbe::Restricted,
                Ok(_) => LocationRestrictionProbe::Allowed,
                Err(err) => {
                    log::debug!("[NET][LocationProbe] LocationStatus failed: {err:?}");
                    LocationRestrictionProbe::Allowed
                }
            }
        }
        GeolocationAccessStatus::Denied => LocationRestrictionProbe::Restricted,
        GeolocationAccessStatus::Unspecified => LocationRestrictionProbe::Unknown,
        _ => LocationRestrictionProbe::Unknown,
    }
}

#[cfg(not(target_os = "windows"))]
fn probe_location_restriction_api() -> LocationRestrictionProbe {
    LocationRestrictionProbe::Unknown
}

fn truncate_for_log(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.trim().to_string();
    }
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            break;
        }
        out.push(ch);
    }
    out.push_str("…");
    out.trim().to_string()
}

fn parse_netsh_networks(output: &str) -> Vec<(String, u32, String)> {
    // returns (ssid, signal, security), dedup by SSID taking strongest signal
    // Netsh output can contain ASCII ':' or fullwidth '：' depending on locale/encoding.
    let re_ssid = Regex::new(r"(?i)^\s*ssid\s*\d*\s*[:：]\s*(.+)$").unwrap();
    // Some locales use fullwidth percent '％'
    let re_signal = Regex::new(r"(?i)^\s*(?:signal|信号)\s*[:：]\s*(\d+)\s*[%％]\s*$").unwrap();
    let re_auth = Regex::new(r"(?i)^\s*(?:authentication|身份验证)\s*[:：]\s*(.+)$").unwrap();

    let mut current_ssid: Option<String> = None;
    let mut current_security = String::from("unknown");
    let mut best_by_ssid: HashMap<String, (u32, String)> = HashMap::new();

    for raw in output.lines() {
        let line = raw.trim_start();
        if let Some(cap) = re_ssid.captures(line) {
            let ssid = cap[1].trim().to_string();
            if !ssid.is_empty() {
                current_ssid = Some(ssid);
                current_security = String::from("unknown");
            }
            continue;
        }
        if let Some(cap) = re_auth.captures(line) {
            let raw = cap[1].trim();
            let low = raw.to_ascii_lowercase();
            // Normalize to our UI-facing labels.
            // - enterprise: typically requires username/password or certs
            // - open: no password
            // - secured: PSK/home-style
            current_security =
                if low.contains("enterprise") || low.contains("802.1x") || raw.contains("企业") {
                    String::from("enterprise")
                } else if low.contains("open") || low.contains("none") {
                    String::from("open")
                } else {
                    String::from("secured")
                };
            continue;
        }
        if let Some(cap) = re_signal.captures(line) {
            if let Ok(sig) = cap[1].parse::<u32>() {
                if let Some(ssid) = current_ssid.clone() {
                    let entry = best_by_ssid
                        .entry(ssid)
                        .or_insert((0, current_security.clone()));
                    if sig > entry.0 {
                        *entry = (sig, current_security.clone());
                    }
                }
            }
            continue;
        }
        // Fallback: line with percent when label localized/garbled
        if (line.contains('%') || line.contains('％'))
            && (line.contains(':') || line.contains('：'))
        {
            let idx = line.find(':').or_else(|| line.find('：'));
            if let Some(idx) = idx {
                let rest = line[idx + 1..].trim_start();
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let (Some(ssid), Ok(sig)) = (current_ssid.clone(), digits.parse::<u32>()) {
                    let entry = best_by_ssid
                        .entry(ssid)
                        .or_insert((0, current_security.clone()));
                    if sig > entry.0 {
                        *entry = (sig, current_security.clone());
                    }
                }
            }
        }
    }

    let mut rows = Vec::new();
    for (ssid, (signal, security)) in best_by_ssid.into_iter() {
        rows.push((ssid, signal, security));
    }
    rows
}

fn parse_netsh_interface(output: &str) -> Option<(String, u32)> {
    // Find connected SSID and signal in 'netsh wlan show interfaces' in any locale
    // Netsh output can contain ASCII ':' or fullwidth '：' depending on locale/encoding.
    let re_ssid = Regex::new(r"(?i)^\s*ssid\s*[:：]\s*(.+)$").unwrap();
    let re_signal = Regex::new(r"(?i)^\s*(?:signal|信号)\s*[:：]\s*(\d+)\s*[%％]\s*$").unwrap();
    let re_state = Regex::new(r"(?i)^\s*(?:state|状态)\s*[:：]\s*(.+)$").unwrap();
    let mut ssid: Option<String> = None;
    let mut signal: Option<u32> = None;
    let mut is_disconnected = false;
    for raw in output.lines() {
        let line = raw.trim_start();
        // Some localizations print "SSID name"; skip those
        if line.to_ascii_lowercase().starts_with("ssid name") {
            continue;
        }
        if let Some(cap) = re_state.captures(line) {
            let v = cap[1].trim().to_ascii_lowercase();
            if v.contains("disconnected") || v.contains("已断开") {
                is_disconnected = true;
            }
            continue;
        }
        if let Some(cap) = re_ssid.captures(line) {
            let s = cap[1].trim().to_string();
            if !s.is_empty() {
                ssid = Some(s);
            }
            continue;
        }
        if let Some(cap) = re_signal.captures(line) {
            if let Ok(v) = cap[1].parse::<u32>() {
                signal = Some(v);
            }
            continue;
        }
        if (line.contains('%') || line.contains('％'))
            && (line.contains(':') || line.contains('：'))
            && signal.is_none()
        {
            let idx = line.find(':').or_else(|| line.find('：'));
            if let Some(idx) = idx {
                let rest = line[idx + 1..].trim_start();
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(v) = digits.parse::<u32>() {
                    signal = Some(v);
                }
            }
        }
    }
    if is_disconnected {
        return None;
    }
    // Some systems/locales may fail to parse signal due to encoding; SSID alone is still useful
    // for correctly marking the connected network.
    match (ssid, signal) {
        (Some(s), Some(sig)) => Some((s, sig)),
        (Some(s), None) => Some((s, 0)),
        _ => None,
    }
}

// Removed advanced helpers for interface/profile parsing as they are no longer needed

#[cfg(target_os = "windows")]
mod wlanapi_impl {
    use super::*;
    use std::{cmp::Ordering, ptr};

    const ERROR_NOT_FOUND: u32 = 1168;

    fn is_enterprise_auth_alg(auth_alg: u32) -> bool {
        // DOT11_AUTH_ALGORITHM values (Win32):
        // - 3: DOT11_AUTH_ALGO_WPA (WPA-Enterprise / 802.1X)
        // - 6: DOT11_AUTH_ALGO_RSNA (WPA2-Enterprise / 802.1X)
        // Note: PSK variants are 4 (WPA_PSK) and 7 (RSNA_PSK).
        matches!(auth_alg, 3 | 6)
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct GUID {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct DOT11_SSID {
        u_ssid_length: u32,
        uc_ssid: [u8; 32],
    }

    #[repr(C)]
    struct WLAN_INTERFACE_INFO {
        interface_guid: GUID,
        str_interface_description: [u16; 256],
        is_state: u32,
    }

    #[repr(C)]
    struct WLAN_INTERFACE_INFO_LIST {
        dw_number_of_items: u32,
        dw_index: u32,
        interface_info: [WLAN_INTERFACE_INFO; 1],
    }

    #[repr(C)]
    struct WLAN_AVAILABLE_NETWORK {
        str_profile_name: [u16; 256],
        dot11_ssid: DOT11_SSID,
        dot11_bss_type: u32,
        u_number_of_bssids: u32,
        b_network_connectable: i32,
        wlan_not_connectable_reason: u32,
        u_number_of_phy_types: u32,
        dot11_phy_types: [u32; 8],
        b_more_phy_types: i32,
        wlan_signal_quality: u32,
        b_security_enabled: i32,
        dot11_default_auth_algorithm: u32,
        dot11_default_cipher_algorithm: u32,
        dw_flags: u32,
        dw_reserved: u32,
    }

    #[repr(C)]
    struct WLAN_AVAILABLE_NETWORK_LIST {
        dw_number_of_items: u32,
        dw_index: u32,
        network: [WLAN_AVAILABLE_NETWORK; 1],
    }

    #[repr(C)]
    struct WLAN_RATE_SET {
        u_rate_set_length: u32,
        us_rate_set: [u16; 126],
    }

    #[repr(C)]
    struct WLAN_BSS_ENTRY {
        dot11_ssid: DOT11_SSID,
        u_phy_id: u32,
        dot11_bssid: [u8; 6],
        dot11_bss_type: u32,
        dot11_phy_type: u32,
        l_rssi: i32,
        u_link_quality: u32,
        b_in_reg_domain: u8,
        _pad1: [u8; 3],
        us_beacon_period: u16,
        _pad2: [u8; 2],
        ull_timestamp: u64,
        ull_host_timestamp: u64,
        us_capability_information: u16,
        _pad3: [u8; 2],
        ul_ch_center_frequency: u32,
        wlan_rate_set: WLAN_RATE_SET,
        ul_ie_offset: u32,
        ul_ie_size: u32,
    }

    #[repr(C)]
    struct WLAN_BSS_LIST {
        dw_total_size: u32,
        dw_number_of_items: u32,
        wlan_bss_entries: [WLAN_BSS_ENTRY; 1],
    }

    const ERROR_SUCCESS: u32 = 0;
    const WLAN_AVAILABLE_NETWORK_CONNECTED: u32 = 0x0000_0001;
    const DOT11_BSS_TYPE_ANY: u32 = 3;
    const WLAN_INTERFACE_STATE_CONNECTED: u32 = 1;

    // Connection-related constants
    const WLAN_CONNECTION_MODE_PROFILE: u32 = 0;

    const NDIS_OBJECT_TYPE_DEFAULT: u8 = 0x80;
    const DOT11_BSSID_LIST_REVISION_1: u8 = 1;

    #[link(name = "wlanapi")]
    extern "system" {
        fn WlanOpenHandle(
            dwClientVersion: u32,
            pReserved: *mut c_void,
            pdwNegotiatedVersion: *mut u32,
            phClientHandle: *mut *mut c_void,
        ) -> u32;
        fn WlanCloseHandle(hClientHandle: *mut c_void, pReserved: *mut c_void) -> u32;

        fn WlanScan(
            hClientHandle: *mut c_void,
            pInterfaceGuid: *const GUID,
            pDot11Ssid: *const DOT11_SSID,
            pIeData: *const c_void,
            pReserved: *mut c_void,
        ) -> u32;
        fn WlanEnumInterfaces(
            hClientHandle: *mut c_void,
            pReserved: *mut c_void,
            ppInterfaceList: *mut *mut WLAN_INTERFACE_INFO_LIST,
        ) -> u32;
        fn WlanGetAvailableNetworkList(
            hClientHandle: *mut c_void,
            pInterfaceGuid: *const GUID,
            dwFlags: u32,
            pReserved: *mut c_void,
            ppAvailableNetworkList: *mut *mut WLAN_AVAILABLE_NETWORK_LIST,
        ) -> u32;
        fn WlanGetNetworkBssList(
            hClientHandle: *mut c_void,
            pInterfaceGuid: *const GUID,
            pDot11Ssid: *const DOT11_SSID,
            dot11BssType: u32,
            bSecurityEnabled: i32,
            pReserved: *mut c_void,
            ppWlanBssList: *mut *mut WLAN_BSS_LIST,
        ) -> u32;
        fn WlanFreeMemory(pMemory: *mut c_void);

        fn WlanGetProfile(
            hClientHandle: *mut c_void,
            pInterfaceGuid: *const GUID,
            strProfileName: *const u16,
            pReserved: *mut c_void,
            pstrProfileXml: *mut *mut u16,
            pdwFlags: *mut u32,
            pdwGrantedAccess: *mut u32,
        ) -> u32;

        fn WlanSetProfile(
            hClientHandle: *mut c_void,
            pInterfaceGuid: *const GUID,
            dwFlags: u32,
            strProfileXml: *const u16,
            strAllUserProfileSecurity: *const u16,
            bOverwrite: i32,
            pReserved: *mut c_void,
            pdwReasonCode: *mut u32,
        ) -> u32;

        fn WlanDeleteProfile(
            hClientHandle: *mut c_void,
            pInterfaceGuid: *const GUID,
            strProfileName: *const u16,
            pReserved: *mut c_void,
        ) -> u32;

        fn WlanConnect(
            hClientHandle: *mut c_void,
            pInterfaceGuid: *const GUID,
            pConnectionParameters: *const WLAN_CONNECTION_PARAMETERS,
            pReserved: *mut c_void,
        ) -> u32;

        fn WlanDisconnect(
            hClientHandle: *mut c_void,
            pInterfaceGuid: *const GUID,
            pReserved: *mut c_void,
        ) -> u32;

        fn WlanRegisterNotification(
            hClientHandle: *mut c_void,
            dwNotifSource: u32,
            bIgnoreDuplicate: i32,
            funcCallback: Option<extern "system" fn(*mut c_void, *mut c_void)>,
            pCallbackContext: *mut c_void,
            pReserved: *mut c_void,
            pdwPrevNotifSource: *mut u32,
        ) -> u32;

        fn WlanReasonCodeToString(
            dwReasonCode: u32,
            dwBufferSize: u32,
            pStringBuffer: *mut u16,
            pReserved: *mut c_void,
        ) -> u32;
    }

    unsafe fn wlan_reason_to_string(reason: u32) -> String {
        let mut buf: Vec<u16> = vec![0u16; 512];
        let res =
            WlanReasonCodeToString(reason, buf.len() as u32, buf.as_mut_ptr(), ptr::null_mut());
        if res != ERROR_SUCCESS {
            return format!("<reason_to_string_failed:{res}>");
        }
        if let Some(pos) = buf.iter().position(|&c| c == 0) {
            buf.truncate(pos);
        }
        String::from_utf16_lossy(&buf)
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NDIS_OBJECT_HEADER {
        type_: u8,
        revision: u8,
        size: u16,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct DOT11_BSSID_LIST {
        header: NDIS_OBJECT_HEADER,
        u_num_of_entries: u32,
        u_total_num_of_entries: u32,
        bssids: [[u8; 6]; 1],
    }

    #[repr(C)]
    struct WLAN_CONNECTION_PARAMETERS {
        wlan_connection_mode: u32,
        str_profile: *const u16,
        p_dot11_ssid: *const DOT11_SSID,
        p_desired_bssid_list: *const DOT11_BSSID_LIST,
        dot11_bss_type: u32,
        dw_flags: u32,
    }

    fn to_wide_null(s: &str) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    unsafe fn wide_ptr_to_string(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        // 添加最大长度限制，防止无 null 终止符导致的越界读取
        const MAX_LEN: usize = 4096;
        let mut len: usize = 0;
        while len < MAX_LEN && *p.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(p, len);
        String::from_utf16_lossy(slice)
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    fn normalize_authentication(s: &str) -> String {
        let raw = s.trim();
        if raw.is_empty() {
            return String::new();
        }
        let up = raw.replace('_', "-").to_ascii_uppercase();
        match up.as_str() {
            "OPEN" => "open".to_string(),
            "WPA2" | "WPA2-ENTERPRISE" | "WPA2 ENTERPRISE" => "WPA2".to_string(),
            "WPA" | "WPA-ENTERPRISE" | "WPA ENTERPRISE" => "WPA".to_string(),
            "WPA2-PSK" | "WPA2 PSK" | "WPA2PSK" => "WPA2PSK".to_string(),
            "WPA-PSK" | "WPA PSK" | "WPAPSK" => "WPAPSK".to_string(),
            "WPA3-SAE" | "WPA3 SAE" | "WPA3SAE" => "WPA3SAE".to_string(),
            _ => raw.to_string(),
        }
    }

    fn is_enterprise_auth(authentication: &str) -> bool {
        let a = normalize_authentication(authentication);
        a.eq_ignore_ascii_case("WPA") || a.eq_ignore_ascii_case("WPA2")
    }

    fn normalize_encryption(s: &str) -> String {
        let raw = s.trim();
        if raw.is_empty() {
            return String::new();
        }
        let up = raw.replace('_', "-").to_ascii_uppercase();
        match up.as_str() {
            "NONE" => "none".to_string(),
            "AES" | "CCMP" => "AES".to_string(),
            "TKIP" => "TKIP".to_string(),
            _ => raw.to_string(),
        }
    }

    fn parse_bssid(s: &str) -> Result<[u8; 6]> {
        let cleaned = s.trim().replace('-', ":").to_ascii_lowercase();
        let parts: Vec<&str> = cleaned.split(':').filter(|p| !p.is_empty()).collect();
        if parts.len() != 6 {
            return Err(format!("Invalid BSSID format: '{s}'").into());
        }
        let mut out = [0u8; 6];
        for (i, p) in parts.iter().enumerate() {
            out[i] = u8::from_str_radix(p, 16)
                .map_err(|_| format!("Invalid BSSID hex at index {i}: '{p}'"))?;
        }
        Ok(out)
    }

    fn create_profile_xml(
        profile_name: &str,
        ssid: &str,
        authentication: &str,
        encryption: &str,
        password: Option<&str>,
    ) -> String {
        let profile_name = xml_escape(profile_name);
        let ssid_esc = xml_escape(ssid);
        let ssid_hex: String = ssid
            .as_bytes()
            .iter()
            .map(|b| format!("{:02X}", b))
            .collect();
        let auth_norm = normalize_authentication(authentication);
        let enc_norm = normalize_encryption(encryption);
        let auth = xml_escape(&auth_norm);
        let enc = xml_escape(&enc_norm);

        // Open networks must not include sharedKey.
        let is_open =
            auth_norm.eq_ignore_ascii_case("open") || enc_norm.eq_ignore_ascii_case("none");

        if is_open {
            format!(
                r#"<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
  <name>{profile_name}</name>
  <SSIDConfig>
    <SSID>
            <hex>{ssid_hex}</hex>
      <name>{ssid_esc}</name>
    </SSID>
  </SSIDConfig>
  <connectionType>ESS</connectionType>
    <connectionMode>auto</connectionMode>
  <MSM>
    <security>
      <authEncryption>
        <authentication>open</authentication>
        <encryption>none</encryption>
        <useOneX>false</useOneX>
      </authEncryption>
    </security>
  </MSM>
</WLANProfile>"#
            )
        } else {
            let key_material = xml_escape(password.unwrap_or_default());
            format!(
                r#"<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
  <name>{profile_name}</name>
  <SSIDConfig>
    <SSID>
            <hex>{ssid_hex}</hex>
      <name>{ssid_esc}</name>
    </SSID>
  </SSIDConfig>
  <connectionType>ESS</connectionType>
    <connectionMode>auto</connectionMode>
  <MSM>
    <security>
      <authEncryption>
        <authentication>{auth}</authentication>
        <encryption>{enc}</encryption>
        <useOneX>false</useOneX>
      </authEncryption>
      <sharedKey>
        <keyType>passPhrase</keyType>
        <protected>false</protected>
        <keyMaterial>{key_material}</keyMaterial>
      </sharedKey>
    </security>
  </MSM>
</WLANProfile>"#
            )
        }
    }
    fn auth_alg_to_profile_auth(auth: u32) -> &'static str {
        // Values from DOT11_AUTH_ALGORITHM (Win32). We only map common home cases.
        match auth {
            1 /* DOT11_AUTH_ALGO_80211_OPEN */ => "open",
            3 /* DOT11_AUTH_ALGO_WPA */ => "WPA",
            4 /* DOT11_AUTH_ALGO_WPA_PSK */ => "WPAPSK",
            6 /* DOT11_AUTH_ALGO_RSNA */ => "WPA2",
            7 /* DOT11_AUTH_ALGO_RSNA_PSK */ => "WPA2PSK",
            // WPA3 values vary; treat unknown as WPA2PSK.
            _ => "WPA2PSK",
        }
    }

    fn cipher_alg_to_profile_enc(cipher: u32) -> &'static str {
        // Values from DOT11_CIPHER_ALGORITHM (Win32)
        match cipher {
            0 /* DOT11_CIPHER_ALGO_NONE */ => "none",
            2 /* DOT11_CIPHER_ALGO_TKIP */ => "TKIP",
            4 /* DOT11_CIPHER_ALGO_CCMP */ => "AES",
            _ => "AES",
        }
    }

    unsafe fn infer_auth_enc_from_available_list(
        h_client: *mut c_void,
        guid: &GUID,
        ssid: &str,
    ) -> Result<(String, String)> {
        let mut avail_ptr: *mut WLAN_AVAILABLE_NETWORK_LIST = ptr::null_mut();
        let avail_res = WlanGetAvailableNetworkList(
            h_client,
            guid as *const GUID,
            0,
            ptr::null_mut(),
            &mut avail_ptr as *mut *mut WLAN_AVAILABLE_NETWORK_LIST,
        );
        if avail_res != ERROR_SUCCESS || avail_ptr.is_null() {
            return Err(format!(
                "WlanGetAvailableNetworkList failed (for auth/enc inference): {avail_res}"
            )
            .into());
        }

        let avail_list = &*avail_ptr;
        let avail_count = avail_list.dw_number_of_items as usize;
        let first_avail_ptr = &avail_list.network as *const [WLAN_AVAILABLE_NETWORK; 1]
            as *const WLAN_AVAILABLE_NETWORK;

        let mut best: Option<(u32, u32, u32, u32)> = None;
        // (signal, security_enabled, auth_alg, cipher_alg)

        for i in 0..avail_count {
            let n = &*first_avail_ptr.add(i);
            let n_ssid = ssid_to_string(&n.dot11_ssid);
            if n_ssid != ssid {
                continue;
            }
            if n.b_network_connectable == 0 {
                continue;
            }

            let sec = if n.b_security_enabled != 0 {
                1u32
            } else {
                0u32
            };
            let signal = n.wlan_signal_quality;

            match best {
                None => {
                    best = Some((
                        signal,
                        sec,
                        n.dot11_default_auth_algorithm,
                        n.dot11_default_cipher_algorithm,
                    ))
                }
                Some((best_signal, best_sec, _, _)) => {
                    // Prefer secured variants (if any) then stronger signal.
                    let replace = (sec > best_sec) || (sec == best_sec && signal > best_signal);
                    if replace {
                        best = Some((
                            signal,
                            sec,
                            n.dot11_default_auth_algorithm,
                            n.dot11_default_cipher_algorithm,
                        ));
                    }
                }
            }
        }

        WlanFreeMemory(avail_ptr as *mut c_void);

        if let Some((_signal, sec, auth_alg, cipher_alg)) = best {
            if sec == 0 {
                return Ok(("open".to_string(), "none".to_string()));
            }
            let auth = auth_alg_to_profile_auth(auth_alg).to_string();
            let enc = cipher_alg_to_profile_enc(cipher_alg).to_string();
            log::info!(
                "[NET][WIFI_CONNECT] inferred auth/enc: auth_alg={} cipher_alg={} -> {}/{}",
                auth_alg,
                cipher_alg,
                auth,
                enc
            );
            return Ok((auth, enc));
        }

        Err(format!(
            "Could not infer auth/encryption for ssid='{ssid}' from available network list"
        )
        .into())
    }

    unsafe fn enum_first_interface_guid(h_client: *mut c_void) -> Result<GUID> {
        let mut if_list_ptr: *mut WLAN_INTERFACE_INFO_LIST = ptr::null_mut();
        let enum_res = WlanEnumInterfaces(
            h_client,
            ptr::null_mut(),
            &mut if_list_ptr as *mut *mut WLAN_INTERFACE_INFO_LIST,
        );
        if enum_res != ERROR_SUCCESS || if_list_ptr.is_null() {
            return Err(format!("WlanEnumInterfaces failed: {enum_res}").into());
        }
        let if_list = &*if_list_ptr;
        let count = if_list.dw_number_of_items as usize;
        if count == 0 {
            WlanFreeMemory(if_list_ptr as *mut c_void);
            return Err("No WLAN interface found".into());
        }

        let base_info_ptr = &if_list.interface_info as *const [WLAN_INTERFACE_INFO; 1]
            as *const WLAN_INTERFACE_INFO;
        let guid = (*base_info_ptr).interface_guid;
        WlanFreeMemory(if_list_ptr as *mut c_void);
        Ok(guid)
    }

    unsafe fn enum_preferred_interface_guid(h_client: *mut c_void) -> Result<GUID> {
        let mut if_list_ptr: *mut WLAN_INTERFACE_INFO_LIST = ptr::null_mut();
        let enum_res = WlanEnumInterfaces(
            h_client,
            ptr::null_mut(),
            &mut if_list_ptr as *mut *mut WLAN_INTERFACE_INFO_LIST,
        );
        if enum_res != ERROR_SUCCESS || if_list_ptr.is_null() {
            return Err(format!("WlanEnumInterfaces failed: {enum_res}").into());
        }
        let if_list = &*if_list_ptr;
        let count = if_list.dw_number_of_items as usize;
        if count == 0 {
            WlanFreeMemory(if_list_ptr as *mut c_void);
            return Err("No WLAN interface found".into());
        }

        let base_info_ptr = &if_list.interface_info as *const [WLAN_INTERFACE_INFO; 1]
            as *const WLAN_INTERFACE_INFO;
        let mut guid = (*base_info_ptr).interface_guid;
        for i in 0..count {
            let info = &*base_info_ptr.add(i);
            if info.is_state == WLAN_INTERFACE_STATE_CONNECTED {
                guid = info.interface_guid;
                break;
            }
        }

        WlanFreeMemory(if_list_ptr as *mut c_void);
        Ok(guid)
    }

    unsafe fn score_available_network_list(
        avail_ptr: *mut WLAN_AVAILABLE_NETWORK_LIST,
    ) -> (usize, usize) {
        if avail_ptr.is_null() {
            return (0, 0);
        }

        let avail_list = &*avail_ptr;
        let avail_count = avail_list.dw_number_of_items as usize;
        let first_avail_ptr = &avail_list.network as *const [WLAN_AVAILABLE_NETWORK; 1]
            as *const WLAN_AVAILABLE_NETWORK;
        let mut unique_ssids: HashMap<String, ()> = HashMap::new();

        for i in 0..avail_count {
            let network = &*first_avail_ptr.add(i);
            let ssid = ssid_to_string(&network.dot11_ssid);
            if !ssid.is_empty() {
                unique_ssids.entry(ssid).or_insert(());
            }
        }

        (unique_ssids.len(), avail_count)
    }

    unsafe fn get_best_available_network_list_after_scan(
        h_client: *mut c_void,
        guid: &GUID,
        scan_started: bool,
    ) -> Result<*mut WLAN_AVAILABLE_NETWORK_LIST> {
        let attempts = if scan_started { 5 } else { 1 };
        let poll_interval = std::time::Duration::from_millis(250);
        let mut best_ptr: *mut WLAN_AVAILABLE_NETWORK_LIST = ptr::null_mut();
        let mut best_score = (0usize, 0usize);
        let mut stable_rounds = 0usize;

        for attempt in 0..attempts {
            if attempt > 0 {
                std::thread::sleep(poll_interval);
            }

            let mut current_ptr: *mut WLAN_AVAILABLE_NETWORK_LIST = ptr::null_mut();
            let avail_res = WlanGetAvailableNetworkList(
                h_client,
                guid as *const GUID,
                0,
                ptr::null_mut(),
                &mut current_ptr as *mut *mut WLAN_AVAILABLE_NETWORK_LIST,
            );
            if avail_res != ERROR_SUCCESS || current_ptr.is_null() {
                if !current_ptr.is_null() {
                    WlanFreeMemory(current_ptr as *mut c_void);
                }

                if best_ptr.is_null() {
                    return Err(format!("WlanGetAvailableNetworkList failed: {avail_res}").into());
                }

                break;
            }

            let current_score = score_available_network_list(current_ptr);
            let improved = current_score > best_score;
            if improved {
                if !best_ptr.is_null() {
                    WlanFreeMemory(best_ptr as *mut c_void);
                }
                best_ptr = current_ptr;
                best_score = current_score;
                stable_rounds = 0;
            } else {
                WlanFreeMemory(current_ptr as *mut c_void);
                stable_rounds += 1;
            }

            if scan_started && best_score.0 >= 3 && stable_rounds >= 1 {
                break;
            }
        }

        if best_ptr.is_null() {
            return Err("WlanGetAvailableNetworkList returned no usable result".into());
        }

        log::debug!(
            "[NET] selected available network list after scan: unique_ssids={} raw_entries={} scan_started={}",
            best_score.0,
            best_score.1,
            scan_started
        );

        Ok(best_ptr)
    }

    fn parse_profile_autoconnect(xml: &str) -> bool {
        let xml_lower = xml.to_ascii_lowercase();
        if let Some(start) = xml_lower.find("<connectionmode>") {
            if let Some(end) = xml_lower[start..].find("</connectionmode>") {
                let inner = &xml_lower[start + "<connectionmode>".len()..start + end];
                return inner.trim() == "auto";
            }
        }
        // If missing, Windows profiles are typically auto-connect.
        true
    }

    fn set_profile_connection_mode(xml: &str, enabled: bool) -> String {
        let desired = if enabled { "auto" } else { "manual" };
        if let Some(start) = xml.find("<connectionMode>") {
            if let Some(end) = xml[start..].find("</connectionMode>") {
                let end_abs = start + end;
                let before = &xml[..start + "<connectionMode>".len()];
                let after = &xml[end_abs..];
                return format!("{before}{desired}{after}");
            }
        }

        // Insert right after connectionType if present, else near the top.
        let insertion = format!("\n    <connectionMode>{desired}</connectionMode>");
        if let Some(ct_end) = xml.find("</connectionType>") {
            let pos = ct_end + "</connectionType>".len();
            let mut out = String::with_capacity(xml.len() + insertion.len());
            out.push_str(&xml[..pos]);
            out.push_str(&insertion);
            out.push_str(&xml[pos..]);
            return out;
        }

        if let Some(name_end) = xml.find("</name>") {
            let pos = name_end + "</name>".len();
            let mut out = String::with_capacity(xml.len() + insertion.len());
            out.push_str(&xml[..pos]);
            out.push_str(&insertion);
            out.push_str(&xml[pos..]);
            return out;
        }

        format!("{xml}{insertion}")
    }

    pub fn system_get_wifi_autoconnect_wlanapi(profile_name: &str) -> Result<bool> {
        let profile_name = profile_name.trim();
        if profile_name.is_empty() {
            return Err("profileName is empty".into());
        }

        unsafe {
            let mut negotiated: u32 = 0;
            let mut h_client: *mut c_void = ptr::null_mut();
            let open = WlanOpenHandle(
                2,
                ptr::null_mut(),
                &mut negotiated as *mut u32,
                &mut h_client as *mut *mut c_void,
            );
            if open != ERROR_SUCCESS || h_client.is_null() {
                return Err(format!("WlanOpenHandle failed: {open}").into());
            }

            let guid = match enum_preferred_interface_guid(h_client) {
                Ok(g) => g,
                Err(e) => {
                    let _ = WlanCloseHandle(h_client, ptr::null_mut());
                    return Err(e);
                }
            };

            let profile_w = to_wide_null(profile_name);
            let mut xml_ptr: *mut u16 = ptr::null_mut();
            let mut flags: u32 = 0;
            let mut granted: u32 = 0;
            let get_res = WlanGetProfile(
                h_client,
                &guid as *const GUID,
                profile_w.as_ptr(),
                ptr::null_mut(),
                &mut xml_ptr as *mut *mut u16,
                &mut flags as *mut u32,
                &mut granted as *mut u32,
            );
            if get_res != ERROR_SUCCESS {
                let _ = WlanCloseHandle(h_client, ptr::null_mut());
                return Err(format!(
                    "WlanGetProfile failed for profile='{}': {}",
                    profile_name, get_res
                )
                .into());
            }

            let xml = if !xml_ptr.is_null() {
                let s = wide_ptr_to_string(xml_ptr as *const u16);
                WlanFreeMemory(xml_ptr as *mut c_void);
                s
            } else {
                String::new()
            };

            let _ = WlanCloseHandle(h_client, ptr::null_mut());
            Ok(parse_profile_autoconnect(&xml))
        }
    }

    pub fn system_set_wifi_autoconnect_wlanapi(profile_name: &str, enabled: bool) -> Result<()> {
        let profile_name = profile_name.trim();
        if profile_name.is_empty() {
            return Err("profileName is empty".into());
        }

        unsafe {
            let mut negotiated: u32 = 0;
            let mut h_client: *mut c_void = ptr::null_mut();
            let open = WlanOpenHandle(
                2,
                ptr::null_mut(),
                &mut negotiated as *mut u32,
                &mut h_client as *mut *mut c_void,
            );
            if open != ERROR_SUCCESS || h_client.is_null() {
                return Err(format!("WlanOpenHandle failed: {open}").into());
            }

            let guid = match enum_preferred_interface_guid(h_client) {
                Ok(g) => g,
                Err(e) => {
                    let _ = WlanCloseHandle(h_client, ptr::null_mut());
                    return Err(e);
                }
            };

            let profile_w = to_wide_null(profile_name);
            let mut xml_ptr: *mut u16 = ptr::null_mut();
            let mut flags: u32 = 0;
            let mut granted: u32 = 0;
            let get_res = WlanGetProfile(
                h_client,
                &guid as *const GUID,
                profile_w.as_ptr(),
                ptr::null_mut(),
                &mut xml_ptr as *mut *mut u16,
                &mut flags as *mut u32,
                &mut granted as *mut u32,
            );
            if get_res != ERROR_SUCCESS {
                let _ = WlanCloseHandle(h_client, ptr::null_mut());
                return Err(format!(
                    "WlanGetProfile failed for profile='{}': {}",
                    profile_name, get_res
                )
                .into());
            }
            let xml = if !xml_ptr.is_null() {
                let s = wide_ptr_to_string(xml_ptr as *const u16);
                WlanFreeMemory(xml_ptr as *mut c_void);
                s
            } else {
                String::new()
            };

            let updated_xml = set_profile_connection_mode(&xml, enabled);
            let updated_w = to_wide_null(&updated_xml);
            let mut reason: u32 = 0;
            let set_res = WlanSetProfile(
                h_client,
                &guid as *const GUID,
                0,
                updated_w.as_ptr(),
                ptr::null(),
                1,
                ptr::null_mut(),
                &mut reason as *mut u32,
            );
            let _ = WlanCloseHandle(h_client, ptr::null_mut());

            if set_res != ERROR_SUCCESS {
                let reason_str = wlan_reason_to_string(reason);
                return Err(format!(
                    "WlanSetProfile failed: {set_res}, reason={reason} ({reason_str})"
                )
                .into());
            }
            Ok(())
        }
    }

    pub fn system_disconnect_wifi_wlanapi() -> Result<()> {
        unsafe {
            let mut negotiated: u32 = 0;
            let mut h_client: *mut c_void = ptr::null_mut();
            let open = WlanOpenHandle(
                2,
                ptr::null_mut(),
                &mut negotiated as *mut u32,
                &mut h_client as *mut *mut c_void,
            );
            if open != ERROR_SUCCESS || h_client.is_null() {
                return Err(format!("WlanOpenHandle failed: {open}").into());
            }

            let guid = match enum_preferred_interface_guid(h_client) {
                Ok(g) => g,
                Err(e) => {
                    let _ = WlanCloseHandle(h_client, ptr::null_mut());
                    return Err(e);
                }
            };

            let res = WlanDisconnect(h_client, &guid as *const GUID, ptr::null_mut());
            let _ = WlanCloseHandle(h_client, ptr::null_mut());
            if res != ERROR_SUCCESS {
                return Err(format!("WlanDisconnect failed: {res}").into());
            }
            Ok(())
        }
    }

    pub fn system_forget_wifi_wlanapi(profile_name: &str) -> Result<()> {
        let profile_name = profile_name.trim();
        if profile_name.is_empty() {
            return Err("profileName is empty".into());
        }

        unsafe {
            let mut negotiated: u32 = 0;
            let mut h_client: *mut c_void = ptr::null_mut();
            let open = WlanOpenHandle(
                2,
                ptr::null_mut(),
                &mut negotiated as *mut u32,
                &mut h_client as *mut *mut c_void,
            );
            if open != ERROR_SUCCESS || h_client.is_null() {
                return Err(format!("WlanOpenHandle failed: {open}").into());
            }

            let guid = match enum_preferred_interface_guid(h_client) {
                Ok(g) => g,
                Err(e) => {
                    let _ = WlanCloseHandle(h_client, ptr::null_mut());
                    return Err(e);
                }
            };

            // Best-effort disconnect first (non-fatal).
            let _ = WlanDisconnect(h_client, &guid as *const GUID, ptr::null_mut());

            let profile_w = to_wide_null(profile_name);
            let del_res = WlanDeleteProfile(
                h_client,
                &guid as *const GUID,
                profile_w.as_ptr(),
                ptr::null_mut(),
            );
            let _ = WlanCloseHandle(h_client, ptr::null_mut());

            if del_res == ERROR_NOT_FOUND {
                return Ok(());
            }
            if del_res != ERROR_SUCCESS {
                return Err(format!(
                    "WlanDeleteProfile failed for profile='{}': {del_res}",
                    profile_name
                )
                .into());
            }
            Ok(())
        }
    }

    pub fn system_connect_wifi_wlanapi(args: &WifiConnectArgs) -> Result<()> {
        let ssid = args.ssid.trim();
        if ssid.is_empty() {
            return Err("SSID is empty".into());
        }

        log::info!(
            "[NET][WIFI_CONNECT] request ssid='{}' authorized={} bssid_present={} profile_name_present={}",
            ssid,
            args.authorized,
            args.bssid.as_deref().map(str::trim).is_some_and(|s| !s.is_empty()),
            args.profile_name.as_deref().map(str::trim).is_some_and(|s| !s.is_empty())
        );
        let bssid = args
            .bssid
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(parse_bssid)
            .transpose()?;

        unsafe {
            let mut negotiated: u32 = 0;
            let mut h_client: *mut c_void = ptr::null_mut();
            let open = WlanOpenHandle(
                2,
                ptr::null_mut(),
                &mut negotiated as *mut u32,
                &mut h_client as *mut *mut c_void,
            );
            if open != ERROR_SUCCESS || h_client.is_null() {
                return Err(format!("WlanOpenHandle failed: {open}").into());
            }

            let guid = match enum_first_interface_guid(h_client) {
                Ok(g) => g,
                Err(e) => {
                    let _ = WlanCloseHandle(h_client, ptr::null_mut());
                    return Err(e);
                }
            };

            let bssid_list = bssid.map(|bssid| DOT11_BSSID_LIST {
                header: NDIS_OBJECT_HEADER {
                    type_: NDIS_OBJECT_TYPE_DEFAULT,
                    revision: DOT11_BSSID_LIST_REVISION_1,
                    size: std::mem::size_of::<DOT11_BSSID_LIST>() as u16,
                },
                u_num_of_entries: 1,
                u_total_num_of_entries: 1,
                bssids: [bssid],
            });
            let desired_bssid_ptr = bssid_list
                .as_ref()
                .map(|l| l as *const DOT11_BSSID_LIST)
                .unwrap_or(ptr::null());

            let connect_profile_name: String = args
                .profile_name
                .as_deref()
                .unwrap_or(ssid)
                .trim()
                .to_string();

            if connect_profile_name.is_empty() {
                let _ = WlanCloseHandle(h_client, ptr::null_mut());
                return Err("profileName/SSID is empty".into());
            }

            // For unauthorized mode, we may overwrite an existing profile; keep a copy for rollback.
            let mut rollback_profile_xml: Option<String> = None;
            let mut created_new_profile: bool = false;

            if args.authorized {
                // Profile name usually matches SSID; accept missing profileName and fallback to SSID.
                // Ensure profile exists; also matches spec's step to call WlanGetProfile.
                let profile_name_w = to_wide_null(&connect_profile_name);
                let mut xml_ptr: *mut u16 = ptr::null_mut();
                let mut flags: u32 = 0;
                let mut granted: u32 = 0;
                let get_res = WlanGetProfile(
                    h_client,
                    &guid as *const GUID,
                    profile_name_w.as_ptr(),
                    ptr::null_mut(),
                    &mut xml_ptr as *mut *mut u16,
                    &mut flags as *mut u32,
                    &mut granted as *mut u32,
                );
                if get_res != ERROR_SUCCESS {
                    let _ = WlanCloseHandle(h_client, ptr::null_mut());
                    return Err(format!(
                        "WlanGetProfile failed for profile='{}': {}",
                        connect_profile_name, get_res
                    )
                    .into());
                }
                if !xml_ptr.is_null() {
                    WlanFreeMemory(xml_ptr as *mut c_void);
                }
            } else {
                // Create a new profile. If auth/encryption are missing, infer from scan results.
                let authentication_opt = args
                    .authentication
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let encryption_opt = args
                    .encryption
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());

                let (authentication, encryption): (String, String) =
                    match (authentication_opt, encryption_opt) {
                        (Some(a), Some(e)) => (a.to_string(), e.to_string()),
                        _ => infer_auth_enc_from_available_list(h_client, &guid, ssid)?,
                    };

                let is_enterprise = is_enterprise_auth(&authentication);

                if authentication.eq_ignore_ascii_case("open") {
                    // ok
                } else if is_enterprise {
                    // Scheme A: enterprise/802.1X networks are configured/connected via system settings.
                    let _ = WlanCloseHandle(h_client, ptr::null_mut());
                    return Err(
                        "enterprise networks must be connected from Windows Wi-Fi settings".into(),
                    );
                } else {
                    if args
                        .password
                        .as_deref()
                        .map(str::trim)
                        .unwrap_or_default()
                        .is_empty()
                    {
                        let _ = WlanCloseHandle(h_client, ptr::null_mut());
                        return Err(
                            "password is required for secured networks (authorized=false)".into(),
                        );
                    }
                }

                // If a profile already exists under this name, we'll overwrite it and restore on failure.
                // If it doesn't exist, we'll create it and delete it on failure.
                let profile_name_w = to_wide_null(&connect_profile_name);
                let mut old_xml_ptr: *mut u16 = ptr::null_mut();
                let mut old_flags: u32 = 0;
                let mut old_granted: u32 = 0;
                let old_get_res = WlanGetProfile(
                    h_client,
                    &guid as *const GUID,
                    profile_name_w.as_ptr(),
                    ptr::null_mut(),
                    &mut old_xml_ptr as *mut *mut u16,
                    &mut old_flags as *mut u32,
                    &mut old_granted as *mut u32,
                );
                if old_get_res == ERROR_SUCCESS {
                    if !old_xml_ptr.is_null() {
                        rollback_profile_xml = Some(wide_ptr_to_string(old_xml_ptr as *const u16));
                        WlanFreeMemory(old_xml_ptr as *mut c_void);
                    }
                } else if old_get_res == ERROR_NOT_FOUND {
                    created_new_profile = true;
                } else {
                    let _ = WlanCloseHandle(h_client, ptr::null_mut());
                    return Err(format!(
                        "WlanGetProfile failed while preparing rollback for profile='{}': {}",
                        connect_profile_name, old_get_res
                    )
                    .into());
                }

                let xml = create_profile_xml(
                    &connect_profile_name,
                    ssid,
                    &authentication,
                    &encryption,
                    args.password.as_deref(),
                );

                log::info!(
                    "[NET][WIFI_CONNECT] setting profile='{}' ssid='{}' auth='{}' enc='{}'",
                    connect_profile_name,
                    ssid,
                    normalize_authentication(&authentication),
                    normalize_encryption(&encryption),
                );

                let xml_w = to_wide_null(&xml);
                let mut reason: u32 = 0;
                let set_res = WlanSetProfile(
                    h_client,
                    &guid as *const GUID,
                    0,
                    xml_w.as_ptr(),
                    ptr::null(),
                    1,
                    ptr::null_mut(),
                    &mut reason as *mut u32,
                );
                if set_res != ERROR_SUCCESS {
                    let _ = WlanCloseHandle(h_client, ptr::null_mut());
                    let reason_str = wlan_reason_to_string(reason);
                    return Err(format!(
                        "WlanSetProfile failed: {set_res}, reason={reason} ({reason_str})"
                    )
                    .into());
                }
            }

            let profile_name_w = to_wide_null(&connect_profile_name);
            let params = WLAN_CONNECTION_PARAMETERS {
                wlan_connection_mode: WLAN_CONNECTION_MODE_PROFILE,
                str_profile: profile_name_w.as_ptr(),
                p_dot11_ssid: ptr::null(),
                p_desired_bssid_list: desired_bssid_ptr,
                dot11_bss_type: DOT11_BSS_TYPE_ANY,
                dw_flags: 0,
            };

            let conn_res = WlanConnect(
                h_client,
                &guid as *const GUID,
                &params as *const WLAN_CONNECTION_PARAMETERS,
                ptr::null_mut(),
            );

            log::info!(
                "[NET][WIFI_CONNECT] WlanConnect returned {} (0=success)",
                conn_res
            );

            if conn_res != ERROR_SUCCESS {
                // Rollback only for unauthorized flow.
                if !args.authorized {
                    if let Some(old_xml) = rollback_profile_xml.as_ref() {
                        log::warn!(
                            "[NET][WIFI_CONNECT] connect failed; restoring previous profile='{}'",
                            connect_profile_name
                        );
                        let old_xml_w = to_wide_null(old_xml);
                        let mut reason: u32 = 0;
                        let restore_res = WlanSetProfile(
                            h_client,
                            &guid as *const GUID,
                            0,
                            old_xml_w.as_ptr(),
                            ptr::null(),
                            1,
                            ptr::null_mut(),
                            &mut reason as *mut u32,
                        );
                        if restore_res != ERROR_SUCCESS {
                            let reason_str = wlan_reason_to_string(reason);
                            log::warn!(
                                "[NET][WIFI_CONNECT] restore profile failed: {}, reason={} ({})",
                                restore_res,
                                reason,
                                reason_str
                            );
                        }
                    } else if created_new_profile {
                        let profile_w = to_wide_null(&connect_profile_name);
                        let del_res = WlanDeleteProfile(
                            h_client,
                            &guid as *const GUID,
                            profile_w.as_ptr(),
                            ptr::null_mut(),
                        );
                        log::warn!(
                            "[NET][WIFI_CONNECT] connect failed; deleted newly-created profile='{}'. del_res={}",
                            connect_profile_name,
                            del_res
                        );
                    }
                }
                let _ = WlanCloseHandle(h_client, ptr::null_mut());
                return Err(format!("WlanConnect failed: {conn_res}").into());
            }

            // NOTE: WlanConnect returning ERROR_SUCCESS only means the request was accepted.
            // The actual connection completes asynchronously and can still fail (wrong password,
            // out of range, captive portal policies, etc.).
            //
            // If we return Ok() immediately, the frontend will assume the switch succeeded and
            // won't fall back to password/other flows. So we do a short best-effort wait for the
            // system to actually report being connected to the target SSID.
            let timeout = std::time::Duration::from_millis(8000);
            let deadline = std::time::Instant::now() + timeout;
            let mut last_seen: Option<String> = None;
            while std::time::Instant::now() < deadline {
                if let Some((cur_ssid, _sig, _sec)) = try_get_current_connection(h_client, &guid) {
                    last_seen = Some(cur_ssid.clone());
                    if cur_ssid == ssid {
                        let _ = WlanCloseHandle(h_client, ptr::null_mut());
                        return Ok(());
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
            let _ = WlanCloseHandle(h_client, ptr::null_mut());
            Err(format!(
                "Wi-Fi connect timeout/failure: target='{}' last_connected={:?}",
                ssid, last_seen
            )
            .into())
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct WLAN_PHY_RADIO_STATE {
        dw_phy_index: u32,
        dot11_software_radio_state: u32,
        dot11_hardware_radio_state: u32,
    }

    #[repr(C)]
    struct WLAN_RADIO_STATE {
        dw_number_of_phys: u32,
        phy_radio_state: [WLAN_PHY_RADIO_STATE; 1],
    }

    const WLAN_INTF_OPCODE_RADIO_STATE: u32 = 4;
    const WLAN_INTF_OPCODE_CURRENT_CONNECTION: u32 = 7;
    const DOT11_RADIO_STATE_ON: u32 = 1;
    const DOT11_RADIO_STATE_OFF: u32 = 2;

    const WLAN_MAX_NAME_LENGTH: usize = 256;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct WLAN_ASSOCIATION_ATTRIBUTES {
        dot11_ssid: DOT11_SSID,
        dot11_bss_type: u32,
        dot11_bssid: [u8; 6],
        dot11_phy_type: u32,
        u_dot11_phy_index: u32,
        wlan_signal_quality: u32,
        ul_rx_rate: u32,
        ul_tx_rate: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct WLAN_SECURITY_ATTRIBUTES {
        b_security_enabled: i32,
        b_one_x_enabled: i32,
        dot11_auth_algorithm: u32,
        dot11_cipher_algorithm: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct WLAN_CONNECTION_ATTRIBUTES {
        is_state: u32,
        wlan_connection_mode: u32,
        str_profile_name: [u16; WLAN_MAX_NAME_LENGTH],
        wlan_association_attributes: WLAN_ASSOCIATION_ATTRIBUTES,
        wlan_security_attributes: WLAN_SECURITY_ATTRIBUTES,
    }

    extern "system" {
        fn WlanQueryInterface(
            h_client_handle: *mut c_void,
            p_interface_guid: *const GUID,
            op_code: u32,
            p_reserved: *mut c_void,
            pdw_data_size: *mut u32,
            pp_data: *mut *mut c_void,
            p_wlan_opcode_value_type: *mut u32,
        ) -> u32;
        fn WlanSetInterface(
            h_client_handle: *mut c_void,
            p_interface_guid: *const GUID,
            op_code: u32,
            dw_data_size: u32,
            p_data: *const c_void,
            p_reserved: *mut c_void,
        ) -> u32;
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct WLAN_NOTIFICATION_DATA {
        notification_source: u32,
        notification_code: u32,
        interface_guid: GUID,
        dw_data_size: u32,
        p_data: *mut c_void,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct WLAN_CONNECTION_NOTIFICATION_DATA {
        wlan_connection_mode: u32,
        str_profile_name: [u16; WLAN_MAX_NAME_LENGTH],
        dot11_ssid: DOT11_SSID,
        dot11_bss_type: u32,
        b_security_enabled: i32,
        wlan_reason_code: u32,
        dw_flags: u32,
    }

    pub fn system_get_wlan_enabled_wlanapi() -> Result<bool> {
        unsafe {
            let mut negotiated: u32 = 0;
            let mut h_client: *mut c_void = ptr::null_mut();
            let open = WlanOpenHandle(
                2,
                ptr::null_mut(),
                &mut negotiated as *mut u32,
                &mut h_client as *mut *mut c_void,
            );
            if open != ERROR_SUCCESS || h_client.is_null() {
                return Err(format!("WlanOpenHandle failed: {open}").into());
            }

            let mut if_list_ptr: *mut WLAN_INTERFACE_INFO_LIST = ptr::null_mut();
            let enum_res = WlanEnumInterfaces(
                h_client,
                ptr::null_mut(),
                &mut if_list_ptr as *mut *mut WLAN_INTERFACE_INFO_LIST,
            );
            if enum_res != ERROR_SUCCESS || if_list_ptr.is_null() {
                let _ = WlanCloseHandle(h_client, ptr::null_mut());
                return Err(format!("WlanEnumInterfaces failed: {enum_res}").into());
            }

            let if_list = &*if_list_ptr;
            let count = if_list.dw_number_of_items as usize;
            if count == 0 {
                WlanFreeMemory(if_list_ptr as *mut c_void);
                let _ = WlanCloseHandle(h_client, ptr::null_mut());
                return Ok(false);
            }

            let base_info_ptr = &if_list.interface_info as *const [WLAN_INTERFACE_INFO; 1]
                as *const WLAN_INTERFACE_INFO;
            let mut any_on: bool = false;
            for i in 0..count {
                let guid = &(*base_info_ptr.add(i)).interface_guid as *const GUID;

                let mut data_size: u32 = 0;
                let mut data_ptr: *mut c_void = ptr::null_mut();
                let q = WlanQueryInterface(
                    h_client,
                    guid,
                    WLAN_INTF_OPCODE_RADIO_STATE,
                    ptr::null_mut(),
                    &mut data_size as *mut u32,
                    &mut data_ptr as *mut *mut c_void,
                    ptr::null_mut(),
                );
                if q != ERROR_SUCCESS || data_ptr.is_null() {
                    continue;
                }

                let radio = &*(data_ptr as *const WLAN_RADIO_STATE);
                let phys = radio.dw_number_of_phys as usize;
                let first_phy_ptr = &radio.phy_radio_state as *const [WLAN_PHY_RADIO_STATE; 1]
                    as *const WLAN_PHY_RADIO_STATE;
                for p in 0..phys {
                    let phy = &*first_phy_ptr.add(p);
                    if phy.dot11_hardware_radio_state == DOT11_RADIO_STATE_ON
                        && phy.dot11_software_radio_state == DOT11_RADIO_STATE_ON
                    {
                        any_on = true;
                        break;
                    }
                }

                WlanFreeMemory(data_ptr as *mut c_void);
                if any_on {
                    break;
                }
            }

            WlanFreeMemory(if_list_ptr as *mut c_void);
            let _ = WlanCloseHandle(h_client, ptr::null_mut());
            Ok(any_on)
        }
    }

    pub fn system_set_wlan_enabled_wlanapi(enabled: bool) -> Result<()> {
        unsafe {
            let mut negotiated: u32 = 0;
            let mut h_client: *mut c_void = ptr::null_mut();
            let open = WlanOpenHandle(
                2,
                ptr::null_mut(),
                &mut negotiated as *mut u32,
                &mut h_client as *mut *mut c_void,
            );
            if open != ERROR_SUCCESS || h_client.is_null() {
                return Err(format!("WlanOpenHandle failed: {open}").into());
            }

            let mut if_list_ptr: *mut WLAN_INTERFACE_INFO_LIST = ptr::null_mut();
            let enum_res = WlanEnumInterfaces(
                h_client,
                ptr::null_mut(),
                &mut if_list_ptr as *mut *mut WLAN_INTERFACE_INFO_LIST,
            );
            if enum_res != ERROR_SUCCESS || if_list_ptr.is_null() {
                let _ = WlanCloseHandle(h_client, ptr::null_mut());
                return Err(format!("WlanEnumInterfaces failed: {enum_res}").into());
            }

            let desired = if enabled {
                DOT11_RADIO_STATE_ON
            } else {
                DOT11_RADIO_STATE_OFF
            };

            let if_list = &*if_list_ptr;
            let count = if_list.dw_number_of_items as usize;
            let base_info_ptr = &if_list.interface_info as *const [WLAN_INTERFACE_INFO; 1]
                as *const WLAN_INTERFACE_INFO;

            let mut any_success = false;
            let mut last_err: Option<u32> = None;

            for i in 0..count {
                let guid = &(*base_info_ptr.add(i)).interface_guid as *const GUID;

                // Query current radio state first to discover PHY count.
                let mut data_size: u32 = 0;
                let mut data_ptr: *mut c_void = ptr::null_mut();
                let q = WlanQueryInterface(
                    h_client,
                    guid,
                    WLAN_INTF_OPCODE_RADIO_STATE,
                    ptr::null_mut(),
                    &mut data_size as *mut u32,
                    &mut data_ptr as *mut *mut c_void,
                    ptr::null_mut(),
                );
                if q != ERROR_SUCCESS || data_ptr.is_null() {
                    last_err = Some(q);
                    continue;
                }

                let radio = &*(data_ptr as *const WLAN_RADIO_STATE);
                let phys = radio.dw_number_of_phys as usize;
                WlanFreeMemory(data_ptr as *mut c_void);

                // Set software radio state per PHY.
                for phy_index in 0..phys {
                    let phy = WLAN_PHY_RADIO_STATE {
                        dw_phy_index: phy_index as u32,
                        dot11_software_radio_state: desired,
                        dot11_hardware_radio_state: 0,
                    };
                    let s = WlanSetInterface(
                        h_client,
                        guid,
                        WLAN_INTF_OPCODE_RADIO_STATE,
                        std::mem::size_of::<WLAN_PHY_RADIO_STATE>() as u32,
                        &phy as *const WLAN_PHY_RADIO_STATE as *const c_void,
                        ptr::null_mut(),
                    );
                    if s == ERROR_SUCCESS {
                        any_success = true;
                    } else {
                        last_err = Some(s);
                    }
                }
            }

            WlanFreeMemory(if_list_ptr as *mut c_void);
            let _ = WlanCloseHandle(h_client, ptr::null_mut());

            if any_success {
                Ok(())
            } else {
                Err(format!(
                    "WlanSetInterface(wlan_intf_opcode_radio_state) failed: {:?}",
                    last_err
                )
                .into())
            }
        }
    }

    fn ssid_to_string(ssid: &DOT11_SSID) -> String {
        let len = (ssid.u_ssid_length as usize).min(ssid.uc_ssid.len());
        let bytes = &ssid.uc_ssid[..len];

        // WiFi SSID 是字节序列，不一定是有效 UTF-8
        // 中文 SSID 可能使用 UTF-8 或 GBK 编码
        // 优先尝试 UTF-8，失败则尝试 GBK
        if let Ok(s) = std::str::from_utf8(bytes) {
            s.trim().to_string()
        } else {
            // 尝试 GBK 解码（常见于中文路由器）
            let (cow, _used, _has_errors) = encoding_rs::GBK.decode(bytes);
            cow.trim().to_string()
        }
    }

    fn bssid_to_string(bssid: &[u8; 6]) -> String {
        format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            bssid[0], bssid[1], bssid[2], bssid[3], bssid[4], bssid[5]
        )
    }

    fn band_from_center_freq_khz(khz: u32) -> &'static str {
        let mhz = khz / 1000;
        if mhz >= 5925 {
            "6G"
        } else if mhz >= 5000 {
            "5G"
        } else if mhz >= 2400 {
            "2.4G"
        } else {
            "unknown"
        }
    }

    fn calculate_network_score(
        link_quality_0_100: u32,
        band: &str,
        security_enabled: bool,
        rssi: i32,
        connected: bool,
    ) -> i32 {
        let mut score = link_quality_0_100 as i32;
        // Band bonus: prefer less congested bands.
        score += match band {
            "6G" => 15,
            "5G" => 10,
            "2.4G" => 0,
            _ => 0,
        };
        // Security bonus.
        if security_enabled {
            score += 5;
        }
        // Mild RSSI normalization: penalize very weak RSSI.
        if rssi != i32::MIN {
            if rssi < -80 {
                score -= 10;
            } else if rssi < -70 {
                score -= 5;
            }
        }
        // Strongly prefer currently-connected network.
        if connected {
            score += 20;
        }
        score
    }

    unsafe fn try_get_current_connection(
        h_client: *mut c_void,
        guid: &GUID,
    ) -> Option<(String, u32, bool)> {
        let mut data_size: u32 = 0;
        let mut data_ptr: *mut c_void = ptr::null_mut();
        let q = WlanQueryInterface(
            h_client,
            guid as *const GUID,
            WLAN_INTF_OPCODE_CURRENT_CONNECTION,
            ptr::null_mut(),
            &mut data_size as *mut u32,
            &mut data_ptr as *mut *mut c_void,
            ptr::null_mut(),
        );
        if q != ERROR_SUCCESS || data_ptr.is_null() {
            return None;
        }

        let attrs = &*(data_ptr as *const WLAN_CONNECTION_ATTRIBUTES);
        let connected = attrs.is_state == WLAN_INTERFACE_STATE_CONNECTED;
        if !connected {
            WlanFreeMemory(data_ptr as *mut c_void);
            return None;
        }

        let ssid = ssid_to_string(&attrs.wlan_association_attributes.dot11_ssid);
        let signal = attrs.wlan_association_attributes.wlan_signal_quality;
        let security_enabled = attrs.wlan_security_attributes.b_security_enabled != 0;
        WlanFreeMemory(data_ptr as *mut c_void);

        if ssid.is_empty() {
            None
        } else {
            Some((ssid, signal, security_enabled))
        }
    }

    pub fn system_get_current_connection_wlanapi() -> Result<Option<(String, u32, bool)>> {
        unsafe {
            let mut negotiated: u32 = 0;
            let mut h_client: *mut c_void = ptr::null_mut();
            let open = WlanOpenHandle(
                2,
                ptr::null_mut(),
                &mut negotiated as *mut u32,
                &mut h_client as *mut *mut c_void,
            );
            if open != ERROR_SUCCESS || h_client.is_null() {
                return Err(format!("WlanOpenHandle failed: {open}").into());
            }

            let guid = match enum_preferred_interface_guid(h_client) {
                Ok(g) => g,
                Err(_) => {
                    let _ = WlanCloseHandle(h_client, ptr::null_mut());
                    return Ok(None);
                }
            };

            let current = try_get_current_connection(h_client, &guid);
            let _ = WlanCloseHandle(h_client, ptr::null_mut());
            Ok(current)
        }
    }

    pub fn system_get_wifi_networks_wlanapi() -> Result<Vec<WifiNetwork>> {
        unsafe {
            let mut negotiated: u32 = 0;
            let mut h_client: *mut c_void = ptr::null_mut();
            let open = WlanOpenHandle(
                2,
                ptr::null_mut(),
                &mut negotiated as *mut u32,
                &mut h_client as *mut *mut c_void,
            );
            if open != ERROR_SUCCESS || h_client.is_null() {
                return Err(format!("WlanOpenHandle failed: {open}").into());
            }

            let mut if_list_ptr: *mut WLAN_INTERFACE_INFO_LIST = ptr::null_mut();
            let enum_res = WlanEnumInterfaces(
                h_client,
                ptr::null_mut(),
                &mut if_list_ptr as *mut *mut WLAN_INTERFACE_INFO_LIST,
            );
            if enum_res != ERROR_SUCCESS || if_list_ptr.is_null() {
                let _ = WlanCloseHandle(h_client, ptr::null_mut());
                return Err(format!("WlanEnumInterfaces failed: {enum_res}").into());
            }

            let if_list = &*if_list_ptr;
            let count = if_list.dw_number_of_items as usize;
            if count == 0 {
                WlanFreeMemory(if_list_ptr as *mut c_void);
                let _ = WlanCloseHandle(h_client, ptr::null_mut());
                return Ok(Vec::new());
            }

            // Prefer a connected interface when available.
            let base_info_ptr = &if_list.interface_info as *const [WLAN_INTERFACE_INFO; 1]
                as *const WLAN_INTERFACE_INFO;
            let mut selected_ptr = base_info_ptr;
            for i in 0..count {
                let info = &*base_info_ptr.add(i);
                if info.is_state == WLAN_INTERFACE_STATE_CONNECTED {
                    selected_ptr = base_info_ptr.add(i);
                    break;
                }
            }
            let guid = (*selected_ptr).interface_guid;

            // Read current connection up front so we can reliably mark the connected SSID,
            // even if the available list is stale right after roaming/switching.
            let current_conn = try_get_current_connection(h_client, &guid);

            // Important: `WlanGetAvailableNetworkList` is backed by the last scan results.
            // If we don't explicitly scan, the list can remain stale (often only showing the
            // currently connected SSID) until something else triggers a scan (e.g. opening the
            // Windows Wi-Fi flyout/settings).
            let scan_res = WlanScan(
                h_client,
                &guid as *const GUID,
                ptr::null(),
                ptr::null(),
                ptr::null_mut(),
            );
            let scan_started = scan_res == ERROR_SUCCESS;
            if !scan_started {
                log::debug!("[NET] WlanScan failed (non-fatal): {scan_res}");
            }

            // Available networks (SSID-level summary)
            let avail_ptr =
                get_best_available_network_list_after_scan(h_client, &guid, scan_started)?;
            if avail_ptr.is_null() {
                WlanFreeMemory(if_list_ptr as *mut c_void);
                let _ = WlanCloseHandle(h_client, ptr::null_mut());
                return Err("WlanGetAvailableNetworkList returned null pointer".into());
            }

            // BSS list (AP-level detail), query all SSIDs at once.
            let mut bss_ptr: *mut WLAN_BSS_LIST = ptr::null_mut();
            let bss_res = WlanGetNetworkBssList(
                h_client,
                &guid as *const GUID,
                ptr::null(),
                DOT11_BSS_TYPE_ANY,
                1,
                ptr::null_mut(),
                &mut bss_ptr as *mut *mut WLAN_BSS_LIST,
            );

            let mut best_bss_by_ssid: HashMap<String, (u32, i32, &'static str)> = HashMap::new();
            // ssid -> (best_link_quality, best_rssi, best_band)

            if bss_res == ERROR_SUCCESS && !bss_ptr.is_null() {
                let bss_list = &*bss_ptr;
                let bss_count = bss_list.dw_number_of_items as usize;
                let first_bss_ptr = &bss_list.wlan_bss_entries as *const [WLAN_BSS_ENTRY; 1]
                    as *const WLAN_BSS_ENTRY;
                for i in 0..bss_count {
                    let entry = &*first_bss_ptr.add(i);
                    let ssid = ssid_to_string(&entry.dot11_ssid);
                    if ssid.is_empty() {
                        continue;
                    }
                    let band = band_from_center_freq_khz(entry.ul_ch_center_frequency);
                    // Base score without security/connected (unknown at BSS level), used only to pick the best AP per SSID.
                    let base_score = calculate_network_score(
                        entry.u_link_quality,
                        band,
                        false,
                        entry.l_rssi,
                        false,
                    );
                    let v = best_bss_by_ssid
                        .entry(ssid)
                        .or_insert((0, i32::MIN, "unknown"));
                    // Prefer AP with highest base score; tie-break by stronger link quality.
                    let current_base = calculate_network_score(v.0, v.2, false, v.1, false);
                    if base_score > current_base
                        || (base_score == current_base && entry.u_link_quality > v.0)
                    {
                        v.0 = entry.u_link_quality;
                        v.1 = entry.l_rssi;
                        v.2 = band;
                    }
                }
            }

            let avail_list = &*avail_ptr;
            let avail_count = avail_list.dw_number_of_items as usize;
            let first_avail_ptr = &avail_list.network as *const [WLAN_AVAILABLE_NETWORK; 1]
                as *const WLAN_AVAILABLE_NETWORK;

            // `WlanGetAvailableNetworkList` may return multiple entries with the same SSID
            // (e.g. multiple saved profiles / security variants). The toolbar UI expects
            // a single row per SSID, so we aggregate by SSID and keep the best-scoring entry.
            let mut best_by_ssid: HashMap<String, (i32, WifiNetwork)> = HashMap::new();
            for i in 0..avail_count {
                let n = &*first_avail_ptr.add(i);
                let ssid = ssid_to_string(&n.dot11_ssid);
                if ssid.is_empty() {
                    continue;
                }

                // IMPORTANT:
                // During Wi-Fi roaming/switching, `WlanGetAvailableNetworkList` can briefly
                // report a stale `WLAN_AVAILABLE_NETWORK_CONNECTED` flag for the previous SSID
                // while `WlanQueryInterface` already reports the new current connection.
                // If we OR them, the UI may show two connected networks.
                //
                // Rule: if we can read current connection, trust it exclusively.
                let mut security_enabled = n.b_security_enabled != 0;
                let connected =
                    if let Some((cur_ssid, _cur_signal, cur_sec)) = current_conn.as_ref() {
                        if &ssid == cur_ssid {
                            security_enabled = security_enabled || *cur_sec;
                            true
                        } else {
                            false
                        }
                    } else {
                        (n.dw_flags & WLAN_AVAILABLE_NETWORK_CONNECTED) != 0
                    };

                let enterprise =
                    security_enabled && is_enterprise_auth_alg(n.dot11_default_auth_algorithm);
                let security = if !security_enabled {
                    "open"
                } else if enterprise {
                    "enterprise"
                } else {
                    "secured"
                }
                .to_string();

                // Prefer BSS-derived link quality/band/RSSI if present; else fallback to available network quality.
                let (bss_signal, bss_rssi, bss_band) = best_bss_by_ssid
                    .get(&ssid)
                    .copied()
                    .unwrap_or((0, i32::MIN, "unknown"));

                let signal = n.wlan_signal_quality.max(bss_signal);
                let score = calculate_network_score(
                    signal,
                    bss_band,
                    security_enabled,
                    bss_rssi,
                    connected,
                );

                let entry = WifiNetwork {
                    ssid: ssid.clone(),
                    signal,
                    security,
                    connected,
                };

                match best_by_ssid.get_mut(&ssid) {
                    None => {
                        best_by_ssid.insert(ssid, (score, entry));
                    }
                    Some((best_score, best_entry)) => {
                        // Merge: if any variant is connected, mark connected.
                        best_entry.connected |= entry.connected;
                        // Merge: prefer "secured" if any variant is secured.
                        if entry.security == "enterprise" {
                            best_entry.security = "enterprise".to_string();
                        } else if best_entry.security == "open" && entry.security == "secured" {
                            best_entry.security = "secured".to_string();
                        }
                        // Merge: keep strongest signal.
                        best_entry.signal = best_entry.signal.max(entry.signal);

                        // Choose representative row by score; connected is already included in score,
                        // but we also enforce it explicitly for safety.
                        let should_replace = (entry.connected && !best_entry.connected)
                            || score > *best_score
                            || (score == *best_score && entry.signal > best_entry.signal);
                        if should_replace {
                            *best_score = score;
                            best_entry.ssid = entry.ssid;
                            best_entry.signal = entry.signal.max(best_entry.signal);
                            best_entry.security = if best_entry.security == "enterprise"
                                || entry.security == "enterprise"
                            {
                                "enterprise".to_string()
                            } else if best_entry.security == "secured"
                                || entry.security == "secured"
                            {
                                "secured".to_string()
                            } else {
                                entry.security
                            };
                            best_entry.connected = best_entry.connected || entry.connected;
                        }
                    }
                }
            }

            // Sort: connected first, then higher signal.
            let mut out: Vec<(i32, WifiNetwork)> = best_by_ssid.into_values().collect();
            out.sort_by(
                |(score_a, a), (score_b, b)| match (a.connected, b.connected) {
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    _ => score_b
                        .cmp(score_a)
                        .then_with(|| b.signal.cmp(&a.signal))
                        .then_with(|| a.ssid.cmp(&b.ssid)),
                },
            );

            let out: Vec<WifiNetwork> = out.into_iter().map(|(_, n)| n).collect();

            // If the currently connected SSID is missing from the available list (can happen
            // briefly during roaming), add it so the UI always shows a connected entry.
            let mut out = out;
            if let Some((cur_ssid, cur_signal, cur_sec)) = current_conn {
                let has = out.iter().any(|n| n.connected && n.ssid == cur_ssid);
                if !has {
                    out.insert(
                        0,
                        WifiNetwork {
                            ssid: cur_ssid,
                            signal: cur_signal,
                            security: if cur_sec { "secured" } else { "open" }.to_string(),
                            connected: true,
                        },
                    );
                }
            }

            if !bss_ptr.is_null() {
                WlanFreeMemory(bss_ptr as *mut c_void);
            }
            WlanFreeMemory(avail_ptr as *mut c_void);
            WlanFreeMemory(if_list_ptr as *mut c_void);
            let _ = WlanCloseHandle(h_client, ptr::null_mut());

            Ok(out)
        }
    }
}

#[tauri::command(async)]
pub async fn system_connect_wifi(args: WifiConnectArgs) -> Result<()> {
    // Step 1: validate input early
    if args.ssid.trim().is_empty() {
        return Err("SSID is empty".into());
    }

    log::info!(
        "[NET][WIFI_CONNECT] invoke ssid='{}' authorized={} bssid_present={}",
        args.ssid,
        args.authorized,
        args.bssid
            .as_deref()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty())
    );

    // Avoid redundant connect if already connected to the target SSID.
    // IMPORTANT: Do not call system_get_wifi_networks() here, as it enumerates/queries
    // available networks and can take seconds, introducing noticeable click-to-connect lag.
    #[cfg(target_os = "windows")]
    {
        let target_ssid = args.ssid.clone();
        if let Ok(join) = tauri::async_runtime::spawn_blocking(
            wlanapi_impl::system_get_current_connection_wlanapi,
        )
        .await
        {
            if let Ok(Some((cur_ssid, _sig, _sec))) = join {
                if cur_ssid == target_ssid {
                    log::info!(
                        "[NET] Already connected to ssid='{}' (fast-path)",
                        target_ssid
                    );
                    return Ok(());
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        match service_backend_command("system_connect_wifi", serde_json::to_value(&args)?).await {
            Ok(_) => return Ok(()),
            Err(err) => {
                log::warn!(
                    "[NET][WIFI_CONNECT] srv connect failed, fallback to UI WLAN API: {}",
                    err
                );
            }
        }

        let join = tauri::async_runtime::spawn_blocking(move || {
            wlanapi_impl::system_connect_wifi_wlanapi(&args)
        });
        join.await??;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("system_connect_wifi is only supported on Windows".into())
}

#[tauri::command(async)]
pub async fn system_get_wlan_enabled() -> Result<bool> {
    #[cfg(target_os = "windows")]
    {
        match service_backend_command("system_get_wlan_enabled", serde_json::json!({})).await {
            Ok(value) => return Ok(value_bool(value, false)),
            Err(err) => log::warn!(
                "[NET][WIFI] srv get wlan enabled failed, fallback to UI WLAN API: {}",
                err
            ),
        }

        let join =
            tauri::async_runtime::spawn_blocking(wlanapi_impl::system_get_wlan_enabled_wlanapi);
        return join.await?;
    }

    #[allow(unreachable_code)]
    Err("system_get_wlan_enabled is only supported on Windows".into())
}

#[tauri::command(async)]
pub async fn system_set_wlan_enabled(enabled: bool) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        match service_backend_command(
            "system_set_wlan_enabled",
            serde_json::json!({ "enabled": enabled }),
        )
        .await
        {
            Ok(_) => return Ok(()),
            Err(err) => log::warn!(
                "[NET][WIFI] srv set wlan enabled failed, fallback to UI WLAN API: {}",
                err
            ),
        }

        let join = tauri::async_runtime::spawn_blocking(move || {
            wlanapi_impl::system_set_wlan_enabled_wlanapi(enabled)
        });
        join.await??;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("system_set_wlan_enabled is only supported on Windows".into())
}

#[tauri::command(async)]
pub async fn system_get_wifi_autoconnect(profile_name: String) -> Result<bool> {
    #[cfg(target_os = "windows")]
    {
        match service_backend_command(
            "system_get_wifi_autoconnect",
            serde_json::json!({ "profileName": profile_name.clone() }),
        )
        .await
        {
            Ok(value) => return Ok(value_bool(value, false)),
            Err(err) => log::warn!(
                "[NET][WIFI] srv get wifi autoconnect failed, fallback to UI WLAN API: {}",
                err
            ),
        }

        let join = tauri::async_runtime::spawn_blocking(move || {
            wlanapi_impl::system_get_wifi_autoconnect_wlanapi(&profile_name)
        });
        return join.await?;
    }

    #[allow(unreachable_code)]
    Err("system_get_wifi_autoconnect is only supported on Windows".into())
}

#[tauri::command(async)]
pub async fn system_set_wifi_autoconnect(profile_name: String, enabled: bool) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        match service_backend_command(
            "system_set_wifi_autoconnect",
            serde_json::json!({ "profileName": profile_name.clone(), "enabled": enabled }),
        )
        .await
        {
            Ok(_) => return Ok(()),
            Err(err) => log::warn!(
                "[NET][WIFI] srv set wifi autoconnect failed, fallback to UI WLAN API: {}",
                err
            ),
        }

        let join = tauri::async_runtime::spawn_blocking(move || {
            wlanapi_impl::system_set_wifi_autoconnect_wlanapi(&profile_name, enabled)
        });
        join.await??;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("system_set_wifi_autoconnect is only supported on Windows".into())
}

#[tauri::command(async)]
pub async fn system_disconnect_wifi() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        if let Err(err) =
            service_backend_command("system_disconnect_wifi", serde_json::json!({})).await
        {
            log::warn!(
                "[NET][WIFI] srv disconnect failed, fallback to UI WLAN API: {}",
                err
            );
            let join =
                tauri::async_runtime::spawn_blocking(wlanapi_impl::system_disconnect_wifi_wlanapi);
            join.await??;
        }

        // Emit a best-effort refresh snapshot. (Notification should also arrive.)
        if let Ok(list) = system_get_wifi_networks().await {
            let app = crate::app::get_app_handle();
            let _ = app.emit(FuncEvent::SystemNetworksChanged, &list);
        }
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("system_disconnect_wifi is only supported on Windows".into())
}

#[tauri::command(async)]
pub async fn system_forget_wifi(profile_name: String) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        if let Err(err) = service_backend_command(
            "system_forget_wifi",
            serde_json::json!({ "profileName": profile_name.clone() }),
        )
        .await
        {
            log::warn!(
                "[NET][WIFI] srv forget failed, fallback to UI WLAN API: {}",
                err
            );
            let join = tauri::async_runtime::spawn_blocking(move || {
                wlanapi_impl::system_forget_wifi_wlanapi(&profile_name)
            });
            join.await??;
        }

        // Emit a best-effort refresh snapshot.
        if let Ok(list) = system_get_wifi_networks().await {
            let app = crate::app::get_app_handle();
            let _ = app.emit(FuncEvent::SystemNetworksChanged, &list);
        }
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("system_forget_wifi is only supported on Windows".into())
}

#[tauri::command(async)]
pub async fn system_get_wifi_networks() -> Result<Vec<WifiNetwork>> {
    match service_backend_command("system_get_wifi_networks", serde_json::json!({})).await {
        Ok(Some(value)) => Ok(serde_json::from_value(value)?),
        Ok(None) => Ok(Vec::new()),
        Err(err) => {
            log::warn!(
                "[NET][WIFI] srv get wifi networks failed, fallback to UI WLAN API: {}",
                err
            );
            system_get_wifi_networks_internal("command").await
        }
    }
}

#[tauri::command(async)]
pub async fn system_get_network_share_devices() -> Result<Vec<NetworkShareDevice>> {
    Ok(get_network_share_devices())
}

#[tauri::command(async)]
pub async fn system_connect_network_share_device(
    device_id: String,
    device_name: String,
) -> Result<()> {
    let device_id = device_id.trim().to_string();
    let device_name = device_name.trim().to_string();

    if device_id.is_empty() {
        return Err("deviceId is empty".into());
    }

    #[cfg(target_os = "windows")]
    {
        return tauri::async_runtime::spawn_blocking(move || {
            send_network_share_connect_message(&device_id, &device_name)
        })
        .await?;
    }

    #[allow(unreachable_code)]
    Err("system_connect_network_share_device is only supported on Windows".into())
}

#[tauri::command(async)]
pub async fn system_disconnect_network_share_device(
    device_id: String,
    device_name: String,
) -> Result<()> {
    let device_id = device_id.trim().to_string();
    let device_name = device_name.trim().to_string();

    if device_id.is_empty() {
        return Err("deviceId is empty".into());
    }

    #[cfg(target_os = "windows")]
    {
        return tauri::async_runtime::spawn_blocking(move || {
            send_network_share_command_message(
                WM_USER_DISCONNECT_NETWORK_SHARE_DEVICE,
                "disconnect",
                &device_id,
                &device_name,
            )
        })
        .await?;
    }

    #[allow(unreachable_code)]
    Err("system_disconnect_network_share_device is only supported on Windows".into())
}

#[cfg(target_os = "windows")]
fn send_network_share_connect_message(device_id: &str, device_name: &str) -> Result<()> {
    send_network_share_command_message(
        WM_USER_CONNECT_NETWORK_SHARE_DEVICE,
        "connect",
        device_id,
        device_name,
    )
}

#[cfg(target_os = "windows")]
fn send_network_share_command_message(
    message: u32,
    action: &str,
    device_id: &str,
    device_name: &str,
) -> Result<()> {
    let hwnd = WindowsApi::find_window(
        None,
        None,
        None,
        Some(CONTROL_CENTER_AUX_WINDOW_CLASS.to_string()),
    )?;

    let payload = serde_json::json!({
        "deviceId": device_id,
        "deviceName": device_name,
        "businessId": NETWORK_SHARE_BUSINESS_ID,
    })
    .to_string();

    let mut bytes = payload.into_bytes();
    bytes.push(0);

    let cds = COPYDATASTRUCT {
        dwData: message as usize,
        cbData: bytes.len() as u32,
        lpData: bytes.as_mut_ptr().cast::<c_void>(),
    };

    unsafe {
        SendMessageW(
            hwnd,
            WM_COPYDATA,
            Some(WPARAM(0)),
            Some(LPARAM(&cds as *const _ as isize)),
        );
    }

    WindowsApi::post_message(hwnd, message, 0, 0)?;

    log::info!(
        "[NET][NetworkShare] Sent {} request to ControlCenterAux: message={}, deviceId={}, deviceName={}, businessId={}",
        action,
        message,
        device_id,
        device_name,
        NETWORK_SHARE_BUSINESS_ID
    );

    Ok(())
}

async fn system_get_wifi_networks_internal(source: &str) -> Result<Vec<WifiNetwork>> {
    #[cfg(target_os = "windows")]
    {
        // Prefer WLAN API for reliable enumeration across locales/encodings.
        match tauri::async_runtime::spawn_blocking(wlanapi_impl::system_get_wifi_networks_wlanapi)
            .await
        {
            Ok(Ok(list)) => {
                log::debug!(
                    "[NET][source={source}] WLAN API networks rows={}",
                    list.len()
                );
                return Ok(list);
            }
            Ok(Err(e)) => {
                log::warn!(
                    "[NET][source={source}] WLAN API failed, falling back to netsh. err={e:?}"
                );
            }
            Err(e) => {
                log::warn!(
                    "[NET][source={source}] WLAN API join failed, falling back to netsh. err={e:?}"
                );
            }
        }
    }

    // Use netsh to enumerate networks and interfaces which avoids depending
    // on windows crate feature flags.
    let mut networks: Vec<WifiNetwork> = Vec::new();
    let location_probe = tauri::async_runtime::spawn_blocking(probe_location_restriction_api)
        .await
        .unwrap_or(LocationRestrictionProbe::Unknown);
    log::debug!("[NET][source={source}] Location probe result: {location_probe:?}");
    let allow_text_fallback = matches!(location_probe, LocationRestrictionProbe::Unknown);
    let mut location_permission_restricted =
        matches!(location_probe, LocationRestrictionProbe::Restricted);
    // Get available networks with BSSID info (via tauri shell to avoid console window)
    let shell = crate::app::get_app_handle().shell();
    let available_out = match shell
        .command("netsh.exe")
        .args(["wlan", "show", "networks", "mode=bssid"])
        .output()
        .await
    {
        Ok(o) => {
            if !o.status.success() {
                log::warn!(
                    "[NET][source={source}] netsh show networks exit={:?}",
                    o.status.code()
                );
            }
            if !o.stderr.is_empty() {
                let stderr = decode_netsh_text(&o.stderr);
                log::debug!(
                    "[NET][source={source}] netsh show networks stderr: {}",
                    truncate_for_log(&stderr, 400)
                );
            }
            let stdout = decode_netsh_text(&o.stdout);
            if allow_text_fallback && is_wlan_location_permission_restricted_output(&stdout) {
                location_permission_restricted = true;
            }
            log::debug!(
                "[NET][source={source}] netsh show networks stdout_len={}",
                stdout.len()
            );
            stdout
        }
        Err(e) => {
            log::error!("[NET][source={source}] Failed to run netsh show networks: {e:?}");
            String::new()
        }
    };

    // Get current connected interface (via tauri shell)
    let interfaces_out = match shell
        .command("netsh.exe")
        .args(["wlan", "show", "interfaces"])
        .output()
        .await
    {
        Ok(o) => {
            if !o.status.success() {
                log::warn!(
                    "[NET][source={source}] netsh show interfaces exit={:?}",
                    o.status.code()
                );
            }
            if !o.stderr.is_empty() {
                let stderr = decode_netsh_text(&o.stderr);
                log::debug!(
                    "[NET][source={source}] netsh show interfaces stderr: {}",
                    truncate_for_log(&stderr, 400)
                );
            }
            let stdout = decode_netsh_text(&o.stdout);
            if allow_text_fallback && is_wlan_location_permission_restricted_output(&stdout) {
                location_permission_restricted = true;
            }
            log::debug!(
                "[NET][source={source}] netsh show interfaces stdout_len={}",
                stdout.len()
            );
            stdout
        }
        Err(e) => {
            log::error!("[NET][source={source}] Failed to run netsh show interfaces: {e:?}");
            String::new()
        }
    };

    if interfaces_out.trim().is_empty() {
        log::warn!("[NET][source={source}] netsh interfaces output is empty; connected SSID may be unknown");
    }

    let connected = parse_netsh_interface(&interfaces_out);
    log::info!(
        "[NET][source={source}] Interface parsed connected: {:?}",
        connected
    );
    let mut rows = parse_netsh_networks(&available_out);
    if rows.is_empty() && !available_out.trim().is_empty() {
        log::warn!(
            "[NET][source={source}] Parsed 0 networks from netsh show networks(mode=bssid). Will retry with simpler command. stdout_head={}",
            truncate_for_log(&available_out, 240)
        );

        let fallback_out = match shell
            .command("netsh.exe")
            .args(["wlan", "show", "networks"])
            .output()
            .await
        {
            Ok(o) => {
                if !o.status.success() {
                    log::warn!(
                        "[NET][source={source}] netsh show networks (fallback) exit={:?}",
                        o.status.code()
                    );
                }
                let stdout = decode_netsh_text(&o.stdout);
                if allow_text_fallback && is_wlan_location_permission_restricted_output(&stdout) {
                    location_permission_restricted = true;
                }
                log::debug!(
                    "[NET][source={source}] netsh show networks (fallback) stdout_len={} head={}",
                    stdout.len(),
                    truncate_for_log(&stdout, 240)
                );
                stdout
            }
            Err(e) => {
                log::error!(
                    "[NET][source={source}] Failed to run netsh show networks (fallback): {e:?}"
                );
                String::new()
            }
        };

        let fallback_rows = parse_netsh_networks(&fallback_out);
        if !fallback_rows.is_empty() {
            rows = fallback_rows;
        }
    }
    if location_permission_restricted && rows.is_empty() {
        log::warn!("[NET][source={source}] WLAN enumeration blocked by location privacy policy");
        return Err(WLAN_LOCATION_PERMISSION_REQUIRED_ERR.into());
    }

    log::debug!("[NET][source={source}] Parsed networks rows={}", rows.len());

    let connected_ssid = connected.as_ref().map(|(s, _)| s.clone());
    let connected_iface_sig = connected.as_ref().map(|(_, sig)| *sig);

    let mut connected_in_list = false;
    let mut connected_signal_in_list: Option<u32> = None;
    for (ssid, signal, security) in rows {
        let is_connected = connected.as_ref().map(|(s, _)| s == &ssid).unwrap_or(false);
        let mut effective_signal = signal;
        if is_connected {
            if let Some((_, iface_sig)) = connected.as_ref() {
                if *iface_sig > 0 {
                    effective_signal = *iface_sig;
                }
            }
            connected_in_list = true;
            connected_signal_in_list = Some(effective_signal);
        }
        networks.push(WifiNetwork {
            ssid,
            signal: effective_signal,
            security,
            connected: is_connected,
        });
    }

    // If connected network was not enumerated in list, add it to the output
    if let Some((ssid, signal)) = connected {
        if !networks.iter().any(|n| n.ssid == ssid) {
            networks.push(WifiNetwork {
                ssid,
                signal,
                security: String::from("unknown"),
                connected: true,
            });
            connected_in_list = true;
            connected_signal_in_list = Some(signal);
        }
    }

    // Consistency logs to debug icon mismatch reports.
    if let Some(ssid) = connected_ssid.as_deref() {
        if !connected_in_list {
            log::warn!(
                "[NET][source={source}] Connected SSID from interfaces not marked connected in list. ssid='{}' iface_signal={:?} list_len={}",
                ssid,
                connected_iface_sig,
                networks.len()
            );
        }
    }
    if connected_in_list {
        if let Some(sig) = connected_signal_in_list {
            if sig == 0 {
                log::warn!(
                    "[NET][source={source}] Connected network signal is 0. connected_ssid={:?} iface_signal={:?} list_len={}",
                    connected_ssid,
                    connected_iface_sig,
                    networks.len()
                );
            }
        }
    }
    log::debug!(
        "[NET][source={source}] Emit snapshot: list_len={}, connected_ssid={:?}, connected_signal={:?}",
        networks.len(),
        connected_ssid,
        connected_signal_in_list
    );
    Ok(networks)
}

#[tauri::command(async)]
pub async fn system_check_captive_portal() -> Result<bool> {
    // Minimal captive-portal heuristic:
    // - Fetch the Microsoft NCSI connect test endpoint with redirects disabled.
    // - If we get a redirect or HTML/unexpected content, assume captive portal.
    // - Network errors/offline => return false (don't show portal UI).
    //
    // This is used only to decide whether to show a "需登录" prompt with an
    // explicit "打开" action (no auto-opening browser).
    use tauri_plugin_http::reqwest;
    use tauri_plugin_http::reqwest::redirect::Policy;
    use tauri_plugin_http::reqwest::StatusCode;

    const TEST_URL: &str = "http://www.msftconnecttest.com/connecttest.txt";
    const EXPECTED_BODY: &str = "Microsoft Connect Test";

    let client = match reqwest::Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(4))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log::debug!("[NET][CAPTIVE] client build failed: {e:?}");
            return Ok(false);
        }
    };

    let resp = match client
        .get(TEST_URL)
        .header(reqwest::header::USER_AGENT, "MagicTaskbar")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            log::debug!("[NET][CAPTIVE] request failed: {e:?}");
            return Ok(false);
        }
    };

    let status = resp.status();

    // Any redirect without following is a strong captive signal.
    if status.is_redirection() {
        log::info!("[NET][CAPTIVE] detected redirect status={status}");
        return Ok(true);
    }

    if status == StatusCode::NO_CONTENT {
        return Ok(false);
    }

    if !status.is_success() {
        log::debug!("[NET][CAPTIVE] non-success status={status}");
        return Ok(false);
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    let body = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            log::debug!("[NET][CAPTIVE] read body failed: {e:?}");
            return Ok(false);
        }
    };

    let body_trim = body.trim();
    if body_trim == EXPECTED_BODY {
        return Ok(false);
    }

    // Common captive portal response is HTML.
    if content_type.contains("text/html")
        || body_trim.to_ascii_lowercase().contains("<html")
        || body_trim.to_ascii_lowercase().contains("<!doctype")
    {
        log::info!("[NET][CAPTIVE] detected html/unexpected body");
        return Ok(true);
    }

    // Unexpected successful response body => treat as captive (safer for UX).
    log::info!(
        "[NET][CAPTIVE] unexpected connecttest body_len={} => captive",
        body_trim.len()
    );
    Ok(true)
}

pub fn system_open_wifi_settings() -> Result<()> {
    // 使用 start 命令更快地打开 WiFi 设置
    let shell = crate::app::get_app_handle().shell();
    log::info!("[WLAN] opening Windows Settings quickly via start: ms-settings:network-wifi");

    // 首选 WLAN 页面
    if shell
        .command("cmd")
        .arg("/c")
        .arg("start ms-settings:network-wifi")
        .spawn()
        .is_err()
    {
        log::info!("[WLAN] wifi page spawn failed, fallback to network root");
        let _ = shell
            .command("cmd")
            .arg("/c")
            .arg("start ms-settings:network")
            .spawn();
    }
    Ok(())
}

pub fn system_open_location_settings() -> Result<()> {
    // 与 WLAN 设置保持一致：走 cmd/start，避免前端直接调用 Run 导致黑框闪现
    let shell = crate::app::get_app_handle().shell();
    log::info!("[WLAN] opening Windows Settings quickly via start: ms-settings:privacy-location");

    // 首选定位隐私页
    if shell
        .command("cmd")
        .arg("/c")
        .arg("start ms-settings:privacy-location")
        .spawn()
        .is_err()
    {
        log::info!("[WLAN] location privacy page spawn failed, fallback to privacy root");
        let _ = shell
            .command("cmd")
            .arg("/c")
            .arg("start ms-settings:privacy")
            .spawn();
    }

    Ok(())
}

pub fn system_open_wlan_flyout() -> Result<()> {
    // Open the bottom-right Available Networks flyout (Win10/Win11).
    // Keep this as a separate API so existing logic (SystemOpenWifiSettings) remains unchanged.
    let shell = crate::app::get_app_handle().shell();
    log::info!("[WLAN] opening WLAN flyout via ms-availablenetworks:");

    // Prefer explorer.exe which can handle URI schemes without cmd quoting quirks.
    if shell
        .command("explorer")
        .arg("ms-availablenetworks:")
        .spawn()
        .is_ok()
    {
        return Ok(());
    }

    // Fallback: cmd/start (some environments may not resolve explorer correctly)
    if shell
        .command("cmd")
        .arg("/c")
        .arg("start ms-availablenetworks:")
        .spawn()
        .is_ok()
    {
        return Ok(());
    }

    log::info!("[WLAN] flyout spawn failed, fallback to wifi settings");
    let _ = system_open_wifi_settings();

    Ok(())
}

pub fn register_network_events() {
    log::info!("[WLAN] network events are forwarded by magictaskbar-srv");
}

#[allow(dead_code)]
fn register_network_events_ui_legacy_disabled() {
    #[cfg(target_os = "windows")]
    {
        use std::ffi::c_void;
        use std::sync::{mpsc, OnceLock};

        static INIT: OnceLock<()> = OnceLock::new();
        if INIT.set(()).is_err() {
            // Already registered
            return;
        }

        // Minimal FFI bindings to wlanapi.dll
        #[link(name = "wlanapi")]
        extern "system" {
            fn WlanOpenHandle(
                dwClientVersion: u32,
                pReserved: *mut c_void,
                pdwNegotiatedVersion: *mut u32,
                phClientHandle: *mut *mut c_void,
            ) -> u32;
            fn WlanRegisterNotification(
                hClientHandle: *mut c_void,
                dwNotifSource: u32,
                bIgnoreDuplicate: i32,
                funcCallback: Option<extern "system" fn(*mut c_void, *mut c_void)>,
                pCallbackContext: *mut c_void,
                pReserved: *mut c_void,
                pdwPrevNotifSource: *mut u32,
            ) -> u32;
            fn WlanCloseHandle(hClientHandle: *mut c_void, pReserved: *mut c_void) -> u32;
        }

        // Struct to read WLAN_NOTIFICATION_DATA as per API docs
        #[repr(C)]
        struct WLAN_NOTIFICATION_DATA {
            notification_source: u32,
            notification_code: u32,
            interface_guid: [u8; 16],
            dw_data_size: u32,
            p_data: *mut c_void,
        }

        // Notification source bits
        const WLAN_NOTIFICATION_SOURCE_ACM: u32 = 0x0000_0008; // Auto Config Manager
        const WLAN_NOTIFICATION_SOURCE_MSM: u32 = 0x0000_0010; // Media Specific Module
                                                               // 正确的 ACM 事件代码 (参照 wlanapi.h 枚举顺序从 0 开始)
                                                               // 索引: 10 => connection_complete, 21 => disconnected
        const WLAN_NOTIFICATION_ACM_CONNECTION_COMPLETE: u32 = 10; // connection_complete
        const WLAN_NOTIFICATION_ACM_DISCONNECTED: u32 = 21; // disconnected

        const ERROR_SUCCESS: u32 = 0;

        // Channel to decouple callback from work
        static TX: OnceLock<mpsc::Sender<NotificationMsg>> = OnceLock::new();
        enum NotificationMsg {
            Connected,
            Disconnected,
            SignalChanged(u32),
            Other, // ignored
        }

        fn is_connected_only_snapshot(list: &[WifiNetwork]) -> bool {
            list.len() == 1 && list.first().map(|item| item.connected).unwrap_or(false)
        }

        fn fetch_event_networks(wait_for_full_list: bool) -> Option<Vec<WifiNetwork>> {
            let attempts = if wait_for_full_list { 6 } else { 1 };
            for attempt in 0..attempts {
                match tauri::async_runtime::block_on(system_get_wifi_networks_internal(
                    if wait_for_full_list {
                        "event-wait-full"
                    } else {
                        "event-immediate"
                    },
                )) {
                    Ok(list) => {
                        if !wait_for_full_list || !is_connected_only_snapshot(&list) {
                            return Some(list);
                        }

                        log::debug!(
                            "[WLAN] Suppressing connected-only event snapshot attempt {}/{}; waiting for fuller list",
                            attempt + 1,
                            attempts
                        );
                    }
                    Err(err) => {
                        log::warn!("[WLAN] Failed to fetch event network snapshot: {err:?}");
                        return None;
                    }
                }

                if attempt + 1 < attempts {
                    std::thread::sleep(std::time::Duration::from_millis(350));
                }
            }

            log::debug!("[WLAN] Dropping connected-only event snapshot after retries");
            None
        }

        // Worker thread consuming notifications and emitting events only on changes
        let (tx, rx) = mpsc::channel::<NotificationMsg>();
        TX.set(tx).ok();
        std::thread::spawn(move || {
            let mut last_signal: Option<u32> = None;
            let mut last_connected: Option<bool> = None;
            let mut last_connected_ssid: Option<String> = None;
            while let Ok(msg) = rx.recv() {
                match msg {
                    NotificationMsg::Connected => {
                        if last_connected != Some(true) {
                            last_connected = Some(true);
                            if let Some(list) = fetch_event_networks(true) {
                                if let Some(current) = list.iter().find(|n| n.connected) {
                                    log::info!(
                                        "[WLAN] Connected: ssid='{}', signal={} (0-100)",
                                        current.ssid,
                                        current.signal
                                    );
                                    last_signal = Some(current.signal as u32);
                                    last_connected_ssid = Some(current.ssid.clone());
                                } else {
                                    log::warn!("[WLAN] Connected event but no connected entry in list (netsh parse mismatch)");
                                    // keep previous ssid if we couldn't parse it this time
                                }
                                let app = crate::app::get_app_handle();
                                let _ = app.emit(FuncEvent::SystemNetworksChanged, &list);
                                if let Some(cur) = list.iter().find(|n| n.connected) {
                                    log::debug!(
                                        "[WLAN] Emit connected snapshot: ssid='{}' signal={}",
                                        cur.ssid,
                                        cur.signal
                                    );
                                } else {
                                    log::debug!("[WLAN] Emit connected snapshot: <none>");
                                }
                            }
                        }
                    }
                    NotificationMsg::Disconnected => {
                        if last_connected != Some(false) {
                            last_connected = Some(false);
                            last_connected_ssid = None;
                            last_signal = None;
                            log::info!("[WLAN] Disconnected");
                            if let Some(list) = fetch_event_networks(false) {
                                let app = crate::app::get_app_handle();
                                let _ = app.emit(FuncEvent::SystemNetworksChanged, &list);
                            }
                        }
                    }
                    NotificationMsg::SignalChanged(sig) => {
                        if last_signal.map(|v| v != sig).unwrap_or(true) {
                            last_signal = Some(sig);
                            log::info!("[WLAN] Signal quality changed: {} (0-100)", sig);
                            if let Some(mut list) = fetch_event_networks(true) {
                                // netsh sometimes reports signal as 0 or fails parsing on some machines.
                                // We already have a reliable 0-100 quality from WLAN notifications, so
                                // patch it into the emitted list to keep the toolbar icon in sync.
                                if last_connected == Some(true) {
                                    if let Some(ssid) = last_connected_ssid.as_deref() {
                                        if let Some(entry) =
                                            list.iter_mut().find(|n| n.ssid == ssid)
                                        {
                                            entry.signal = sig;
                                            entry.connected = true;
                                            log::debug!(
                                                "[WLAN] Patched signal into list by ssid='{}': {}",
                                                ssid,
                                                sig
                                            );
                                        } else if let Some(entry) =
                                            list.iter_mut().find(|n| n.connected)
                                        {
                                            entry.signal = sig;
                                            log::debug!("[WLAN] Patched signal into list by connected entry: {}", sig);
                                        } else {
                                            log::warn!("[WLAN] SignalChanged but no connected entry to patch (ssid_hint='{}')", ssid);
                                        }
                                    } else if let Some(entry) =
                                        list.iter_mut().find(|n| n.connected)
                                    {
                                        entry.signal = sig;
                                        log::debug!("[WLAN] Patched signal into list by connected entry: {}", sig);
                                    } else {
                                        log::warn!("[WLAN] SignalChanged but connected SSID is unknown and list has no connected entry");
                                    }
                                }
                                let app = crate::app::get_app_handle();
                                let _ = app.emit(FuncEvent::SystemNetworksChanged, &list);
                            }
                        } else {
                            log::debug!("[WLAN] Signal unchanged: {} (0-100)", sig);
                        }
                    }
                    NotificationMsg::Other => { /* ignore */ }
                }
            }
        });

        // Registration thread
        std::thread::spawn(|| {
            unsafe {
                let mut negotiated_version: u32 = 0;
                let mut h_client: *mut c_void = std::ptr::null_mut();
                let res = WlanOpenHandle(
                    2,
                    std::ptr::null_mut(),
                    &mut negotiated_version as *mut u32,
                    &mut h_client as *mut *mut c_void,
                );
                if res != ERROR_SUCCESS {
                    log::error!("WlanOpenHandle failed: {}", res);
                    return;
                }
                log::info!(
                    "[WLAN] Opened handle. negotiated_version={}",
                    negotiated_version
                );

                extern "system" fn wlan_callback(p_data: *mut c_void, _context: *mut c_void) {
                    if p_data.is_null() {
                        return;
                    }
                    let data = unsafe { &*(p_data as *const WLAN_NOTIFICATION_DATA) };
                    let src = data.notification_source;
                    let code = data.notification_code;

                    // MSM: 信号强度变化 (payload 为 4 字节 DWORD 0-100)
                    if src & WLAN_NOTIFICATION_SOURCE_MSM != 0 {
                        if data.dw_data_size == 4 && !data.p_data.is_null() {
                            let sig = unsafe { *(data.p_data as *const u32) };
                            if let Some(tx) = TX.get() {
                                let _ = tx.send(NotificationMsg::SignalChanged(sig));
                            }
                        } else {
                            // 其它 MSM 事件忽略（避免重复噪声）
                        }
                    } else if src & WLAN_NOTIFICATION_SOURCE_ACM != 0 {
                        // 只处理连接完成与断开，其它忽略
                        if code == WLAN_NOTIFICATION_ACM_CONNECTION_COMPLETE {
                            if let Some(tx) = TX.get() {
                                let _ = tx.send(NotificationMsg::Connected);
                            }
                        } else if code == WLAN_NOTIFICATION_ACM_DISCONNECTED {
                            if let Some(tx) = TX.get() {
                                let _ = tx.send(NotificationMsg::Disconnected);
                            }
                        } else {
                            // 忽略 scan / available 等事件
                        }
                    } else {
                        if let Some(tx) = TX.get() {
                            let _ = tx.send(NotificationMsg::Other);
                        }
                    }
                }

                let notify_sources = WLAN_NOTIFICATION_SOURCE_ACM | WLAN_NOTIFICATION_SOURCE_MSM;
                let dw_reg = WlanRegisterNotification(
                    h_client,
                    notify_sources,
                    1,
                    Some(wlan_callback),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
                if dw_reg != ERROR_SUCCESS {
                    log::error!("WlanRegisterNotification failed: {}", dw_reg);
                    let _ = WlanCloseHandle(h_client, std::ptr::null_mut());
                    return;
                }

                // Keep thread alive
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(3600));
                }
            }
        });
    }
}
