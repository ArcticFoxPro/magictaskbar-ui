use crate::app::get_app_handle;
use crate::cli::ServicePipe;
use crate::error::Result;
use serde::Serialize;
use slu_ipc::messages::SvcAction;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc;
use std::sync::{LazyLock, Mutex};
use std::thread::{self, sleep};
use std::time::Duration;
use tauri::Emitter;
use windows::core::{Interface, GUID, PCWSTR};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ActivateKeyboardLayout, GetKeyboardLayout, LoadKeyboardLayoutW, ACTIVATE_KEYBOARD_LAYOUT_FLAGS,
    HKL, KLF_ACTIVATE, KLF_REORDER, KLF_SETFORPROCESS, KLF_SUBSTITUTE_OK,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_InputProcessorProfiles, ITfInputProcessorProfileMgr, ITfInputProcessorProfiles,
    GUID_TFCAT_TIP_KEYBOARD, TF_INPUTPROCESSORPROFILE, TF_LANGUAGEPROFILE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, GetWindowThreadProcessId, PostMessageW, HC_ACTION, HHOOK,
    WM_INPUTLANGCHANGEREQUEST, WM_KEYUP, WM_SYSKEYUP,
};
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use winreg::RegKey;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TsfProfile {
    pub clsid: String,
    pub langid: u16,
    pub guid_profile: String,
    pub name: String,
    pub description: String,
}

pub type TsfActiveProfile = TsfProfile;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KeyboardLayoutProfile {
    pub hkl: String,
    pub klid: String,
    pub langid: u16,
    pub description: String,
    pub active: bool,
}

#[derive(Clone, Copy)]
struct SendHhook(HHOOK);
unsafe impl Send for SendHhook {}
unsafe impl Sync for SendHhook {}

static INPUT_METHODS: LazyLock<Mutex<Vec<TsfProfile>>> = LazyLock::new(|| Mutex::new(Vec::new()));
static LAST_INDEX: AtomicI32 = AtomicI32::new(-1);
static PROGRAMMATIC_SWITCH_ACTIVE: AtomicBool = AtomicBool::new(false);
static HOOK: Mutex<Option<SendHhook>> = Mutex::new(None);
static SWITCH_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static PROBE_WORKER: LazyLock<Mutex<Option<mpsc::Sender<ProbeRequest>>>> =
    LazyLock::new(|| Mutex::new(None));
const HOTKEY_PROBE_DELAYS_MS: [u64; 3] = [120, 220, 360];
const HOTKEY_MAX_ATTEMPTS: usize = 6;
const ISOLATED_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const KEYEVENTF_KEYUP_FLAG: u32 = 0x0002;

struct ProbeRequest {
    refresh_profiles: bool,
    reply: mpsc::SyncSender<core::result::Result<Option<TsfActiveProfile>, String>>,
}

unsafe extern "system" {
    fn keybd_event(b_vk: u8, b_scan: u8, dw_flags: u32, dw_extra_info: usize);
}

struct ProgrammaticSwitchGuard;

impl ProgrammaticSwitchGuard {
    fn enter() -> Self {
        PROGRAMMATIC_SWITCH_ACTIVE.store(true, Ordering::SeqCst);
        Self
    }
}

impl Drop for ProgrammaticSwitchGuard {
    fn drop(&mut self) {
        PROGRAMMATIC_SWITCH_ACTIVE.store(false, Ordering::SeqCst);
    }
}

fn with_tsf_com<T>(operation: impl FnOnce() -> T) -> T {
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = operation();
        if initialized {
            CoUninitialize();
        }
        result
    }
}

fn guid_to_string(g: &GUID) -> String {
    format!(
        "{{{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}}}",
        g.data1,
        g.data2,
        g.data3,
        g.data4[0],
        g.data4[1],
        g.data4[2],
        g.data4[3],
        g.data4[4],
        g.data4[5],
        g.data4[6],
        g.data4[7]
    )
}

fn hkl_to_u64(hkl: HKL) -> u64 {
    hkl.0 as isize as u64
}

