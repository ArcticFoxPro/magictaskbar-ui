mod queue;
use image::{GenericImageView, ImageBuffer, RgbaImage};
use itertools::Itertools;
use queue::{IconExtractor, IconExtractorRequest};
use windows::core::PCWSTR;
use windows::Win32::{
    Foundation::HWND,
    Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, DIB_RGB_COLORS,
    },
    Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES,
    UI::{
        Controls::{IImageList, ILD_TRANSPARENT},
        Shell::{
            ExtractIconW, SHGetFileInfoW, SHGetImageList, SHFILEINFOW, SHGFI_SYSICONINDEX,
            SHIL_JUMBO,
        },
        WindowsAndMessaging::{DestroyIcon, GetIconInfoExW, HICON, ICONINFOEXW},
    },
};

use libs_core::state::Icon;

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::{
    __m128i, _mm_loadu_si128, _mm_setr_epi8, _mm_shuffle_epi8, _mm_storeu_si128,
};

#[cfg(target_arch = "aarch64")]
use std::arch::aarch64::{uint8x16_t, vld1q_u8, vqtbl1q_u8, vst1q_u8};

use std::io::BufRead;
use std::path::{Path, PathBuf};
use tokio::time::Duration;

use crate::error::Result;
use crate::modules::start::application::START_MENU_MANAGER;
use crate::modules::uwp::UwpManager;
use crate::state::application::FULL_STATE;
use crate::trace_lock;
use crate::utils::constants::VAR_COMMON;
use crate::utils::icon_whitelist::is_app_whitelisted;
use crate::utils::{path_based_hash_id, umid_based_hash_id};
use crate::windows_api::types::AppUserModelId;
use crate::windows_api::WindowsApi;

/// Convert BGRA to RGBA
///
/// Uses SIMD to go fast
#[cfg(target_arch = "x86_64")]
pub fn bgra_to_rgba(data: &mut [u8]) {
    // The shuffle mask for converting BGRA -> RGBA
    let mask: __m128i = unsafe {
        _mm_setr_epi8(
            2, 1, 0, 3, // First pixel
            6, 5, 4, 7, // Second pixel
            10, 9, 8, 11, // Third pixel
            14, 13, 12, 15, // Fourth pixel
        )
    };
    // For each 16-byte chunk in your data
    for chunk in data.chunks_exact_mut(16) {
        let mut vector = unsafe { _mm_loadu_si128(chunk.as_ptr() as *const __m128i) };
        vector = unsafe { _mm_shuffle_epi8(vector, mask) };
        unsafe { _mm_storeu_si128(chunk.as_mut_ptr() as *mut __m128i, vector) };
    }
}

// Uses NEON intrinsics to go fast
#[cfg(target_arch = "aarch64")]
pub fn bgra_to_rgba(data: &mut [u8]) {
    // The shuffle mask for converting BGRA -> RGBA
    let maskplain: [u8; 16] = [
        2, 1, 0, 3, // First pixel
        6, 5, 4, 7, // Second pixel
        10, 9, 8, 11, // Third pixel
        14, 13, 12, 15, // Fourth pixel
    ];
    // The shuffle mask for the conversion in NEON intrinsics
    let mask: uint8x16_t = unsafe { vld1q_u8(maskplain.as_ptr()) };
    // For each 16-byte chunk in your data
    for chunk in data.chunks_exact_mut(16) {
        let mut vector: uint8x16_t = unsafe { vld1q_u8(chunk.as_ptr()) };
        vector = unsafe { vqtbl1q_u8(vector, mask) };
        unsafe { vst1q_u8(chunk.as_mut_ptr(), vector) };
    }
}

pub fn convert_hicon_to_rgba_image(hicon: &HICON) -> Result<RgbaImage> {
    unsafe {
        let mut icon_info = ICONINFOEXW {
            cbSize: std::mem::size_of::<ICONINFOEXW>() as u32,
            ..Default::default()
        };

        if !GetIconInfoExW(*hicon, &mut icon_info).as_bool() {
            return Err("Failed to get icon info".into());
        }
        let hdc_screen = CreateCompatibleDC(None);
        let hdc_mem = CreateCompatibleDC(Some(hdc_screen));
        let _hbm_old = SelectObject(hdc_mem, icon_info.hbmColor.into());

        let mut bmp_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: icon_info.xHotspot as i32 * 2,
                biHeight: -(icon_info.yHotspot as i32 * 2),
                biPlanes: 1,
                biBitCount: 32, // 4 bytes per pixel
                biCompression: DIB_RGB_COLORS.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut buffer: Vec<u8> =
            vec![0; (icon_info.xHotspot * 2 * icon_info.yHotspot * 2 * 4) as usize];

        let dibits_result = GetDIBits(
            hdc_mem,
            icon_info.hbmColor,
            0,
            icon_info.yHotspot * 2,
            Some(buffer.as_mut_ptr() as *mut _),
            &mut bmp_info,
            DIB_RGB_COLORS,
        );

        // Clean up - 纭繚鍦ㄦ墍鏈夎矾寰勪笂閮介噴鏀捐祫婧?        SelectObject(hdc_mem, hbm_old);
        DeleteDC(hdc_mem).ok()?;
        DeleteDC(hdc_screen).ok()?;
        DeleteObject(icon_info.hbmColor.into()).ok()?;
        DeleteObject(icon_info.hbmMask.into()).ok()?;

        if dibits_result == 0 {
            return Err("Failed to get dibits".into());
        }

        if bmp_info.bmiHeader.biBitCount != 32 {
            return Err("Icon is not 32 bit".into());
        }

        bgra_to_rgba(buffer.as_mut_slice());

        let image = ImageBuffer::from_raw(icon_info.xHotspot * 2, icon_info.yHotspot * 2, buffer)
            .expect("Failed to create image buffer");
        Ok(image)
    }
}

