use std::path::PathBuf;

use windows::{
    ApplicationModel::AppInfo,
    Win32::{
        Foundation::HANDLE,
        Storage::Packaging::Appx::GetApplicationUserModelId,
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        System::Threading::{PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION},
    },
};

use crate::error::Result;

use super::{
    string_utils::WindowsString, types::AppUserModelId, window::Window, HandleWrapper, WindowsApi,
};

// https://stackoverflow.com/questions/47300622/meaning-of-flags-in-process-extended-basic-information-struct
#[allow(dead_code)]
pub enum ProcessInformationFlag {
    IsProtectedProcess = 0x1,
    IsWow64Process = 0x2,
    IsProcessInJob = 0x4,
    IsCrossSessionCreate = 0x8,
    IsFrozen = 0x10,
    IsBackground = 0x20,
    IsStronglyNamed = 0x40,
    IsSecureProcess = 0x80,
    IsSubsystemProcess = 0x100,
}

pub struct Process(u32);

impl Process {
    pub fn from_window(window: &Window) -> Self {
        let (process_id, _) = WindowsApi::window_thread_process_id(window.hwnd());
        Self(process_id)
    }

    pub fn id(&self) -> u32 {
        self.0
    }

    pub fn open_handle(&self) -> Result<HANDLE> {
        WindowsApi::open_process(PROCESS_QUERY_INFORMATION, false, self.0)
    }

    /// will fail if the process is owned by another user
    pub fn open_limited_handle(&self) -> Result<HANDLE> {
        WindowsApi::open_process(PROCESS_QUERY_LIMITED_INFORMATION, false, self.0)
    }

    pub fn is_frozen(&self) -> Result<bool> {
        WindowsApi::is_process_frozen(self.0)
    }

    /// package app user model id, (appx, eg: "Microsoft.WindowsTerminal_8wekyb3d8bbwe!TerminalApp")
    pub fn package_app_user_model_id(&self) -> Result<AppUserModelId> {
        let hprocess = self.open_limited_handle()?;
        let _handle_wrapper = HandleWrapper::new(hprocess);
        let mut len = 1024_u32;
        let mut id = WindowsString::new_to_fill(len as usize);
        unsafe { GetApplicationUserModelId(hprocess, &mut len, Some(id.as_pwstr())).ok()? };
        Ok(AppUserModelId::Appx(id.to_string()))
    }

    #[allow(dead_code)]
    pub fn package_app_info(&self) -> Result<AppInfo> {
        let app_info = AppInfo::GetFromAppUserModelId(&self.package_app_user_model_id()?.into())?;
        Ok(app_info)
    }

    pub fn program_path(&self) -> Result<PathBuf> {
        let path_string = WindowsApi::exe_path_by_process(self.0)?;
        if path_string.is_empty() {
            return Err("exe path is empty".into());
        }
        Ok(PathBuf::from(path_string))
    }

    /// program path filename
    pub fn program_exe_name(&self) -> Result<String> {
        Ok(self
            .program_path()?
            .file_name()
            .ok_or("there is no file name")?
            .to_string_lossy()
            .to_string())
    }

    pub fn program_display_name(&self) -> Result<String> {
        let path = self.program_path()?;
        match WindowsApi::get_executable_display_name(&path) {
            Ok(name) => Ok(name.trim_end_matches(".exe").to_owned()),
            Err(_) => Ok(path
                .file_stem()
                .ok_or("there is no file stem")?
                .to_string_lossy()
                .to_string()),
        }
    }

    pub fn is_taskbar(&self) -> bool {
        if let Ok(exe) = self.program_path() {
            return exe.ends_with("magictaskbar-ui.exe");
        }
        false
    }

    /// 使用 CreateToolhelp32Snapshot 获取进程名（不需要 OpenProcess，避免权限问题）
    /// 适用于无法通过 OpenProcess 访问的进程（如管理员权限运行的进程）
    pub fn exe_name_by_snapshot(&self) -> Result<String> {
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?;
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };

            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    if entry.th32ProcessID == self.0 {
                        // 找到目标进程，获取进程名
                        let exe_name = String::from_utf16_lossy(
                            &entry.szExeFile[..entry
                                .szExeFile
                                .iter()
                                .position(|&c| c == 0)
                                .unwrap_or(entry.szExeFile.len())],
                        );
                        let _ = windows::Win32::Foundation::CloseHandle(snapshot);
                        return Ok(exe_name);
                    }
                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = windows::Win32::Foundation::CloseHandle(snapshot);
            Err(format!("Process with PID {} not found in snapshot", self.0).into())
        }
    }
}