fn format_hkl(hkl: HKL) -> String {
    format!("0x{:016X}", hkl_to_u64(hkl))
}

fn langid_from_hkl(hkl: HKL) -> u16 {
    (hkl_to_u64(hkl) & 0xFFFF) as u16
}

fn foreground_keyboard_layout() -> HKL {
    unsafe {
        let foreground = GetForegroundWindow();
        if !foreground.0.is_null() {
            let thread_id = GetWindowThreadProcessId(foreground, None);
            if thread_id != 0 {
                return GetKeyboardLayout(thread_id);
            }
        }

        GetKeyboardLayout(0)
    }
}

fn primary_langid(langid: u16) -> u16 {
    langid & 0x03ff
}

fn keyboard_klid_from_langid(langid: u16) -> String {
    format!("0000{:04X}", langid)
}

fn is_english_keyboard_layout(langid: u16) -> bool {
    primary_langid(langid) == 0x09
}

fn keyboard_layout_flags(
    flags: &[ACTIVATE_KEYBOARD_LAYOUT_FLAGS],
) -> ACTIVATE_KEYBOARD_LAYOUT_FLAGS {
    ACTIVATE_KEYBOARD_LAYOUT_FLAGS(flags.iter().fold(0, |acc, flag| acc | flag.0))
}

fn normalize_klid(klid: &str) -> Result<String> {
    let normalized = klid.trim().to_uppercase();
    if normalized.len() != 8 || !normalized.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!("Invalid keyboard layout id: {klid}").into());
    }
    Ok(normalized)
}

fn pcwstr_from_str(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn activate_keyboard_layout_by_klid(normalized_klid: &str) -> Result<HKL> {
    let wide_klid = pcwstr_from_str(normalized_klid);
    log::info!(
        "[TSF] activate keyboard layout start klid={}",
        normalized_klid
    );

    let hkl = unsafe {
        LoadKeyboardLayoutW(
            PCWSTR(wide_klid.as_ptr()),
            keyboard_layout_flags(&[KLF_ACTIVATE, KLF_SUBSTITUTE_OK, KLF_REORDER]),
        )?
    };
    log::info!(
        "[TSF] keyboard layout loaded klid={} hkl={}",
        normalized_klid,
        format_hkl(hkl)
    );

    unsafe {
        let activated_result = ActivateKeyboardLayout(
            hkl,
            keyboard_layout_flags(&[KLF_ACTIVATE, KLF_SETFORPROCESS, KLF_REORDER]),
        );
        match activated_result {
            Ok(activated) => log::info!(
                "[TSF] ActivateKeyboardLayout ok klid={} hkl={} previous={}",
                normalized_klid,
                format_hkl(hkl),
                format_hkl(activated)
            ),
            Err(err) => log::warn!(
                "[TSF] ActivateKeyboardLayout failed klid={} hkl={} error={}",
                normalized_klid,
                format_hkl(hkl),
                err
            ),
        }

        let foreground = GetForegroundWindow();
        if !foreground.0.is_null() {
            let _ = PostMessageW(
                Some(foreground),
                WM_INPUTLANGCHANGEREQUEST,
                WPARAM(0),
                LPARAM(hkl.0 as isize),
            );
            log::info!(
                "[TSF] WM_INPUTLANGCHANGEREQUEST posted klid={} hwnd={:?}",
                normalized_klid,
                foreground
            );
        }
    }

    Ok(hkl)
}

fn read_keyboard_layout_description(klid: &str) -> Option<String> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let path = format!(
        r"SYSTEM\CurrentControlSet\Control\Keyboard Layouts\{}",
        klid
    );
    hklm.open_subkey(path)
        .ok()
        .and_then(|key| key.get_value::<String, _>("Layout Text").ok())
        .filter(|value| !value.trim().is_empty())
}

