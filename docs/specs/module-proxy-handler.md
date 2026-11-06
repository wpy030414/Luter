# Spec: LLM 代理处理器

## 要构建什么
对外暴露 `/v1/*` 五个端点，经 IR 中间表示层统一协议转换，把客户端请求转发到上游 Provider，并提供 Service Key 认证、限流、配额、密钥轮换、故障转移与用量记录。

## 行为
| 方法 | 路径 | 认证 | 说明 |
|------|------|------|------|
| POST | `/v1/messages` | Service Key | Anthropic Messages |
| POST | `/v1/chat/completions` | Service Key | OpenAI Chat Completions |
| POST | `/v1/responses` | Service Key | OpenAI Responses |
| GET | `/v1/models` | Service Key + 配额 | 可用模型列表 |
| GET | `/v1/user/balance` | Service Key | 配额用量（CCSwitch 兼容） |

全部套 `rate_limit_middleware`（令牌桶，**128 req/min per Service Key**）与 `DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES)`（**64 MiB**，`api/proxy/mod.rs`）。`/mcp` 是独立端点（2 MiB），不在本节。

### 请求体示例

```json
// /v1/messages
{ "model": "claude-opus-4-8", "messages": [{"role":"user","content":"Hi"}], "max_tokens": 4096, "stream": true }
// /v1/chat/completions
{ "model": "gpt-4o", "messages": [{"role":"user","content":"Hi"}], "stream": true }
// /v1/responses
{ "model": "o1", "input": [{"role":"user","content":"Hi"}], "stream": true }
```

### GET /v1/models

- 查询 `models JOIN providers`（均 `enabled=1`），按 `tier, display_name` 排序；字段 `id` / `object:"model"` / `owned_by` / `display_name` / `tier` / `context_window` / `max_output_tokens` / `capabilities`（JSON 数组解析）
- 追加全部 `enabled` 组合（`owned_by:"combo"`、`tier:"combo"`、`context_window/max_output_tokens=0`）
- `allowed_models` 非空时按 `display_name` 过滤
- 同样受配额约束（触顶返回 429）

### GET /v1/user/balance

返回 `{ "data": { "quota_5_hour": {...}, "quota_7_day": {...} } }`；仅输出**已设限**（`quota > 0`）的窗口，每项含 `usage_percentage` 与 `resets_at`（RFC3339）。

**认证头**：`x-api-key: <service-key>` 或 `Authorization: Bearer <service-key>`。

```
handler.rs（薄入口：认证 → 配额 → 白名单 → stream 标记 → session_inject → 估算 → 审查上下文）
  └→ stream.rs::proxy_stream()
       ├ 搜索 / 抓取 / 通知工具剔除（MCP 三开关）
       ├ client_wants_stream=false → non_stream.rs::proxy_non_stream()（独立实现）
       └ 流式：resolve_candidates → 上下文预警 → 立即返回 Response → 后台 spawn 双循环
             → forward.rs::forward_stream_ir（上游字节 → IR 事件 → 客户端 SSE）
```

### handler.rs 的准备步骤（`authenticate_and_stream`）

1. `verify_service_key`：Argon2 校验，失败 401 `authentication_error`
2. `check_quota`：5h/7d 滚动窗口，触顶 429
3. `allowed_models` 白名单：非空且模型不在内 → 403 `forbidden`
4. 保存 `client_wants_stream = ir_request.stream`，随后**强制** `ir_request.stream = true`
5. **session_inject 注入**：`settings.session_inject` 非空时前置到系统提示词——`Text` 用 `\n` 拼接、`Blocks` 追加一个块、无 system 则新建 `Text` 块
6. `estimate_input_tokens`（chars/4）→ `header_timeout_for` 预算头超时
7. `audit_enabled` 开启时构建 `AuditCapture`（审查在转发完成后写 `conversations`）

### stream.rs（流式路径）

路由解析（`resolve_combo` 优先 → `resolve_candidates`）→ MCP 三开关剔除/注入 → 估算超 `context_window` 仅 warn → 立即返回 `text/event-stream` Response（含 `:keepalive`）→ 后台 spawn 双循环（provider × key）→ 转发；上游等待期每 **15s** 发 keepalive。

### non_stream.rs（非流式路径，独立实现，约 1220 行）

客户端 `stream: false` 时由 `stream.rs` 在工具剔除后分支到 `proxy_non_stream()`。**不是「收集已渲染的 SSE 再返回」**，而是独立实现：

