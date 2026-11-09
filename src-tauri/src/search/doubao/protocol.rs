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
    /// `model_config.model_extra_params`（专家模式需要窗口参数）
    pub model_extra_params: Option<&'static [(&'static str, &'static str)]>,
}

pub const PRESET_EXPERT: ModelPreset = ModelPreset {
    model_item_key: "3",
    menu_type: 3,
    agent_mode: 2,
    mode_id: "1",
    reasoning_effort: 4,
    model_extra_params: Some(&[("total_window_size", "256000")]),
};

pub const PRESET_FAST: ModelPreset = ModelPreset {
    model_item_key: "0",
    menu_type: 0,
    agent_mode: 2,
    mode_id: "1",
    reasoning_effort: 3,
    model_extra_params: None,
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

/// 把用户的搜索意图包装成上游 prompt。
///
/// 豆包按模型意图决定是否联网（`llm_intention=browsing`）。实测：明确要求联网、
/// 且问题本身带时效性时触发率最高；「不要作答」反而会让它跳过检索。
pub fn build_search_prompt(query: &str) -> String {
    format!("请联网搜索并告诉我关于「{query}」的最新资料和相关信息，需要引用具体网页来源。")
}

/// 构造 `/chat/completion` 请求体。字段逐字对齐 xrp-doubao 已跑通版本。
pub fn build_request_body(
    prompt: &str,
    preset: ModelPreset,
    bot_id: &str,
    _device: &Device,
) -> Value {
    let message_id = uuid_like();
    let now_ms = chrono::Utc::now().timestamp_millis();

    // 各种 ext 字段上游期望字符串（不是数字），照抄 xrp-doubao
    let agent_mode_s = preset.agent_mode.to_string();
    let menu_type_s = preset.menu_type.to_string();
    let reasoning_effort_s = preset.reasoning_effort.to_string();

    let mut client_meta = serde_json::Map::new();
    client_meta.insert("local_conversation_id".into(), json!(format!("local_{}", &message_id[..12])));
    client_meta.insert("conversation_id".into(), json!(""));
    client_meta.insert("bot_id".into(), json!(bot_id));
    client_meta.insert("last_section_id".into(), json!(""));
    client_meta.insert("last_message_index".into(), Value::Null);
    client_meta.insert(
        "local_permissions".into(),
        json!([
            {"permission_name": "ACCESS_COARSE_LOCATION", "status": 3},
            {"permission_name": "ACCESS_FINE_LOCATION", "status": 3},
            {"permission_name": "ACCESS_BACKGROUND_LOCATION", "status": 3}
        ]),
    );

    let mut text_block = serde_json::Map::new();
    text_block.insert("text".into(), json!(prompt));
    text_block.insert("icon_url".into(), json!(""));
    text_block.insert("icon_url_dark".into(), json!(""));
    text_block.insert("summary".into(), json!(""));

    let mut block_content = serde_json::Map::new();
    block_content.insert("text_block".into(), Value::Object(text_block));
    block_content.insert("pc_event_block".into(), json!(""));

    let mut block = serde_json::Map::new();
    block.insert("block_type".into(), json!(10000));
    block.insert("content".into(), Value::Object(block_content));
    block.insert("block_id".into(), json!(uuid_like()));
    block.insert("parent_id".into(), json!(""));
    block.insert("meta_info".into(), json!([]));
    block.insert("extra_info".into(), json!({}));
    block.insert("append_fields".into(), json!([]));

    let mut message = serde_json::Map::new();
    message.insert("local_message_id".into(), json!(message_id));
    message.insert("content_block".into(), json!([Value::Object(block)]));
    message.insert("message_status".into(), json!(0));

    let mut init_ext = serde_json::Map::new();
    init_ext.insert("model_item_key".into(), json!(preset.model_item_key));
    init_ext.insert("reasoning_effort".into(), json!(reasoning_effort_s));
    init_ext.insert("mode_id".into(), json!(preset.mode_id));

    let mut model_config = serde_json::Map::new();
    model_config.insert("model_item_key".into(), json!(preset.model_item_key));
    let mut extra = serde_json::Map::new();
    if let Some(params) = preset.model_extra_params {
        for (k, v) in params.iter() {
            extra.insert((*k).to_string(), json!(v));
        }
    }
    model_config.insert("model_extra_params".into(), Value::Object(extra));
    model_config.insert("reasoning_effort".into(), json!(preset.reasoning_effort));

    let conversation_mode = preset.mode_id.parse::<i64>().unwrap_or(1);
    let mut aggregate = serde_json::Map::new();
    aggregate.insert("mention_skill_list".into(), json!("[]"));
    aggregate.insert("mention_plugin_list".into(), json!("[]"));
    aggregate.insert("mention_ext".into(), json!("[{}]"));
    aggregate.insert("conversation_mode".into(), json!(preset.mode_id));
    aggregate.insert("mode_id".into(), json!(preset.mode_id));
    aggregate.insert("model_item_key".into(), json!(preset.model_item_key));
    aggregate.insert("agent_mode".into(), json!(agent_mode_s));
    aggregate.insert("reasoning_effort".into(), json!(reasoning_effort_s));
    aggregate.insert("provider_id".into(), json!(""));

    let mut recovery = serde_json::Map::new();
    recovery.insert("is_recovery".into(), json!(false));
    recovery.insert("req_create_time_sec".into(), json!(now_ms / 1000));
    recovery.insert("append_sse_event_scene".into(), json!(0));

    let mut option = serde_json::Map::new();
    option.insert("send_message_scene".into(), json!(""));
    option.insert("create_time_ms".into(), json!(now_ms));
    option.insert("collect_id".into(), json!(""));
    option.insert("is_audio".into(), json!(false));
    option.insert("answer_with_suggest".into(), json!(false));
    option.insert("agent_mode".into(), json!(preset.agent_mode));
    option.insert("tts_switch".into(), json!(false));
    option.insert("need_deep_think".into(), json!(preset.menu_type));
    option.insert("click_clear_context".into(), json!(false));
    option.insert("from_suggest".into(), json!(false));
    option.insert("is_regen".into(), json!(false));
    option.insert("is_replace".into(), json!(false));
    option.insert("is_from_click_option".into(), json!(false));
    option.insert("is_from_click_softlink".into(), json!(false));
    option.insert("disable_sse_cache".into(), json!(false));
    option.insert("select_text_action".into(), json!(""));
    option.insert("is_select_text".into(), json!(false));
    option.insert("resend_for_regen".into(), json!(false));
    option.insert("scene_type".into(), json!(0));
    option.insert("unique_key".into(), json!(uuid_like()));
    option.insert("start_seq".into(), json!(0));
    option.insert("need_create_conversation".into(), json!(true));
    option.insert("conversation_init_option".into(), json!({"need_ack_conversation": true}));
    option.insert("conversation_init_ext".into(), Value::Object(init_ext));
    option.insert("regen_query_id".into(), json!([]));
    option.insert("edit_query_id".into(), json!([]));
    option.insert("regen_instruction".into(), json!(""));
    option.insert("no_replace_for_regen".into(), json!(false));
    option.insert("message_from".into(), json!(0));
    option.insert("shared_app_name".into(), json!(""));
    option.insert("shared_app_id".into(), json!(""));
    option.insert("sse_recv_event_options".into(), json!({"support_chunk_delta": true}));
    option.insert("support_lazy_fetch_stream".into(), json!(true));
    option.insert("is_ai_playground".into(), json!(false));
    option.insert("is_old_user".into(), json!(false));
    option.insert("general_task_param".into(), json!({"server_do_compare": true}));
    option.insert("recovery_option".into(), Value::Object(recovery));
    option.insert("message_storage_type".into(), json!(0));
    option.insert("related_deleted_message_ids".into(), json!({}));
    option.insert("connector_info_list".into(), json!([]));
    option.insert("model_config".into(), Value::Object(model_config));
    option.insert("aggregate_params".into(), Value::Object(aggregate));
    option.insert("conversation_mode".into(), json!(conversation_mode));

    let mut ext = serde_json::Map::new();
    ext.insert("agent_mode".into(), json!(agent_mode_s));
    ext.insert("use_deep_think".into(), json!(menu_type_s));
    ext.insert("sub_conv_firstmet_type".into(), json!("1"));
    ext.insert("general_task_param".into(), json!("{\"server_do_compare\":true}"));
    ext.insert("collection_id".into(), json!(""));
    ext.insert("is_finish".into(), json!("1"));
    ext.insert("conversation_init_option".into(), json!("{\"need_ack_conversation\":true}"));
    ext.insert("commerce_credit_config_enable".into(), json!("0"));

    let mut root = serde_json::Map::new();
    root.insert("client_meta".into(), Value::Object(client_meta));
    root.insert("messages".into(), json!([Value::Object(message)]));
    root.insert("option".into(), Value::Object(option));
    root.insert("user_context".into(), json!([]));
    root.insert("ext".into(), Value::Object(ext));
    Value::Object(root)
}

pub(crate) fn uuid_like() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut bytes = [0u8; 16];
    rng.fill(&mut bytes);
    // RFC 4122 v4：第 7 字节高 4 位固定 0100，第 9 字节高 2 位固定 10
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
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
        // ext 上的字段上游要求字符串（实测：数字会被判 common invalid param）
        assert_eq!(body["ext"]["use_deep_think"], "3");
        assert_eq!(body["ext"]["agent_mode"], "2");
        assert_eq!(body["ext"]["sub_conv_firstmet_type"], "1");
        // aggregate_params.conversation_mode 也是字符串
        assert_eq!(body["option"]["aggregate_params"]["conversation_mode"], "1");
        // 专家模式需要窗口参数
        assert_eq!(
            body["option"]["model_config"]["model_extra_params"]["total_window_size"],
            "256000"
        );
        // 权限项用 permission_name
        assert_eq!(
            body["client_meta"]["local_permissions"][0]["permission_name"],
            "ACCESS_COARSE_LOCATION"
        );
        // unique_key 必须是合法 UUID v4 形状
        let key = body["option"]["unique_key"].as_str().unwrap();
        assert_eq!(key.len(), 36);
        assert_eq!(&key[14..15], "4");
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
