//! 托盘图标：纯色剪影。
//!
//! 两平台共用一个几何（`render::silhouette`），差别只在于「颜色谁来定」：
//!
//! | 平台 | 机制 | 颜色 |
//! |------|------|------|
//! | macOS | 不透明黑剪影 + `isTemplate = YES`，系统按菜单栏明暗主题自动反色 | 应用不选 |
//! | Windows | 系统不认模板标记，按系统托盘设置（浅色/深色）自行选墨色 | 系统设置决定 |
//!
//! 不能用 `default_window_icon`：那是彩色应用图标，缩小成 16px 托盘尺寸后
//! 既看不清也不随主题变色。

use tauri::image::Image;

use super::render;

/// 位图边长（像素）：托盘图标名义 16–22pt，这里给 2 倍屏 44px 并留一点余量。
/// macOS 模板图标由系统按需要缩放；Windows 由托盘自行缩到 DPI 对应的尺寸。
pub const SIZE: u32 = 48;

/// macOS 模板像素本体颜色（系统会整体重着色，只需不透明）。
#[cfg(target_os = "macos")]
const INK: (u8, u8, u8) = (0, 0, 0);
/// 浅色托盘背景上的墨色：近黑，留一点灰以免纯黑显得生硬。
#[cfg(not(target_os = "macos"))]
const INK_ON_LIGHT: (u8, u8, u8) = (32, 32, 32);
/// 深色托盘背景上的墨色：近白。
#[cfg(not(target_os = "macos"))]
const INK_ON_DARK: (u8, u8, u8) = (236, 236, 236);

/// 托盘图标（预乘 RGBA，符合 Tauri `Image` 的字节序约定）。
///
/// macOS 产出供系统着色的黑色模板；Windows 按系统托盘深浅色选墨色。
pub fn tray_image() -> Image<'static> {
    tray_image_for(platform_is_dark())
}

fn tray_image_for(dark: bool) -> Image<'static> {
    let mut pixmap = render::silhouette(SIZE, ink_for(dark));
    render::premultiply(pixmap.data_mut());
    Image::new(pixmap.take().leak(), SIZE, SIZE)
}

/// 当前平台托盘背景是否为深色（只影响 Windows 的墨色）。
#[cfg(target_os = "macos")]
fn platform_is_dark() -> bool {
    false
}
#[cfg(windows)]
fn platform_is_dark() -> bool {
    system_tray_is_dark()
}
#[cfg(not(any(target_os = "macos", windows)))]
fn platform_is_dark() -> bool {
    false
}

fn ink_for(dark: bool) -> (u8, u8, u8) {
    #[cfg(target_os = "macos")]
    {
        let _ = dark;
        INK
    }
    #[cfg(not(target_os = "macos"))]
    {
        if dark {
            INK_ON_DARK
        } else {
            INK_ON_LIGHT
        }
    }
}

/// 系统托盘当前是否为深色（决定墨色）。
///
/// 与前端 `theme` 设置无关——托盘背景由系统绘制，只有系统设置说了算。
/// 读 `HKCU\...\Themes\Personalize\SystemUsesLightTheme`（0 = 深色）；
/// 读不到（老系统、远程会话、注册表异常）时按浅色处理。
#[cfg(windows)]
pub fn system_tray_is_dark() -> bool {
    use windows::core::w;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
        REG_DWORD,
    };

    let mut key = HKEY::default();
    let open = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            None,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    if open.is_err() {
        return false;
    }
    let mut ty = Default::default();
    let mut data = 1u32;
    let mut len = std::mem::size_of::<u32>() as u32;
    let query = unsafe {
        RegQueryValueExW(
            key,
            w!("SystemUsesLightTheme"),
            None,
            Some(&mut ty),
            Some(&mut data as *mut u32 as *mut u8),
            Some(&mut len),
        )
    };
    let _ = unsafe { RegCloseKey(key) };
    query.is_ok() && ty == REG_DWORD && len == 4 && data == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 不透明像素必须是单一墨色（预乘后 R=G=B）；抗锯齿边缘允许半透明。
    fn assert_monochrome(img: &Image<'_>, expect: (u8, u8, u8)) {
        let px = img.rgba();
        assert_eq!(px.len(), (SIZE * SIZE * 4) as usize);
        let (pixels, _) = px.as_chunks::<4>();
        for c in pixels.iter().filter(|c| c[3] == 255) {
            assert_eq!((c[0], c[1], c[2]), expect);
        }
    }

    #[test]
    fn image_is_monochrome_silhouette() {
        let img = tray_image();
        assert_monochrome(&img, ink_for(platform_is_dark()));
        let px = img.rgba();
        let (pixels, _) = px.as_chunks::<4>();
        assert!(pixels.iter().any(|c| c[3] == 0), "需保留透明留白");
        assert!(pixels.iter().any(|c| c[3] == 255), "需有不透明实心");
        // 剪影中心与边缘都要有内容 / 留白
        let at = |x: u32, y: u32| px[((y * SIZE + x) * 4 + 3) as usize];
        let k = SIZE as f32 / render::CANVAS;
        // 顶块中心、枢纽竖条中点都在剪影内（允许抗锯齿取样的偏差）
        assert!(at((render::TOP.cx() * k) as u32, (render::TOP.cy() * k) as u32) > 200);
        assert!(
            at(
                (render::TRUNK.cx() * k) as u32,
                (render::TRUNK.cy() * k) as u32
            ) > 200
        );
        // 四角必须留白
        assert_eq!(at(0, 0), 0);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn windows_swaps_ink_by_theme() {
        assert_monochrome(&tray_image_for(true), INK_ON_DARK);
        // 浅色主题墨色必须比深色主题暗
        let lum = |c: (u8, u8, u8)| c.0 as u16 + c.1 as u16 + c.2 as u16;
        assert!(lum(INK_ON_LIGHT) < lum(INK_ON_DARK));
    }
}
