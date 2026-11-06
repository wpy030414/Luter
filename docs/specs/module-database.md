# Spec: 数据库 Schema 与迁移

## 要构建什么
管理 SQLite 数据库的 Schema 定义和版本迁移，确保数据一致性和向后兼容。

## 行为
所有 DDL 以 Rust 字符串内联在 `src-tauri/src/db/schema.rs` 的 `MIGRATIONS` 数组中，**没有**独立的 `.sql` 文件。版本 = 数组长度，**当前 V25**。

### 执行逻辑（`db/mod.rs::Database::migrate`）

1. 先 `CREATE TABLE IF NOT EXISTS schema_version`，读 `MAX(version)`（缺失视为 0）
2. 若 `current >= MIGRATIONS.len()` 直接返回
3. 对剩余迁移逐条执行：`BEGIN IMMEDIATE; {migration}; INSERT OR REPLACE INTO schema_version(version, applied_at) ...; COMMIT;`
   ——**迁移与版本写入在同一事务内原子提交**；失败时显式 `ROLLBACK` 并返回错误（启动失败，不会出现半应用状态）
4. 打开库时 `PRAGMA foreign_keys = ON`、`journal_mode = WAL`、`busy_timeout = 5000`、`synchronous = NORMAL`

`version` 写入使用 `INSERT OR REPLACE` 是安全的：`schema_version` 无子表，不存在 CASCADE 风险。

1. **只追加**：新迁移追加到 `MIGRATIONS` 末尾，**不得**修改已有迁移
2. `CREATE TABLE IF NOT EXISTS` **不会**给既有表补列——新增列必须用 `ALTER TABLE ... ADD COLUMN`
3. SQLite 不支持 `ADD COLUMN IF NOT EXISTS`（V23 曾因此导致迁移解析失败）；幂等性由「事务 + 版本号」保证，直接写普通 `ADD COLUMN`
4. 业务写入一律 `ON CONFLICT DO UPDATE`（UPSERT），**禁止** `INSERT OR REPLACE`（`ON DELETE CASCADE` 会删掉子行）
5. 新迁移不能破坏旧数据：列尽量带 `DEFAULT`，语义「未知」的数值列允许 NULL
6. 新增数据表必须同步 `settings.rs` 的 `export_sql` 表清单与 `reset_all_data` 删除顺序

| 版本 | 改动 |
|------|------|
| V1 | 基础表：providers / models / api_keys / routes / usage_log / schema_version + 索引 |
| V2 | service_keys + 哈希索引 |
| V3 | models.capabilities；清空旧 keys；删除 custom / deap provider |
| V4–V5 | usage_log.service_key_id；密钥状态纯内存化（status 列不再读写） |
| V6 | settings（key-value） |
| V7–V10 | 缓存 token 列；价格单位 1K→MTok 后整体删除价格列；cache_creation 并入 prompt_tokens |
| V11 | plugins |
| V12 | usage_log 重建为自包含快照（去掉全部 FK） |
| V13 | providers.sort_order |
| V14 | service_keys.quota_5h / quota_7d |
| V15 | provider kind 重命名：`openai`→`chat_completions`、`anthropic`→`messages` |
| V16 | `websearch_hijack` → `mcp_websearch`（旧键保留不读） |
| V17 / V19 | MCP 设置键默认行：`mcp_vision*`（V17，工具已在  移除，仅保留数据）、`mcp_notify` |
| V18 | combos / combo_members |
| V20–V21 | local_models；新增 thinking 列 |
| V22–V23 | conversations；新增 last_message / last_message_raw |
| V24 | plugins.work_dir / autostart；删除插件 provider 的历史密钥行 |
| V25 | usage_log.first_token_ms（TTFT，可空） |

### providers

```sql
CREATE TABLE providers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,               -- "messages" | "chat_completions" | "responses" | "plugin"
    base_url TEXT NOT NULL,
    api_path TEXT DEFAULT '/chat/completions',
    enabled INTEGER DEFAULT 1,
    config_json TEXT NOT NULL DEFAULT '{}',
    sort_order INTEGER NOT NULL DEFAULT 0,  -- V13：拖拽排序，越小越优先
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
```

### models