fn enumerate_preload_keyboard_layouts() -> Vec<KeyboardLayoutProfile> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let preload = match hkcu.open_subkey(r"Keyboard Layout\Preload") {
        Ok(key) => key,
        Err(_) => return Vec::new(),
    };

    let active_hkl = foreground_keyboard_layout();
    let active_langid = langid_from_hkl(active_hkl);

    let mut values = preload
        .enum_values()
        .filter_map(|entry| entry.ok())
        .filter_map(|(name, _)| {
            let order = name.parse::<u32>().ok()?;
            let klid = preload.get_value::<String, _>(&name).ok()?;
            Some((order, klid))
        })
        .collect::<Vec<_>>();
    values.sort_by_key(|(order, _)| *order);

    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter_map(|(_, klid)| {
            let normalized_klid = klid.trim().to_uppercase();
            if normalized_klid.is_empty() || !seen.insert(normalized_klid.clone()) {
                return None;
            }

            let langid = u16::from_str_radix(
                normalized_klid
                    .get(normalized_klid.len().saturating_sub(4)..)
                    .unwrap_or("0"),
                16,
            )
            .unwrap_or(0);
            if !is_english_keyboard_layout(langid) {
                return None;
            }

            let description = read_keyboard_layout_description(&normalized_klid)
                .unwrap_or_else(|| format!("Keyboard Layout {}", normalized_klid));

            Some(KeyboardLayoutProfile {
                hkl: format!("0x{:016X}", langid as u64),
                klid: normalized_klid,
                langid,
                description,
                active: active_langid == langid,
            })
        })
        .collect()
}

fn profile_matches_active(profile: &TsfProfile, active: &TF_INPUTPROCESSORPROFILE) -> bool {
    profile.langid == active.langid
        && profile
            .clsid
            .eq_ignore_ascii_case(&guid_to_string(&active.clsid))
        && profile
            .guid_profile
            .eq_ignore_ascii_case(&guid_to_string(&active.guidProfile))
}

fn active_profile_index(profile: &TsfProfile) -> Option<usize> {
    let ims = INPUT_METHODS.lock().unwrap();
    ims.iter()
        .position(|im| im.guid_profile.eq_ignore_ascii_case(&profile.guid_profile))
}

fn cache_active_profile(profile: &TsfProfile) {
    if let Some(idx) = active_profile_index(profile) {
        LAST_INDEX.store(idx as i32, Ordering::SeqCst);
    }
}

fn emit_active_profile(profile: TsfActiveProfile) {
    let _ = get_app_handle().emit("tsf_active_profile_changed", profile);
}

pub fn enumerate_input_methods() -> Result<Vec<TsfProfile>> {
    with_tsf_com(|| unsafe {
        let mut list = Vec::new();
        let profiles: ITfInputProcessorProfiles =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_ALL)?;

        let mut langids_ptr: *mut u16 = std::ptr::null_mut();
        let mut count = 0u32;
        profiles.GetLanguageList(&mut langids_ptr, &mut count)?;

        if !langids_ptr.is_null() {
            let langids = std::slice::from_raw_parts(langids_ptr, count as usize);
            for &langid in langids {
                if let Ok(enum_profiles) = profiles.EnumLanguageProfiles(langid) {
                    let mut fetched = 0u32;
                    let mut lp: [TF_LANGUAGEPROFILE; 1] = [std::mem::zeroed()];
                    while enum_profiles.Next(&mut lp, &mut fetched).is_ok() && fetched > 0 {
                        if let Ok(desc) = profiles.GetLanguageProfileDescription(
                            &lp[0].clsid,
                            langid,
                            &lp[0].guidProfile,
                        ) {
                            let name = desc.to_string();
                            list.push(TsfProfile {
                                clsid: guid_to_string(&lp[0].clsid),
                                langid,
                                guid_profile: guid_to_string(&lp[0].guidProfile),
                                name: name.clone(),
                                description: name,
                            });
                        }
                    }
                }
            }
            CoTaskMemFree(Some(langids_ptr as _));
        }

        let mut seen = HashSet::new();
        list.retain(|profile| seen.insert(profile.guid_profile.to_lowercase()));

        let mut ims = INPUT_METHODS.lock().unwrap();
        *ims = list.clone();
        Ok(list)
    })
}

