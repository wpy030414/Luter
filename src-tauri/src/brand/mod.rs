//! 品牌图形（Luter 的 macOS 图标资源）。
//!
//! 几何与光栅原语在 `render`；各平台文件只决定「底版长什么样」。
//!
//! | 消费方 | 产物 | 平台 |
//! |--------|------|------|
//! | `icon`（bin，构建期） | `icons/icon.icns`（旧图标 + 纯白底） | macOS |
//! | `icon`（bin，构建期） | `icons/tray-template.png`（纯色剪影） | macOS |
//! | `tray`（运行期） | 菜单栏模板位图（预乘 RGBA） | macOS |
//!
//! Windows / Linux 的 `icon.ico`、`128x128@2x.png` 不在本模块范围内。

pub mod render;

#[cfg(target_os = "macos")]
pub mod glass;
#[cfg(target_os = "macos")]
pub mod tray;

#[cfg(target_os = "macos")]
pub use glass::render_icon;
