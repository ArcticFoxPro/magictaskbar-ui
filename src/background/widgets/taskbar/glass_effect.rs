/// 液态玻璃亚克力效果模块
///
/// 在 Tauri 任务栏窗口内创建一个 WS_CHILD + WS_EX_NOREDIRECTIONBITMAP 子窗口，
/// 通过 liquidglass_dll.dll 在该子窗口的指定区域应用 Windows Composition 模糊效果。
/// 子窗口 Z 序位于 WebView2 子窗口下方，模糊效果透过透明 CSS 背景显示。
use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};

use crossbeam_channel::bounded;
use windows::core::PCWSTR;
use windows::Win32::{
    Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED},
    System::LibraryLoader::{GetProcAddress, LoadLibraryW},
    UI::WindowsAndMessaging::*,
};

use crate::{
    error::Result,
    utils::spawn_named_thread,
    windows_api::{string_utils::WindowsString, WindowsApi},
};

// ── 自定义窗口消息 ──────────────────────────────────────
const WM_GLASS_UPDATE_BLUR: u32 = WM_USER + 300;
const WM_GLASS_SHOW: u32 = WM_USER + 301;
const WM_GLASS_HIDE: u32 = WM_USER + 302;
const WM_GLASS_RESIZE: u32 = WM_USER + 303;
const WM_GLASS_REFRESH: u32 = WM_USER + 304;

/// 供 hook.rs 异步刷新时引用的消息常量
pub(crate) const WM_GLASS_REFRESH_MSG: u32 = WM_GLASS_REFRESH;

// ── DLL 函数签名 ────────────────────────────────────────
type FnCreate = unsafe extern "C" fn(HWND, f32, f32, f32, f32, f32) -> *mut c_void;
type FnResize = unsafe extern "C" fn(*mut c_void, f32, f32, f32, f32, f32, f32);
type FnDestroy = unsafe extern "C" fn(*mut c_void);

