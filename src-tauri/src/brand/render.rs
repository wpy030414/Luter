//! 品牌图形：几何 + 光栅原语（tiny-skia）。
//!
//! 坐标系 = **原始图标的 1024 画布**，所有几何常量直接写成原图实测的像素值，
//! 因此「这段代码对应原图哪一块」可以一眼对上（见下表）。
//!
//! 原图标（旧 `icon.icns`）的结构，按原图像素实测：
//!
//! | 区域 | 原图范围 (x0,y0,x1,y1) | 原色 |
//! |------|------------------------|------|
//! | 左侧长臂 | 85, 359, 384, 661 | 蓝 (44,123,212) |
//! | 顶块 | 704, 85, 917, 291 | 蓝 |
//! | 中块 | 704, 405, 917, 619 | 浅灰 (219,224,226) |
//! | 底块 | 704, 730, 917, 938 | 蓝 |
//! | 枢纽竖条 | 512, 170, 554, 852 | 深灰蓝 (100,113,124) |
//! | 核心圆环 | 圆心 (512,512)，外径 130 / 内径 84 | 深灰蓝 |
//!
//! 本文件只改变**材质**（实色 → Liquid Glass），布局、尺寸、位置一个不动。

use tiny_skia::{
    BlendMode, Color, FillRule, GradientStop, LinearGradient, Paint, Path, PathBuilder, Pixmap,
    PixmapPaint, Point, Rect, Shader, SpreadMode, Transform,
};

// ── 原图坐标系 ──

/// 原始图标画布边长。
pub const CANVAS: f32 = 1024.0;
/// 外框圆角（原图 squircle 常量）。
pub const CORNER: f32 = 224.0;

/// 原图里的一个色块（原始像素坐标）。
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub radius: f32,
}

impl Region {
    pub fn cx(&self) -> f32 { (self.x0 + self.x1) / 2.0 }
    pub fn cy(&self) -> f32 { (self.y0 + self.y1) / 2.0 }
    pub fn w(&self) -> f32 { self.x1 - self.x0 }
    pub fn h(&self) -> f32 { self.y1 - self.y0 }

    /// 圆角矩形路径（原始像素坐标，无需缩放）。
    pub fn path(&self) -> Path {
        rounded_rect(self.cx(), self.cy(), self.w(), self.h(), self.radius).expect("色块路径")
    }
}

/// 左侧长臂（原图 x 85–384 / y 359–661）。
pub const ARM: Region = Region { x0: 85.0, y0: 359.0, x1: 384.0, y1: 661.0, radius: 0.0 };
/// 顶块。
pub const TOP: Region = Region { x0: 704.0, y0: 85.0, x1: 917.0, y1: 291.0, radius: 0.0 };
/// 中块（原图浅灰）。
pub const MID: Region = Region { x0: 704.0, y0: 405.0, x1: 917.0, y1: 619.0, radius: 0.0 };
/// 底块。
pub const BOTTOM: Region = Region { x0: 704.0, y0: 730.0, x1: 917.0, y1: 938.0, radius: 0.0 };
/// 枢纽竖条。
pub const TRUNK: Region = Region { x0: 512.0, y0: 170.0, x1: 554.0, y1: 852.0, radius: 21.0 };

/// 全部矩形色块，供统一遍历。
pub fn regions() -> [Region; 5] {
    [ARM, TOP, MID, BOTTOM, TRUNK]
}

// ── 调色（Liquid Glass 化，色相全部从原色派生） ──