```sql
CREATE TABLE models (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,        -- FK providers ON DELETE CASCADE
    model_id TEXT NOT NULL,           -- 真实模型名（发给上游）
    display_name TEXT NOT NULL,       -- 别名（暴露给客户端）
    tier TEXT NOT NULL DEFAULT 'custom',
    context_window INTEGER DEFAULT 128000,
    max_output_tokens INTEGER DEFAULT 4096,
    capabilities TEXT NOT NULL DEFAULT '["text"]',  -- JSON 数组（V3）
    enabled INTEGER DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(provider_id, model_id)
);
```

> `display_name` 无唯一约束——同一别名可挂在多个 provider 上，路由时按 provider `sort_order` 取候选（failover 的候选池）。

### api_keys（Provider Key）

```sql
CREATE TABLE api_keys (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,        -- FK providers ON DELETE CASCADE
    name TEXT NOT NULL DEFAULT '',
    key_hash TEXT NOT NULL,           -- 实为 AES-256-GCM 密文（列名历史遗留）
    key_masked TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'unknown',  -- V5 后不读不写
    last_error TEXT, last_error_code INTEGER, last_error_time INTEGER,
    last_used_at INTEGER,
    balance REAL, balance_updated_at INTEGER,
    total_requests INTEGER DEFAULT 0,
    total_tokens INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
```

### service_keys（客户端 Key）

```sql
CREATE TABLE service_keys (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL DEFAULT '',
    key_hash TEXT NOT NULL,           -- Argon2 哈希，不可逆
    key_masked TEXT NOT NULL,
    allowed_models TEXT NOT NULL DEFAULT '[]',  -- JSON 数组，空 = 不限
    quota_5h INTEGER NOT NULL DEFAULT 0,        -- V14：5h 窗口 token 上限，0 = 不设限
    quota_7d INTEGER NOT NULL DEFAULT 0,        -- V14：7d 窗口 token 上限，0 = 不设限
    total_requests INTEGER DEFAULT 0,
    total_tokens INTEGER DEFAULT 0,
    last_used_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
```

### combos / combo_members（V18）

```sql
CREATE TABLE combos (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,          -- 组合别名，不得撞 models.display_name（handler 层双向校验）
    enabled INTEGER DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE combo_members (
    id TEXT PRIMARY KEY,
    combo_id TEXT NOT NULL,             -- FK combos ON DELETE CASCADE
    member_alias TEXT NOT NULL,         -- TEXT 软引用 models.display_name
    position INTEGER NOT NULL DEFAULT 0,
    UNIQUE(combo_id, member_alias)
);
```

**注意**：`member_alias` 是软引用（`display_name` 非唯一，无法建硬 FK）；保存组合时用事务「UPSERT 头 + DELETE 重插成员」。

### usage_log（V12 重建，无外键）

```sql
CREATE TABLE usage_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp INTEGER NOT NULL,
    provider_id TEXT NOT NULL,  provider_name TEXT NOT NULL DEFAULT '',
    model_id TEXT NOT NULL,     model_display_name TEXT NOT NULL DEFAULT '',
    key_id TEXT,                key_name TEXT NOT NULL DEFAULT '',
    key_masked TEXT NOT NULL DEFAULT '',
    service_key_id TEXT,        service_key_name TEXT NOT NULL DEFAULT '',
    service_key_masked TEXT NOT NULL DEFAULT '',
    request_type TEXT NOT NULL,
    prompt_tokens INTEGER NOT NULL,        -- 含 cache write
    completion_tokens INTEGER NOT NULL,
    latency_ms INTEGER NOT NULL,
    success INTEGER NOT NULL,
    error_message TEXT,
    cache_read_input_tokens INTEGER NOT NULL DEFAULT 0,  -- V7
    first_token_ms INTEGER                               -- V25：TTFT，NULL = 未知
);
```

所有名称字段写入时快照，删除 Provider / Model / Key 不影响历史统计。

### plugins（V11 + V24）

```sql
CREATE TABLE plugins (
    id TEXT PRIMARY KEY,
    provider_id TEXT REFERENCES providers(id) ON DELETE SET NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    last_heartbeat_at INTEGER,
    work_dir TEXT,                          -- V24：插件项目根目录（伴生启动 cwd）
    autostart INTEGER NOT NULL DEFAULT 0,   -- V24：Router 启动时自动拉起插件
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
```

### local_models（V20 + V21）