pub fn get_current_input_method_index() -> i32 {
    with_tsf_com(|| unsafe {
        let profiles: ITfInputProcessorProfiles =
            match CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_ALL) {
                Ok(p) => p,
                Err(_) => return -1,
            };
        let mgr: ITfInputProcessorProfileMgr = match profiles.cast() {
            Ok(m) => m,
            Err(_) => return -1,
        };

        let mut active: TF_INPUTPROCESSORPROFILE = std::mem::zeroed();
        if mgr
            .GetActiveProfile(&GUID_TFCAT_TIP_KEYBOARD, &mut active)
            .is_err()
        {
            return -1;
        }

        {
            let ims = INPUT_METHODS.lock().unwrap();
            for (i, im) in ims.iter().enumerate() {
                if profile_matches_active(im, &active) {
                    return i as i32;
                }
            }
        }

        // If not found, it might be a newly installed input method. Try re-enumerating.
        log::info!("[TSF] Active profile not found in cache, re-enumerating...");
        if let Ok(new_list) = enumerate_input_methods() {
            for (i, im) in new_list.iter().enumerate() {
                if profile_matches_active(im, &active) {
                    return i as i32;
                }
            }
        }

        -1
    })
}

fn simulate_ctrl_shift_hotkey() -> Result<()> {
    unsafe {
        keybd_event(0x11, 0, 0, 0);
        keybd_event(0x10, 0, 0, 0);
        keybd_event(0x10, 0, KEYEVENTF_KEYUP_FLAG, 0);
        keybd_event(0x11, 0, KEYEVENTF_KEYUP_FLAG, 0);
    }

    Ok(())
}

fn probe_active_profile_current_thread(refresh_profiles: bool) -> Result<Option<TsfActiveProfile>> {
    if refresh_profiles {
        let _ = enumerate_input_methods();
    }

    let idx = get_current_input_method_index();
    if idx < 0 {
        return Ok(None);
    }

    let ims = INPUT_METHODS.lock().unwrap();
    if (idx as usize) < ims.len() {
        return Ok(Some(ims[idx as usize].clone()));
    }

    Ok(None)
}

fn spawn_probe_worker() -> Result<mpsc::Sender<ProbeRequest>> {
    let (sender, receiver) = mpsc::channel::<ProbeRequest>();

    thread::Builder::new()
        .name("input-method-tsf-probe".into())
        .spawn(move || {
            while let Ok(request) = receiver.recv() {
                let result = with_tsf_com(|| {
                    probe_active_profile_current_thread(request.refresh_profiles)
                        .map_err(|err| format!("{err:?}"))
                });

                let _ = request.reply.send(result);
            }
        })
        .map_err(|err| format!("Failed to start TSF probe worker: {err}"))?;

    Ok(sender)
}

fn get_probe_worker_sender() -> Result<mpsc::Sender<ProbeRequest>> {
    let mut worker = PROBE_WORKER
        .lock()
        .map_err(|_| "TSF probe worker mutex poisoned")?;

    if let Some(sender) = worker.as_ref() {
        return Ok(sender.clone());
    }

    let sender = spawn_probe_worker()?;
    *worker = Some(sender.clone());
    Ok(sender)
}

fn clear_probe_worker_sender() {
    if let Ok(mut worker) = PROBE_WORKER.lock() {
        *worker = None;
    }
}

fn probe_active_profile_isolated(refresh_profiles: bool) -> Result<Option<TsfActiveProfile>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let request = ProbeRequest {
        refresh_profiles,
        reply: sender,
    };

    match get_probe_worker_sender()?.send(request) {
        Ok(()) => {}
        Err(err) => {
            clear_probe_worker_sender();
            get_probe_worker_sender()?
                .send(err.0)
                .map_err(|_| "Input method probe worker disconnected")?;
        }
    }

    match receiver.recv_timeout(ISOLATED_PROBE_TIMEOUT) {
        Ok(Ok(profile)) => Ok(profile),
        Ok(Err(message)) => Err(message.into()),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            clear_probe_worker_sender();
            Ok(None)
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            clear_probe_worker_sender();
            Err("Input method probe thread disconnected".into())
        }
    }
}