/// this is the best solution having in consideration that a transparent image and have separated pixels
/// with transparent gaps, so search side by side and crop them is the best approach.
pub fn crop_transparent_borders(rgba_image: &RgbaImage) -> RgbaImage {
    let (width, height) = rgba_image.dimensions();
    let mut top = None;
    let mut bottom = None;
    let mut left = None;
    let mut right = None;

    'outer: for y in 0..height {
        for x in 0..width {
            let pixel = rgba_image.get_pixel(x, y);
            if pixel.0[3] != 0 {
                top = Some(y);
                break 'outer;
            }
        }
    }

    let top = match top {
        Some(top) => top,
        None => return RgbaImage::new(1, 1),
    };

    'outer: for y in (top..height).rev() {
        for x in 0..width {
            let pixel = rgba_image.get_pixel(x, y);
            if pixel.0[3] != 0 {
                bottom = Some(y);
                break 'outer;
            }
        }
    }

    let bottom = match bottom {
        Some(bottom) => bottom,
        None => return RgbaImage::new(1, 1),
    };

    'outer: for x in 0..width {
        for y in top..bottom {
            let pixel = rgba_image.get_pixel(x, y);
            if pixel.0[3] != 0 {
                left = Some(x);
                break 'outer;
            }
        }
    }

    let left = match left {
        Some(left) => left,
        None => return RgbaImage::new(1, 1),
    };

    'outer: for x in (left..width).rev() {
        for y in top..bottom {
            let pixel = rgba_image.get_pixel(x, y);
            if pixel.0[3] != 0 {
                right = Some(x);
                break 'outer;
            }
        }
    }

    let right = match right {
        Some(right) => right,
        None => return RgbaImage::new(1, 1),
    };

    rgba_image
        .view(left, top, right - left + 1, bottom - top + 1)
        .to_image()
}

