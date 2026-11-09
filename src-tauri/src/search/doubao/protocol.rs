//! 豆包网页端协议：请求体构造 + SSE 解析。
//!
//! 逆向来源见 `~/Codes/xrp-doubao/docs/DOUBAO_REVERSE.md`。本模块只保留搜索需要的
//! 最小子集：构造 `/chat/completion` 请求体，并从中提取 `block_type=10025`
//! 的 `search_query_result_block`。
//!
//! 设计纪律：
//! - 纯函数、无 I/O，便于用固定 JSON 做单测；
//! - 字段名逐字对齐上游（驼峰/下划线混用是上游原样，不要「修正」）。

use serde_json::{json, Value};

/// 豆包网页端模型模式（搜索用「专家」，意图识别更稳）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelPreset {
    pub model_item_key: &'static str,
    pub menu_type: i64,
    pub agent_mode: i64,
    pub mode_id: &'static str,
    pub reasoning_effort: i64,
}

pub const PRESET_EXPERT: ModelPreset = ModelPreset {
    model_item_key: "3",
    menu_type: 3,
    agent_mode: 2,
    mode_id: "1",
    reasoning_effort: 4,
};

pub const PRESET_FAST: ModelPreset = ModelPreset {
    model_item_key: "0",
    menu_type: 0,
    agent_mode: 2,
    mode_id: "1",
    reasoning_effort: 3,
};

/// 按别名解析模型预设；未知回退快速。
pub fn resolve_preset(alias: &str) -> ModelPreset {
    match alias.trim().to_ascii_lowercase().as_str() {
        "doubao-expert" | "expert" | "专家" => PRESET_EXPERT,
        _ => PRESET_FAST,
    }
}

/// 设备指纹（登录时生成并随凭证保存）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub device_id: String,
    pub web_id: String,
    pub fp: String,
}

impl Device {
    /// 生成新的设备指纹（上游无校验，只需稳定且像真实值）。
    pub fn generate() -> Self {
        Self {
            device_id: random_numeric_id(19),
            web_id: random_numeric_id(19),
            fp: format!(
                "verify_{}_{}_{}_{}_{}",
                random_alnum(8),
                random_alnum(4),
                random_alnum(4),
                random_alnum(4),
                random_alnum(12)
            ),
        }
    }
}

fn random_numeric_id(len: usize) -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|i| {
            let digit = rng.gen_range(1..=9);
            if i == 0 {
                digit.to_string()
            } else {
                rng.gen_range(0..=9).to_string()
            }
        })
        .collect()
}

fn random_alnum(len: usize) -> String {
    use rand::Rng;
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char)
        .collect()
}

/// 把用户的搜索意图包装成上游 prompt，尽量把 `llm_intention` 拉向 browsing。
pub fn build_search_prompt(query: &str) -> String {
    format!(
        "请联网搜索下面这个问题，只列出你检索到的网页资料（标题和链接），不要总结、不要作答。\n\n问题：{query}"
    )
}