/// 原图蓝 (44,123,212) → 玻璃蓝：提亮、降饱和，保留色相。
pub const GLASS_BLUE: (u8, u8, u8, f32) = (64, 148, 242, 0.62);
pub const GLASS_BLUE_HI: (u8, u8, u8, f32) = (128, 190, 250, 0.86);
/// 玻璃蓝的内高光 / 边光（近白，带一丝青）。
pub const GLASS_BLUE_LIGHT: (u8, u8, u8, f32) = (232, 246, 255, 0.62);
/// 原图深灰蓝 (100,113,124) → 玻璃石板色。
pub const GLASS_SLATE: (u8, u8, u8, f32) = (86, 104, 130, 0.66);
pub const GLASS_SLATE_HI: (u8, u8, u8, f32) = (140, 162, 190, 0.84);
pub const GLASS_SLATE_LIGHT: (u8, u8, u8, f32) = (208, 222, 238, 0.58);
/// 原图浅灰 (219,224,226) → 玻璃冷白。
pub const GLASS_WHITE: (u8, u8, u8, f32) = (214, 226, 238, 0.80);
pub const GLASS_WHITE_HI: (u8, u8, u8, f32) = (245, 250, 255, 0.95);
pub const GLASS_WHITE_LIGHT: (u8, u8, u8, f32) = (255, 255, 255, 0.85);
/// 投影。
pub const SHADOW: (u8, u8, u8, f32) = (26, 52, 96, 0.46);
/// 模板图标（菜单栏）像素本体。
pub const TEMPLATE: (u8, u8, u8) = (0, 0, 0);

/// 构造 sRGB 颜色。
pub fn rgba(c: (u8, u8, u8, f32)) -> Color {
    Color::from_rgba(c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0, c.3)
        .expect("色值在合法范围内")
}

// ── 路径 ──

/// 圆角矩形路径（原始像素坐标；四角标准 0.5523 贝塞尔系数）。
pub fn rounded_rect(cx: f32, cy: f32, w: f32, h: f32, radius: f32) -> Option<Path> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let r = radius.clamp(0.0, hw.min(hh));
    let (l, t, rt, b) = (cx - hw, cy - hh, cx + hw, cy + hh);
    if r <= 0.0 {
        return Some(PathBuilder::from_rect(Rect::from_ltrb(l, t, rt, b)?));
    }
    let k = r * 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(l + r, t);
    pb.line_to(rt - r, t);
    pb.cubic_to(rt - r + k, t, rt, t + r - k, rt, t + r);
    pb.line_to(rt, b - r);
    pb.cubic_to(rt, b - r + k, rt - r + k, b, rt - r, b);
    pb.line_to(l + r, b);
    pb.cubic_to(l + r - k, b, l, b - r + k, l, b - r);
    pb.line_to(l, t + r);
    pb.cubic_to(l, t + r - k, l + r - k, t, l + r, t);
    pb.close();
    pb.finish()
}

/// 外框路径（满幅圆角方形）。
pub fn plate() -> Path {
    rounded_rect(CANVAS / 2.0, CANVAS / 2.0, CANVAS, CANVAS, CORNER).expect("外框路径")
}

// ── 光栅原语 ──

/// 空白图层。
pub fn blank(size: u32) -> Pixmap {
    let mut p = Pixmap::new(size, size).expect("分配图层");
    p.fill(Color::TRANSPARENT);
    p
}

