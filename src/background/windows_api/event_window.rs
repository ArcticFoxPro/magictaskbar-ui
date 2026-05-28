use crate::app::get_app_handle;
use crate::modules::network::infrastructure::{
    add_network_share_devices, remove_network_share_devices_by_ids,
    set_network_share_device_connected, update_network_share_devices,
};
use crate::modules::notification::application::AppNotificationService;
use lazy_static::lazy_static;
use parking_lot::Mutex;
use serde::Deserialize;
use std::sync::{
    atomic::{AtomicIsize, AtomicU32, Ordering},
    Arc,
};
use tauri::Emitter;
use windows::Win32::{
    Devices::Display::GUID_DEVINTERFACE_MONITOR,
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostQuitMessage,
        RegisterClassW, RegisterDeviceNotificationW, RegisterShellHookWindow,
        RegisterWindowMessageW, TranslateMessage, DBT_DEVTYP_DEVICEINTERFACE,
        DEVICE_NOTIFY_WINDOW_HANDLE, DEV_BROADCAST_DEVICEINTERFACE_W, MSG, WINDOW_EX_STYLE,
        WINDOW_STYLE, WM_COPYDATA, WM_DESTROY, WM_USER, WNDCLASSW,
    },
};

use crate::{
    error::{Result, WindowsResultExt},
    log_error, trace_lock,
    utils::spawn_named_thread,
};
use libs_core::system_state::NetworkShareDevice;

use super::{string_utils::WindowsString, WindowsApi};

type Callback = Box<dyn Fn(u32, usize, isize) -> Result<()> + Send + Sync + 'static>;

lazy_static! {
    static ref CALLBACKS: Arc<Mutex<Vec<Callback>>> = Arc::new(Mutex::new(Vec::new()));
    static ref NOTIFICATION_SERVICE: Mutex<AppNotificationService> =
        Mutex::new(AppNotificationService::new());
}

pub static mut WM_SHELLHOOKMESSAGE: u32 = u32::MAX;
pub static BACKGROUND_HWND: AtomicIsize = AtomicIsize::new(0);

/// TaskbarCreated 消息 ID，用于监听 explorer.exe 重启
/// https://learn.microsoft.com/en-us/windows/win32/shell/taskbar#taskbarcreated-message
pub static WM_TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
/// 缓存的 explorer.exe 进程 PID，用于检测进程重启
pub static CACHED_EXPLORER_PID: AtomicU32 = AtomicU32::new(0);

const COPYDATA_DEVICE_ONLINE: usize = (WM_USER + 200) as usize;
const COPYDATA_ALL_ONLINE: usize = (WM_USER + 201) as usize;
const COPYDATA_DEVICE_OFFLINE: usize = (WM_USER + 202) as usize;
const COPYDATA_CONNECT_COMM: usize = (WM_USER + 203) as usize;

#[repr(C)]
#[allow(non_snake_case)]
pub struct CopyDataStruct {
    pub dwData: usize,                    // ULONG_PTR
    pub cbData: u32,                      // DWORD
    pub lpData: *const core::ffi::c_void, // PVOID
}

#[derive(Debug, Deserialize)]
struct DeviceInfoItemPayload {
    #[serde(rename = "deviceId", default)]
    device_id: String,
    #[serde(rename = "deviceName", default)]
    device_name: String,
    #[serde(rename = "deviceType", default)]
    device_type: String,
    #[serde(rename = "isNetShareSchedulable")]
    is_net_share_schedulable: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct DeviceInfoListPayload {
    #[serde(default)]
    message: String,
    #[serde(rename = "items_length")]
    items_length: i32,
    #[serde(default)]
    items: Vec<DeviceInfoItemPayload>,
}

#[derive(Debug, Deserialize)]
struct ConnectCommPayload {
    #[serde(default)]
    message: String,
    #[serde(rename = "msgType", default)]
    msg_type: String,
    #[serde(rename = "businessId", default)]
    business_id: String,
    #[serde(rename = "remoteDeviceId", default)]
    remote_device_id: String,
    #[serde(rename = "extraString", default)]
    extra_string: String,
}

fn decode_copydata_text(cds: &CopyDataStruct) -> Option<String> {
    if cds.lpData.is_null() || cds.cbData == 0 {
        return None;
    }

    let bytes = unsafe { std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize) };
    let (text, _, _) = encoding_rs::UTF_8.decode(bytes);
    let mut text = text.into_owned();
    if let Some(pos) = text.find('\0') {
        text.truncate(pos);
    }
    Some(text)
}

