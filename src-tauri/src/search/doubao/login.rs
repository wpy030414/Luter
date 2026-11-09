//! 豆包登录：Tauri WebView 打开官方登录页 → 抖音 OAuth → 读 Cookie。
//!
//! 为什么用 WebView 而不是 HTTP 直连：
//! - 登录必须人机交互（扫码 / 短信验证码），且抖音 `redirect_uri` 只白名单豆包官方
//!   callback（localhost 会被判「非法重定向链接」）；
//! - 抖音 OAuth 的 `code` 换票由**豆包页面自己**在浏览器内完成，Node/Rust 重放 code
//!   会返回「非法应用」。因此只能从登录完成后的浏览器上下文取 Cookie。
//!
//! 实现要点：
//! - `WebviewWindow::cookies_for_url("https://www.doubao.com")` 可读到 **HttpOnly**
//!   的 `sessionid`（文档明确包含 HTTP-only 与 secure）；
//! - 登录窗隐藏式轮询：每 1.5s 读一次 cookie，出现 `sessionid` 即成功，关闭窗口；
//! - 全程不打印 cookie 值。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use super::protocol::Device;

/// 登录窗标签（全局唯一，避免重复打开）。
const LOGIN_WINDOW_LABEL: &str = "doubao-login";

/// 登录结果。
#[derive(Debug, Clone)]
pub struct LoginOutcome {
    pub cookie_header: String,
    pub uid: Option<String>,
    pub nickname: Option<String>,
    pub device: Device,
}

/// 轮询间隔与总超时。
const POLL_INTERVAL: Duration = Duration::from_millis(1500);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// 一次登录尝试的共享状态（供前端查询进度）。
#[derive(Debug, Default)]
pub struct LoginState {
    inner: Mutex<LoginInner>,
}

#[derive(Debug, Default)]
struct LoginInner {
    /// `idle` | `pending` | `success` | `failed`
    status: String,
    message: String,
}

impl LoginState {
    pub fn status_text(&self) -> (String, String) {
        let guard = self.inner.lock().ok();
        match guard {
            Some(g) => (
                if g.status.is_empty() {
                    "idle".to_string()
                } else {
                    g.status.clone()
                },
                g.message.clone(),
            ),
            None => ("failed".to_string(), "登录状态锁损坏".to_string()),
        }
    }

    pub fn set(&self, status: &str, message: &str) {
        if let Ok(mut g) = self.inner.lock() {
            g.status = status.to_string();
            g.message = message.to_string();
        }
    }

    pub fn is_pending(&self) -> bool {
        self.inner
            .lock()
            .map(|g| g.status == "pending")
            .unwrap_or(false)
    }
}

/// 打开登录窗并等待用户完成授权，返回凭证。
///
/// 调用方负责把结果加密落盘（见 `auth::save`）。
pub async fn login(app: AppHandle, state: Arc<LoginState>) -> anyhow::Result<LoginOutcome> {
    if state.is_pending() {
        bail!("已有登录窗口在进行中");
    }
    state.set("pending", "已打开豆包登录窗口，请完成扫码或短信验证");

    // 复用已存在的窗口（用户重复点击时）
    if let Some(existing) = app.get_webview_window(LOGIN_WINDOW_LABEL) {
        let _ = existing.close();
    }

    let window = WebviewWindowBuilder::new(
        &app,
        LOGIN_WINDOW_LABEL,
        WebviewUrl::External("https://www.doubao.com/chat/".parse()?),
    )
    .title("登录豆包（完成授权后窗口自动关闭）")
    .inner_size(480.0, 720.0)
    .center()
    .build()
    .map_err(|e| anyhow!("打开登录窗口失败：{e}"))?;

    let result = wait_for_login(&window).await;

    // 无论成败都关闭登录窗
    let _ = window.close();

    match &result {
        Ok(outcome) => {
            state.set(
                "success",
                &format!(
                    "登录成功{}",
                    outcome
                        .nickname
                        .as_deref()
                        .map(|n| format!("（{n}）"))
                        .unwrap_or_default()
                ),
            );
        }
        Err(e) => state.set("failed", &e.to_string()),
    }
    result
}

/// 轮询登录窗 cookie，直到出现 `sessionid` 或超时/窗口被关。
async fn wait_for_login(window: &tauri::WebviewWindow) -> anyhow::Result<LoginOutcome> {
    let deadline = Instant::now() + LOGIN_TIMEOUT;
    let url: url::Url = "https://www.doubao.com".parse().expect("static doubao url");

    loop {
        if Instant::now() >= deadline {
            bail!("登录超时（{}s），请重试", LOGIN_TIMEOUT.as_secs());
        }

        // 窗口被用户手动关闭 → 视为取消
        if window.is_visible().is_err() {
            bail!("登录窗口已关闭");
        }

        match window.cookies_for_url(url.clone()) {
            Ok(cookies) if !cookies.is_empty() => {
                let header = cookies
                    .iter()
                    .map(|c| format!("{}={}", c.name(), c.value()))
                    .collect::<Vec<_>>()
                    .join("; ");
                // sessionid 是登录态的判据
                let has_session = cookies
                    .iter()
                    .any(|c| c.name() == "sessionid" && !c.value().is_empty());
                if has_session {
                    return Ok(LoginOutcome {
                        cookie_header: header,
                        uid: None,
                        nickname: None,
                        device: Device::generate(),
                    });
                }
            }
            _ => {}
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_login_state_transitions() {
        let state = LoginState::default();
        assert_eq!(state.status_text().0, "idle");
        state.set("pending", "等待");
        assert!(state.is_pending());
        state.set("success", "完成");
        assert_eq!(state.status_text().0, "success");
        assert!(!state.is_pending());
    }
}