pub fn get_icon_from_file(path: &Path, use_local_icon: bool) -> Result<RgbaImage> {
    // 浼樺厛妫€鏌ユ湰鍦拌嚜瀹氫箟鍥炬爣鐩綍 (C:\Program Files\HONOR\MagicAnimation\icon)
    if use_local_icon {
        if let Some(file_name) = path.file_name() {
            if let Some(png_data) =
                crate::utils::icon_whitelist::get_local_process_icon(&file_name.to_string_lossy())
            {
                if let Ok(img) = image::load_from_memory(&png_data) {
                    log::trace!("Using local custom icon for path: {:?}", path);
                    return Ok(img.to_rgba8());
                }
            }
        }
    }

    unsafe {
        // UNC 缃戠粶璺緞锛圽\server\share\...锛夋棤娉曢€氳繃 canonicalize() 瑙勮寖鍖栵紝鐩存帴浣跨敤鍘熷璺緞
        let path_str_raw = path.to_string_lossy();
        let is_unc = path_str_raw.starts_with(r"\\");
        let normalized = if is_unc {
            log::trace!(
                "[IconExtractor] UNC path detected, skipping canonicalize: {}",
                path_str_raw
            );
            path_str_raw.to_string()
        } else {
            path.canonicalize()?
                .to_string_lossy()
                .trim_start_matches(r"\\?\")
                .to_owned()
        };
        let path_str = normalized.encode_utf16().chain(Some(0)).collect_vec();

        let mut file_info = SHFILEINFOW::default();
        let result = SHGetFileInfoW(
            PCWSTR(path_str.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut file_info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_SYSICONINDEX,
        );

        if result == 0 {
            return Err("Failed to get file information".into());
        }

        // file_info.iIcon = 0 is a valid icon but it is the default icon for files on Windows
        // so we will handle this as no icon to avoid generate unnecessary artifacts.
        // 渚嬪锛歎NC 璺緞涓?iIcon 鍙兘涓?0 浣嗗疄闄呭浘鏍囨湁鏁堬紝灏濊瘯鐢?ExtractIconW 鍐掍釜
        if file_info.iIcon == 0 {
            if is_unc {
                // UNC 璺緞澶囬€夛細灏濊瘯 ExtractIconW 鐩存帴浠庡彲鎵ц鏂囦欢鎻愬彇鍥炬爣
                log::trace!(
                    "[IconExtractor] UNC path iIcon==0, falling back to ExtractIconW: {}",
                    normalized
                );
                let hicon = ExtractIconW(None, PCWSTR(path_str.as_ptr()), 0);
                if !hicon.is_invalid() && hicon.0 as isize > 1 {
                    let image = crop_transparent_borders(&convert_hicon_to_rgba_image(&hicon)?);
                    let _ = DestroyIcon(hicon);
                    return Ok(image);
                }
            }
            return Err("Icon index is 0".into());
        }

        let image_list: IImageList = SHGetImageList(SHIL_JUMBO as i32)?;
        // if 256x256 icon is not available, will use the icons with the most color depth and size
        // this is useful for some icons where color depth is less than 32,
        // example: icon of 124x124 16bits and other 64x64 32bits this will return the 32bits icon
        // color depth is prioritized over size
        let icon = image_list.GetIcon(file_info.iIcon, ILD_TRANSPARENT.0)?;
        let image = crop_transparent_borders(&convert_hicon_to_rgba_image(&icon)?);
        DestroyIcon(icon)?;
        Ok(image)
    }
}

const SQUARE_MARGIN: f32 = 0.1;
const ASPECT_TOLERANCE: f32 = 0.05;
const OPACITY_THRESHOLD: u8 = 254;
const CORNER_CHECK_SIZE_RATIO: f32 = 0.15; // 瑙掕惤妫€鏌ュ尯鍩熷ぇ灏忔瘮渚?
pub fn is_aproximately_a_square(rgba_image: &RgbaImage) -> bool {
    let (width, height) = rgba_image.dimensions();

    // verify if the image is not empty
    if width == 0 || height == 0 {
        return false;
    }

    // verify if the image is a square
    let aspect_ratio = width as f32 / height as f32;
    if (aspect_ratio - 1.0).abs() > ASPECT_TOLERANCE {
        return false;
    }

    // Calculate margin
    let margin_x = (width as f32 * SQUARE_MARGIN) as u32;
    let margin_y = (height as f32 * SQUARE_MARGIN) as u32;
    let inner_width = width - 2 * margin_x;
    let inner_height = height - 2 * margin_y;

    // verify if the image is a square
    for y in margin_y..margin_y + inner_height {
        for x in margin_x..margin_x + inner_width {
            let pixel = rgba_image.get_pixel(x, y);
            if pixel.0[3] < OPACITY_THRESHOLD {
                return false;
            }
        }
    }

    // Calculate corner check size
    let corner_check_size = (width as f32 * CORNER_CHECK_SIZE_RATIO) as u32;
    if corner_check_size == 0 {
        return true;
    }

    // Check if the icon has rounded corners by examining the four corners
    let has_rounded_corners = {
        let mut any_corner_transparent = false;

        // Top-left corner
        for y in 0..corner_check_size {
            for x in 0..corner_check_size {
                let pixel = rgba_image.get_pixel(x, y);
                if pixel.0[3] < OPACITY_THRESHOLD {
                    any_corner_transparent = true;
                    break;
                }
            }
            if any_corner_transparent {
                break;
            }
        }

        // Top-right corner
        if !any_corner_transparent {
            for y in 0..corner_check_size {
                for x in (width - corner_check_size)..width {
                    let pixel = rgba_image.get_pixel(x, y);
                    if pixel.0[3] < OPACITY_THRESHOLD {
                        any_corner_transparent = true;
                        break;
                    }
                }
                if any_corner_transparent {
                    break;
                }
            }
        }

        // Bottom-left corner
        if !any_corner_transparent {
            for y in (height - corner_check_size)..height {
                for x in 0..corner_check_size {
                    let pixel = rgba_image.get_pixel(x, y);
                    if pixel.0[3] < OPACITY_THRESHOLD {
                        any_corner_transparent = true;
                        break;
                    }
                }
                if any_corner_transparent {
                    break;
                }
            }
        }

        // Bottom-right corner
        if !any_corner_transparent {
            for y in (height - corner_check_size)..height {
                for x in (width - corner_check_size)..width {
                    let pixel = rgba_image.get_pixel(x, y);
                    if pixel.0[3] < OPACITY_THRESHOLD {
                        any_corner_transparent = true;
                        break;
                    }
                }
                if any_corner_transparent {
                    break;
                }
            }
        }

        any_corner_transparent
    };

    if has_rounded_corners {
        return true;
    } else {
        return false;
    }
}

// maintain this function as documentation for url files
#[allow(dead_code)]
fn get_icon_from_url_file(path: &Path, use_local_icon: bool) -> Result<RgbaImage> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);

    let mut path = None;
    // in theory .url files are encoded in UTF-8 so we don't need to use OsString
    for line in reader.lines() {
        if let Some(stripped) = line?.strip_prefix("IconFile=") {
            path = Some(PathBuf::from(stripped));
            break;
        }
    }

    let path = match path {
        Some(icon_file) => icon_file,
        None => return Err("Failed to get icon".into()),
    };

    get_icon_from_file(&path, use_local_icon)
}