fn log_device_info_payload(kind: &str, text: &str) {
    match serde_json::from_str::<DeviceInfoListPayload>(text) {
        Ok(payload) => {
            log::warn!(
                "[DeviceInfo] {} payload parsed: message={}, items_length={}, actual_items={}",
                kind,
                payload.message,
                payload.items_length,
                payload.items.len()
            );

            for (index, item) in payload.items.iter().enumerate() {
                log::warn!(
                    "[DeviceInfo] {} item[{}]: deviceId={}, deviceName={}, deviceType={}, isNetShareSchedulable={}",
                    kind,
                    index,
                    item.device_id,
                    item.device_name,
                    item.device_type,
                    item.is_net_share_schedulable.unwrap_or(-1)
                );
            }

            if kind == "ALL_ONLINE" {
                let devices: Vec<NetworkShareDevice> = payload
                    .items
                    .iter()
                    .filter(|item| {
                        item.is_net_share_schedulable == Some(1)
                            && item.device_type.to_uppercase().contains("PHONE")
                    })
                    .map(|item| NetworkShareDevice {
                        device_id: item.device_id.clone(),
                        device_name: item.device_name.clone(),
                        connected: false,
                    })
                    .collect();

                log::info!(
                    "[DeviceInfo] ALL_ONLINE schedulable devices filtered: {}",
                    devices.len()
                );
                update_network_share_devices(devices);
            } else if kind == "DEVICE_ONLINE" {
                let devices: Vec<NetworkShareDevice> = payload
                    .items
                    .iter()
                    .filter(|item| {
                        item.is_net_share_schedulable == Some(1)
                            && item.device_type.to_uppercase().contains("PHONE")
                    })
                    .map(|item| NetworkShareDevice {
                        device_id: item.device_id.clone(),
                        device_name: item.device_name.clone(),
                        connected: false,
                    })
                    .collect();

                log::info!(
                    "[DeviceInfo] DEVICE_ONLINE schedulable devices filtered: {}",
                    devices.len()
                );
                add_network_share_devices(devices);
            } else if kind == "DEVICE_OFFLINE" {
                let device_ids: Vec<String> = payload
                    .items
                    .iter()
                    .map(|item| item.device_id.clone())
                    .filter(|device_id| !device_id.is_empty())
                    .collect();

                log::info!(
                    "[DeviceInfo] DEVICE_OFFLINE device ids filtered: {}",
                    device_ids.len()
                );
                remove_network_share_devices_by_ids(device_ids);
            }
        }
        Err(error) => {
            log::error!(
                "[DeviceInfo] {} payload parse failed: {} | raw={}",
                kind,
                error,
                text
            );
        }
    }
}

