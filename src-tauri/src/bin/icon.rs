//! **仅 macOS** 的图标生成器。Windows / Linux 的 `icon.ico`、`128x128@2x.png`
//! 不在本工具的作用范围内（沿用仓库里现有的文件）。
//!
//! 产物：
//!
//! | 文件 | 内容 | 用途 |
//! |------|------|------|
//! | `icons/icon.icns` | 旧图标内容 + 纯白底，11 档 | macOS 应用图标 |
//! | `icons/tray-template.png` | 48px 纯色剪影（RGB 黑 / A 形状） | macOS 菜单栏模板图标 |
//!
//! 应用图标的**内容完整取自原图标**（`icons/legacy-icon.icns` 的 1024 画布），
//! 只在外层加一张纯白圆角白板并等比居中摆放——不重绘、不换字形。
//! 菜单栏图标同理：按原图配色取「有色块」做剪影，系统再按菜单栏主题着色。
//!
//! 用法：`cargo run --bin icon`（工作目录 `src-tauri/`，仅 macOS 有效）。

use std::path::{Path, PathBuf};

use xrl_router_lib::brand;

/// ICNS 的 (类型码, 边长)：16/32/128/256/512 各含 @2x，共 11 档
/// （icp6 为第三方工具通用的 64px 补充档）。
const ICNS_TYPES: [(&[u8; 4], u32); 11] = [
    (b"icp4", 16),
    (b"icp5", 32),
    (b"icp6", 64),
    (b"ic07", 128),
    (b"ic08", 256),
    (b"ic09", 512),
    (b"ic10", 1024),
    (b"ic11", 32),
    (b"ic12", 64),
    (b"ic13", 256),
    (b"ic14", 512),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let icons = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("icons");
    std::fs::create_dir_all(&icons)?;

    #[cfg(target_os = "macos")]
    {
        let master = macos_master(&icons);
        let mut body = Vec::new();
        for (kind, side) in ICNS_TYPES {
            let png = png_from_rgba(&downscale(&master, MASTER_SIDE, side))?;
            body.extend_from_slice(kind);
            body.extend_from_slice(&(png.len() as u32 + 8).to_be_bytes());
            body.extend_from_slice(&png);
        }
        let mut out = Vec::with_capacity(body.len() + 8);
        out.extend_from_slice(b"icns");
        out.extend_from_slice(&(body.len() as u32 + 8).to_be_bytes());
        out.extend_from_slice(&body);
        write(&icons.join("icon.icns"), &out)?;

        // 菜单栏模板：RGB 全 0，A 承载剪影
        let png = tray_template_png(&icons)?;
        write(&icons.join("tray-template.png"), &png)?;
        println!("icon.icns / tray-template.png 已生成");
    }

    Ok(())
}

/// 原图里「属于结构」的像素：蓝块、浅灰块、深灰蓝块。
///
/// 白底与白色留白不算——这正是让剪影保持镂空的关键判据。
#[cfg(target_os = "macos")]
fn is_structural(r: u8, g: u8, b: u8, a: u8) -> bool {
    if a < 96 {
        return false;
    }
    let blue = b as i16 - r as i16 > 55 && r < 110; // 蓝块 (44,123,212)
    let slate = (75..=135).contains(&r) && (90..=145).contains(&g) && (100..=155).contains(&b); // (100,113,124)
    let light = (185..=245).contains(&r) && (195..=250).contains(&g) && b >= g; // 浅灰 (219,224,226)
    blue || slate || light
}

