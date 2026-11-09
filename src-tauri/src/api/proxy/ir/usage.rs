//! IR usage 提取 — 从三种上游格式的流式 chunk 中提取 token usage。
//!
//! 每个 `extract_xxx_usage` 函数从单个 chunk 中提取增量 usage，
//! 由调用方累积到 `IrUsage` 中。

use serde_json::Value;

use super::types::{IrRequest, IrSystemContent, IrUsage};

/// 从 Anthropic Messages 流式 chunk 提取 usage 增量。
///
/// - `message_start`: input_tokens（未缓存口径，cache_creation 单独存放）
///   + cache_creation + cache_read
/// - `message_delta`: output_tokens + cache_read
/// - `content_block_delta`: output_chars（text/thinking 字符数，回退估算用）
#[cfg(test)]
pub fn extract_messages_usage(chunk: &Value) -> IrUsage {
    let event_type = chunk["type"].as_str().unwrap_or("");
    let mut usage = IrUsage::default();

    match event_type {
        "message_start" => {
            let u = &chunk["message"]["usage"];
            usage.input_tokens = u["input_tokens"].as_u64().unwrap_or(0);
            usage.cache_read_input_tokens =
                u["cache_read_input_tokens"].as_u64().unwrap_or(0);
            usage.cache_creation_input_tokens =
                u["cache_creation_input_tokens"].as_u64().unwrap_or(0);
        }
        "message_delta" => {
            let u = &chunk["usage"];
            usage.output_tokens = u["output_tokens"].as_u64().unwrap_or(0);
            usage.cache_read_input_tokens =
                u["cache_read_input_tokens"].as_u64().unwrap_or(0);
        }
        "content_block_delta" => {
            let delta = &chunk["delta"];
            let chars = match delta["type"].as_str() {
                Some("text_delta") => delta["text"]
                    .as_str()
                    .map(|s| s.chars().count() as u64)
                    .unwrap_or(0),
                Some("thinking_delta") => delta["thinking"]
                    .as_str()
                    .map(|s| s.chars().count() as u64)
                    .unwrap_or(0),
                _ => 0,
            };
            usage.output_chars = chars;
        }
        _ => {}
    }

    usage
}

/// 从 OpenAI Chat Completions 流式 chunk 提取 usage 增量。
///
/// OpenAI 的 usage 在最后一个 chunk 中给出（需 `stream_options.include_usage`）。
/// 支持多种缓存字段：
/// - `prompt_cache_hit_tokens`（DeepSeek/Kimi）
/// - `prompt_tokens_details.cached_tokens`（OpenAI 标准）
/// - `cache_read_input_tokens`（部分兼容上游）
pub fn extract_chat_completions_usage(chunk: &Value) -> IrUsage {
    let mut usage = IrUsage::default();

    if let Some(u) = chunk.get("usage") {
        if let Some(pt) = u.get("prompt_tokens").and_then(|v| v.as_u64()) {
            // 缓存命中量可能来自三种字段，必须先剔除 0 值再合并：
            // 部分网关（如 WorkBuddy）同时返回 `prompt_cache_hit_tokens: 0` 与
            // `prompt_tokens_details.cached_tokens: N`，`Some(0)` 会让 or_else 短路。
            let hit = u
                .get("prompt_cache_hit_tokens")
                .and_then(|v| v.as_u64())
                .filter(|v| *v > 0)
                .or_else(|| {
                    u.get("prompt_tokens_details")
                        .and_then(|d| d.get("cached_tokens"))
                        .and_then(|v| v.as_u64())
                        .filter(|v| *v > 0)
                });
            // 上游自报的 cache_read：部分网关恒为 0，不能无条件采用（会覆盖命中量）
            let reported_read = u
                .get("cache_read_input_tokens")
                .and_then(|v| v.as_u64())
                .filter(|v| *v > 0);
            let cache_read = hit.unwrap_or(0).max(reported_read.unwrap_or(0));

            if cache_read > 0 {
                usage.cache_read_input_tokens = cache_read;
                // 没有 miss 字段时按 prompt_tokens - 命中量拆分，
                // 保证 prompt_tokens 始终是「未命中」的互斥口径（UI 分母 = 未命中 + 命中）
                usage.input_tokens = match u
                    .get("prompt_cache_miss_tokens")
                    .and_then(|v| v.as_u64())
                {
                    Some(miss) => miss,
                    None => pt.saturating_sub(cache_read),
                };
            } else {
                usage.input_tokens = pt;
            }
        }
        if let Some(ct) = u.get("completion_tokens").and_then(|v| v.as_u64()) {
            usage.output_tokens = ct;
        }
        // 缓存写入量（Anthropic 的 cache_creation；网关也用它或 prompt_cache_write_tokens 表示）
        if let Some(cc) = u
            .get("cache_creation_input_tokens")
            .and_then(|v| v.as_u64())
            .filter(|v| *v > 0)
            .or_else(|| {
                u.get("prompt_cache_write_tokens")
                    .and_then(|v| v.as_u64())
                    .filter(|v| *v > 0)
            })
        {
            usage.cache_creation_input_tokens = cc;
        }
    }

    // 字符数回退估算
    if let Some(content) = chunk
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("delta"))
        .and_then(|d| d.get("content"))
        .and_then(|c| c.as_str())
    {
        usage.output_chars = content.chars().count() as u64;
    }

    usage
}