fn log_connect_comm_payload(text: &str) {
    match serde_json::from_str::<ConnectCommPayload>(text) {
        Ok(payload) => {
            log::warn!(
                "[ConnectComm] payload parsed: message={}, msgType={}, businessId={}, remoteDeviceId={}, extraString={}",
                payload.message,
                payload.msg_type,
                payload.business_id,
                payload.remote_device_id,
                payload.extra_string
            );

            if payload.business_id == "HnNetworkShare" {
                if payload.msg_type == "Disconnected" {
                    log::info!(
                        "[ConnectComm] HnNetworkShare device disconnected: remoteDeviceId={}",
                        payload.remote_device_id
                    );
                    set_network_share_device_connected(&payload.remote_device_id, false);
                } else if payload.msg_type == "ConnectSuccess" {
                    log::info!(
                        "[ConnectComm] HnNetworkShare device connected: remoteDeviceId={}",
                        payload.remote_device_id
                    );
                    set_network_share_device_connected(&payload.remote_device_id, true);
                }
            }
        }
        Err(error) => {
            log::error!(
                "[ConnectComm] payload parse failed: {} | raw={}",
                error,
                text
            );
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if msg == WM_DESTROY {
        PostQuitMessage(0);
        return LRESULT(0);
    }

    if msg == unsafe { WM_SHELLHOOKMESSAGE } {
        // 处理 Shell Hook 事件，特别是 HSHELL_FLASH 事件
        let wparam = w_param.0 as u32;
        let mut notification_service = NOTIFICATION_SERVICE.lock();
        notification_service.process_shell_hook_event(wparam, l_param.0);
    }

    // 处理 TaskbarCreated 消息（explorer.exe 重启）
    let taskbar_created_msg = WM_TASKBAR_CREATED.load(Ordering::SeqCst);
    if taskbar_created_msg != 0 && msg == taskbar_created_msg {
        log::warn!("[EventWindow] TaskbarCreated message received!");
        handle_explorer_restart();
    }

    // 处理快捷键消息
    if msg == WM_COPYDATA {
        if l_param.0 == 0 {
            return LRESULT(0);
        }

        let cds = unsafe { &*(l_param.0 as *const CopyDataStruct) };

        log::info!(
            "[WM_COPYDATA] dwData={}, cbData={}, lpData={:p}",
            cds.dwData,
            cds.cbData,
            cds.lpData
        );
        if cds.cbData > 1024 * 1024 {
            log::error!("cbData too large: {}", cds.cbData);
            return LRESULT(0);
        }

        if matches!(
            cds.dwData,
            COPYDATA_DEVICE_ONLINE
                | COPYDATA_ALL_ONLINE
                | COPYDATA_DEVICE_OFFLINE
                | COPYDATA_CONNECT_COMM
        ) {
            let Some(text) = decode_copydata_text(cds) else {
                log::warn!(
                    "[WM_COPYDATA] Serialized payload missing: dwData={}, cbData={}",
                    cds.dwData,
                    cds.cbData
                );
                return LRESULT(0);
            };

            match cds.dwData {
                COPYDATA_DEVICE_ONLINE => log_device_info_payload("DEVICE_ONLINE", &text),
                COPYDATA_ALL_ONLINE => log_device_info_payload("ALL_ONLINE", &text),
                COPYDATA_DEVICE_OFFLINE => log_device_info_payload("DEVICE_OFFLINE", &text),
                COPYDATA_CONNECT_COMM => log_connect_comm_payload(&text),
                _ => {}
            }

            return LRESULT(1);
        }

        if cds.dwData == 1 as usize {
            if !cds.lpData.is_null() && cds.cbData > 0 {
                let slice = unsafe {
                    std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize)
                };

                // C 字符串（以 \0 结尾）
                if let Ok(text) = std::ffi::CStr::from_bytes_with_nul(slice) {
                    if let Ok(text) = text.to_str() {
                        log::info!("[Shortcut] Received key: {}", text);

                        // 发送图标事件到前端
                        let app_handle = get_app_handle();
                        let _ = app_handle.emit("shortcut-message", text);

                        return LRESULT(1); // 告诉发送方“我收到了”
                    }
                }
            }
        }

        // dwData == 0x02: 三方软件管控列表数据
        if cds.dwData == 0x02 {
            log::info!("[ThirdPartyControl] >>> WM_COPYDATA received with dwData=0x02");
            if !cds.lpData.is_null() && cds.cbData > 0 {
                log::info!("[ThirdPartyControl] lpData is valid, cbData={}", cds.cbData);
                let json_bytes = unsafe {
                    std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize)
                };

                // 使用 UTF-8 解码（MagicSpaceTurbo 发送的是 UTF-8 编码的中文）
                let (json_cow, _used, _had_errors) = encoding_rs::UTF_8.decode(json_bytes);
                let mut json_str = json_cow.to_string();

                // 移除末尾的 \0 字符
                if let Some(pos) = json_str.find('\0') {
                    json_str.truncate(pos);
                }

                log::info!("[ThirdPartyControl] Received JSON: {}", json_str);

                // 发送到前端
                let app_handle = get_app_handle();
                match app_handle.emit("third-party-control-list", &json_str) {
                    Ok(_) => log::info!("[ThirdPartyControl] Event emitted successfully"),
                    Err(e) => log::warn!("[ThirdPartyControl] Failed to emit event: {}", e),
                }

                return LRESULT(1); // 告诉发送方“我收到了”
            } else {
                log::warn!(
                    "[ThirdPartyControl] Invalid data: lpData={:?}, cbData={}",
                    cds.lpData,
                    cds.cbData
                );
            }
        }

        // dwData == 0x03: 开机自启列表数据
        if cds.dwData == 0x03 {
            log::info!("[AppStartup] >>> WM_COPYDATA received with dwData=0x03");
            if !cds.lpData.is_null() && cds.cbData > 0 {
                let json_bytes = unsafe {
                    std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize)
                };

                let (json_cow, _used, _had_errors) = encoding_rs::UTF_8.decode(json_bytes);
                let mut json_str = json_cow.to_string();

                if let Some(pos) = json_str.find('\0') {
                    json_str.truncate(pos);
                }

                log::info!("[AppStartup] Received JSON: {}", json_str);

                let app_handle = get_app_handle();
                match app_handle.emit("app-startup-list", &json_str) {
                    Ok(_) => log::info!("[AppStartup] Event emitted successfully"),
                    Err(e) => log::warn!("[AppStartup] Failed to emit event: {}", e),
                }

                return LRESULT(1);
            } else {
                log::warn!(
                    "[AppStartup] Invalid data: lpData={:?}, cbData={}",
                    cds.lpData,
                    cds.cbData
                );
            }
        }

        return LRESULT(0);
    }

    if msg != WM_COPYDATA {
        for callback in trace_lock!(CALLBACKS).iter() {
            log_error!(callback(msg, w_param.0, l_param.0));
        }
    }

    DefWindowProcW(hwnd, msg, w_param, l_param)
}

