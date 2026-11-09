//! 豆包联网搜索引擎。
//!
//! 流程：读取登录凭证 → 构造 `/chat/completion` 请求（Expert 模式）→ 读取 SSE，
//! 找到 `block_type=10025` 即中断上游并返回资料列表。
//!
//! 关键约束（均来自逆向实测，见 xrp-doubao/docs/DOUBAO_REVERSE.md）：
//! - 豆包**没有搜索开关**：是否联网由模型意图判定（`llm_intention=browsing`）。
//!   本引擎用 prompt 引导，未触发时返回空列表而非报错。
//! - 只有标题与 URL，没有正文摘要；`SearchResult.snippet` 置空，
//!   由 `content/` 层按需抓取原文。
//! - 风控（`710022004`）与登录失效（`710012001`）分别映射为可识别的错误文本，
//!   交由上层记日志；不走密钥轮换（搜索不属于 Provider 池）。

pub mod auth;
pub mod login;
pub mod protocol;

use std::collections::HashSet;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use futures::StreamExt;
use serde_json::Value;

use super::SearchResult;

/// 单次搜索超时（上游平均 3-8s，留足余量）。
const SEARCH_TIMEOUT: Duration = Duration::from_secs(45);

/// 运行时依赖。
pub struct DoubaoSearchCtx<'a> {
    pub client: &'a reqwest::Client,
    pub base_url: &'a str,
    pub aid: &'a str,
    pub pc_version: &'a str,
    pub bot_id: &'a str,
    pub model: &'a str,
    pub credential: &'a auth::Credential,
}

/// 已识别的上游错误类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoubaoErrorKind {
    Auth,
    Risk,
    Upstream,
}

/// 豆包搜索错误（便于上层区分「需要重新登录」）。
#[derive(Debug)]
pub struct DoubaoSearchError {
    pub kind: DoubaoErrorKind,
    pub message: String,
    pub code: Option<i64>,
}

impl std::fmt::Display for DoubaoSearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for DoubaoSearchError {}

/// 从 Cookie 头里取指定名字的值。
fn cookie_value(header: &str, name: &str) -> Option<String> {
    for part in header.split(';') {
        let mut kv = part.trim().splitn(2, '=');
        let (k, v) = (kv.next()?, kv.next()?);
        if k.trim() == name {
            let value = v.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn classify(code: Option<i64>, message: &str) -> DoubaoErrorKind {
    const RISK: [i64; 6] = [
        710022001, 710022002, 710022003, 710022004, 710022005, 710022006,
    ];
    const AUTH: [i64; 4] = [710012001, 710012002, 710012003, 710012004];
    match code {
        Some(c) if RISK.contains(&c) => DoubaoErrorKind::Risk,
        Some(c) if AUTH.contains(&c) => DoubaoErrorKind::Auth,
        _ if message.contains("登录") => DoubaoErrorKind::Auth,
        _ => DoubaoErrorKind::Upstream,
    }
}

/// 执行一次搜索，返回命中的资料（可能为空）。
pub async fn search(ctx: &DoubaoSearchCtx<'_>, query: &str) -> anyhow::Result<Vec<SearchResult>> {
    let query = query.trim();
    if query.is_empty() {
        bail!("empty query");
    }
    if ctx.credential.sessionid().is_none() {
        return Err(anyhow!(DoubaoSearchError {
            kind: DoubaoErrorKind::Auth,
            message: "豆包未登录：请在设置页完成登录".into(),
            code: None,
        }));
    }

    let preset = protocol::resolve_preset(ctx.model);
    let prompt = protocol::build_search_prompt(query);
    let body = protocol::build_request_body(&prompt, preset, ctx.bot_id, &ctx.credential.device);

    // URL 参数逐字对齐 xrp-doubao 的 webParams：
    // 缺 fp / web_tab_id / tz_name 会被上游判 common invalid param。
    let mut url = format!(
        "{}{}?aid={}&real_aid={}&device_id={}&tea_uuid={}&web_id={}\
         &device_platform=web&doubao_device_platform=web\
         &pc_version={}&doubao_pc_version={}&pkg_type=release_version\
         &version_code=20800&language=zh&region=CN&sys_region=CN\
         &samantha_web=1&web_platform=browser&use-olympus-account=1\
         &fp={}&web_tab_id={}&tz_name=Asia%2FShanghai",
        ctx.base_url.trim_end_matches('/'),
        crate::search::config::DOUBAO_CHAT_PATH,
        ctx.aid,
        ctx.aid,
        ctx.credential.device.device_id,
        ctx.credential.device.web_id,
        ctx.credential.device.web_id,
        ctx.pc_version,
        ctx.pc_version,
        ctx.credential.device.fp,
        protocol::uuid_like(),
    );
    if let Some(ms_token) = cookie_value(&ctx.credential.cookie_header, "msToken") {
        url.push_str("&msToken=");
        url.push_str(&ms_token);
    }

    let csrf =
        cookie_value(&ctx.credential.cookie_header, "passport_csrf_token").unwrap_or_default();
    let mut request = ctx
        .client
        .post(&url)
        .header("Cookie", &ctx.credential.cookie_header)
        .header("Accept", "text/event-stream")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("Content-Type", "application/json")
        .header("Origin", ctx.base_url.trim_end_matches('/'))
        .header(
            "Referer",
            format!("{}/chat/", ctx.base_url.trim_end_matches('/')),
        )
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/147.0.0.0 Safari/537.36",
        )
        .header("agw-js-conv", "str")
        .header(
            "sec-ch-ua",
            "\"Chromium\";v=\"147\", \"Not.A/Brand\";v=\"8\"",
        )
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", "\"Windows\"");
    if !csrf.is_empty() {
        request = request.header("x-tt-passport-csrf-token", csrf);
    }
    let response = request
        .json(&body)
        .timeout(SEARCH_TIMEOUT)
        .send()
        .await
        .context("豆包搜索请求失败")?;

    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if !status.is_success() || !content_type.contains("event-stream") {
        let text = response.text().await.unwrap_or_default();
        let (code, message) = extract_upstream_error(&text);
        let kind = classify(code, &message);
        return Err(anyhow!(DoubaoSearchError {
            kind,
            message: if message.is_empty() {
                format!("豆包 HTTP {}（非 SSE 响应）", status.as_u16())
            } else {
                message
            },
            code,
        }));
    }

    // 增量读 SSE：按空行分帧，找到 10025 即收尾
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut sources: Vec<SearchResult> = Vec::new();
    let mut search_seen = false;

    'read: while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("读取豆包 SSE 失败")?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // 逐帧处理（SSE 以空行分隔）
        loop {
            let Some(idx) = buffer.find("\n\n") else {
                break;
            };
            let frame = buffer[..idx].to_string();
            buffer.drain(..idx + 2);
            if let Some(err) = handle_frame(&frame, &mut seen, &mut sources, &mut search_seen) {
                return Err(err);
            }
            if search_seen {
                break 'read;
            }
        }
    }

    // 尾部残留帧
    if !search_seen && !buffer.trim().is_empty() {
        if let Some(err) = handle_frame(&buffer, &mut seen, &mut sources, &mut search_seen) {
            return Err(err);
        }
    }

    Ok(sources)
}