```sql
CREATE TABLE local_models (
    id TEXT PRIMARY KEY,                 -- lm-{uuid}
    repo_id TEXT NOT NULL,
    filename TEXT NOT NULL,              -- GGUF 文件名
    format TEXT NOT NULL DEFAULT 'gguf',
    backend TEXT NOT NULL DEFAULT 'auto',        -- auto/metal/cuda/vulkan/rocm/cpu
    status TEXT NOT NULL DEFAULT 'downloaded',   -- downloaded/running/error
    model_id TEXT NOT NULL,              -- 引擎上报的模型名
    ctx_size INTEGER NOT NULL DEFAULT 32768,
    n_gpu_layers INTEGER NOT NULL DEFAULT 99,
    autostart INTEGER NOT NULL DEFAULT 1,
    thinking INTEGER NOT NULL DEFAULT 1, -- V21：1 = --reasoning on，0 = off
    file_size INTEGER,
    local_path TEXT NOT NULL,
    port INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
```

### conversations（V22 + V23，对话审查）

```sql
CREATE TABLE conversations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    fingerprint TEXT NOT NULL UNIQUE,       -- 前 3 条消息 SHA256 指纹
    service_key_id TEXT NOT NULL DEFAULT '',
    service_key_name TEXT NOT NULL DEFAULT '',
    messages TEXT NOT NULL DEFAULT '[]',    -- 最新完整消息（图片替换为 "[image]"）
    message_count INTEGER NOT NULL DEFAULT 0,
    request_count INTEGER NOT NULL DEFAULT 1,
    first_user_message TEXT NOT NULL DEFAULT '',
    last_message TEXT NOT NULL DEFAULT '',      -- V23
    last_message_raw TEXT NOT NULL DEFAULT '',  -- V23
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
```

### routes（未使用）

V1 创建的预留表（`model_id` / `provider_id` / `priority` / `weight`），**代码中没有任何读写**，仅保留在 schema 与导出中。不要在此基础上扩展路由。

### settings / schema_version

```sql
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL);
```

`settings` 承载：`mcp_websearch` / `mcp_webfetch` / `mcp_notify`、`webfetch_jina_fallback`、`search_backend`、`audit_enabled`、`session_inject`、主题 / 色相（`theme` / `hue`）/ 语言（`locale`）、密钥轮询指针（`keypool_index_{provider_id}`）、`wallpaper_enabled`。旧键 `websearch_hijack` 由 V16 复制后保留、代码不再读取；`failover_enabled` 已不读（failover 强制常开，见 `gateway/server.rs`）。

### 索引

| 表 | 索引 |
|----|------|
| models / api_keys | `idx_models_provider`、`idx_keys_provider`、`idx_keys_status` |
| service_keys | `idx_service_keys_hash` |
| usage_log | `idx_usage_timestamp`、`idx_usage_provider`、`idx_usage_service_key` |
| routes | `idx_routes_model`、`idx_routes_provider` |
| plugins | `idx_plugins_provider`、`idx_plugins_status` |
| combo_members | `idx_combo_members_combo` |
| local_models | `idx_local_models_status` |
| conversations | `idx_conv_sk`、`idx_conv_updated(updated_at DESC)` |

管理 API：`GET /api/data/export`、`POST /api/data/import`（body `{sql}`）、`POST /api/data/reset`（`api/handlers/data.rs`）。

- **`export_sql()`**：表序 `providers → models → api_keys → service_keys → plugins → usage_log → settings → combos → combo_members → conversations`（共 10 张，FK 顺序），每表 `DROP TABLE IF EXISTS` + 原 `sqlite_master` DDL + 逐行 INSERT，单事务包裹，文本值转义单引号、Blob 记 NULL
- **`import_sql()`**：直接 `execute_batch`（替换式导入）
- **`reset_all_data()`**：按 `usage_log → conversations → plugins → service_keys → api_keys → models → combo_members → combos → providers → settings` 顺序 DELETE，保留 `schema_version`

## 输入 / 输出
- 不适用：本模块无此项。

## 约束
- 不适用：本模块无此项。

## 边界条件
- 不适用：本模块无此项。

## 验收标准
- 不适用：本模块无此项。

## 完成定义
- [x] V1→V25 增量迁移，按序执行、跳过已应用版本
- [x] 每个迁移与版本号同事务原子提交，失败回滚
- [x] 业务写入使用 `ON CONFLICT DO UPDATE`
- [x] `usage_log` 自包含快照（无外键）
- [x] 导出 / 导入 / 重置覆盖全部业务表（含 conversations）
- [x] 测试：`db/mod.rs` 内联测试覆盖空库全量迁移与 UPSERT 不级联删除
