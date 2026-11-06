# Spec: 对话审查（Conversation Audit）

## 要构建什么
在 `audit_enabled` 开关开启时，把经过网关的每次请求按「对话」归并为一行存储，供管理端查看会话内容、请求次数与最近活动；默认关闭。

## 行为
| 设置键 | 默认 | 读写 |
|--------|------|------|
| `audit_enabled` | `false` | `settings` 表持久化；`AppState.audit_enabled: Arc<AtomicBool>` 运行时读写，`/api/settings` GET/PUT 热更新（`api/handlers/stats.rs`） |

关闭时 handler 不构建 `AuditCapture`，转发路径零额外开销、零写入。

```
handler.rs（audit_enabled → 构建 AuditCapture{db, sk_id, sk_name, request_messages}）
  → forward.rs / non_stream.rs 转发期间累积助手文本
  → 转发完成后（响应非空时）：
      all_msgs = 请求消息 + [assistant 回复]
      sanitize_messages → compute_fingerprint → upsert_conversation
```

- **累积范围**：仅助手 Text 增量；响应为空（失败/无内容）不落库。
- **清洗**（`api/proxy/audit.rs::sanitize_messages`）：Image 块 → `"[image]"`；Text 块截断 10,000 字符；ToolResult 递归清洗；去掉 `cache_control`。
- **指纹**（`compute_fingerprint`）：`SHA256(service_key_id + ":" + role + ":" + 每条文本截断 200 字符 ...)`，**最多取前 3 条消息**。同一对话的多轮请求共享前缀 → 同一指纹。
- **upsert**（`db/conversations.rs`）：`ON CONFLICT(fingerprint)` 时 `request_count + 1`、`messages` 更新为最新版本、`message_count` / `updated_at` / 预览字段刷新。
- **预览字段**：`first_user_message`（首条 user 文本，≤100 字符）、`last_message`（末条文本剥除 `<system-reminder>` 等注入标签，≤100）、`last_message_raw`（未剥除，≤100）。

字段见 module-database。索引：`idx_conv_sk(service_key_id)`、`idx_conv_updated(updated_at DESC)`。

| 路由 | 说明 |
|------|------|
| `GET /api/audit?page=&page_size=&service_key_id=` | 列表，`updated_at DESC`，page 默认 1，page_size 默认 20、clamp 1–100；返回 `{total, page, page_size, data}` |
| `GET /api/audit/:id` | 详情（含完整 `messages` JSON）；不存在返回 `{"error":"not found"}` |
| `DELETE /api/audit/:id` | 删除单条 |

数据导出 / 重置包含 `conversations` 表（见 module-database）。

## 输入 / 输出
- 不适用：本模块无此项。

## 约束
1. **隐私**：默认关闭；开启意味着对话全文落到本地 SQLite，UI 需明示。
2. **写放大**：每个请求一次 upsert（含全量 messages 序列化），关闭开关即完全消除。
3. **归并依赖指纹稳定性**：系统提示词变化或首条消息变化会产生新会话行——这是预期行为，不做模糊合并。
4. **无图片数据**：base64 一律替换为 `"[image]"`，体积可控。

## 边界条件
- 不适用：本模块无此项。

## 验收标准
- 不适用：本模块无此项。

## 完成定义
- `src-tauri/src/api/proxy/audit.rs` — 清洗 / 指纹 / 预览提取
- `src-tauri/src/api/proxy/stream.rs` — `AuditCapture` 定义
- `src-tauri/src/api/proxy/{forward,non_stream}.rs` — 累积回复 + 转发后 upsert
- `src-tauri/src/db/conversations.rs` — 存储查询
- `src-tauri/src/api/handlers/audit.rs` — REST
- `src/views/AuditView.tsx` — 前端列表 / 详情

- [x] `audit_enabled` 开关（默认关，持久化 + 热更新）
- [x] 流式 / 非流式两条路径都在转发完成后写入
- [x] 图片剥离、文本截断、3 条消息指纹、按指纹 upsert
- [x] 列表分页（可按 service_key 过滤）/ 详情 / 删除
- [x] 导出与重置覆盖 `conversations`