/// 菜单栏模板图：旧图标结构的**紧凑剪影**（纯黑 + alpha）。
///
/// 不能把原图标整体当剪影——它的白色留白会把结构糊成一整块。正确做法是
/// 按原图配色分类，只保留「有色块」（蓝 / 浅灰 / 深灰蓝），留白与透明都丢弃，
/// 于是竖条、横臂、四块之间的缝隙在剪影里依然分明。
///
/// 颜色由系统按菜单栏主题着色，这里只给形状，像素一律纯黑。
#[cfg(target_os = "macos")]
fn tray_template_png(icons: &Path) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let side = brand::tray::SIZE;
    let src = read_legacy_icon(&icons.join("legacy-icon.icns"))
        .or_else(|| read_legacy_icon(&icons.join("icon.icns")));
    let mut rgba = vec![0u8; (side * side * 4) as usize];
    if let Some(src) = src {
        let (bx0, by0, bx1, by1) = structural_bounds(&src, MASTER_SIDE);
        let cw = (bx1 - bx0 + 1) as f32;
        let ch = (by1 - by0 + 1) as f32;
        if cw > 0.0 && ch > 0.0 {
            let pad = 1.5f32;
            let scale = (side as f32 - pad * 2.0) / cw.max(ch);
            // 目标宽高强行为偶数（画布也是偶数），保证镜像中线落在整数边界
            let dw = ((cw * scale) as u32).next_multiple_of(2);
            let dh = ((ch * scale) as u32).next_multiple_of(2);
            let ox = (side - dw) / 2;
            let oy = (side - dh) / 2;
            for ty in 0..dh {
                let sy = by0 + (ty as f32 / scale) as u32;
                if sy > by1 {
                    continue;
                }
                for tx in 0..dw {
                    let sx = bx0 + (tx as f32 / scale) as u32;
                    if sx > bx1 {
                        continue;
                    }
                    let si = ((sy * MASTER_SIDE + sx) * 4) as usize;
                    let (r, g, b, a) = (src[si], src[si + 1], src[si + 2], src[si + 3]);
                    if !is_structural(r, g, b, a) {
                        continue;
                    }
                    let d = (((oy + ty) * side + ox + tx) * 4) as usize;
                    rgba[d + 3] = rgba[d + 3].max(a);
                }
            }
        }
    }
    mirror_rows(&mut rgba, side);
    png_from_rgba(&rgba)
}

/// 把位图强制成关于画布水平中线严格轴对称。
///
/// 中线在 `side`（偶数）的两行像素之间：第 `side/2-1` 行与第 `side/2` 行互为镜像。
/// 对每一对镜像像素取 alpha 的**并集**（谁不透明就照抄给另一边），
/// 于是无论缩放取整把某个抗锯齿像素舍去了哪一侧，结果都严格对称。
#[cfg(target_os = "macos")]
fn mirror_rows(rgba: &mut [u8], side: u32) {
    let n = side as usize;
    for dy in 0..n / 2 {
        let yu = n / 2 - 1 - dy;
        let yl = n / 2 + dy;
        for x in 0..n {
            let ui = (yu * n + x) * 4 + 3;
            let li = (yl * n + x) * 4 + 3;
            let merged = rgba[ui].max(rgba[li]);
            rgba[ui] = merged;
            rgba[li] = merged;
        }
    }
}

/// macOS 主图的画布边长。
#[cfg(target_os = "macos")]
const MASTER_SIDE: u32 = 1024;

/// macOS 主图 = 旧图标内容 + 纯白圆角白板。
///
/// 旧图标内容**原样保留**（等比居中缩放至白板内），不重绘字形。
/// 若 `icons/icon.icns` 已被本工具覆盖成纯内容，则退化为纯白板。
#[cfg(target_os = "macos")]
fn macos_master(icons: &Path) -> Vec<u8> {
    let mut canvas = brand::render::blank(MASTER_SIDE);
    let mut white = tiny_skia::Paint::default();
    white.set_color(tiny_skia::Color::WHITE);
    white.anti_alias = true;
    canvas.fill_path(
        &brand::render::plate(),
        &white,
        tiny_skia::FillRule::Winding,
        tiny_skia::Transform::identity(),
        None,
    );
    let mut out = canvas.take();
    // 内容源：优先读 legacy-icon.icns（原始素材的只读副本），
    // 其次读当前的 icon.icns（首次运行前它还是旧图标）。
    let src = read_legacy_icon(&icons.join("legacy-icon.icns"))
        .or_else(|| read_legacy_icon(&icons.join("icon.icns")));
    if let Some(src) = src {
        composite_centered(&mut out, MASTER_SIDE, &src, MASTER_SIDE);
    } else {
        println!("警告：未找到可用的原始图标，输出为纯白板");
    }
    out
}

/// 读旧 `icon.icns` 里最大的一帧（通常是 1024），返回未预乘 RGBA。
///
/// 只解析 ICNS 容器里的 PNG 帧；找不到返回 `None`。
#[cfg(target_os = "macos")]
fn read_legacy_icon(path: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 8 || &bytes[0..4] != b"icns" {
        return None;
    }
    let mut best: Option<(u32, Vec<u8>)> = None;
    let mut pos = 8usize;
    while pos + 8 <= bytes.len() {
        let kind = &bytes[pos..pos + 4];
        let len = u32::from_be_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]]) as usize;
        if len < 8 || pos + len > bytes.len() {
            break;
        }
        let payload = &bytes[pos + 8..pos + len];
        if payload.starts_with(&[0x89, b'P', b'N', b'G']) {
            if let Some((side, rgba)) = decode_png(payload) {
                if best.as_ref().is_none_or(|(b, _)| side > *b) {
                    best = Some((side, rgba));
                }
            }
        }
        let _ = kind;
        pos += len;
    }
    best.map(|(_, rgba)| rgba)
}