pub fn extract_and_save_icon_from_file<T: AsRef<Path>>(path: T, use_local_icon: bool) {
    IconExtractor::request(IconExtractorRequest::Path {
        path: path.as_ref().to_path_buf(),
        use_local_icon,
    });
}

/// returns the path of the icon extracted from the executable or copied if is an UWP app.
///
/// If the icon already exists, it returns the path instead overriding, this is needed for allow user custom icons.
///
/// umid on this case only applys to Property Store umid
pub fn _extract_and_save_icon_from_file(
    origin: &Path,
    umid: Option<String>,
    use_local_icon: bool,
) -> Result<()> {
    if !origin.exists() || origin.is_dir() {
        // 馃敡 鏂囦欢涓嶅瓨鍦ㄦ椂锛屽皾璇曚粠娲昏穬绐楀彛鎻愬彇鍥炬爣锛堝畨瑁呯▼搴忓満鏅級
        if let Some(file_name) = origin.file_name() {
            let process_name = file_name.to_string_lossy().to_string();
            if let Some(hwnd) = find_hwnd_by_process_name(&process_name) {
                log::info!(
                    "[IconExtractor] File not found {:?}, extracting from active window hwnd={}",
                    origin,
                    hwnd
                );
                if extract_and_save_icon_from_window(
                    hwnd,
                    &process_name,
                    use_local_icon,
                    Some(origin),
                ) {
                    return Ok(());
                }
            }
        }
        return Err(format!("File not found: {}", origin.display()).into());
    }

    let origin_ext = match origin.extension() {
        Some(ext) => ext.to_string_lossy().to_lowercase(),
        // no extension === no icon
        None => return Ok(()),
    };

    // ico files are by itself an icon
    if origin_ext == "ico" {
        return Ok(());
    }

    let is_exe_file = origin_ext == "exe";
    let is_lnk_file = origin_ext == "lnk";
    let is_url_file = origin_ext == "url";

    let file_name = origin.file_name().ok_or("Failed to get file name")?;
    let filestem = origin.file_stem().ok_or("Failed to get file stem")?;

    // 浼樺厛妫€鏌ユ湰鍦拌嚜瀹氫箟鍥炬爣鐩綍 (C:\Program Files\HONOR\MagicAnimation\icon)
    // 鍗充娇缂撳瓨涓凡鏈夊浘鏍囷紝涔熶紭鍏堜娇鐢ㄦ湰鍦扮洰褰曠殑鍥炬爣杩涜瑕嗙洊鏇存柊
    let local_custom_icon = if use_local_icon {
        crate::utils::icon_whitelist::get_local_process_icon(&file_name.to_string_lossy())
    } else {
        // 鐧借壊鑳屾澘妯″紡锛氭煡鎵?{process_name}-white.png
        crate::utils::icon_whitelist::get_local_process_icon_white(&file_name.to_string_lossy())
    };

    let mutex = FULL_STATE.load().icon_packs().clone();
    let mut icon_manager = trace_lock!(mutex);

    if local_custom_icon.is_none() {
        if is_exe_file || is_lnk_file || is_url_file {
            if icon_manager.has_app_icon(None, Some(origin)) {
                drop(icon_manager);
                let _ = FULL_STATE.load().emit_icon_packs();
                return Ok(());
            }
        } else {
            if icon_manager.has_app_icon(None, Some(origin)) {
                drop(icon_manager);
                let _ = FULL_STATE.load().emit_icon_packs();
                return Ok(());
            } else if icon_manager.get_file_icon(origin).is_some() {
                drop(icon_manager);
                let _ = FULL_STATE.load().emit_icon_packs();
                return Ok(());
            }
            let process_name = file_name.to_string_lossy().to_string();
            if let Some(hwnd) = find_hwnd_by_process_name(&process_name) {
                log::info!(
                    "[IconExtractor] Cache miss for {:?}, re-extracting from active window hwnd={}",
                    origin,
                    hwnd
                );
                drop(icon_manager);
                // 鏃犺鎴愬姛涓庡惁閮界洿鎺ヨ繑鍥烇紝闈瀍xe鏂囦欢鍚庣画涔熸棤娉曚粠鏂囦欢鎻愬彇鍥炬爣
                let _ = extract_and_save_icon_from_window(
                    hwnd,
                    &process_name,
                    use_local_icon,
                    Some(origin),
                );
                return Ok(());
            }
        }
    }

    let root = VAR_COMMON.user_icons_path().join("system");
    // 浣跨敤璺緞鍝堝笇鐢熸垚绋冲畾鏂囦欢鍚嶏紝閬垮厤閲嶅缂撳瓨
    let gen_icon_filename = format!(
        "{}_{}.png",
        filestem.to_string_lossy(),
        path_based_hash_id(origin)
    );
    let mut gen_icon = Icon {
        base: Some(gen_icon_filename.clone()),
        ..Default::default()
    };

    if origin_ext == "url" {
        if let Ok(icon) = get_icon_from_url_file(origin, use_local_icon) {
            // 鏍规嵁鑳屾澘妯″紡璁＄畻 is_aproximately_square
            let is_square = if use_local_icon {
                // 閫忔槑鑳屾澘妯″紡锛氭鏌ユ槸鍚︽湁鏈湴鍥炬爣
                crate::utils::icon_whitelist::has_local_process_icon(&file_name.to_string_lossy())
            } else {
                crate::utils::icon_whitelist::has_local_process_icon_white(
                    &file_name.to_string_lossy(),
                )
            };
            gen_icon.is_aproximately_square = is_square;
            icon.save(root.join(&gen_icon_filename))?;
            icon_manager.add_system_app_icon(None, Some(origin), gen_icon);
            icon_manager.write_system_icon_pack()?;
            drop(icon_manager);
            let _ = FULL_STATE.load().emit_icon_packs();
        }
        return Ok(());
    }

    if is_lnk_file {
        let (lnk_icon_path, lnk_icon_idx) = match WindowsApi::resolve_lnk_custom_icon_path(origin) {
            Ok((icon_path, icon_idx)) => (icon_path, icon_idx),
            Err(_) => {
                let (target, _) = WindowsApi::resolve_lnk_target(origin)?;
                (target, 0)
            }
        };

        let lnk_target_ext = lnk_icon_path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase());
        let should_redirect = lnk_target_ext
            .as_deref()
            .is_some_and(|ext| ext == "exe" || ext == "lnk" || ext == "url");

        // 褰撳浘鏍囩储寮?> 0 涓旂洰鏍囨槸 .exe 鏃讹紝涓嶈兘閲嶅畾鍚戯紝鍥犱负閲嶅畾鍚戜細鎻愬彇 exe 榛樿鍥炬爣锛堢储寮?0锛?        // 鑰屼笉鏄?.lnk 鎸囧畾鐨勭壒瀹氬浘鏍囥€傚吀鍨嬪満鏅細Androws 瀛愬簲鐢ㄥ叡鐢?AndrowsLauncher.exe锛?        // 涓嶅悓瀛愬簲鐢ㄩ€氳繃涓嶅悓鍥炬爣绱㈠紩鍖哄垎锛岄噸瀹氬悜浼氬鑷存墍鏈夊瓙搴旂敤閮芥樉绀哄惎鍔ㄥ櫒鍥炬爣銆?        //
        // 褰撹皟鐢ㄦ柟浼犲叆浜?UMID 鏃讹紙鏉ヨ嚜 _extract_and_save_icon_umid 鐨?PropertyStore 璺緞锛夛紝
        let can_redirect = should_redirect && lnk_icon_idx == 0 && umid.is_none();

        if can_redirect {
            // 濡傛灉鏈湴宸叉湁鑷畾涔夊浘鏍囷紝鍒欎笉闇€瑕侀噸瀹氬悜鎻愬彇锛岀洿鎺ョ户缁悗缁繚瀛橀€昏緫
            if local_custom_icon.is_none() {
                drop(icon_manager);
                _extract_and_save_icon_from_file(&lnk_icon_path, umid.clone(), use_local_icon)?;

                let mut icon_manager = trace_lock!(mutex);

                // 璁＄畻鐩爣鏂囦欢鐨勫浘鏍囨枃浠跺悕
                let lnk_filestem = lnk_icon_path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                let lnk_gen_icon_filename = format!(
                    "{}_{}.png",
                    lnk_filestem,
                    path_based_hash_id(&lnk_icon_path)
                );

                // 鏍规嵁鑳屾澘妯″紡璁＄畻 is_aproximately_square
                let lnk_file_name = lnk_icon_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let is_square = if use_local_icon {
                    crate::utils::icon_whitelist::has_local_process_icon(&lnk_file_name)
                } else {
                    crate::utils::icon_whitelist::has_local_process_icon_white(&lnk_file_name)
                };

                // 鏇存柊鐩爣鏂囦欢鐨勭紦瀛樻潯鐩紝璁剧疆 is_aproximately_square
                let updated_icon = Icon {
                    base: Some(lnk_gen_icon_filename),
                    is_aproximately_square: is_square,
                    ..Default::default()
                };

                icon_manager.add_system_app_icon(
                    umid.as_deref(),
                    Some(&lnk_icon_path),
                    updated_icon,
                );
                icon_manager.add_system_icon_redirect(umid, origin, &lnk_icon_path);
                icon_manager.write_system_icon_pack()?;
                drop(icon_manager);
                let _ = FULL_STATE.load().emit_icon_packs();
                return Ok(());
            }
        }
    }

    // 浼樺厛浣跨敤鏈湴鍥炬爣鎴朣endMessage鏂规硶锛屽け璐ュ垯浣跨敤榛樿鏂规硶
    let (icon, _is_from_local) = if let Some(png_data) = local_custom_icon
        .as_ref()
        .cloned()
        .or_else(|| _extract_icon_with_sendmessage(origin, use_local_icon))
    {
        let from_local = local_custom_icon.is_some();
        match image::load_from_memory(&png_data) {
            Ok(dynamic_image) => (dynamic_image.to_rgba8(), from_local),
            Err(_) => match get_icon_from_file(origin, use_local_icon) {
                Ok(icon) => (icon, false),
                Err(_) => {
                    log::trace!("Icon not found for {}", origin.display());
                    return Ok(());
                }
            },
        }
    } else {
        match get_icon_from_file(origin, use_local_icon) {
            Ok(icon) => (icon, false),
            Err(_) => {
                log::trace!("Icon not found for {}", origin.display());
                return Ok(());
            }
        }
    };

    // 鏈湴鍥炬爣涓嶅姞鑳屾澘锛岄潪鏈湴鍥炬爣鍔犺儗鏉?    gen_icon.is_aproximately_square = is_from_local;

    // 鐩存帴浣跨敤鍘熷鍥炬爣
    let final_icon = icon;

    if is_exe_file || is_lnk_file {
        final_icon.save(root.join(&gen_icon_filename))?;
        icon_manager.add_system_app_icon(umid.as_deref(), Some(origin), gen_icon);
    } else {
        let gen_icon_filename = format!("{}_{}.png", origin_ext, path_based_hash_id(origin));
        final_icon.save(root.join(&gen_icon_filename))?;
        gen_icon.base = Some(gen_icon_filename);
        icon_manager.add_system_file_icon(&origin_ext, gen_icon);
    }
    icon_manager.write_system_icon_pack()?;
    drop(icon_manager);
    let _ = FULL_STATE.load().emit_icon_packs();

    Ok(())
}