/// debug 模式下从 MagicTaskbar_binary 目录加载 DLL，release 模式下按默认搜索顺序
fn get_dll_path(dll_name: &str) -> String {
    #[cfg(debug_assertions)]
    {
        // CARGO_MANIFEST_DIR 指向 bar0325/src/，向上一级到 bar0325/
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("MagicTaskbar_binary")
            .join(dll_name);
        let result = path.to_string_lossy().to_string();
        log::info!(
            "[GlassEffect] debug DLL path: {result}, exists: {}",
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
struct GlassThreadState {
    effect_handle: *mut c_void,
    fn_resize: FnResize,
    fn_destroy: FnDestroy,
}

/// 模糊区域参数（通过 PostMessage 传递，由接收方 Box::from_raw 释放）
#[repr(C)]
struct BlurParams {
    blur_x: f32,
    blur_y: f32,
    blur_width: f32,
    blur_height: f32,
    window_width: f32,
    window_height: f32,
}

/// 子窗口尺寸参数（通过 PostMessage 传递）
#[repr(C)]
struct ResizeParams {
    width: i32,
    height: i32,
}

// ── 窗口过程（运行在 STA 线程） ────────────────────────
unsafe extern "system" fn glass_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_GLASS_UPDATE_BLUR => {
            let ptr = lparam.0 as *mut BlurParams;
            if !ptr.is_null() {
                let params = Box::from_raw(ptr);
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GlassThreadState;
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
        WM_GLASS_SHOW => {
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
        WM_GLASS_HIDE => {
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_GLASS_REFRESH => {
            // 强制 DWM 重新采样 BackdropBrush，但不改变可见状态
            // 使用 SetWindowPos + SWP_FRAMECHANGED 触发重绘
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            LRESULT(0)
        }
        WM_GLASS_RESIZE => {
            let ptr = lparam.0 as *mut ResizeParams;
            if !ptr.is_null() {
                let params = Box::from_raw(ptr);
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_BOTTOM),
                    0,
                    0,
                    params.width,
                    params.height,
                    SWP_NOACTIVATE,
                );
            }
            LRESULT(0)
        }
        WM_NCHITTEST => {
            // 子窗口的 WS_EX_TRANSPARENT 只影响绘制顺序，不实现鼠标穿透
            // 返回 HTTRANSPARENT(-1) 使所有鼠标消息穿透到下层窗口
            LRESULT(-1)
        }
        WM_NCCALCSIZE => {
            // 声明无非客户区，彻底消除边框
            LRESULT(0)
        }
        WM_NCPAINT => {
            // 跳过非客户区绘制
            LRESULT(0)
        }
        WM_NCACTIVATE => {
            // 返回 TRUE(1) 允许激活但跳过默认非客户区重绘
            LRESULT(1)
        }
        WM_ERASEBKGND => {
            // 告诉系统背景已擦除，阻止默认填充
            LRESULT(1)
        }
        WM_DESTROY => {
            // 释放 DLL 效果和线程局部状态
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GlassThreadState;
            if !state_ptr.is_null() {
                let state = Box::from_raw(state_ptr);
                if !state.effect_handle.is_null() {
                    (state.fn_destroy)(state.effect_handle);
                }
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ── 公开 API ────────────────────────────────────────────

/// 管理任务栏容器区域的液态玻璃亚克力效果。
///
/// 作为 Tauri 任务栏窗口的子窗口，通过 liquidglass_dll 实现实时高斯模糊。
/// 子窗口自动跟随父窗口显示/隐藏，无需手动管理 Z 序。
/// Drop 时自动销毁窗口和 DLL 资源。
pub struct TaskbarGlassEffect {
    glass_hwnd: AtomicIsize,
}

unsafe impl Send for TaskbarGlassEffect {}
unsafe impl Sync for TaskbarGlassEffect {}

impl TaskbarGlassEffect {
    /// 创建玻璃效果子窗口。
    ///
    /// - `taskbar_hwnd`: Tauri 任务栏窗口句柄（作为父窗口）
    /// - `parent_width/height`: 父窗口客户区尺寸
    /// - `blur_x/y/w/h`: 初始模糊区域（相对于子窗口客户区）
    /// - `corner_radius`: 模糊区域圆角半径
    pub fn new(
        taskbar_hwnd: HWND,
        parent_width: i32,
        parent_height: i32,
        blur_x: f32,
        blur_y: f32,
        blur_width: f32,
        blur_height: f32,
        corner_radius: f32,
    ) -> Result<Self> {
        let (tx, rx) = bounded::<Result<isize>>(1);

        let tb_hwnd_raw = taskbar_hwnd.0 as isize;
        let init_w = parent_width;
        let init_h = parent_height;
        let init_blur_x = blur_x;
        let init_blur_y = blur_y;
        let init_blur_w = blur_width;
        let init_blur_h = blur_height;
        let init_radius = corner_radius;

        spawn_named_thread("GlassEffect-STA", move || unsafe {
            // 1. 初始化 COM STA
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

            // 2. 加载 DLL
            // debug 模式下设置 DLL 搜索目录，确保 WinRT 激活能找到 Microsoft.Graphics.Canvas.dll
            #[cfg(debug_assertions)]
            {
                let binary_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .unwrap()
                    .join("MagicTaskbar_binary");
                let dir_str = WindowsString::from_str(&binary_dir.to_string_lossy());
                let _ =
                    windows::Win32::System::LibraryLoader::SetDllDirectoryW(dir_str.as_pcwstr());
                log::info!("[GlassEffect] SetDllDirectoryW -> {}", binary_dir.display());
            }
            let dll_path = get_dll_path("liquidglass_dll.dll");
            let dll_name = WindowsString::from_str(&dll_path);
            let dll = match LoadLibraryW(dll_name.as_pcwstr()) {
                Ok(h) => h,
                Err(e) => {
                    log::error!("[GlassEffect] LoadLibrary 失败: {e:?}");
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
            let class_name = WindowsString::from_str("TaskbarGlassEffect_Class");
            let instance = match WindowsApi::module_handle_w() {
                Ok(h) => h,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
            let wc = WNDCLASSW {
                lpfnWndProc: Some(glass_wnd_proc),
                lpszClassName: class_name.as_pcwstr(),
                hInstance: instance.into(),
                ..Default::default()
            };
            let _ = RegisterClassW(&wc);

            // 5. 创建子窗口（WS_CHILD + WS_EX_NOREDIRECTIONBITMAP）
            let parent = HWND(tb_hwnd_raw as *mut _);
            let hwnd = match CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TRANSPARENT,
                class_name.as_pcwstr(),
                PCWSTR::null(),
                WS_CHILD | WS_VISIBLE,
                0,
                0,
                init_w,
                init_h,
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

            // 6. 调用 DLL 创建模糊效果
            let effect_handle = fn_create(
                hwnd,
                init_blur_x,
                init_blur_y,
                init_blur_w,
                init_blur_h,
                init_radius,
            );
            if effect_handle.is_null() {
                log::error!("[GlassEffect] LiquidGlassCreate 返回 NULL");
                let _ = DestroyWindow(hwnd);
                let _ = tx.send(Err("LiquidGlassCreate returned NULL".into()));
                return;
            }

            // 7. 将 DLL 状态存入窗口 UserData
            let state = Box::new(GlassThreadState {
                effect_handle,
                fn_resize,
                fn_destroy,
            });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);

            // 8. 放在子窗口 Z 序最底层（WebView2 子窗口之下）
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_BOTTOM),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );

            // 返回句柄
            let _ = tx.send(Ok(hwnd.0 as isize));

            // 9. 消息循环
            let mut msg = std::mem::zeroed();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = DispatchMessageW(&msg);
            }
        })?;

        let hwnd_val = rx.recv()??;
        log::info!("[GlassEffect] 玻璃子窗口已创建 HWND={:#x}", hwnd_val);

        Ok(Self {
            glass_hwnd: AtomicIsize::new(hwnd_val),
        })
    }

    /// 获取玻璃窗口句柄
    pub fn hwnd(&self) -> HWND {
        HWND(self.glass_hwnd.load(Ordering::Relaxed) as *mut _)
    }

    /// 调整子窗口尺寸以匹配父窗口（异步发送到 STA 线程）
    pub fn resize(&self, width: i32, height: i32) {
        let hwnd = self.hwnd();
        if hwnd.0.is_null() {
            return;
        }
        let params = Box::new(ResizeParams { width, height });
        unsafe {
            let _ = PostMessageW(
                Some(hwnd),
                WM_GLASS_RESIZE,
                WPARAM(0),
                LPARAM(Box::into_raw(params) as isize),
            );
        }
    }

    /// 更新模糊区域（异步发送到 STA 线程执行）
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
                WM_GLASS_UPDATE_BLUR,
                WPARAM(0),
                LPARAM(Box::into_raw(params) as isize),
            );
        }
    }

    /// 显示玻璃子窗口
    pub fn show(&self) {
        let hwnd = self.hwnd();
        if !hwnd.0.is_null() {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_GLASS_SHOW, WPARAM(0), LPARAM(0));
            }
        }
    }

    /// 隐藏玻璃子窗口
    pub fn hide(&self) {
        let hwnd = self.hwnd();
        if !hwnd.0.is_null() {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_GLASS_HIDE, WPARAM(0), LPARAM(0));
            }
        }
    }
}

impl Drop for TaskbarGlassEffect {
    fn drop(&mut self) {
        let hwnd = self.hwnd();
        if !hwnd.0.is_null() {
            log::info!(
                "[GlassEffect] Dropping 玻璃子窗口 HWND={:#x}",
                hwnd.0 as isize
            );
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
}
