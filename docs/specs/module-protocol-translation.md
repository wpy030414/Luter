# Spec: 协议转换（IR 中间表示层）

## 要构建什么
通过 IR（Intermediate Representation，中间表示层）实现 Anthropic Messages API、OpenAI Chat Completions API、OpenAI Responses API 三种协议的统一抽象与互转。

## 行为
```
客户端格式 → IR (from_*.rs) → 内部处理 → IR (to_*.rs) → 上游/客户端格式
```

所有客户端格式先转换为 IR 统一抽象，内部逻辑（搜索/抓取/通知工具剔除、session_inject、审计清洗、usage 追踪、错误构造）只操作 IR 类型，再渲染为目标格式。IR 以 Anthropic Messages 为骨架（内容块模型最丰富），并集覆盖三种格式的全部字段。

### IrRequest（统一请求体）

```rust
pub struct IrRequest {
    pub model: String,
    pub system: Option<IrSystemContent>,     // Text | Blocks(Vec<IrSystemBlock>)
    pub messages: Vec<IrMessage>,
    pub tools: Vec<IrTool>,
    pub tool_choice: Option<IrToolChoice>,   // Auto | Any | None | Tool(name)
    pub max_tokens: Option<u64>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub thinking: Option<IrThinkingConfig>,
    pub stream: bool,
}
```

### IrContentBlock（统一内容块）

```rust
pub enum IrContentBlock {
    Text { text, cache_control },
    Image { source },                     // IrImageSource: Base64{media_type,data} | Url{url}
    Thinking { thinking, signature },
    ToolUse { id, name, input },
    ToolResult { tool_use_id, content, is_error },
}
```

### IrStreamEvent（统一流式事件，6 种变体，以 Anthropic SSE 为骨架）

```rust
pub enum IrStreamEvent {
    MessageStart { id, model, usage: Option<IrUsage> },
    ContentBlockStart { index, block: IrContentBlockStart },   // Text | Thinking{signature} | ToolUse{id,name}
    ContentBlockDelta { index, delta: IrContentDelta },         // TextDelta | ThinkingDelta | InputJsonDelta
    ContentBlockStop { index },
    MessageDelta { stop_reason: Option<IrStopReason>, usage: Option<IrUsage> },
    MessageStop,
}
pub enum IrStopReason { EndTurn, ToolUse, MaxTokens }           // 各协议字符串由方法映射
```

### IrUsage（统一 token 统计）

```rust
pub struct IrUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub output_chars: u64,
}
```

### from_*.rs（客户端格式 → IR）

| 模块 | 输入 | 特殊处理 |
|------|------|---------|
| `from_messages.rs` | Anthropic Messages | server-side `web_search_*` 工具归一化为 `name="web_search"` |
| `from_chat_completions.rs` | OpenAI Chat Completions | `reasoning_content` → `Thinking` 块 |
| `from_responses.rs` | OpenAI Responses API | `input` → `messages`（按 `type` 分发，含 `reasoning` item），`response.completed` 事件解析 |

### to_*.rs（IR → 客户端格式）

| 模块 | 输出 | 特殊处理 |
|------|------|---------|
| `to_messages.rs` | Anthropic Messages | `message_delta.usage` 补上 `input_tokens` |
| `to_chat_completions.rs` | OpenAI Chat Completions | `Thinking` → `reasoning_content` |
| `to_responses.rs` | OpenAI Responses API | IR → Responses output items；function_call 的 `id`/`call_id` 由本层生成；thinking → reasoning item（summary_text） |

### usage.rs（token 提取）

从三种协议格式的响应中提取 `IrUsage`：
- Anthropic Messages：`usage.input_tokens` / `usage.output_tokens` / `usage.cache_read_input_tokens`
- OpenAI Chat Completions：`usage.prompt_tokens` / `usage.completion_tokens` / `usage.prompt_tokens_details.cached_tokens`
- OpenAI Responses：`usage.input_tokens` 减去 `input_tokens_details.cached_tokens`（增量口径）

`estimate_input_tokens()`（chars/4）供 message_start 占位与大上下文响应头超时估算。

| 项 | 规则 |
|----|------|
| 合并策略 | **真实值覆盖估算占位**（不用 max）——`forward.rs` 预填的 `chars/4` 估算值偏大，max 会永久压住真实值 |
| Responses input_tokens | 减去 `cached_tokens`，保持增量口径 |
| message_delta 补全 | IR → Messages 渲染时补上 `input_tokens`（此前缺失） |