/// will lock until the window is closed
unsafe fn _create_background_window(done: &crossbeam_channel::Sender<()>) -> Result<()> {
    let title = WindowsString::from("UI Background Window");
    let class = WindowsString::from("BackgroundWindow");

    let h_module = WindowsApi::module_handle_w()?;

    let wnd_class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: h_module.into(),
        lpszClassName: class.as_pcwstr(),
        ..Default::default()
    };

    RegisterClassW(&wnd_class);

    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        class.as_pcwstr(),
        title.as_pcwstr(),
        WINDOW_STYLE::default(),
        0,
        0,
        0,
        0,
        None,
        None,
        Some(wnd_class.hInstance),
        None,
    )?;

    let handle: isize = hwnd.0 as isize;
    BACKGROUND_HWND.store(handle, Ordering::Relaxed);
    // register window to recieve device notifications for monitor changes
    {
        let mut notification_filter = DEV_BROADCAST_DEVICEINTERFACE_W {
            dbcc_size: std::mem::size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
            dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0,
            dbcc_reserved: 0,
            dbcc_classguid: GUID_DEVINTERFACE_MONITOR,
            dbcc_name: [0; 1],
        };
        RegisterDeviceNotificationW(
            hwnd.into(),
            &mut notification_filter as *mut _ as *mut _,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )?;
    }

    // register window to recieve shell events
    {
        RegisterShellHookWindow(hwnd).ok().filter_fake_error()?;
        let msg = WindowsString::from("SHELLHOOK");
        WM_SHELLHOOKMESSAGE = RegisterWindowMessageW(msg.as_pcwstr());
    }

    // 注册 TaskbarCreated 消息，用于监听 explorer.exe 重启
    {
        let taskbar_created_msg = WindowsString::from("TaskbarCreated");
        let msg_id = RegisterWindowMessageW(taskbar_created_msg.as_pcwstr());
        WM_TASKBAR_CREATED.store(msg_id, Ordering::SeqCst);
        log::info!(
            "[EventWindow] Registered TaskbarCreated message: {}",
            msg_id
        );

        // 初始化缓存的 explorer PID
        if let Ok(explorer_hwnd) =
            WindowsApi::find_window(None, None, None, Some("Shell_TrayWnd".to_string()))
        {
            let (pid, _) = WindowsApi::window_thread_process_id(explorer_hwnd);
            CACHED_EXPLORER_PID.store(pid, Ordering::SeqCst);
            log::info!("[EventWindow] Initial explorer PID: {}", pid);
        }
    }

    done.send(())?;
    let mut msg = MSG::default();

    // GetMessageW will run until PostQuitMessage(0) is called
    while GetMessageW(&mut msg, Some(hwnd), 0, 0).into() {
        TranslateMessage(&msg).ok().filter_fake_error()?;
        DispatchMessageW(&msg);
    }
    Ok(())
}