/// 从 OpenAI Responses 流式 chunk 提取 usage 增量。
///
/// Responses API 的 usage 在 `response.completed` 事件中给出。
pub fn extract_responses_usage(chunk: &Value) -> IrUsage {
    let mut usage = IrUsage::default();

    // response.completed 事件携带 usage
    if chunk["type"].as_str() == Some("response.completed") {
        if let Some(u) = chunk.get("response").and_then(|r| r.get("usage")) {
            usage.input_tokens = u
                .get("input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            usage.output_tokens = u
                .get("output_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            // Responses API 缓存字段
            if let Some(details) = u.get("input_tokens_details") {
                usage.cache_read_input_tokens = details
                    .get("cached_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                // 上游 input_tokens 含缓存命中部分，减去保持增量口径
                // （与 Chat Completions 的 prompt_tokens - cached_tokens 一致）
                usage.input_tokens = usage
                    .input_tokens
                    .saturating_sub(usage.cache_read_input_tokens);
            }
        }
    }

    // 字符数回退估算
    if let Some(delta) = chunk.get("delta").and_then(|d| d.as_str()) {
        usage.output_chars = delta.chars().count() as u64;
    }

    usage
}

/// 粗估 IR 请求的输入 token 数（chars / 4），用于 message_start 占位。
///
/// 估算口径：system + messages 的文本字符数 / 4（粗略，仅占位用）。
/// 至少返回 1，避免占位为 0。
pub fn estimate_input_tokens(req: &IrRequest) -> u64 {
    let mut chars: usize = 0;

    // System prompt
    if let Some(ref system) = req.system {
        match system {
            IrSystemContent::Text(t) => chars += t.chars().count(),
            IrSystemContent::Blocks(blocks) => {
                for b in blocks {
                    chars += b.text.chars().count();
                }
            }
        }
    }

    // Messages
    for msg in &req.messages {
        for block in &msg.content {
            match block {
                super::types::IrContentBlock::Text { text, .. } => {
                    chars += text.chars().count();
                }
                super::types::IrContentBlock::Thinking { thinking, .. } => {
                    chars += thinking.chars().count();
                }
                super::types::IrContentBlock::ToolResult { content, .. } => match content {
                    super::types::IrToolResultContent::Text(t) => chars += t.chars().count(),
                    super::types::IrToolResultContent::Blocks(blocks) => {
                        for b in blocks {
                            if let super::types::IrContentBlock::Text { text, .. } = b {
                                chars += text.chars().count();
                            }
                        }
                    }
                },
                _ => {}
            }
        }
    }

    // 4 字符 ≈ 1 token；至少返回 1
    ((chars / 4) as u64).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_messages_usage_message_start() {
        let chunk = json!({
            "type": "message_start",
            "message": {
                "usage": {
                    "input_tokens": 200,
                    "cache_creation_input_tokens": 1500,
                    "cache_read_input_tokens": 8000,
                    "output_tokens": 0
                }
            }
        });
        let u = extract_messages_usage(&chunk);
        assert_eq!(u.input_tokens, 200); // 未缓存口径，不含 cache_creation
        assert_eq!(u.cache_read_input_tokens, 8000);
        assert_eq!(u.cache_creation_input_tokens, 1500);
    }

    #[test]
    fn test_extract_messages_usage_message_delta() {
        let chunk = json!({
            "type": "message_delta",
            "delta": {"stop_reason": "end_turn"},
            "usage": {"output_tokens": 300, "cache_read_input_tokens": 8000}
        });
        let u = extract_messages_usage(&chunk);
        assert_eq!(u.output_tokens, 300);
        assert_eq!(u.cache_read_input_tokens, 8000);
    }

    #[test]
    fn test_extract_messages_usage_content_block_delta() {
        let chunk = json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "text_delta", "text": "Hello world"}
        });
        let u = extract_messages_usage(&chunk);
        assert_eq!(u.output_chars, 11);
    }

    #[test]
    fn test_extract_chat_completions_usage_basic() {
        let chunk = json!({
            "id": "chatcmpl-1",
            "choices": [],
            "usage": {
                "prompt_tokens": 9700,
                "completion_tokens": 300,
                "prompt_tokens_details": {"cached_tokens": 8000}
            }
        });
        let u = extract_chat_completions_usage(&chunk);
        assert_eq!(u.input_tokens, 1700); // 9700 - 8000
        assert_eq!(u.output_tokens, 300);
        assert_eq!(u.cache_read_input_tokens, 8000);
    }

    #[test]
    fn test_extract_chat_completions_usage_deepseek_cache() {
        let chunk = json!({
            "id": "chatcmpl-1",
            "choices": [],
            "usage": {
                "prompt_tokens": 9700,
                "completion_tokens": 300,
                "prompt_cache_hit_tokens": 8000,
                "prompt_cache_miss_tokens": 1700
            }
        });
        let u = extract_chat_completions_usage(&chunk);
        assert_eq!(u.input_tokens, 1700);
        assert_eq!(u.cache_read_input_tokens, 8000);
    }

    #[test]
    fn test_extract_chat_completions_usage_no_cache() {
        let chunk = json!({
            "id": "chatcmpl-1",
            "choices": [],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 50
            }
        });
        let u = extract_chat_completions_usage(&chunk);
        assert_eq!(u.input_tokens, 100);
        assert_eq!(u.output_tokens, 50);
        assert_eq!(u.cache_read_input_tokens, 0);
    }

    /// 回归：WorkBuddy 网关真实响应里 `cache_read_input_tokens` 恒为 0，
    /// 命中量只出现在 prompt_cache_hit_tokens / prompt_tokens_details.cached_tokens。
    /// 旧实现会无条件用 0 覆盖命中量 → Router 缓存命中率永远显示 0%。
    #[test]
    fn test_extract_chat_completions_usage_zero_cache_read_does_not_override() {
        let chunk = json!({
            "choices": [],
            "usage": {
                "prompt_tokens": 4450,
                "completion_tokens": 1,
                "total_tokens": 4451,
                "prompt_cache_hit_tokens": 4224,
                "prompt_cache_miss_tokens": 226,
                "prompt_tokens_details": {"cached_tokens": 4224},
                "cache_read_input_tokens": 0,
                "cache_creation_input_tokens": 0,
                "prompt_cache_write_tokens": 0
            }
        });
        let u = extract_chat_completions_usage(&chunk);
        assert_eq!(u.input_tokens, 226);
        assert_eq!(u.cache_read_input_tokens, 4224);
    }

    /// 只给 cache_read_input_tokens 的上游也要能识别，并拆出互斥的未命中口径
    #[test]
    fn test_extract_chat_completions_usage_cache_read_only() {
        let chunk = json!({
            "choices": [],
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 10,
                "cache_read_input_tokens": 800
            }
        });
        let u = extract_chat_completions_usage(&chunk);
        assert_eq!(u.input_tokens, 200);
        assert_eq!(u.cache_read_input_tokens, 800);
    }

    /// 命中字段为 0 时不能被 or_else 短路，必须回退到 details.cached_tokens
    #[test]
    fn test_extract_chat_completions_usage_zero_hit_falls_back_to_details() {
        let chunk = json!({
            "choices": [],
            "usage": {
                "prompt_tokens": 500,
                "completion_tokens": 5,
                "prompt_cache_hit_tokens": 0,
                "prompt_tokens_details": {"cached_tokens": 400}
            }
        });
        let u = extract_chat_completions_usage(&chunk);
        assert_eq!(u.cache_read_input_tokens, 400);
        assert_eq!(u.input_tokens, 100);
    }

    /// 缓存写入量：兼容 cache_creation_input_tokens 与 prompt_cache_write_tokens
    #[test]
    fn test_extract_chat_completions_usage_cache_creation() {
        let chunk = json!({
            "choices": [],
            "usage": {
                "prompt_tokens": 300,
                "completion_tokens": 5,
                "prompt_cache_write_tokens": 300
            }
        });
        let u = extract_chat_completions_usage(&chunk);
        assert_eq!(u.cache_creation_input_tokens, 300);
    }

    #[test]
    fn test_extract_chat_completions_usage_char_count() {
        let chunk = json!({
            "id": "chatcmpl-1",
            "choices": [{"index": 0, "delta": {"content": "Hello"}}]
        });
        let u = extract_chat_completions_usage(&chunk);
        assert_eq!(u.output_chars, 5);
    }

    #[test]
    fn test_extract_responses_usage_completed() {
        let chunk = json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 500,
                    "output_tokens": 200,
                    "input_tokens_details": {"cached_tokens": 300}
                }
            }
        });
        let u = extract_responses_usage(&chunk);
        // input_tokens 应减去缓存命中部分（500 - 300 = 200），保持增量口径
        assert_eq!(u.input_tokens, 200);
        assert_eq!(u.output_tokens, 200);
        assert_eq!(u.cache_read_input_tokens, 300);
    }

    #[test]
    fn test_extract_responses_usage_no_cache() {
        let chunk = json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 400,
                    "output_tokens": 100
                }
            }
        });
        let u = extract_responses_usage(&chunk);
        // 无缓存详情时 input_tokens 不变
        assert_eq!(u.input_tokens, 400);
        assert_eq!(u.output_tokens, 100);
        assert_eq!(u.cache_read_input_tokens, 0);
    }

    #[test]
    fn test_extract_responses_usage_full_cache() {
        let chunk = json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 1000,
                    "output_tokens": 50,
                    "input_tokens_details": {"cached_tokens": 1000}
                }
            }
        });
        let u = extract_responses_usage(&chunk);
        // 全部命中缓存 → input_tokens = 0（纯增量 = 0）
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.cache_read_input_tokens, 1000);
    }

    #[test]
    fn test_extract_responses_usage_delta_chars() {
        let chunk = json!({
            "type": "response.output_text.delta",
            "delta": "Hello world"
        });
        let u = extract_responses_usage(&chunk);
        assert_eq!(u.output_chars, 11);
    }

    #[test]
    fn test_estimate_input_tokens_basic() {
        let req = IrRequest {
            model: "test".to_string(),
            system: Some(IrSystemContent::Text("You are helpful.".to_string())),
            messages: vec![super::super::types::IrMessage {
                role: super::super::types::IrRole::User,
                content: vec![super::super::types::IrContentBlock::Text {
                    text: "Hello world, this is a test.".to_string(),
                    cache_control: None,
                }],
            }],
            tools: vec![],
            tool_choice: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            thinking: None,
            stream: false,
        };
        let est = estimate_input_tokens(&req);
        // "You are helpful." (16) + "Hello world, this is a test." (28) = 44 chars / 4 = 11
        assert_eq!(est, 11);
    }

    #[test]
    fn test_estimate_input_tokens_minimum_one() {
        let req = IrRequest {
            model: "test".to_string(),
            system: None,
            messages: vec![],
            tools: vec![],
            tool_choice: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            thinking: None,
            stream: false,
        };
        assert_eq!(estimate_input_tokens(&req), 1);
    }
}