/// 构造 `/chat/completion` 请求体。
///
/// 只填必要字段；上游对未知字段宽容，对缺失字段也宽容（已由 xrp-doubao 实测）。
pub fn build_request_body(
    prompt: &str,
    preset: ModelPreset,
    bot_id: &str,
    _device: &Device,
) -> Value {
    let message_id = uuid_like();
    let now_ms = chrono::Utc::now().timestamp_millis();

    json!({
        "client_meta": {
            "local_conversation_id": format!("local_{}", &message_id[..12]),
            "conversation_id": "",
            "bot_id": bot_id,
            "last_section_id": "",
            "last_message_index": null,
            "local_permissions": [
                {"permission": "location", "status": 3},
                {"permission": "camera", "status": 3},
                {"permission": "microphone", "status": 3}
            ]
        },
        "messages": [{
            "local_message_id": message_id,
            "content_block": [{
                "block_type": 10000,
                "block_id": uuid_like(),
                "content": {
                    "text_block": {
                        "text": prompt,
                        "icon_url": "",
                        "icon_url_dark": "",
                        "summary": ""
                    },
                    "pc_event_block": ""
                },
                "meta_info": [],
                "extra_info": {},
                "append_fields": []
            }],
            "message_status": 0
        }],
        "option": {
            "send_message_scene": "",
            "create_time_ms": now_ms,
            "collect_id": "",
            "is_audio": false,
            "answer_with_suggest": false,
            "agent_mode": preset.agent_mode,
            "tts_switch": false,
            "need_deep_think": preset.menu_type,
            "click_clear_context": false,
            "from_suggest": false,
            "is_regen": false,
            "is_replace": false,
            "is_from_click_option": false,
            "is_from_click_softlink": false,
            "disable_sse_cache": true,
            "select_text_action": "",
            "is_select_text": false,
            "resend_for_regen": false,
            "scene_type": 0,
            "unique_key": uuid_like(),
            "start_seq": 0,
            "need_create_conversation": true,
            "conversation_init_option": {"need_ack_conversation": true},
            "conversation_init_ext": {
                "model_item_key": preset.model_item_key,
                "reasoning_effort": preset.reasoning_effort,
                "mode_id": preset.mode_id
            },
            "message_from": "chat",
            "sse_recv_event_options": {"support_chunk_delta": true},
            "support_lazy_fetch_stream": true,
            "model_config": {
                "model_item_key": preset.model_item_key,
                "reasoning_effort": preset.reasoning_effort
            },
            "aggregate_params": {
                "mention_skill_list": "[]",
                "mention_plugin_list": "[]",
                "mention_ext": "[{}]",
                "conversation_mode": preset.mode_id.parse::<i64>().unwrap_or(1),
                "mode_id": preset.mode_id,
                "model_item_key": preset.model_item_key,
                "agent_mode": preset.agent_mode,
                "reasoning_effort": preset.reasoning_effort
            },
            "conversation_mode": preset.mode_id.parse::<i64>().unwrap_or(1)
        },
        "ext": {
            "agent_mode": preset.agent_mode,
            "use_deep_think": preset.menu_type,
            "sub_conv_firstmet_type": 1,
            "conversation_init_option": {"need_ack_conversation": true}
        }
    })
}

fn uuid_like() -> String {
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        rand_u32(),
        rand_u16(),
        rand_u16(),
        rand_u16(),
        rand_u48()
    )
}

fn rand_u32() -> u32 {
    use rand::Rng;
    rand::thread_rng().gen()
}
fn rand_u16() -> u16 {
    use rand::Rng;
    rand::thread_rng().gen()
}
fn rand_u48() -> u64 {
    use rand::Rng;
    rand::thread_rng().gen::<u64>() & 0xffff_ffff_ffff
}

/// 一条命中的资料。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub index: i64,
    pub title: String,
    pub url: String,
    pub doc_id: Option<String>,
    pub original_doc_rank: Option<i64>,
}

/// 从单个 SSE `data:` JSON 中提取搜索结果。
///
/// 返回 `(是否出现 10025 块, 本批资料)`。调用方负责跨事件去重与累积。
pub fn parse_search_block(value: &Value) -> (bool, Vec<Source>) {
    let mut found = false;
    let mut out = Vec::new();

    let visit_blocks = |blocks: &Value, out: &mut Vec<Source>| {
        let Some(list) = blocks.as_array() else {
            return false;
        };
        let mut hit = false;
        for block in list {
            let bt = block
                .get("block_type")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            if bt != 10025 {
                continue;
            }
            hit = true;
            let Some(block_ref) = block
                .pointer("/content/search_query_result_block")
                .and_then(Value::as_object)
            else {
                continue;
            };
            let Some(results) = block_ref.get("results").and_then(Value::as_array) else {
                continue;
            };
            for item in results {
                let Some(card) = item.get("text_card") else {
                    continue;
                };
                let title = card
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                let url = card.get("url").and_then(Value::as_str).unwrap_or("").trim();
                if title.is_empty() || url.is_empty() {
                    continue;
                }
                out.push(Source {
                    index: card.get("index").and_then(Value::as_i64).unwrap_or(0),
                    title: title.to_string(),
                    url: url.to_string(),
                    doc_id: card
                        .get("doc_id")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    original_doc_rank: card.get("original_doc_rank").and_then(Value::as_i64),
                });
            }
        }
        hit
    };

    // 三种承载形态：顶层 content_block / patch_op[].patch_value.content_block
    if let Some(blocks) = value.get("content_block") {
        found |= visit_blocks(blocks, &mut out);
    }
    if let Some(content) = value.get("content") {
        if let Some(blocks) = content.get("content_block") {
            found |= visit_blocks(blocks, &mut out);
        }
    }
    if let Some(patches) = value.get("patch_op").and_then(Value::as_array) {
        for patch in patches {
            let pv = patch.get("patch_value").unwrap_or(patch);
            if let Some(blocks) = pv.get("content_block") {
                found |= visit_blocks(blocks, &mut out);
            }
        }
    }

    (found, out)
}