/// 叠加图层（SourceOver）。
fn over(dst: &mut Pixmap, src: &Pixmap) {
    dst.draw_pixmap(0, 0, src.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
}

/// 叠加图层（指定混合模式）。
fn over_with(dst: &mut Pixmap, src: &Pixmap, mode: BlendMode) {
    dst.draw_pixmap(
        0,
        0,
        src.as_ref(),
        &PixmapPaint { opacity: 1.0, blend_mode: mode, ..Default::default() },
        Transform::identity(),
        None,
    );
}

fn fill_all(layer: &mut Pixmap, shader: Shader<'_>, mode: BlendMode) {
    let s = layer.width() as f32;
    let rect = Rect::from_xywh(0.0, 0.0, s, s).expect("整幅矩形");
    let paint = Paint { shader, blend_mode: mode, anti_alias: true, ..Default::default() };
    layer.fill_rect(rect, &paint, Transform::identity(), None);
}

/// 纯色实心填充。
pub fn fill_solid(dst: &mut Pixmap, path: &Path, c: Color) {
    let mut paint = Paint::default();
    paint.set_color(c);
    paint.anti_alias = true;
    dst.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn fill_vertical(dst: &mut Pixmap, path: &Path, y0: f32, y1: f32, stops: &[(f32, Color)], rule: FillRule) {
    let shader = LinearGradient::new(
        Point::from_xy(0.0, y0),
        Point::from_xy(0.0, y1),
        stops.iter().map(|(p, c)| GradientStop::new(*p, *c)).collect(),
        SpreadMode::Pad,
        Transform::identity(),
    )
    .expect("竖直渐变");
    let paint = Paint { shader, anti_alias: true, ..Default::default() };
    dst.fill_path(path, &paint, rule, Transform::identity(), None);
}

fn ramp_alpha_linear(layer: &mut Pixmap, from: (f32, f32), to: (f32, f32), stops: &[(f32, f32)]) {
    let shader = LinearGradient::new(
        Point::from_xy(from.0, from.1),
        Point::from_xy(to.0, to.1),
        stops
            .iter()
            .map(|(p, a)| GradientStop::new(*p, Color::from_rgba(1.0, 1.0, 1.0, *a).expect("α")))
            .collect(),
        SpreadMode::Pad,
        Transform::identity(),
    )
    .expect("α 渐变");
    fill_all(layer, shader, BlendMode::SourceIn);
}

/// 用 `c` 替换图层颜色、保留 α 形状。
fn tint(shape: &Pixmap, c: Color) -> Pixmap {
    let s = shape.width() as f32;
    let mut out = blank(shape.width());
    let mut paint = Paint::default();
    paint.set_color(c);
    paint.anti_alias = true;
    out.fill_rect(Rect::from_xywh(0.0, 0.0, s, s).expect("矩形"), &paint, Transform::identity(), None);
    over_with(&mut out, shape, BlendMode::DestinationIn);
    out
}

/// 柔和投影图层：形状 α → 上淡下浓 → 下移 → 染成阴影色。
fn shadow_of(path: &Path, size: u32, rule: FillRule) -> Pixmap {
    let mut shape = blank(size);
    let mut paint = Paint::default();
    paint.set_color(Color::WHITE);
    paint.anti_alias = true;
    shape.fill_path(path, &paint, rule, Transform::identity(), None);
    ramp_alpha_linear(&mut shape, (0.0, 0.0), (0.0, CANVAS), &[(0.0, 0.0), (1.0, 1.0)]);
    let mut out = blank(size);
    out.draw_pixmap(0, 0, shape.as_ref(), &PixmapPaint::default(), Transform::from_translate(0.0, 26.0), None);
    tint(&out, rgba(SHADOW))
}

// ── 色块 → 玻璃 ──

/// 单个色块的玻璃图层。
///
/// Liquid Glass 在白底上的可读信息只有三层：**底色梯度 / 贴边内高光 / 锐利边光**。
/// 全部用极窄的色带实现——柔和的宽渐变会把它画成水彩，而不是玻璃。
fn glass_region(region: &Region, base: Color, hi: Color, light: Color, size: u32) -> Pixmap {
    let path = region.path();
    let mut layer = blank(size);
    let (y0, y1) = (region.y0, region.y1);
    let h = region.h();

    // ① 底色：顶亮 → 本品色 → 底部深一档（玻璃厚度的体积感）
    fill_vertical(
        &mut layer,
        &path,
        y0,
        y1,
        &[
            (0.0, hi),
            (0.10, base),
            (0.76, base),
            (1.0, darken(base, 0.74)),
        ],
        FillRule::Winding,
    );

    // ② 顶面反光：紧贴上缘的一条细亮线（只 3~6px，柔和的下沿过渡）
    let band_h = 2.0f32.max(h * 0.012);
    let mut sheen = blank(size);
    if let Some(band) = rounded_rect(region.cx(), y0 + band_h, region.w(), band_h * 2.0, region.radius) {
        fill_solid(&mut sheen, &band, light);
        ramp_alpha_linear(
            &mut sheen,
            (0.0, y0),
            (0.0, y0 + band_h * 2.0),
            &[(0.0, 0.35), (0.35, 1.0), (1.0, 0.0)],
        );
        over(&mut layer, &clip(&sheen, &path, size));
    }

    // ③ 轮廓：一圈极淡的冷白描边（玻璃与白底的接缝），上亮下暗
    if let Some(inner) = inset_path(region, RIM_WIDTH) {
        let mut outline = blank(size);
        fill_solid(&mut outline, &path, light);
        let mut cut = blank(size);
        fill_solid(&mut cut, &inner, Color::WHITE);
        over_with(&mut outline, &cut, BlendMode::DestinationOut);
        ramp_alpha_linear(&mut outline, (0.0, y0), (0.0, y1), &[(0.0, 0.85), (0.5, 0.40), (1.0, 0.18)]);
        over(&mut layer, &outline);
    }

    layer
}

/// 用路径裁剪图层（只保留路径内的像素）。
fn clip(src: &Pixmap, path: &Path, size: u32) -> Pixmap {
    let mut mask = blank(size);
    fill_solid(&mut mask, path, Color::WHITE);
    let mut out = blank(size);
    over(&mut out, src);
    over_with(&mut out, &mask, BlendMode::DestinationIn);
    out
}

/// 边光宽度（原图像素）。
const RIM_WIDTH: f32 = 4.0;

/// 色块内缩 `d` 后的路径（用于抠出边光的内圈）。
fn inset_path(region: &Region, d: f32) -> Option<Path> {
    let w = region.w() - d * 2.0;
    let h = region.h() - d * 2.0;
    if w <= 1.0 || h <= 1.0 {
        return None;
    }
    rounded_rect(region.cx(), region.cy(), w, h, (region.radius - d).max(0.0))
}

/// 把颜色整体压暗 `k` 倍（0..1）。
fn darken(c: Color, k: f32) -> Color {
    Color::from_rgba(c.red() * k, c.green() * k, c.blue() * k, c.alpha()).expect("压暗色")
}

/// 彩色图形整体叠到 `canvas` 上（白底 / 透明底由平台决定）。
pub fn paint_foreground(canvas: &mut Pixmap) {
    let size = canvas.width();

    // 2. 投影：落在白底上，只保留露在色块之外的部分
    let mut shadow = blank(size);
    for r in regions() {
        over(&mut shadow, &shadow_of(&r.path(), size, FillRule::Winding));
    }
    // 交点与臂的投影随后被色块盖住，不必单独处理
    let mut reveal = blank(size);
    for r in regions() {
        fill_solid(&mut reveal, &r.path(), Color::WHITE);
    }
    over_with(&mut shadow, &reveal, BlendMode::DestinationOut);
    over(canvas, &shadow);

    // 3. 色块（玻璃材质）
    over(canvas, &glass_region(&ARM, rgba(GLASS_BLUE), rgba(GLASS_BLUE_HI), rgba(GLASS_BLUE_LIGHT), size));
    over(canvas, &glass_region(&TOP, rgba(GLASS_BLUE), rgba(GLASS_BLUE_HI), rgba(GLASS_BLUE_LIGHT), size));
    over(canvas, &glass_region(&MID, rgba(GLASS_WHITE), rgba(GLASS_WHITE_HI), rgba(GLASS_WHITE_LIGHT), size));
    over(canvas, &glass_region(&BOTTOM, rgba(GLASS_BLUE), rgba(GLASS_BLUE_HI), rgba(GLASS_BLUE_LIGHT), size));
    over(canvas, &glass_region(&TRUNK, rgba(GLASS_SLATE), rgba(GLASS_SLATE_HI), rgba(GLASS_SLATE_LIGHT), size));
}

/// 纯色剪影（菜单栏模板）：全部色块，单色不透明。
pub fn silhouette(size: u32, c: (u8, u8, u8)) -> Pixmap {
    let mut out = blank(size);
    let color = Color::from_rgba8(c.0, c.1, c.2, 255);
    // 几何以 1024 原图坐标写出，输出按画布缩放
    let scale = size as f32 / CANVAS;
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    let tf = Transform::from_scale(scale, scale);
    for r in regions() {
        out.fill_path(&r.path(), &paint, FillRule::Winding, tf, None);
    }
    out
}

/// 预乘 RGBA（tiny-skia 的输出格式；平台图标编码器要求预乘）。
pub fn premultiply(buf: &mut [u8]) {
    for c in buf.chunks_exact_mut(4) {
        let a = c[3] as f32 / 255.0;
        c[0] = (c[0] as f32 * a) as u8;
        c[1] = (c[1] as f32 * a) as u8;
        c[2] = (c[2] as f32 * a) as u8;
    }
}