/// 迷你 PNG 解码器（8 位 RGBA / RGB / 灰阶，非隔行）。
///
/// 只为读回自己的 `icon.icns` 写一版最小实现，避免为构建脚本引入 `image` 依赖
/// （那会进正式构建的依赖树）。帧都是本工具自己写的 RGBA，带一般性的健壮分支。
#[cfg(target_os = "macos")]
fn decode_png(data: &[u8]) -> Option<(u32, Vec<u8>)> {
    let mut pos = 8usize;
    let mut w = 0u32;
    let mut h = 0u32;
    let mut color_type = 0u8;
    let mut bit_depth = 0u8;
    let mut idat: Vec<u8> = Vec::new();
    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        let kind = &data[pos + 4..pos + 8];
        let body = data.get(pos + 8..pos + 8 + len)?;
        match kind {
            b"IHDR" => {
                w = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
                h = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
                bit_depth = body[8];
                color_type = body[9];
                if body[12] != 0 {
                    return None; // 隔行不支持
                }
            }
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }
    if bit_depth != 8 || w == 0 || h == 0 {
        return None;
    }
    let channels = match color_type {
        6 => 4,
        2 => 3,
        0 => 1,
        _ => return None,
    };
    let raw = inflate(&idat)?;
    let bpp = channels as usize;
    let stride = w as usize * bpp;
    let mut prev = vec![0u8; stride];
    let mut cur = vec![0u8; stride];
    let mut out = vec![0u8; (w * h * 4) as usize];
    let mut p = 0usize;
    for y in 0..h as usize {
        let filter = raw[p];
        p += 1;
        cur.copy_from_slice(&raw[p..p + stride]);
        p += stride;
        for i in 0..stride {
            let a = if i >= bpp { cur[i - bpp] } else { 0 };
            let b = prev[i];
            let c = if i >= bpp { prev[i - bpp] } else { 0 };
            let v = match filter {
                0 => cur[i],
                1 => cur[i].wrapping_add(a),
                2 => cur[i].wrapping_add(b),
                3 => cur[i].wrapping_add(((a as u16 + b as u16) / 2) as u8),
                4 => cur[i].wrapping_add(paeth(a, b, c)),
                _ => return None,
            };
            cur[i] = v;
        }
        for x in 0..w as usize {
            let s = x * bpp;
            let d = (y * w as usize + x) * 4;
            match channels {
                4 => out[d..d + 4].copy_from_slice(&cur[s..s + 4]),
                3 => {
                    out[d] = cur[s];
                    out[d + 1] = cur[s + 1];
                    out[d + 2] = cur[s + 2];
                    out[d + 3] = 255;
                }
                _ => {
                    out[d] = cur[s];
                    out[d + 1] = cur[s];
                    out[d + 2] = cur[s];
                    out[d + 3] = 255;
                }
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    Some((w, out))
}

#[cfg(target_os = "macos")]
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let pa = (p - a as i16).abs();
    let pb = (p - b as i16).abs();
    let pc = (p - c as i16).abs();
    if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
}

/// zlib 解压（PNG 的 IDAT 是 zlib 流）。
#[cfg(target_os = "macos")]
fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data).read_to_end(&mut out).ok()?;
    Some(out)
}

/// 图标内容占白板边长的比例（越大越满，越小越"内缩"）。
///
/// 0.68 是观感边界：再大图标在 Dock 里比邻居显胖，再小内容细节就糊了。
#[cfg(target_os = "macos")]
const CONTENT_RATIO: f32 = 0.68;

