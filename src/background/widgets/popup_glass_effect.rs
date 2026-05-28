/// Popup 玻璃模糊效果模块
///
/// 为 toolbar 上的 popup（如 power-menu）创建子窗口实现背景模糊。
/// 作为 toolbar 窗口的子窗口，通过 liquidglass_dll 实现高斯模糊效果。
use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};

use crossbeam_channel::bounded;
use windows::core::PCWSTR;
use windows::Win32::{
    Foundation::{FreeLibrary, HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, WPARAM},
    System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED},
    System::LibraryLoader::{GetProcAddress, LoadLibraryW},
    UI::WindowsAndMessaging::*,
};

use crate::{
    error::Result,
    utils::spawn_named_thread,
    windows_api::{string_utils::WindowsString, WindowsApi},
};

// 自定义窗口消息
const WM_POPUP_GLASS_UPDATE: u32 = WM_USER + 400;
const WM_POPUP_GLASS_SHOW: u32 = WM_USER + 401;
const WM_POPUP_GLASS_HIDE: u32 = WM_USER + 402;
const WM_POPUP_GLASS_MOVE: u32 = WM_USER + 403;

// DLL 函数签名
type FnCreate = unsafe extern "C" fn(HWND, f32, f32, f32, f32, f32) -> *mut c_void;
type FnResize = unsafe extern "C" fn(*mut c_void, f32, f32, f32, f32, f32, f32);
type FnDestroy = unsafe extern "C" fn(*mut c_void);

/// debug 模式下从 MagicTaskbar_binary 目录加载 DLL，release 模式下按默认搜索顺序
fn get_dll_path(dll_name: &str) -> String {
    #[cfg(debug_assertions)]
    {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("MagicTaskbar_binary")
            .join(dll_name);
        let result = path.to_string_lossy().to_string();
        log::info!(
            "[PopupGlass] debug DLL path: {result}, exists: {}",
            path.exists()
        );
        result
    }
    #[cfg(not(debug_assertions))]
    {
        dll_name.to_string()
    }
}

/// 存储在窗口 GWLP_USERDATA 中的 STA 线程局部状态
struct PopupGlassState {
    effect_handle: *mut c_void,
    fn_resize: FnResize,
    fn_destroy: FnDestroy,
    dll_handle: HMODULE, // 保存 DLL 句柄用于释放
}

/// 模糊区域参数
#[repr(C)]
struct BlurParams {
    blur_x: f32,
    blur_y: f32,
    blur_width: f32,
    blur_height: f32,
    window_width: f32,
    window_height: f32,
}

/// 窗口位置参数
#[repr(C)]
struct MoveParams {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    corner_radius: f32,
}