/// 处理单帧；返回 `Some(err)` 表示上游报错。
fn handle_frame(
    frame: &str,
    seen: &mut HashSet<String>,
    out: &mut Vec<SearchResult>,
    search_seen: &mut bool,
) -> Option<anyhow::Error> {
    let (event, data) = protocol::parse_sse_event(frame);
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return None;
    }
    let value: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return None,
    };

    // 上游错误事件
    let event_name = event.as_deref().unwrap_or("").to_ascii_lowercase();
    let err_code = value
        .get("error_code")
        .and_then(Value::as_i64)
        .filter(|c| *c != 0);
    if event_name == "gateway-error" || event_name == "stream_error" || err_code.is_some() {
        let code = err_code.or_else(|| value.get("code").and_then(Value::as_i64));
        let message = value
            .get("error_msg")
            .or_else(|| value.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("豆包上游错误")
            .to_string();
        let kind = classify(code, &message);
        return Some(anyhow!(DoubaoSearchError {
            kind,
            message,
            code
        }));
    }

    let (found, batch) = protocol::parse_search_block(&value);
    if found {
        *search_seen = true;
    }
    for source in batch {
        let key = source.doc_id.clone().unwrap_or_else(|| source.url.clone());
        if !seen.insert(key) {
            continue;
        }
        out.push(SearchResult {
            title: source.title,
            url: source.url,
            // 豆包只给标题与链接，摘要交给 content/ 层抓取
            snippet: String::new(),
        });
    }
    None
}

/// 从非 SSE 响应体里尽力提取 `(code, message)`。
fn extract_upstream_error(text: &str) -> (Option<i64>, String) {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        let code = value
            .get("code")
            .or_else(|| value.get("error_code"))
            .and_then(Value::as_i64);
        let message = value
            .get("message")
            .or_else(|| value.get("msg"))
            .or_else(|| value.get("error_msg"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        return (code, message);
    }
    (
        None,
        text.chars()
            .take(200)
            .collect::<String>()
            .trim()
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_classify_errors() {
        assert_eq!(classify(Some(710022004), ""), DoubaoErrorKind::Risk);
        assert_eq!(classify(Some(710012001), ""), DoubaoErrorKind::Auth);
        assert_eq!(classify(None, "登录已过期"), DoubaoErrorKind::Auth);
        assert_eq!(classify(Some(500), "boom"), DoubaoErrorKind::Upstream);
    }

    #[test]
    fn test_handle_frame_collects_and_dedupes() {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        let mut found = false;
        let frame = format!(
            "event: STREAM_CHUNK\ndata: {}",
            json!({
                "patch_op": [{"patch_value": {"content_block": [{
                    "block_type": 10025,
                    "content": {"search_query_result_block": {"results": [
                        {"text_card": {"title": "甲", "url": "https://a.com", "doc_id": "d1", "index": 1}},
                        {"text_card": {"title": "甲重复", "url": "https://a.com/x", "doc_id": "d1"}}
                    ]}}
                }]}}]
            })
        );
        assert!(handle_frame(&frame, &mut seen, &mut out, &mut found).is_none());
        assert!(found);
        assert_eq!(out.len(), 1, "同 doc_id 应去重");
        assert_eq!(out[0].title, "甲");
        assert_eq!(out[0].snippet, "");
    }

    #[test]
    fn test_handle_frame_error_event() {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        let mut found = false;
        let frame = "event: STREAM_ERROR\ndata: {\"error_code\":710022004,\"error_msg\":\"风控\"}";
        let err = handle_frame(frame, &mut seen, &mut out, &mut found).expect("should error");
        let downcast = err.downcast_ref::<DoubaoSearchError>().unwrap();
        assert_eq!(downcast.kind, DoubaoErrorKind::Risk);
        assert_eq!(downcast.code, Some(710022004));
    }

    #[test]
    fn test_extract_upstream_error_json() {
        let (code, msg) = extract_upstream_error("{\"code\":710012001,\"msg\":\"登录失效\"}");
        assert_eq!(code, Some(710012001));
        assert_eq!(msg, "登录失效");
    }
}