| 特性 | Anthropic Messages | OpenAI Chat Completions | OpenAI Responses |
|------|--------------------|-------------------------|-----------------|
| 文本消息 | ✅ text block | ✅ content string | ✅ input/output text |
| 系统提示 | ✅ system 顶层 | ✅ role:system | ✅ instructions |
| 工具调用 | ✅ tool_use block | ✅ tool_calls | ✅ function_call（`id`=`fc_*` + `call_id`=`call_*`） |
| 工具结果 | ✅ tool_result block | ✅ role:tool | ✅ function_call_output |
| 思考过程 | ✅ thinking block | ⚠️ reasoning_content | ✅ reasoning output item（summary_text，往返保留） |
| 工具选择 | ✅ tool_choice | ✅ tool_choice | ✅ tool_choice |
| 流式响应 | ✅ SSE 6 事件 | ✅ SSE choices | ✅ SSE response.* |
| 缓存 token | ✅ cache_read | ❌ | ✅ input_tokens_details |
| 图片 | ✅ image block | ✅ image_url | ✅ input_image |

## 输入 / 输出
- 不适用：本模块无此项。

## 约束
1. **流式转换**: 逐 chunk 实时转换；`forward.rs` 只解析 IR 事件并渲染，不缓冲完整响应（非流式由 `non_stream.rs` 自行收集 IR 事件组装，不复用 SSE 渲染）
2. **字段覆盖边界**: IR 是强类型并集（`IrRequest` 字段见上），**未被建模的字段不会原样透传**；修改协议支持面时必须先扩 `types.rs`
3. **错误容忍**: 单个 chunk 转换失败不影响整个流
4. **thinking 字段**: thinking/reasoning_content 双向转换，内容原样传递（无截断）
5. **usage 真实值覆盖**: 上游真实 usage 覆盖 `forward.rs` 预填的估算占位
6. **Responses 工具调用**: function_call item 的 `id`（`fc_*`）与 `call_id`（`call_*`）都由渲染层生成，added/done 一致；对应上游 tool_use id 不回传
7. **server-side 工具归一化**: `from_messages.rs` / `from_responses.rs` 把无 name 的 `web_search_*` 工具归一为 `name="web_search"`，供代理侧剔除检测

## 边界条件
| 场景 | 行为 |
|------|------|
| 请求格式错误 | 返回 400，不转发 |
| 响应格式错误 | 跳过该 chunk，继续处理 |
| 未知字段 | 原样传递 |
| 转换失败 | 记录 warn 日志，继续处理 |

## 验收标准
1. **单元测试**: 每个转换函数的输入输出
2. **集成测试**: 完整请求-响应流程
3. **边界测试**: 空消息、多工具调用、thinking 双向转换
4. **流式测试**: 逐 chunk 转换的正确性
5. **usage 测试**: 真实值覆盖估算、Responses 增量口径、无缓存场景、全缓存场景

## 完成定义
- `src-tauri/src/api/proxy/ir/types.rs` — IR 类型定义（IrRequest / IrMessage / IrContentBlock / IrStreamEvent / IrUsage）
- `src-tauri/src/api/proxy/ir/from_messages.rs` — Anthropic Messages → IR
- `src-tauri/src/api/proxy/ir/from_chat_completions.rs` — OpenAI Chat Completions → IR
- `src-tauri/src/api/proxy/ir/from_responses.rs` — OpenAI Responses API → IR
- `src-tauri/src/api/proxy/ir/to_messages.rs` — IR → Anthropic Messages
- `src-tauri/src/api/proxy/ir/to_chat_completions.rs` — IR → OpenAI Chat Completions
- `src-tauri/src/api/proxy/ir/to_responses.rs` — IR → OpenAI Responses API
- `src-tauri/src/api/proxy/ir/usage.rs` — Token usage 提取（三种格式）

- [x] Anthropic Messages ↔ IR 双向转换
- [x] OpenAI Chat Completions ↔ IR 双向转换
- [x] OpenAI Responses API ↔ IR 双向转换
- [x] 工具调用转换（tools + tool_choice）
- [x] thinking 字段处理（双向转换，原样传递）
- [x] token 统计（真实值覆盖估算 + Responses 增量口径）
- [x] server-side web_search 工具归一化
- [x] 通过所有单元测试