/// 读取 SSE 事件名（`event: xxx`）与 data 行。
pub fn parse_sse_event(raw: &str) -> (Option<String>, String) {
    let mut event = None;
    let mut data_lines: Vec<&str> = Vec::new();
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start());
        }
    }
    (event, data_lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_preset() {
        assert_eq!(resolve_preset("doubao-expert"), PRESET_EXPERT);
        assert_eq!(resolve_preset("unknown"), PRESET_FAST);
    }

    #[test]
    fn test_build_body_has_required_fields() {
        let device = Device {
            device_id: "7000000000000000001".into(),
            web_id: "7000000000000000002".into(),
            fp: "verify_test".into(),
        };
        let body = build_request_body("查询", PRESET_EXPERT, "7338286299411103781", &device);
        assert_eq!(body["client_meta"]["bot_id"], "7338286299411103781");
        assert_eq!(body["option"]["model_config"]["model_item_key"], "3");
        assert_eq!(body["option"]["need_deep_think"], 3);
        assert_eq!(body["ext"]["use_deep_think"], 3);
        assert_eq!(
            body["messages"][0]["content_block"][0]["content"]["text_block"]["text"],
            "查询"
        );
    }

    #[test]
    fn test_parse_search_block_from_patch_op() {
        let value = json!({
            "patch_op": [{
                "patch_object": 1,
                "patch_type": 1,
                "patch_value": {
                    "content_block": [{
                        "block_type": 10025,
                        "content": {
                            "search_query_result_block": {
                                "summary": "搜索 1 个关键词",
                                "results": [
                                    {"text_card": {"id": 1, "title": "甲", "url": "https://a.com", "doc_id": "d1", "index": 1, "original_doc_rank": 1}},
                                    {"text_card": {"id": 2, "title": "", "url": "https://skip.me"}},
                                    {"text_card": {"id": 3, "title": "乙", "url": "https://b.com", "index": 2}}
                                ]
                            }
                        }
                    }]
                }
            }]
        });
        let (found, sources) = parse_search_block(&value);
        assert!(found);
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].title, "甲");
        assert_eq!(sources[0].doc_id.as_deref(), Some("d1"));
        assert_eq!(sources[1].original_doc_rank, None);
    }

    #[test]
    fn test_parse_search_block_absent() {
        let (found, sources) = parse_search_block(&json!({"patch_op": []}));
        assert!(!found);
        assert!(sources.is_empty());
    }

    #[test]
    fn test_parse_sse_event() {
        let raw = "event: STREAM_CHUNK\ndata: {\"a\":1}\nid: 11";
        let (event, data) = parse_sse_event(raw);
        assert_eq!(event.as_deref(), Some("STREAM_CHUNK"));
        assert_eq!(data, "{\"a\":1}");
    }

    #[test]
    fn test_device_generate_shape() {
        let d = Device::generate();
        assert_eq!(d.device_id.len(), 19);
        assert!(d.fp.starts_with("verify_"));
    }
}
