use crate::app::get_app_handle;
use crate::error::Result;
use std::sync::{mpsc, Mutex};
use tauri::Emitter;
use tauri_plugin_shell::ShellExt;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::{
    core::implement,
    core::PCWSTR,
    Win32::Media::Audio::Endpoints::{
        IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
    },
    Win32::Media::Audio::{
        eCommunications, eMultimedia, eRender, EDataFlow, ERole, IMMDevice, IMMDeviceEnumerator,
        IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator,
        AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE,
    },
    Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    },
};

fn report_string(report_id: &str, content: &str) -> bool {
    log::debug!(
        "[Report] data bridge disabled; skip report id={} content={}",
        report_id,
        content
    );
    false
}

fn with_endpoint_volume<T>(f: impl FnOnce(IAudioEndpointVolume) -> T) -> Result<T> {
    unsafe {
        // Initialize COM for this thread
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let mut res: Option<T> = None;
        let result = (|| -> Result<()> {
            // Create MMDeviceEnumerator
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            // Get default audio endpoint (render, multimedia)
            let device: IMMDevice = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
            // Activate IAudioEndpointVolume
            let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
            res = Some(f(endpoint));
            Ok(())
        })();
        CoUninitialize();
        match result {
            Ok(()) => Ok(res.expect("endpoint callback did not set result")),
            Err(e) => Err(e),
        }
    }
}

pub fn system_get_master_volume() -> Result<u8> {
    let vol = with_endpoint_volume(|ep| unsafe {
        let level = ep.GetMasterVolumeLevelScalar().unwrap_or(0.5);
        ((level * 100.0).round() as i32).clamp(0, 100) as u8
    })?;
    log::info!("[Volume] get master volume finished: {}%", vol);
    Ok(vol)
}

pub fn system_set_master_volume(volume: u8) -> Result<()> {
    log::info!("[Volume] set master volume request: {}", volume);
    with_endpoint_volume(|ep| unsafe {
        let scalar = (volume as f32 / 100.0).clamp(0.0, 1.0);
        let _ = ep.SetMasterVolumeLevelScalar(scalar, std::ptr::null());
    })?;
    Ok(())
}

pub fn system_get_master_muted() -> Result<bool> {
    let muted = with_endpoint_volume(|ep| unsafe { ep.GetMute().unwrap_or(false.into()) })?;
    Ok(muted.into())
}

pub fn system_set_master_muted(muted: bool) -> Result<()> {
    log::info!(
        "[Volume] set master mute request: {}",
        if muted { "mute" } else { "unmute" }
    );
    with_endpoint_volume(|ep| unsafe {
        let _ = ep.SetMute(muted, std::ptr::null());
    })?;
    Ok(())
}

pub fn system_open_volume_mixer() -> Result<()> {
    // Windows 11: ms-settings:sound; classic mixer: sndvol.exe
    let shell = get_app_handle().shell();
    if shell
        .command("cmd")
        .arg("/c")
        .arg("start ms-settings:apps-volume")
        .spawn()
        .is_err()
    {
        let _ = shell
            .command(
                crate::utils::constants::VAR_COMMON
                    .system_dir()
                    .join("SndVol.exe"),
            )
            .spawn();
    }
    Ok(())
}

// 实现 IAudioEndpointVolumeCallback 接口
#[implement(IAudioEndpointVolumeCallback)]
struct VolumeCallback;

impl IAudioEndpointVolumeCallback_Impl for VolumeCallback_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows_core::Result<()> {
        if let Some(data) = unsafe { data.as_ref() } {
            let volume = (data.fMasterVolume * 100.0).round() as u8;
            let muted = data.bMuted.as_bool();
            log::info!("[Volume] volume={}, muted={}", volume, muted);

            let payload = libs_core::system_state::VolumeState { volume, muted };
            let _ = get_app_handle().emit(
                libs_core::handlers::FuncEvent::SystemVolumeChanged,
                &payload,
            );
        }
        Ok(())
    }
}

enum VolumeWorkerMsg {
    RebindDefaultEndpoint,
}

#[implement(IMMNotificationClient)]
struct AudioEndpointNotificationClient {
    tx: mpsc::Sender<VolumeWorkerMsg>,
}

impl IMMNotificationClient_Impl for AudioEndpointNotificationClient_Impl {
    fn OnDeviceStateChanged(
        &self,
        _pwstr_device_id: &PCWSTR,
        _dw_new_state: DEVICE_STATE,
    ) -> windows_core::Result<()> {
        Ok(())
    }

