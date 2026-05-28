use libs_core::{
    resource::{ResourceId, ResourceKind, SluResource},
    state::Theme,
};

use crate::{error::Result, log_error, resources::RESOURCES};
use std::sync::Arc;

#[tauri::command(async)]
pub fn remove_resource(kind: ResourceKind, id: ResourceId) -> Result<()> {
    match kind {
        ResourceKind::Theme => {
            RESOURCES.themes.retain(|_, v| {
                if *v.id == id && !v.metadata.internal.bundled {
                    log_error!(v.delete());
                    return false;
                }
                true
            });
        }
        ResourceKind::IconPack => {
            RESOURCES.icon_packs.retain(|_, v| {
                if *v.id == id && !v.metadata.internal.bundled {
                    log_error!(v.delete());
                    return false;
                }
                true
            });
        }
        _ => {}
    }
    RESOURCES.emit_kind_changed(&kind)?;
    Ok(())
}

#[tauri::command(async)]
pub fn state_get_themes() -> Vec<Arc<Theme>> {
    let mut themes = Vec::new();
    RESOURCES.themes.scan(|_, v| {
        themes.push(v.clone());
    });
    themes
}

/// 发送开机自启状态变更到 MagicSpaceTurbo 窗口（WM_COPYDATA dwData=0x03）
/// 按照接收方 StartUpSofwareInfoFixed 结构体布局发送二进制数据
#[tauri::command(async)]
pub fn send_app_startup_status(
    name: String,
    display_name: String,
    display_name_utf8: String,
    description: String,
    description_utf8: String,
    status: bool,
) -> crate::error::Result<()> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::System::DataExchange::COPYDATASTRUCT;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SendMessageW, WM_COPYDATA};

    log::info!(
        "[AppStartup] >>> 发送状态变更: name={}, status={}",
        name,
        status
    );

    let target_hwnd = match unsafe { FindWindowW(windows_core::w!("MagicSpaceTurbo"), None) } {
        Ok(h) if !h.is_invalid() => h,
        _ => {
            let code = unsafe { windows::Win32::Foundation::GetLastError().0 };
            log::warn!("[AppStartup] FindWindowW failed, code={}", code);
            return Ok(());
        }
    };

    log::info!("[AppStartup] Window found, HWND: {:?}", target_hwnd);

    // 构造与接收方 StartUpSofwareInfoFixed 完全一致的二进制结构体
    #[repr(C)]
    struct StartUpSofwareInfoFixed {
        name: [u8; 256],
        display_name: [u8; 256],
        display_name_utf8: [u8; 256],
        description: [u8; 512],
        description_utf8: [u8; 512],
        status: i32,
    }

    let mut info = StartUpSofwareInfoFixed {
        name: [0u8; 256],
        display_name: [0u8; 256],
        display_name_utf8: [0u8; 256],
        description: [0u8; 512],
        description_utf8: [0u8; 512],
        status: if status { 1 } else { 0 },
    };

    // 填入所有字段
    let copy_str = |dst: &mut [u8], src: &str| {
        let bytes = src.as_bytes();
        let len = bytes.len().min(dst.len() - 1);
        dst[..len].copy_from_slice(&bytes[..len]);
    };
    copy_str(&mut info.name, &name);
    copy_str(&mut info.display_name, &display_name);
    copy_str(&mut info.display_name_utf8, &display_name_utf8);
    copy_str(&mut info.description, &description);
    copy_str(&mut info.description_utf8, &description_utf8);

    let struct_size = std::mem::size_of::<StartUpSofwareInfoFixed>();

    let cds = COPYDATASTRUCT {
        dwData: 0x03,
        cbData: struct_size as u32,
        lpData: &info as *const _ as *mut c_void,
    };

    log::info!(
        "[AppStartup] Sending WM_COPYDATA: dwData=0x03, cbData={}, name={}, status={}",
        struct_size,
        name,
        info.status
    );

    unsafe {
        SendMessageW(
            target_hwnd,
            WM_COPYDATA,
            Some(WPARAM(0)),
            Some(LPARAM(&cds as *const _ as isize)),
        );
        log::info!("[AppStartup] WM_COPYDATA sent!");
    }

    Ok(())
}

/// 发送三方软件管控状态变更到 MagicSpaceTurbo 窗口
#[tauri::command(async)]
pub fn send_third_party_app_status(
    category: String,
    app_name: String,
    status: String,
) -> Result<()> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::System::DataExchange::COPYDATASTRUCT;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SendMessageW};

    log::info!(
        "[ThirdPartyControl] >>> 开始发送 WM_COPYDATA 消息：appName={}, status={}",
        app_name,
        status
    );

    // 查找 MagicSpaceTurbo 窗口
    let target_hwnd = match unsafe { FindWindowW(windows_core::w!("MagicSpaceTurbo"), None) } {
        Ok(h) if !h.is_invalid() => h,
        _ => {
            let code = unsafe { windows::Win32::Foundation::GetLastError().0 };
            log::warn!("[ThirdPartyControl] FindWindowW failed, code={}", code);
            return Ok(());
        }
    };

    log::info!("[ThirdPartyControl] Window found, HWND: {:?}", target_hwnd);

    // 构造 BlockedAppInfo 结构体
    #[repr(C)]
    struct BlockedAppInfo {
        category: [u16; 64],
        app_name: [u16; 128],
        status: [u16; 32],
    }

    let mut info = BlockedAppInfo {
        category: [0; 64],
        app_name: [0; 128],
        status: [0; 32],
    };

    // 转换字符串为 UTF-16 并复制到结构体
    let category_utf16: Vec<u16> = category.encode_utf16().collect();
    let app_name_utf16: Vec<u16> = app_name.encode_utf16().collect();
    let status_utf16: Vec<u16> = status.encode_utf16().collect();

    // 复制数据（确保不超过数组边界）
    for (i, &ch) in category_utf16.iter().take(63).enumerate() {
        info.category[i] = ch;
    }
    for (i, &ch) in app_name_utf16.iter().take(127).enumerate() {
        info.app_name[i] = ch;
    }
    for (i, &ch) in status_utf16.iter().take(31).enumerate() {
        info.status[i] = ch;
    }

    // 构造 COPYDATASTRUCT
    let cds = COPYDATASTRUCT {
        dwData: 0x02, // 三方软件管控业务标识
        cbData: std::mem::size_of::<BlockedAppInfo>() as u32,
        lpData: &mut info as *mut _ as *mut c_void,
    };

    log::info!(
        "[ThirdPartyControl] Sending WM_COPYDATA: dwData=0x02, cbData={}",
        cds.cbData
    );

    // 发送消息
    unsafe {
        SendMessageW(
            target_hwnd,
            windows::Win32::UI::WindowsAndMessaging::WM_COPYDATA,
            Some(WPARAM(0)),
            Some(LPARAM(&cds as *const _ as isize)),
        );

        log::info!("[ThirdPartyControl] WM_COPYDATA sent!");
    }

    Ok(())
}