fn resolve_target_profile_by_guid(target_guid_profile: &str) -> Result<TsfProfile> {
    let normalized_target = target_guid_profile.trim();
    if normalized_target.is_empty() {
        return Err("Input method target guid_profile is empty".into());
    }

    {
        let ims = INPUT_METHODS.lock().unwrap();
        if let Some(profile) = ims
            .iter()
            .find(|im| im.guid_profile.eq_ignore_ascii_case(normalized_target))
            .cloned()
        {
            return Ok(profile);
        }
    }

    enumerate_input_methods()?
        .into_iter()
        .find(|im| im.guid_profile.eq_ignore_ascii_case(normalized_target))
        .ok_or_else(|| format!("Input method guid_profile not found: {normalized_target}").into())
}

fn activate_input_profile_by_hotkey_target(
    target_profile: &TsfProfile,
) -> Result<Option<TsfActiveProfile>> {
    let _switch_guard = ProgrammaticSwitchGuard::enter();
    let normalized_target = target_profile.guid_profile.to_lowercase();

    let profiles = enumerate_input_methods()?;
    let attempt_limit = std::cmp::max(
        1,
        std::cmp::min(HOTKEY_MAX_ATTEMPTS, profiles.len().saturating_add(1)),
    );
    log::debug!(
        "[TSF] hotkey switch start target={} desc={} profiles={}",
        normalized_target,
        target_profile.description,
        profiles.len()
    );

    if let Some(current) = probe_active_profile_isolated(true)? {
        log::debug!(
            "[TSF] hotkey switch before current={} desc={}",
            current.guid_profile,
            current.description
        );
        if current
            .guid_profile
            .eq_ignore_ascii_case(&normalized_target)
        {
            cache_active_profile(&current);
            emit_active_profile(current.clone());
            return Ok(Some(current));
        }
    }

    let active_langid = langid_from_hkl(foreground_keyboard_layout());
    if primary_langid(active_langid) != primary_langid(target_profile.langid) {
        let target_klid = keyboard_klid_from_langid(target_profile.langid);
        log::info!(
            "[TSF] hotkey switch language bridge active_lang=0x{:04X} target_lang=0x{:04X} klid={}",
            active_langid,
            target_profile.langid,
            target_klid
        );
        let _ = activate_keyboard_layout_by_klid(&target_klid)?;
        sleep(Duration::from_millis(180));

        if let Some(current) = probe_active_profile_isolated(true)? {
            log::debug!(
                "[TSF] hotkey switch after language bridge current={} desc={}",
                current.guid_profile,
                current.description
            );
            if current
                .guid_profile
                .eq_ignore_ascii_case(&normalized_target)
            {
                cache_active_profile(&current);
                emit_active_profile(current.clone());
                return Ok(Some(current));
            }
        }
    }

    for attempt in 1..=attempt_limit {
        log::debug!(
            "[TSF] hotkey switch attempt {attempt}/{attempt_limit} target={normalized_target}"
        );
        simulate_ctrl_shift_hotkey()?;

        for delay in HOTKEY_PROBE_DELAYS_MS {
            sleep(Duration::from_millis(delay));

            if let Some(probed) = probe_active_profile_isolated(true)? {
                log::debug!(
                    "[TSF] hotkey switch probe attempt={} delay={} guid={} desc={}",
                    attempt,
                    delay,
                    probed.guid_profile,
                    probed.description
                );
                if probed.guid_profile.eq_ignore_ascii_case(&normalized_target) {
                    cache_active_profile(&probed);
                    emit_active_profile(probed.clone());
                    return Ok(Some(probed));
                }
            }
        }
    }

    let fallback = probe_active_profile_isolated(true)?;
    if let Some(profile) = &fallback {
        log::debug!(
            "[TSF] hotkey switch fallback target={} guid={} desc={}",
            normalized_target,
            profile.guid_profile,
            profile.description
        );
        cache_active_profile(profile);
        emit_active_profile(profile.clone());
    } else {
        log::debug!(
            "[TSF] hotkey switch fallback target={} profile=<null>",
            normalized_target
        );
    }
    Ok(fallback)
}