/// Extract icon from window handle and save it
/// This is useful when the process path is not an .exe file (e.g., .pak, .tmp, etc.)
/// Returns true if icon was successfully extracted and saved
pub fn extract_and_save_icon_from_window(
    hwnd: isize,
    process_name: &str,
    use_local_icon: bool,
    origin_path: Option<&Path>,
) -> bool {
    let hwnd = HWND(hwnd as _);

    // 浼樺厛妫€鏌ユ湰鍦拌嚜瀹氫箟鍥炬爣鐩綍
    if use_local_icon {
        if let Some(png_data) = crate::utils::icon_whitelist::get_local_process_icon(process_name) {
            log::trace!(
                "[IconExtractor] Using local custom icon for window: {}",
                process_name
            );
            if let Ok(dynamic_image) = image::load_from_memory(&png_data) {
                let icon = dynamic_image.to_rgba8();
                let root = VAR_COMMON.user_icons_path().join("system");
                let filename = format!(
                    "{}.png",
                    process_name.strip_suffix(".exe").unwrap_or(process_name)
                );
                if icon.save(root.join(&filename)).is_ok() {
                    // 鏇存柊缂撳瓨
                    let mutex = FULL_STATE.load().icon_packs().clone();
                    let mut icon_manager = trace_lock!(mutex);
                    let gen_icon = Icon {
                        base: Some(filename.clone()),
                        is_aproximately_square: true,
                        ..Default::default()
                    };
                    // 寤虹珛璺緞鍒板浘鏍囩殑鏄犲皠
                    icon_manager.add_system_app_icon(None, origin_path, gen_icon);
                    let _ = icon_manager.write_system_icon_pack();
                    drop(icon_manager);
                    let _ = FULL_STATE.load().emit_icon_packs();
                    return true;
                }
            }
        }
    }

    // 灏濊瘯浠庣獥鍙ｅ彞鏌勮幏鍙栧ぇ鍥炬爣
    let window_icon = {
        use base64::Engine as _;
        use slu_ipc::messages::SvcAction;

        crate::cli::ServicePipe::request_with_response_blocking(
            SvcAction::GetWindowIconPng {
                hwnd: hwnd.0 as isize,
                large: true,
            },
            Duration::from_millis(1200),
        )
        .ok()
        .flatten()
        .and_then(|encoded| {
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()
        })
        .and_then(|png| image::load_from_memory(&png).ok())
        .map(|img| img.to_rgba8())
    };

    if let Some(rgba_image) = window_icon {
        log::info!(
            "[IconExtractor] Got icon from window handle: hwnd={:?}, process={}",
            hwnd,
            process_name
        );

        let icon = crop_transparent_borders(&rgba_image);

        let root = VAR_COMMON.user_icons_path().join("system");
        let stem = process_name.strip_suffix(".exe").unwrap_or(process_name);
        let filename = format!("{}.png", stem);

        if icon.save(root.join(&filename)).is_ok() {
            log::info!("[IconExtractor] Icon saved successfully: {}", filename);
            let mutex = FULL_STATE.load().icon_packs().clone();
            let mut icon_manager = trace_lock!(mutex);
            let gen_icon = Icon {
                base: Some(filename.clone()),
                is_aproximately_square: false,
                ..Default::default()
            };
            log::info!(
                "[IconExtractor] Adding path mapping: {:?} -> {}",
                origin_path,
                filename
            );
            icon_manager.add_system_app_icon(None, origin_path, gen_icon);
            let has_icon = icon_manager.has_app_icon(None, origin_path);
            log::info!(
                "[IconExtractor] Path mapping verification: has_icon={}",
                has_icon
            );
            let _ = icon_manager.write_system_icon_pack();
            drop(icon_manager);
            let _ = FULL_STATE.load().emit_icon_packs();
            return true;
        } else {
            log::warn!("[IconExtractor] Failed to save icon file: {}", filename);
        }
    } else {
        log::warn!(
            "[IconExtractor] No icon found for window: hwnd={:?}, process={}",
            hwnd,
            process_name
        );
    }

    false
}

