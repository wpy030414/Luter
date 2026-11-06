# Spec: Token 配额（5h/7d 滚动窗口）

## 要构建什么
- 目标：按 Service Key 限制用量：每个客户端密钥可设 5 小时 / 7 天两个滚动窗口的 token 上限，
任一窗口超限即在代理入口返回 429；同时向客户端暴露余额查询端点（`/v1/user/balance`），
供 Claude Code 等客户端展示剩余用量。
- 适用范围：检查发生在代理认证链（`api/proxy/quota.rs::check_quota`）：Service Key 验证通过后、
  路由/转发之前（见 module-auth 认证流程步骤 5）。
- `/v1/messages`、`/v1/chat/completions`、`/v1/responses` 均受控；
  `/v1/models` 同样**受配额拦截**（超限即 429，模型列表不可用）；`/v1/user/balance` 不受拦截（只看不拦）。

## 行为
- 不适用：本模块无此项。

## 输入 / 输出
- **配额上限**：随 Service Key 创建/编辑设置（`service_keys.quota_5h` / `quota_7d`，
  单位 token；`0` 表示该窗口不设限）。上限是唯一持久化的配额数据。
- **已用量**：不维护计数器，检查时按需从 `usage_log` 聚合（`db::usage::get_service_key_usage`，窗口常量 5×3600 / 7×86400 秒）。

- 输入：`service_keys.quota_5h` / `quota_7d`（单位 token，`0` = 该窗口不设限；上限是唯一持久化的配额数据）。
- 输出（超限 429）：

- 头：`retry-after`（秒）。
- 体：`quota_error` JSON，指明超限窗口（`5h`/`7d`）与预计重置时间。
- 输出（`GET /v1/user/balance`）：

- 与 `/v1/*` 相同鉴权（Bearer Service Key）。
- 返回 `{ "data": { "quota_5_hour"?: { "usage_percentage": 0~1, "resets_at": RFC3339 }, "quota_7_day"?: {...} } }`（ZenMux 兼容，CCSwitch 使用）。
- **只输出已设限窗口**（`quota > 0`）；未设限窗口省略字段，客户端不渲染徽章。

## 约束
- 
| 约束 | 原因 |
|------|------|
| 只持久化上限，用量实时聚合 | 避免双写计数器与 usage_log 漂移 |
| 窗口口径与统计一致 | 聚合查询与 StatsView 共用同一窗口常量 |
| 检查先于上游调用 | 超限请求不消耗任何上游 key |
| 限流与配额分层 | 令牌桶限流（module-proxy-handler）管突发，配额管总量 |

## 边界条件
- 聚合查询失败：`unwrap_or((0, 0))` 放行（不因统计故障拒绝服务）。
- 超限：429 + `retry-after` + `quota_error`（不进入转发）。
- `quota=0`：该窗口永不拦截，`/v1/user/balance` 中省略该窗口字段。

## 验收标准
- [x] 5h/7d 双窗口检查，超限 429 带 `retry-after` 与 `quota_error`
- [x] `quota=0` 不限该窗口
- [x] `/v1/user/balance` 返回可读余额
- [x] 窗口常量与用量统计口径一致

## 完成定义
- 不适用：本模块无此项。
