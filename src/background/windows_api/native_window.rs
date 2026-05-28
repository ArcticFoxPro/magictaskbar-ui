use std::sync::atomic::{AtomicIsize, Ordering};

use crossbeam_channel::bounded;
use windows::Win32::{
    Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostMessageW,
        PostQuitMessage, RegisterClassW, WINDOW_STYLE, WM_CLOSE, WM_DESTROY, WNDCLASSW,
        WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TRANSPARENT,
    },
};

use crate::{
    error::Result,
    utils::spawn_named_thread,
    windows_api::{string_utils::WindowsString, WindowsApi},
};

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_DESTROY {
        PostQuitMessage(0);
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// 轻量级原生 Win32 透明窗口，用于替代 WebView2 AppBar 占位窗口。
/// 在独立线程中运行消息循环，Drop 时自动销毁窗口并退出线程。
pub struct NativeAppBarWindow {
    hwnd: AtomicIsize,
}

unsafe impl Send for NativeAppBarWindow {}
unsafe impl Sync for NativeAppBarWindow {}

impl NativeAppBarWindow {
    pub fn new(class_suffix: &str) -> Result<Self> {
        let (tx, rx) = bounded::<Result<isize>>(1);
        let suffix = class_suffix.to_string();

        spawn_named_thread(&format!("NativeAppBar-{suffix}"), move || unsafe {
            let class_name = format!("MagicTaskbarAppBar");
            let wc_name = WindowsString::from_str(&class_name);

            let instance = match WindowsApi::module_handle_w() {
                Ok(h) => h,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };

            let wc = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                lpszClassName: wc_name.as_pcwstr(),
                hInstance: instance.into(),
                ..Default::default()
            };
            // 忽略重复注册错误（同一 monitor_id 重建时）
            let _ = RegisterClassW(&wc);

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE,
                wc_name.as_pcwstr(),
                windows::core::PCWSTR::null(),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(HINSTANCE(instance.0)),
                None,
            );

            match hwnd {
                Ok(h) => {
                    let _ = tx.send(Ok(h.0 as isize));
                }
                Err(e) => {
                    let _ = tx.send(Err(e.into()));
                    return;
                }
            }

            // 消息循环：直到 WM_DESTROY 触发 PostQuitMessage
            let mut msg = std::mem::zeroed();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                DispatchMessageW(&msg);
            }
        })?;

        let hwnd_val = rx.recv()??;
        Ok(Self {
            hwnd: AtomicIsize::new(hwnd_val),
        })
    }

    pub fn hwnd(&self) -> HWND {
        HWND(self.hwnd.load(Ordering::Relaxed) as *mut _)
    }
}

impl Drop for NativeAppBarWindow {
    fn drop(&mut self) {
        // WM_CLOSE → DefWindowProcW 调用 DestroyWindow
        // → WM_DESTROY → PostQuitMessage(0) → GetMessageW 返回 0 → 线程退出
        unsafe {
            let _ = PostMessageW(Some(self.hwnd()), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}