/// 閫氳繃杩涚▼鍚嶅湪褰撳墠娲昏穬绐楀彛涓煡鎵?HWND
fn find_hwnd_by_process_name(process_name: &str) -> Option<isize> {
    use crate::windows_api::WindowEnumerator;
    let process_name_lower = process_name.to_lowercase();
    let mut found_hwnd: Option<isize> = None;
    let _ = WindowEnumerator::new().for_each(|window| {
        if found_hwnd.is_some() {
            return;
        }
        if !window.is_visible() {
            return;
        }
        if let Ok(name) = window.process().program_exe_name() {
            if name.to_lowercase() == process_name_lower {
                found_hwnd = Some(window.hwnd().0 as isize);
            }
        }
    });
    found_hwnd
}

pub fn extract_and_save_icon_umid(aumid: &AppUserModelId, use_local_icon: bool) {
    IconExtractor::request(IconExtractorRequest::AppUMID {
        umid: aumid.clone(),
        use_local_icon,
    });
}

/// Extract icon using SendMessage method for whitelisted applications
fn _extract_icon_with_sendmessage(origin: &Path, use_local_icon: bool) -> Option<Vec<u8>> {
    // Check if the application is in the whitelist
    let file_name = origin.file_name()?.to_string_lossy().to_string();

    if use_local_icon {
        if let Some(data) = crate::utils::icon_whitelist::get_local_process_icon(&file_name) {
            log::trace!("Using local custom icon for: {}", file_name);
            return Some(data);
        }
    }

    if !is_app_whitelisted(&file_name) {
        // Not in whitelist, use default extraction method
        return None;
    }

    log::trace!(
        "Using SendMessage method for icon extraction: {}",
        file_name
    );

    // For whitelisted applications, we should extract icon from window, not from file
    // This maintains consistency with MagicTaskbar implementation
    // Return None here to let the caller use window-based extraction
    None
}