    fn OnDeviceAdded(&self, _pwstr_device_id: &PCWSTR) -> windows_core::Result<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, _pwstr_device_id: &PCWSTR) -> windows_core::Result<()> {
        Ok(())
    }

    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        role: ERole,
        _pwstr_default_device_id: &PCWSTR,
    ) -> windows_core::Result<()> {
        if flow == eRender && (role == eMultimedia || role == eCommunications) {
            let _ = self.tx.send(VolumeWorkerMsg::RebindDefaultEndpoint);
        }
        Ok(())
    }

    fn OnPropertyValueChanged(
        &self,
        _pwstr_device_id: &PCWSTR,
        _key: &PROPERTYKEY,
    ) -> windows_core::Result<()> {
        Ok(())
    }
}

pub fn register_volume_events() {
    log::info!("[Volume] volume events are forwarded by magictaskbar-srv");
}

#[allow(dead_code)]
fn register_volume_events_ui_legacy_disabled() {
    std::thread::spawn(|| unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        // 创建 MMDeviceEnumerator
        let enumerator =
            match CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
            {
                Ok(e) => e,
                Err(_) => {
                    log::warn!("[Volume] Failed to create MMDeviceEnumerator");
                    let json = serde_json::json!({ "Event": "Create MMDeviceEnumerator fail" })
                        .to_string();
                    report_string("669000010", &json);
                    CoUninitialize();
                    return;
                }
            };

        let callback = IAudioEndpointVolumeCallback::from(VolumeCallback);
        let current_endpoint: Mutex<Option<IAudioEndpointVolume>> = Mutex::new(None);

        let rebind = |enumerator: &IMMDeviceEnumerator, callback: &IAudioEndpointVolumeCallback| {
            let device = match enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) {
                Ok(d) => d,
                Err(_) => {
                    log::warn!("[Volume] GetDefaultAudioEndpoint failed");
                    return;
                }
            };
            let endpoint = match device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                Ok(ep) => ep,
                Err(_) => {
                    log::warn!("[Volume] Activate IAudioEndpointVolume failed");
                    return;
                }
            };

            if let Ok(mut guard) = current_endpoint.lock() {
                if let Some(old) = guard.take() {
                    let _ = old.UnregisterControlChangeNotify(callback);
                }
                if endpoint.RegisterControlChangeNotify(callback).is_ok() {
                    *guard = Some(endpoint.clone());
                    if let (Ok(vol), Ok(muted)) =
                        (endpoint.GetMasterVolumeLevelScalar(), endpoint.GetMute())
                    {
                        let payload = libs_core::system_state::VolumeState {
                            volume: (vol * 100.0).round() as u8,
                            muted: muted.as_bool(),
                        };
                        let _ = get_app_handle().emit(
                            libs_core::handlers::FuncEvent::SystemVolumeChanged,
                            &payload,
                        );
                    }
                    log::info!("[Volume] Endpoint volume callback (re)bound to default endpoint");
                } else {
                    log::warn!("[Volume] Failed to register volume callback on endpoint");
                    let json =
                        serde_json::json!({ "Event": "Register volume callback fail" }).to_string();
                    report_string("669000010", &json);
                }
            }
        };

        rebind(&enumerator, &callback);

        let (tx, rx) = mpsc::channel::<VolumeWorkerMsg>();
        let notify_client = IMMNotificationClient::from(AudioEndpointNotificationClient { tx });
        let notification_ok = enumerator
            .RegisterEndpointNotificationCallback(&notify_client)
            .is_ok();
        if notification_ok {
            log::info!("[Volume] Default endpoint notification callback registered");
        } else {
            log::warn!("[Volume] Failed to register default endpoint notification callback");
            let json = serde_json::json!({ "Event": "Register default endpoint notification callback fail" }).to_string();
            report_string("669000010", &json);
        }

        loop {
            if notification_ok {
                match rx.recv_timeout(std::time::Duration::from_secs(60)) {
                    Ok(VolumeWorkerMsg::RebindDefaultEndpoint) => {
                        rebind(&enumerator, &callback);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            } else {
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        }

        if let Ok(mut guard) = current_endpoint.lock() {
            if let Some(old) = guard.take() {
                let _ = old.UnregisterControlChangeNotify(&callback);
            }
        }
        if notification_ok {
            let _ = enumerator.UnregisterEndpointNotificationCallback(&notify_client);
        }
        CoUninitialize();
    });
}