- 与流式共享 `resolve_candidates` / 双循环 / `pick_key_for` / `update_key_health` / `extract_stream_error` 等基础设施；上游仍以 `stream: true` 请求
- 直接在请求处理内 `await` 上游泳**读取并按 provider_kind 解析 IR 事件**（`IrContentBlockStart` / `IrContentDelta` / `IrUsage` / `IrStopReason`），自建内容块索引，最后组装对应客户端格式的完整 JSON（Messages / Chat Completions / Responses）
- 错误用**真实 HTTP 状态码 + JSON 错误体**表达（非流式没有 SSE 通道），不再出现「non-streaming request answered with a stream」错配
- 无后台 spawn / 无 keepalive；主机头超时与 chunk 间隔超时（120s）沿用流式常量
- 密钥级错误（400 配额耗尽 / 401 / 402 / 403 / 429，含 200 + 非 SSE JSON 错误体）轮换语义与流式一致

failover **强制常开**（`AppState.failover_enabled` 恒 `true`，历史 `failover_enabled` setting 不再读取）。同一 `display_name` 的全部 provider 候选按序尝试：

- **候选顺序**（`route.rs`）：本地模型 `tier='local'` 恒排前 → `providers.sort_order ASC, created_at ASC`；按 provider 去重；跳过插件离线
- **组合优先**：模型名命中 `enabled` 组合 → 按 `position` 逐成员展开、按 `(provider_id, real_model_id)` 去重；组合空集直接 400
- **双层循环**：外层 provider（冷却中跳过），内层 key 轮换
- **冷却**：`failover.rs` 内存表，provider 级失败（5xx / 网络 / 头超时）冷却 **60s**，2xx 清除
- **请求体**：IR 在循环外按主候选渲染，循环内覆写 `model = real_model_id`

- 真实值覆盖估算占位（不用 max）
- Responses `input_tokens` 减去 `cached_tokens`
- `first_token_ms` 记录首个内容增量耗时（见 module-usage-tracking.md）

| 状态 | 场景 |
|------|------|
| 400 | 请求格式错误 / 模型不存在 / 无路由候选 / 组合无成员 |
| 401 | Service Key 无效 |
| 403 | 模型不在白名单 |
| 429 | 限流，或 5h/7d 配额触顶（`quota_error` + `retry-after`） |
| 500 | 内部错误 |
| 502 / 504 | 非流式：全部候选网络失败 / 头超时 |
| 503 | 无可用密钥 |

流式路径的 provider 级失败（无后续候选）以 SSE error event 告知，而非 HTTP 状态码。

## 输入 / 输出
- 不适用：本模块无此项。

## 约束
1. 上游强制流式；客户端两路径分离（SSE 转发 / 非流式独立收集）
2. `display_name` → 上游 `model_id` 替换；用真实模型名调用会被拒绝
3. 超时：响应头 300s / 480s（≥50k 估算）/ 600s（≥100k 估算），chunk 间隔 120s，keepalive 15s
4. 密钥健康：401/403 标 Red，402/429 标 Yellow；HTTP 200 + 流内密钥级错误（`forward.rs::extract_stream_error` 按关键词推断 401/402/403/429）未发内容前换 key
5. 内层最多重试当前 provider 的 key 数（插件占位密钥恒 1 次）

## 边界条件
- 不适用：本模块无此项。

## 验收标准
- 不适用：本模块无此项。

## 完成定义
- `api/proxy/handler.rs` 薄入口 + `proxy_list_models`
- `api/proxy/stream.rs` 流式引擎（`proxy_stream` / `resolve_candidates` / `strip_*_tools`）
- `api/proxy/non_stream.rs` 非流式独立收集与组装
- `api/proxy/forward.rs` 统一 IR 转发
- `api/proxy/{auth,quota,route,failover,key_rotation,audit}.rs`
- `api/proxy/ir/` IR 层（见 module-protocol-translation.md）
- `middleware/rate_limit.rs` 令牌桶

- [x] 五端点（messages / chat_completions / responses / models / user/balance）
- [x] 128 req/min 限流 + 64 MiB 请求体上限
- [x] 5h/7d 配额 + 白名单 + session_inject
- [x] 流式与非流式双路径
- [x] failover（候选池 + 双层循环 + 60s 冷却）
- [x] 真实 usage 覆盖估算并写 `usage_log`