/// 把 `src`（边长 `src_side`）等比居中缩放进 `dst`，越界处保持 `dst` 原样。
#[cfg(target_os = "macos")]
fn composite_centered(dst: &mut [u8], dst_side: u32, src: &[u8], src_side: u32) {
    // 旧图内容并不充满画布（实测 bbox 833×854），先求出有效内容框
    let (bx0, by0, bx1, by1) = content_bounds(src, src_side);
    let cw = (bx1 - bx0 + 1) as f32;
    let ch = (by1 - by0 + 1) as f32;
    if cw <= 0.0 || ch <= 0.0 {
        return;
    }
    // macOS 图标网格：内容占白板约 72%。
    // 苹果官方的图标网格留白明显大于满幅（圆角板本身已向内收了一圈，
    // 内容再贴边就会显得整块图标「太大」、在 Dock 里比邻居胖一号）。
    let target = dst_side as f32 * CONTENT_RATIO;
    let scale = target / cw.max(ch);
    let dw = (cw * scale) as u32;
    let dh = (ch * scale) as u32;
    let ox = (dst_side - dw) / 2;
    let oy = (dst_side - dh) / 2;
    for ty in 0..dh {
        let sy = by0 + (ty as f32 / scale) as u32;
        if sy > by1 {
            continue;
        }
        for tx in 0..dw {
            let sx = bx0 + (tx as f32 / scale) as u32;
            if sx > bx1 {
                continue;
            }
            let si = ((sy * src_side + sx) * 4) as usize;
            let a1 = src[si + 3] as f32 / 255.0;
            if a1 <= 0.0 {
                continue;
            }
            let di = (((oy + ty) * dst_side + ox + tx) * 4) as usize;
            let a0 = dst[di + 3] as f32 / 255.0;
            let ao = a1 + a0 * (1.0 - a1);
            for k in 0..3 {
                dst[di + k] = ((src[si + k] as f32 * a1 + dst[di + k] as f32 * a0 * (1.0 - a1)) / ao) as u8;
            }
            dst[di + 3] = (ao * 255.0) as u8;
        }
    }
}

/// 结构像素的外接框（用于把剪影缩放撑满）。
#[cfg(target_os = "macos")]
fn structural_bounds(px: &[u8], side: u32) -> (u32, u32, u32, u32) {
    let (mut x0, mut y0, mut x1, mut y1) = (side, side, 0, 0);
    for y in 0..side {
        for x in 0..side {
            let i = ((y * side + x) * 4) as usize;
            if is_structural(px[i], px[i + 1], px[i + 2], px[i + 3]) {
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    (x0, y0, x1, y1)
}

/// 不透明内容的外接框（含抗锯齿的半透明边缘）。
#[cfg(target_os = "macos")]
fn content_bounds(px: &[u8], side: u32) -> (u32, u32, u32, u32) {
    let (mut x0, mut y0, mut x1, mut y1) = (side, side, 0, 0);
    for y in 0..side {
        for x in 0..side {
            if px[((y * side + x) * 4 + 3) as usize] > 8 {
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    (x0, y0, x1, y1)
}

fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)?;
    println!("  {} ({} 字节)", path.display(), bytes.len());
    Ok(())
}

/// 面积平均降采样：RGBA 在 alpha 上加权（透明像素不拉低颜色）。
///
/// 主图画布本身带预乘语义之外的干净色值，所以这里按「非预乘 + alpha 加权」平均，
/// 4 倍以上整数缩小（1024→256/128…）不会出现暗边。
fn downscale(src: &[u8], src_side: u32, dst_side: u32) -> Vec<u8> {
    if src_side == dst_side {
        return src.to_vec();
    }
    let ratio = src_side / dst_side;
    let mut out = vec![0u8; (dst_side * dst_side * 4) as usize];
    for dy in 0..dst_side {
        for dx in 0..dst_side {
            let (mut r, mut g, mut b, mut a) = (0f32, 0f32, 0f32, 0f32);
            for sy in 0..ratio {
                for sx in 0..ratio {
                    let i = ((((dy * ratio + sy) * src_side) + (dx * ratio + sx)) * 4) as usize;
                    let w = src[i + 3] as f32;
                    r += src[i] as f32 * w;
                    g += src[i + 1] as f32 * w;
                    b += src[i + 2] as f32 * w;
                    a += w;
                }
            }
            let n = (ratio * ratio) as f32;
            let o = (((dy * dst_side) + dx) * 4) as usize;
            if a > 0.0 {
                out[o] = (r / a) as u8;
                out[o + 1] = (g / a) as u8;
                out[o + 2] = (b / a) as u8;
            }
            out[o + 3] = (a / n).round() as u8;
        }
    }
    out
}

/// RGBA（未预乘）→ PNG 字节。
fn png_from_rgba(rgba: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // tiny-skia 的编码器要求预乘 alpha
    let mut buf = rgba.to_vec();
    brand::render::premultiply(&mut buf);
    let side = ((buf.len() / 4) as f64).sqrt() as u32;
    let pixmap = tiny_skia::Pixmap::from_vec(buf, tiny_skia::IntSize::from_wh(side, side).unwrap())
        .ok_or("构造 pixmap 失败")?;
    Ok(pixmap.encode_png()?)
}