fn activate_input_profile_via_service(
    target_profile: &TsfProfile,
) -> Result<Option<TsfActiveProfile>> {
    let _switch_guard = ProgrammaticSwitchGuard::enter();
    let normalized_target = target_profile.guid_profile.to_lowercase();

    log::debug!(
        "[TSF] service hotkey switch start target={} desc={}",
        normalized_target,
        target_profile.description
    );

    let active_langid = langid_from_hkl(foreground_keyboard_layout());
    if primary_langid(active_langid) != primary_langid(target_profile.langid) {
        let target_klid = keyboard_klid_from_langid(target_profile.langid);
        log::info!(
            "[TSF] service hotkey language bridge active_lang=0x{:04X} target_lang=0x{:04X} klid={}",
            active_langid,
            target_profile.langid,
            target_klid
        );
        let _ = activate_keyboard_layout_by_klid(&target_klid)?;
        sleep(Duration::from_millis(180));

        if let Some(current) = probe_active_profile_isolated(true)? {
            log::debug!(
                "[TSF] service hotkey after language bridge current={} desc={}",
                current.guid_profile,
                current.description
            );
            if current
                .guid_profile
                .eq_ignore_ascii_case(&normalized_target)
            {
                cache_active_profile(&current);
                emit_active_profile(current.clone());
                return Ok(Some(current));
            }
        }
    }

    ServicePipe::request(SvcAction::SwitchInputMethodHotkey {
        guid_profile: normalized_target.clone(),
    })?;

    for delay in HOTKEY_PROBE_DELAYS_MS {
        sleep(Duration::from_millis(delay + 40));

        if let Some(probed) = probe_active_profile_isolated(true)? {
            log::debug!(
                "[TSF] service hotkey switch probe delay={} guid={} desc={}",
                delay,
                probed.guid_profile,
                probed.description
            );
            if probed.guid_profile.eq_ignore_ascii_case(&normalized_target) {
                cache_active_profile(&probed);
                emit_active_profile(probed.clone());
                return Ok(Some(probed));
            }
        }
    }

    let fallback = probe_active_profile_isolated(true)?;
    if let Some(profile) = &fallback {
        cache_active_profile(profile);
        emit_active_profile(profile.clone());
    }
    Ok(fallback)
}

pub fn activate_input_profile(guid_profile: String) -> Result<Option<TsfActiveProfile>> {
    let target_profile = resolve_target_profile_by_guid(&guid_profile)?;
    let _switch_lock = SWITCH_MUTEX
        .lock()
        .map_err(|_| "Input method switch mutex poisoned")?;
    if ServicePipe::is_running() {
        return activate_input_profile_via_service(&target_profile);
    }
    activate_input_profile_by_hotkey_target(&target_profile)
}

/// 处理来自服务进程（管理员权限）的输入法切换通知
pub fn handle_tsf_change_from_service(index: i32) {
    if index < 0 {
        return;
    }

    let last = LAST_INDEX.load(Ordering::SeqCst);
    println!(
        "[TSF-Backend-DEBUG] handle_tsf_change_from_service index={}, last={}",
        index, last
    );
    log::info!(
        "[TSF] handle_tsf_change_from_service: received index={}, last={}",
        index,
        last
    );
    if PROGRAMMATIC_SWITCH_ACTIVE.load(Ordering::SeqCst) {
        log::debug!(
            "[TSF] service watcher message suppressed during programmatic switch index={} last={}",
            index,
            last
        );
        return;
    }
    if index != last {
        LAST_INDEX.store(index, Ordering::SeqCst);
        log::info!(
            "[TSF] Input method changed index to: {} (notified by service)",
            index
        );

        {
            let ims = INPUT_METHODS.lock().unwrap();
            if (index as usize) < ims.len() {
                let im = ims[index as usize].clone();
                drop(ims);
                let _ = get_app_handle().emit("tsf_active_profile_changed", im);
                return;
            }
        }

        // Index out of bounds, try re-enumerating
        log::info!("[TSF] Index {} out of bounds, re-enumerating...", index);
        if let Ok(new_list) = enumerate_input_methods() {
            if (index as usize) < new_list.len() {
                let _ = get_app_handle().emit(
                    "tsf_active_profile_changed",
                    new_list[index as usize].clone(),
                );
            } else {
                log::warn!(
                    "[TSF] Index {} still out of bounds after re-enumeration",
                    index
                );
            }
        }
    }
}

