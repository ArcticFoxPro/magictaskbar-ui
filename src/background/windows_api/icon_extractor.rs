use std::path::Path;
use windows::Win32::{
    Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    },
    Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES,
    UI::{
        Shell::{
            ExtractIconW, SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON,
            SHGFI_SHELLICONSIZE, SHGFI_SMALLICON,
        },
        WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO},
    },
};

use crate::{error::Result, windows_api::string_utils::WindowsString};

/// Icon size enum matching ManagedShell's IconSize
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconSize {
    Small,
    Large,
}

/// Icon extractor based on ManagedShell's ApplicationWindow implementation
pub struct IconExtractor;

impl IconExtractor {
    /// Get icon from file path
    /// Implements IconHelper.GetIconByFilename logic (line 548)
    pub fn get_icon_by_filename(path: &Path, size: IconSize) -> Option<HICON> {
        let path_str = WindowsString::from_os_string(path.as_os_str());

        let mut shfi = SHFILEINFOW::default();
        let flags = SHGFI_ICON
            | SHGFI_SHELLICONSIZE
            | if size == IconSize::Large {
                SHGFI_LARGEICON
            } else {
                SHGFI_SMALLICON
            };

        unsafe {
            let result = SHGetFileInfoW(
                path_str.as_pcwstr(),
                FILE_FLAGS_AND_ATTRIBUTES(0),
                Some(&mut shfi),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                flags,
            );

            if result != 0 && !shfi.hIcon.is_invalid() {
                return Some(shfi.hIcon);
            }
        }

        // Fallback: try ExtractIconW
        unsafe {
            // For war3.exe, try multiple indices and pick the largest icon
            let is_war3 = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| {
                    let name_lower = name.to_lowercase();
                    name_lower == "war3.exe" || name_lower == "warcraft iii.exe"
                })
                .unwrap_or(false);

            log::info!(
                "[IconExtractor] get_icon_by_filename: {:?}, is_war3: {}",
                path,
                is_war3
            );

            if is_war3 {
                // Try indices 0-10 and pick the largest one (by PNG size)
                let mut best_hicon: Option<HICON> = None;
                let mut best_size = 0usize;

                log::info!("[War3] Starting icon index scan (0-10)");
                for index in 0..11 {
                    let hicon = ExtractIconW(None, path_str.as_pcwstr(), index);
                    if !hicon.is_invalid() && hicon.0 as isize > 1 {
                        if let Ok(png_data) = Self::hicon_to_png(HICON(hicon.0)) {
                            let size = png_data.len();
                            log::info!("[War3] Index {}: {} bytes", index, size);

                            if size > best_size {
                                // Destroy previous best if exists
                                if let Some(prev) = best_hicon {
                                    let _ = DestroyIcon(prev);
                                }
                                best_hicon = Some(HICON(hicon.0));
                                best_size = size;
                            } else {
                                let _ = DestroyIcon(HICON(hicon.0));
                            }
                        } else {
                            let _ = DestroyIcon(HICON(hicon.0));
                        }
                    }
                }

                if let Some(hicon) = best_hicon {
                    log::info!("[War3] Selected icon with {} bytes", best_size);
                    return Some(hicon);
                }
            } else {
                // For other executables, use index 0
                let hicon = ExtractIconW(None, path_str.as_pcwstr(), 0);
                if !hicon.is_invalid() && hicon.0 as isize > 1 {
                    return Some(HICON(hicon.0));
                }
            }
        }

        None
    }

    /// Convert HICON to PNG bytes
    /// Implements IconImageConverter.GetImageFromHIcon logic
    pub fn hicon_to_png(hicon: HICON) -> Result<Vec<u8>> {
        unsafe {
            let mut icon_info = ICONINFO::default();
            GetIconInfo(hicon, &mut icon_info)?;

            // Ensure bitmaps are freed even if errors occur later
            let color_bitmap = icon_info.hbmColor;
            let mask_bitmap = icon_info.hbmMask;

            // Get bitmap info
            let mut bitmap = BITMAP::default();
            GetObjectW(
                color_bitmap.into(),
                std::mem::size_of::<BITMAP>() as i32,
                Some(&mut bitmap as *mut _ as *mut _),
            );

            let width = bitmap.bmWidth;
            let height = bitmap.bmHeight;
            log::trace!("IconExtractor: HICON size = {}x{}", width, height);

            // Create bitmap info header
            let bi = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // Top-down DIB
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                ..Default::default()
            };

            // Allocate buffer for pixel data
            let buffer_size = (width * height * 4) as usize;
            let mut pixel_data = vec![0u8; buffer_size];

            // Get pixel data
            let hdc = CreateCompatibleDC(None);
            let bmi = BITMAPINFO {
                bmiHeader: bi,
                ..Default::default()
            };

            GetDIBits(
                hdc,
                color_bitmap,
                0,
                height as u32,
                Some(pixel_data.as_mut_ptr() as *mut _),
                &bmi as *const _ as *mut _,
                DIB_RGB_COLORS,
            );

            // Clean up resources before potential error returns
            let _ = DeleteDC(hdc);
            let _ = DeleteObject(color_bitmap.into());
            if !mask_bitmap.is_invalid() {
                let _ = DeleteObject(mask_bitmap.into());
            }

            // Convert BGRA to RGBA and ensure Alpha is fully opaque
            for i in (0..buffer_size).step_by(4) {
                pixel_data.swap(i, i + 2);
            }

            // Encode to PNG
            let mut png_data = Vec::new();
            {
                use image::{ImageBuffer, Rgba};
                let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
                    ImageBuffer::from_raw(width as u32, height as u32, pixel_data)
                        .ok_or("Failed to create image buffer")?;

                let mut cursor = std::io::Cursor::new(&mut png_data);
                img.write_to(&mut cursor, image::ImageFormat::Png)
                    .map_err(|e| format!("PNG encoding failed: {}", e))?;
            }

            Ok(png_data)
        }
    }

    /// Destroy icon handle (line 566)
    pub fn destroy_icon(hicon: HICON) -> Result<()> {
        unsafe {
            DestroyIcon(hicon)?;
        }
        Ok(())
    }
}
