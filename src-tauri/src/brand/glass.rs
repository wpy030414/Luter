//! macOS 图标：纯白底。
//!
//! 画布 1024×1024，白色圆角方板（squircle 圆角 224），**不绘制任何色块**。
//! 几何、玻璃材质仍完整保留在 `render` 里（托盘模板图标在用），
//! 应用图标只是暂时不取用那部分。

use tiny_skia::{Color, FillRule, Paint, Pixmap, Transform};

use super::render;

/// 图标画布边长（与 `render::CANVAS` 一致）。
pub const SIZE: u32 = render::CANVAS as u32;

/// 渲染整张图标，返回 RGBA 字节（行优先、**未预乘**，由编码器预处理）。
pub fn render_icon() -> Vec<u8> {
    let mut canvas = Pixmap::new(SIZE, SIZE).expect("图标画布");

    // 纯白底（圆角方板）
    let mut white = Paint::default();
    white.set_color(Color::WHITE);
    white.anti_alias = true;
    canvas.fill_path(&render::plate(), &white, FillRule::Winding, Transform::identity(), None);

    canvas.take()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_transparent_body_pure_white() {
        let px = render_icon();
        let at = |x: usize, y: usize| {
            let i = (y * SIZE as usize + x) * 4;
            [px[i], px[i + 1], px[i + 2], px[i + 3]]
        };
        // 四角透明（圆角外）
        assert_eq!(at(0, 0)[3], 0);
        assert_eq!(at(SIZE as usize - 1, 0)[3], 0);
        // 板内每个不透明像素都是纯白（不画任何色块）
        for c in px.chunks_exact(4).filter(|c| c[3] == 255) {
            assert_eq!((c[0], c[1], c[2]), (255, 255, 255));
        }
        assert_eq!(at(300, 120), [255, 255, 255, 255]);
        assert_eq!(at(SIZE as usize / 2, SIZE as usize / 2), [255, 255, 255, 255]);
    }
}