/// 窗口过程（运行在 STA 线程）
unsafe extern "system" fn popup_glass_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_POPUP_GLASS_UPDATE => {
            let ptr = lparam.0 as *mut BlurParams;
            if !ptr.is_null() {
                let params = Box::from_raw(ptr);
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PopupGlassState;
                if !state_ptr.is_null() {
                    let state = &*state_ptr;
                    if !state.effect_handle.is_null() {
                        (state.fn_resize)(
                            state.effect_handle,
                            params.blur_x,
                            params.blur_y,
                            params.blur_width,
                            params.blur_height,
                            params.window_width,
                            params.window_height,
                        );
                    }
                }
            }
            LRESULT(0)
        }
        WM_POPUP_GLASS_SHOW => {
            let _ = ShowWindow(hwnd, SW_SHOWNA);
            // 确保在子窗口 Z 序最底层（WebView2 子窗口之下）
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_BOTTOM),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            LRESULT(0)
        }
        WM_POPUP_GLASS_HIDE => {
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_POPUP_GLASS_MOVE => {
            let ptr = lparam.0 as *mut MoveParams;
            if !ptr.is_null() {
                let params = Box::from_raw(ptr);
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_BOTTOM),
                    params.x,
                    params.y,
                    params.width,
                    params.height,
                    SWP_NOACTIVATE,
                );
                // 同步更新模糊区域
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PopupGlassState;
                if !state_ptr.is_null() {
                    let state = &*state_ptr;
                    if !state.effect_handle.is_null() {
                        (state.fn_resize)(
                            state.effect_handle,
                            0.0,
                            0.0, // blur_x, blur_y 从 0 开始
                            params.width as f32,
                            params.height as f32,
                            params.width as f32,
                            params.height as f32,
                        );
                        log::info!(
                            "[PopupGlass] WM_POPUP_GLASS_MOVE: ({}, {}) size {}x{}, blur updated",
                            params.x,
                            params.y,
                            params.width,
                            params.height
                        );
                    }
                }
            }
            LRESULT(0)
        }
        WM_NCHITTEST => {
            // 返回 HTTRANSPARENT 使鼠标消息穿透
            LRESULT(-1)
        }
        WM_NCCALCSIZE => LRESULT(0),
        WM_NCPAINT => LRESULT(0),
        WM_NCACTIVATE => LRESULT(1),
        WM_ERASEBKGND => LRESULT(1),
        WM_DESTROY => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PopupGlassState;
            if !state_ptr.is_null() {
                let state = Box::from_raw(state_ptr);
                if !state.effect_handle.is_null() {
                    (state.fn_destroy)(state.effect_handle);
                }
                // 释放 DLL 句柄
                if !state.dll_handle.0.is_null() {
                    let _ = FreeLibrary(state.dll_handle);
                }
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Popup 玻璃模糊效果
pub struct PopupGlassEffect {
    glass_hwnd: AtomicIsize,
}

unsafe impl Send for PopupGlassEffect {}
unsafe impl Sync for PopupGlassEffect {}

impl PopupGlassEffect {
    /// 创建玻璃效果子窗口
    ///
    /// - `parent_hwnd`: 父窗口句柄（toolbar 窗口）
    /// - `corner_radius`: 模糊区域圆角半径
    pub fn new(parent_hwnd: HWND, corner_radius: f32) -> Result<Self> {
        let (tx, rx) = bounded::<Result<isize>>(1);
        let parent_raw = parent_hwnd.0 as isize;

        spawn_named_thread("PopupGlass-STA", move || unsafe {
            // 1. 初始化 COM STA
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

            // 2. 加载 DLL
            #[cfg(debug_assertions)]
            {
                let binary_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .unwrap()
                    .join("MagicTaskbar_binary");
                let dir_str = WindowsString::from_str(&binary_dir.to_string_lossy());
                let _ =
                    windows::Win32::System::LibraryLoader::SetDllDirectoryW(dir_str.as_pcwstr());
            }
            let dll_path = get_dll_path("liquidglass_dll.dll");
            let dll_name = WindowsString::from_str(&dll_path);
            let dll = match LoadLibraryW(dll_name.as_pcwstr()) {
                Ok(h) => h,
                Err(e) => {
                    log::error!("[PopupGlass] LoadLibrary 失败: {e:?}");
                    let _ = tx.send(Err(format!("LoadLibrary failed: {e:?}").into()));
                    return;
                }
            };

            // 3. 获取函数指针
            let fn_create: FnCreate =
                match GetProcAddress(dll, windows::core::s!("LiquidGlassCreate")) {
                    Some(f) => std::mem::transmute(f),
                    None => {
                        let _ = tx.send(Err("GetProcAddress LiquidGlassCreate failed".into()));
                        return;
                    }
                };
            let fn_resize: FnResize =
                match GetProcAddress(dll, windows::core::s!("LiquidGlassResize")) {
                    Some(f) => std::mem::transmute(f),
                    None => {
                        let _ = tx.send(Err("GetProcAddress LiquidGlassResize failed".into()));
                        return;
                    }
                };
            let fn_destroy: FnDestroy =
                match GetProcAddress(dll, windows::core::s!("LiquidGlassDestroy")) {
                    Some(f) => std::mem::transmute(f),
                    None => {
                        let _ = tx.send(Err("GetProcAddress LiquidGlassDestroy failed".into()));
                        return;
                    }
                };

            // 4. 注册窗口类
            let class_name = WindowsString::from_str("PopupGlassEffect_Class");
            let instance = match WindowsApi::module_handle_w() {
                Ok(h) => h,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
            let wc = WNDCLASSW {
                lpfnWndProc: Some(popup_glass_wnd_proc),
                lpszClassName: class_name.as_pcwstr(),
                hInstance: instance.into(),
                ..Default::default()
            };
            let _ = RegisterClassW(&wc);

            // 5. 创建子窗口（初始隐藏）
            let parent = HWND(parent_raw as *mut _);
            let hwnd = match CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TRANSPARENT,
                class_name.as_pcwstr(),
                PCWSTR::null(),
                WS_CHILD, // 初始不显示 WS_VISIBLE
                0,
                0,
                0,
                0, // 初始位置和尺寸为 0
                Some(parent),
                None,
                Some(HINSTANCE(instance.0)),
                None,
            ) {
                Ok(h) => h,
                Err(e) => {
                    let _ = tx.send(Err(format!("CreateWindowEx failed: {e:?}").into()));
                    return;
                }
            };

            // 6. 调用 DLL 创建模糊效果（初始区域为 0，后续通过 update_blur 设置）
            let effect_handle = fn_create(hwnd, 0.0, 0.0, 0.0, 0.0, corner_radius);
            if effect_handle.is_null() {
                log::error!("[PopupGlass] LiquidGlassCreate 返回 NULL");
                let _ = DestroyWindow(hwnd);
                let _ = tx.send(Err("LiquidGlassCreate returned NULL".into()));
                return;
            }

            // 7. 将 DLL 状态存入窗口 UserData
            let state = Box::new(PopupGlassState {
                effect_handle,
                fn_resize,
                fn_destroy,
                dll_handle: dll, // 保存 DLL 句柄
            });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);

            // 返回句柄
            let _ = tx.send(Ok(hwnd.0 as isize));

            // 8. 消息循环
            let mut msg = std::mem::zeroed();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = DispatchMessageW(&msg);
            }
        })?;

        let hwnd_val = rx.recv()??;
        log::info!("[PopupGlass] 玻璃子窗口已创建 HWND={:#x}", hwnd_val);

        Ok(Self {
            glass_hwnd: AtomicIsize::new(hwnd_val),
        })
    }

    /// 获取玻璃窗口句柄
    pub fn hwnd(&self) -> HWND {
        HWND(self.glass_hwnd.load(Ordering::Relaxed) as *mut _)
    }

    /// 显示并设置模糊窗口位置和尺寸
    pub fn show_at(&self, x: i32, y: i32, width: i32, height: i32, corner_radius: f32) {
        let hwnd = self.hwnd();
        if hwnd.0.is_null() {
            return;
        }

        // 移动窗口并同步更新模糊区域（在 WM_POPUP_GLASS_MOVE 处理中完成）
        let params = Box::new(MoveParams {
            x,
            y,
            width,
            height,
            corner_radius,
        });
        unsafe {
            let _ = PostMessageW(
                Some(hwnd),
                WM_POPUP_GLASS_MOVE,
                WPARAM(0),
                LPARAM(Box::into_raw(params) as isize),
            );
            // 显示窗口
            let _ = PostMessageW(Some(hwnd), WM_POPUP_GLASS_SHOW, WPARAM(0), LPARAM(0));
        }
    }

    /// 更新模糊区域
    pub fn update_blur_region(
        &self,
        blur_x: f32,
        blur_y: f32,
        blur_width: f32,
        blur_height: f32,
        window_width: f32,
        window_height: f32,
    ) {
        let hwnd = self.hwnd();
        if hwnd.0.is_null() {
            return;
        }
        let params = Box::new(BlurParams {
            blur_x,
            blur_y,
            blur_width,
            blur_height,
            window_width,
            window_height,
        });
        unsafe {
            let _ = PostMessageW(
                Some(hwnd),
                WM_POPUP_GLASS_UPDATE,
                WPARAM(0),
                LPARAM(Box::into_raw(params) as isize),
            );
        }
    }

    /// 隐藏玻璃子窗口（同步执行，确保立即生效）
    pub fn hide(&self) {
        let hwnd = self.hwnd();
        if !hwnd.0.is_null() {
            unsafe {
                // 使用 SendMessageTimeoutW 同步隐藏，超时 100ms
                // 这样可以确保窗口立即隐藏，避免视觉延迟
                let mut result: usize = 0;
                let _ = SendMessageTimeoutW(
                    hwnd,
                    WM_POPUP_GLASS_HIDE,
                    WPARAM(0),
                    LPARAM(0),
                    SMTO_BLOCK | SMTO_ABORTIFHUNG,
                    100,
                    Some(&mut result),
                );
            }
        }
    }
}

impl Drop for PopupGlassEffect {
    fn drop(&mut self) {
        let hwnd = self.hwnd();
        if !hwnd.0.is_null() {
            log::info!(
                "[PopupGlass] Dropping 玻璃子窗口 HWND={:#x}",
                hwnd.0 as isize
            );
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
}