extern "system" fn keyboard_proc(n_code: i32, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
    if n_code == HC_ACTION as i32
        && (w_param.0 as u32 == WM_KEYUP || w_param.0 as u32 == WM_SYSKEYUP)
    {
        let cur = get_current_input_method_index();
        let last = LAST_INDEX.load(Ordering::SeqCst);
        if cur >= 0 && cur != last {
            LAST_INDEX.store(cur, Ordering::SeqCst);
            log::info!("[TSF] Input method changed index to: {}", cur);

            // Notify frontend
            let ims = INPUT_METHODS.lock().unwrap();
            if let Some(im) = ims.get(cur as usize) {
                let _ = get_app_handle().emit("tsf_active_profile_changed", im.clone());
            }
        }
    }
    unsafe { CallNextHookEx(None, n_code, w_param, l_param) }
}

pub fn get_active_input_profile() -> Result<Option<TsfActiveProfile>> {
    let profile = probe_active_profile_isolated(true)?;
    if let Some(active) = &profile {
        log::debug!(
            "[TSF] get_active_input_profile returning {}",
            active.guid_profile
        );
    } else {
        log::debug!("[TSF] get_active_input_profile: no active profile resolved");
    }
    Ok(profile)
}
pub fn get_installed_input_profiles() -> Result<Vec<TsfProfile>> {
    enumerate_input_methods()
}

pub fn get_installed_keyboard_layouts() -> Result<Vec<KeyboardLayoutProfile>> {
    let layouts = enumerate_preload_keyboard_layouts();
    log::debug!("[TSF] keyboard layouts enumerated count={}", layouts.len());
    Ok(layouts)
}

pub fn activate_keyboard_layout(klid: String) -> Result<Option<KeyboardLayoutProfile>> {
    let _switch_lock = SWITCH_MUTEX
        .lock()
        .map_err(|_| "Input method switch mutex poisoned")?;
    let normalized_klid = normalize_klid(&klid)?;
    let langid = u16::from_str_radix(&normalized_klid[4..], 16)
        .map_err(|err| format!("Invalid keyboard layout langid in {normalized_klid}: {err}"))?;
    let hkl = activate_keyboard_layout_by_klid(&normalized_klid)?;

    sleep(Duration::from_millis(180));

    let mut layouts = enumerate_preload_keyboard_layouts();
    if let Some(active) = layouts
        .iter_mut()
        .find(|layout| layout.klid.eq_ignore_ascii_case(&normalized_klid))
    {
        active.active = true;
        return Ok(Some(active.clone()));
    }

    Ok(Some(KeyboardLayoutProfile {
        hkl: format_hkl(hkl),
        klid: normalized_klid.clone(),
        langid,
        description: read_keyboard_layout_description(&normalized_klid)
            .unwrap_or_else(|| format!("Keyboard Layout {}", normalized_klid)),
        active: true,
    }))
}
pub fn get_last_active_input_profile_cached() -> Option<TsfActiveProfile> {
    let idx = LAST_INDEX.load(Ordering::SeqCst);
    let ims = INPUT_METHODS.lock().unwrap();
    if idx >= 0 && idx < ims.len() as i32 {
        Some(ims[idx as usize].clone())
    } else {
        None
    }
}
pub fn activate_input_profile_by_name(name: String) -> Result<()> {
    activate_input_profile(name).map(|_| ())
}

pub fn activate_keyboard_layout_via_tsf(id: String, _handle: String) -> Result<()> {
    activate_input_profile(id).map(|_| ())
}

pub fn set_simple_mode(_enabled: bool) {}