/// returns the path of the icon extracted from the app with the specified package app user model id.
pub fn _extract_and_save_icon_umid(aumid: &AppUserModelId, use_local_icon: bool) -> Result<()> {
    let icon_manager_mutex = FULL_STATE.load().icon_packs().clone();

    let app_umid = match aumid {
        AppUserModelId::Appx(u) => u,
        AppUserModelId::PropertyStore(u) => u,
    };
    let path = UwpManager::get_app_path(app_umid).ok().flatten();

    let mut local_icon_data = None;
    if let Some(p) = &path {
        if let Some(file_name) = p.file_name() {
            if use_local_icon {
                // 閫忔槑鑳屾澘妯″紡锛氭煡鎵?{name}.png
                local_icon_data = crate::utils::icon_whitelist::get_local_process_icon(
                    &file_name.to_string_lossy(),
                );
            } else {
                // 鐧借壊鑳屾澘妯″紡锛氭煡鎵?{name}-white.png
                local_icon_data = crate::utils::icon_whitelist::get_local_process_icon_white(
                    &file_name.to_string_lossy(),
                );
            }
        }
    }

    if local_icon_data.is_none() {
        let manager = trace_lock!(icon_manager_mutex);
        if manager.has_app_icon(Some(aumid.as_str()), path.as_deref()) {
            // 馃敡 缂撳瓨鍛戒腑鏃朵篃鍙戦€佷簨浠讹紝纭繚鍓嶇鑳芥敹鍒伴€氱煡
            drop(manager);
            let _ = FULL_STATE.load().emit_icon_packs();
            return Ok(());
        }
    }

    match aumid {
        AppUserModelId::Appx(app_umid) => {
            let mut gen_icon = Icon::default();

            if local_icon_data.is_some() {
                if let Some(png_data) = local_icon_data {
                    if let Ok(dynamic_image) = image::load_from_memory(&png_data) {
                        let icon = dynamic_image.to_rgba8();
                        let root = VAR_COMMON.user_icons_path().join("system");
                        let name = umid_based_hash_id(app_umid);
                        let filename = format!("{}.png", name);

                        icon.save(root.join(&filename))?;
                        gen_icon.base = Some(filename);
                        // 瀵逛簬鏈湴鑷畾涔夊浘鏍囷紝寮哄埗涓嶆樉绀鸿儗鏉?(is_approximately_square = true)
                        gen_icon.is_aproximately_square = true;

                        let mut icon_manager = trace_lock!(icon_manager_mutex);
                        icon_manager.add_system_app_icon(Some(app_umid), path.as_deref(), gen_icon);
                        icon_manager.write_system_icon_pack()?;
                        drop(icon_manager);
                        let _ = FULL_STATE.load().emit_icon_packs();
                        return Ok(());
                    }
                }
            }

            let (light_path, dark_path) = UwpManager::get_high_quality_icon_path(app_umid)?;

            let root = VAR_COMMON.user_icons_path().join("system");
            let name = umid_based_hash_id(app_umid);

            let light_rgba = image::open(&light_path)?.to_rgba8();
            let light_rgba = crop_transparent_borders(&light_rgba);

            if light_path != dark_path {
                let dark_rgba = image::open(&dark_path)?.to_rgba8();
                let dark_rgba = crop_transparent_borders(&dark_rgba);

                light_rgba.save(root.join(format!("{name}_light.png")))?;
                dark_rgba.save(root.join(format!("{name}_dark.png")))?;

                gen_icon.light = Some(format!("{name}_light.png"));
                gen_icon.dark = Some(format!("{name}_dark.png"));
            } else {
                light_rgba.save(root.join(format!("{name}.png")))?;
                gen_icon.base = Some(format!("{name}.png"));
            }

            // 鏍规嵁鑳屾澘绫诲瀷鍐冲畾鏄惁璁＄畻鍥炬爣鏄惁鏄鏂瑰舰
            // 涓ょ妯″紡閮斤細闈炴湰鍦板浘鏍囧叏閮ㄥ姞鑳屾澘
            gen_icon.is_aproximately_square = false;

            let mut icon_manager = trace_lock!(icon_manager_mutex);
            icon_manager.add_system_app_icon(Some(app_umid), path.as_deref(), gen_icon);
            icon_manager.write_system_icon_pack()?;
            drop(icon_manager);
            let _ = FULL_STATE.load().emit_icon_packs();
            Ok(())
        }
        AppUserModelId::PropertyStore(app_umid) => {
            let start = START_MENU_MANAGER.load();
            let lnk = start
                .get_by_file_umid(app_umid)
                .ok_or(format!("No shortcut found for umid {app_umid}"))?;

            // 楠岃瘉 get_by_file_umid 杩斿洖鐨勬槸绮剧‘鍖归厤杩樻槸妯＄硦鍖归厤
            // 绮剧‘鍖归厤锛?lnk 鐨?PropertyStore UMID 涓庤姹備竴鑷达紝鎴栫洰鏍囪矾寰勪互 UMID 缁撳熬
            // 妯＄硦鍖归厤鍙兘杩斿洖閿欒鐨?.lnk锛堝 Androws 瀛愬簲鐢ㄥ尮閰嶅埌鍚姩鍣ㄥ揩鎹锋柟寮忥級锛?            // 鎻愬彇鐨勫浘鏍囦細鏄惎鍔ㄥ櫒鍥炬爣鑰岄潪瀛愬簲鐢ㄥ浘鏍囷紝搴旇烦杩囪 Path 璇锋眰姝ｇ‘澶勭悊
            let is_direct_match = lnk.umid.as_deref() == Some(app_umid)
                || lnk
                    .target
                    .as_ref()
                    .is_some_and(|t| t.to_string_lossy().ends_with(app_umid));
            if !is_direct_match {
                log::debug!("[IconExtractor] Skipping fuzzy-matched shortcut for PropertyStore UMID '{}': {:?}", app_umid, lnk.path);
                return Ok(());
            }

            {
                let manager = trace_lock!(icon_manager_mutex);
                if manager.has_app_icon(Some(aumid.as_str()), Some(&lnk.path)) {
                    // 馃敡 缂撳瓨鍛戒腑鏃朵篃鍙戦€佷簨浠讹紝纭繚鍓嶇鑳芥敹鍒伴€氱煡
                    drop(manager);
                    let _ = FULL_STATE.load().emit_icon_packs();
                    return Ok(());
                }
            }

            _extract_and_save_icon_from_file(&lnk.path, Some(app_umid.clone()), use_local_icon)?;
            Ok(())
        }
    }
}
