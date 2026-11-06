//! 菜单栏（NSStatusBar）图标：纯色剪影模板。
//!
//! 提供一个**不透明黑色**的剪影位图 + `isTemplate = YES`（Tauri 的
//! `set_icon_with_as_template(true)`），系统据此按菜单栏明暗主题自动反色：
//! 浅色菜单栏画成黑、深色画成白；选中高亮反色亦然。应用自己永远不选颜色。

use tauri::image::Image;

use super::render;

/// 位图边长（像素）：菜单栏图标名义 18–22pt，这里给 2 倍屏 44px 并留一点余量。
/// 模板图标由系统按需要缩放，尺寸只需 ≥ 最大显示尺寸。
pub const SIZE: u32 = 48;
/// 模板像素本体颜色（系统会整体重着色，只需不透明）。
const INK: (u8, u8, u8) = (0, 0, 0);

/// 菜单栏图标（预乘 RGBA，符合 Tauri `Image` 的字节序约定）。
pub fn tray_image() -> Image<'static> {
    let mut pixmap = render::silhouette(SIZE, INK);
    render::premultiply(pixmap.data_mut());
    Image::new(pixmap.take().leak(), SIZE, SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_is_opaque_black() {
        let img = tray_image();
        let px = img.rgba();
        assert_eq!(px.len(), (SIZE * SIZE * 4) as usize);
        // 有色像素必须是纯黑（预乘后 R=G=B）。抗锯齿边缘允许半透明，
        // 但不透明像素必须写实——模板图标靠 α 承载形状，颜色只有一个值。
        for c in px.chunks_exact(4).filter(|c| c[3] == 255) {
            assert_eq!((c[0], c[1], c[2]), (0, 0, 0));
        }
        assert!(px.chunks_exact(4).any(|c| c[3] == 0), "需保留透明留白");
        assert!(px.chunks_exact(4).any(|c| c[3] == 255), "需有不透明实心");
        // 剪影中心与边缘都要有内容 / 留白
        let at = |x: u32, y: u32| px[((y * SIZE + x) * 4 + 3) as usize];
        let k = SIZE as f32 / render::CANVAS;
        // 顶块中心、枢纽竖条中点都在剪影内（允许抗锯齿取样的偏差）
        assert!(at((render::TOP.cx() * k) as u32, (render::TOP.cy() * k) as u32) > 200);
        assert!(at((render::TRUNK.cx() * k) as u32, (render::TRUNK.cy() * k) as u32) > 200);
        // 四角必须留白
        assert_eq!(at(0, 0), 0);
    }
}