/// the objective with this window is having a thread that will receive window events
/// and propagate them across the application (common events are keyboard, power, display, etc)
pub fn create_background_window() -> Result<()> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    spawn_named_thread("Background Window", move || {
        log::trace!("Creating background window...");
        log_error!(unsafe { _create_background_window(&tx) });
    })?;
    rx.recv()?;
    log::trace!("Background window created");
    Ok(())
}

pub fn subscribe_to_background_window<F>(callback: F)
where
    F: Fn(u32, usize, isize) -> Result<()> + Send + Sync + 'static,
{
    trace_lock!(CALLBACKS).push(Box::new(callback));
}

/// 处理 explorer.exe 重启事件
/// 当收到 TaskbarCreated 消息时调用
fn handle_explorer_restart() {
    log::warn!("[EventWindow] Explorer restart detected (TaskbarCreated message received)");

    // 获取新的 explorer PID
    let new_pid = match WindowsApi::find_window(None, None, None, Some("Shell_TrayWnd".to_string()))
    {
        Ok(hwnd) => {
            let (pid, _) = WindowsApi::window_thread_process_id(hwnd);
            pid
        }
        Err(e) => {
            log::error!("[EventWindow] Failed to find Shell_TrayWnd: {:?}", e);
            return;
        }
    };

    // 获取缓存的旧 PID
    let old_pid = CACHED_EXPLORER_PID.load(Ordering::SeqCst);

    log::info!(
        "[EventWindow] Explorer PID comparison: old={}, new={}",
        old_pid,
        new_pid
    );

    // 只有在 PID 确实发生变化时才重新注册 AppBar
    if old_pid != 0 && old_pid != new_pid {
        log::info!(
            "[EventWindow] Explorer PID changed from {} to {}, will re-register AppBars if in fixed mode",
            old_pid, new_pid
        );

        // 更新缓存的 PID
        CACHED_EXPLORER_PID.store(new_pid, Ordering::SeqCst);

        // 重新注册所有处于固定模式的 toolbar AppBar
        reregister_toolbar_appbars();
    } else {
        // 即使 PID 没变，也更新缓存（可能是首次获取）
        CACHED_EXPLORER_PID.store(new_pid, Ordering::SeqCst);
        log::debug!(
            "[EventWindow] Explorer PID unchanged ({}), no AppBar re-registration needed",
            new_pid
        );
    }
}

/// 重新注册所有处于固定模式的 toolbar AppBar
fn reregister_toolbar_appbars() {
    use crate::app::APP_MANAGER;
    use crate::state::application::FULL_STATE;
    use crate::trace_lock;
    use libs_core::state::HideMode;

    let state = FULL_STATE.load();
    let hide_mode = state.settings.by_widget.fancy_toolbar.hide_mode;

    // 只有固定模式（HideMode::Never）才需要重新注册 AppBar
    if hide_mode != HideMode::Never {
        log::debug!(
            "[EventWindow] Toolbar is not in fixed mode (hide_mode={:?}), skipping AppBar re-registration",
            hide_mode
        );
        return;
    }

    log::info!("[EventWindow] Re-registering AppBars for fixed mode toolbars...");

    let manager = crate::trace_read!(APP_MANAGER);
    for instance in &manager.instances {
        let mut toolbar = trace_lock!(instance.toolbar);
        if let Some(tl) = toolbar.as_mut() {
            if let Err(e) = tl.reregister_appbar() {
                log::error!("[EventWindow] Failed to re-register AppBar: {:?}", e);
            }
        }
    }

    log::info!("[EventWindow] AppBar re-registration completed");
}
