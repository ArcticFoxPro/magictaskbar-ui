use std::{
    ffi::{CStr, CString, OsStr},
    os::{raw::c_char, windows::ffi::OsStrExt},
    path::PathBuf,
    sync::{LazyLock, Mutex},
};

use serde_json::Value;
use tauri::{AppHandle, Emitter};
use windows::{
    core::{PCSTR, PCWSTR},
    Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW},
};

use crate::error::Result;

type StartServerFn =
    unsafe extern "system" fn(u16, Option<extern "system" fn(*const c_char)>) -> i32;
type JsonFn = unsafe extern "system" fn(*const c_char) -> *mut c_char;
type FreeStringFn = unsafe extern "system" fn(*mut c_char);

struct GrpcBridgeApi {
    start_server: StartServerFn,
    register_scenes: JsonFn,
    unregister_scenes: JsonFn,
    send_yoyo_scene: JsonFn,
    get_music_data: JsonFn,
    send_screen_recognition: JsonFn,
    free_string: FreeStringFn,
}

static API: LazyLock<std::result::Result<GrpcBridgeApi, String>> = LazyLock::new(load_api);
static APP_HANDLE: LazyLock<Mutex<Option<AppHandle>>> = LazyLock::new(|| Mutex::new(None));

fn dll_path() -> Result<PathBuf> {
    Ok(std::env::current_exe()?.with_file_name("grpc_bridge.dll"))
}

fn wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

unsafe fn load_symbol<T>(name: &'static [u8]) -> std::result::Result<T, String> {
    static MODULE: LazyLock<std::result::Result<isize, String>> = LazyLock::new(|| {
        let path = dll_path().map_err(|err| err.to_string())?;
        let wide = wide_null(path.as_os_str());
        let module = unsafe { LoadLibraryW(PCWSTR(wide.as_ptr())) }
            .map_err(|err| format!("LoadLibrary grpc_bridge.dll failed: {err:?}"))?;
        Ok(module.0 as isize)
    });

    let module = MODULE.as_ref().map_err(Clone::clone)?;
    let proc = GetProcAddress(
        windows::Win32::Foundation::HMODULE(*module as _),
        PCSTR(name.as_ptr()),
    )
    .ok_or_else(|| {
        format!(
            "GetProcAddress {} failed",
            String::from_utf8_lossy(&name[..name.len().saturating_sub(1)])
        )
    })?;
    Ok(std::mem::transmute_copy(&proc))
}

fn load_api() -> std::result::Result<GrpcBridgeApi, String> {
    unsafe {
        Ok(GrpcBridgeApi {
            start_server: load_symbol(b"grpc_bridge_start_server\0")?,
            register_scenes: load_symbol(b"grpc_bridge_register_scenes\0")?,
            unregister_scenes: load_symbol(b"grpc_bridge_unregister_scenes\0")?,
            send_yoyo_scene: load_symbol(b"grpc_bridge_send_yoyo_scene\0")?,
            get_music_data: load_symbol(b"grpc_bridge_get_music_data\0")?,
            send_screen_recognition: load_symbol(b"grpc_bridge_send_screen_recognition\0")?,
            free_string: load_symbol(b"grpc_bridge_free_string\0")?,
        })
    }
}

fn api() -> Result<&'static GrpcBridgeApi> {
    API.as_ref().map_err(|err| err.clone().into())
}

extern "system" fn ui_event_callback(json: *const c_char) {
    if json.is_null() {
        return;
    }

    let Ok(payload) = (unsafe { CStr::from_ptr(json) }).to_str() else {
        return;
    };

    let Ok(value) = serde_json::from_str::<Value>(payload) else {
        return;
    };

    if value.get("event").and_then(|v| v.as_str()) != Some("scene_notify") {
        return;
    }

    let Some(notify) = value.get("notify").and_then(|v| v.as_str()) else {
        return;
    };

    crate::exposed::handle_scene_notify_value(&value);

    if let Ok(guard) = APP_HANDLE.lock() {
        if let Some(app) = guard.as_ref() {
            let _ = app.emit("ai-recommend:notify", notify.to_string());
            if let Some(process_data) = value.get("ProcessData").and_then(|v| v.as_str()) {
                let _ = app.emit("ai-recommend:process", process_data.to_string());
            }
        }
    }
}

pub fn start_server(port: u16, app: AppHandle) -> Result<()> {
    if let Ok(mut guard) = APP_HANDLE.lock() {
        *guard = Some(app);
    }

    let code = unsafe { (api()?.start_server)(port, Some(ui_event_callback)) };
    if code == 0 {
        Ok(())
    } else {
        Err(format!("grpc_bridge_start_server failed with code {code}").into())
    }
}

async fn call_json(func: JsonFn, payload: Value) -> Result<Value> {
    tokio::task::spawn_blocking(move || {
        let api = api()?;
        let json = CString::new(payload.to_string())
            .map_err(|err| format!("grpc bridge json contains nul byte: {err}"))?;
        let ptr = unsafe { func(json.as_ptr()) };
        if ptr.is_null() {
            return Err("grpc bridge returned null".into());
        }
        let text = unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned();
        unsafe {
            (api.free_string)(ptr);
        }
        Ok(serde_json::from_str::<Value>(&text)?)
    })
    .await
    .map_err(|err| crate::error::AppError::from(format!("grpc bridge task join failed: {err}")))?
}

fn ensure_ok(value: Value) -> Result<Value> {
    if value.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        Ok(value)
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("grpc bridge call failed")
            .to_string()
            .into())
    }
}

pub async fn register_scenes(payload: Value) -> Result<Value> {
    ensure_ok(call_json(api()?.register_scenes, payload).await?)
}

pub async fn unregister_scenes(payload: Value) -> Result<Value> {
    ensure_ok(call_json(api()?.unregister_scenes, payload).await?)
}

pub async fn send_yoyo_scene(payload: Value) -> Result<Value> {
    ensure_ok(call_json(api()?.send_yoyo_scene, payload).await?)
}

pub async fn get_music_data(payload: Value) -> Result<Value> {
    ensure_ok(call_json(api()?.get_music_data, payload).await?)
}

pub async fn send_screen_recognition(payload: Value) -> Result<Value> {
    ensure_ok(call_json(api()?.send_screen_recognition, payload).await?)
}
