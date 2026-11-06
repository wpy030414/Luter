# Spec: 密钥池管理

## 要构建什么
管理多个 Provider 的多个 API Key，实现轮询、健康状态跟踪、持久化。

## 行为
### KeyPool

```rust
pub struct KeyPool {
    keys: Arc<RwLock<HashMap<String, Vec<KeyEntry>>>>,
    current_index: Arc<RwLock<HashMap<String, usize>>>,
    database: Arc<RwLock<Option<Database>>>,
    key_stats_tx: tokio::sync::broadcast::Sender<serde_json::Value>,
}
```

### KeyEntry

```rust
pub struct KeyEntry {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    pub key_hash: String,   // 池内存的是**解密后的明文 key**（匹配上游用）
    pub key_masked: String,
    pub status: KeyStatus,
    pub last_error_time: Option<i64>,
    pub total_requests: u64,
    pub total_tokens: u64,
}
```

### KeyStatus

```rust
pub enum KeyStatus {
    Green,   // 正常
    Yellow,  // 配额低（402/429），5分钟冷却
    Red,     // 失效（401/403），永久跳过
    Unknown, // 未验证，视为 Green
}
```

## 输入 / 输出
### 选取下一个 Key（`rotation.rs::get_next_key`）

```rust
pub fn get_next_key(&self, provider_id: &str) -> Result<KeyEntry>
```

**返回**:
- `Ok(KeyEntry)` 可用 key，指针推进到 `idx+1`
- `Err(KeyNotFound)` provider 不在池中
- `Err(NoAvailableKeys)` 全部 Red，或全部 Yellow 且冷却未到期

代理层入口是 `key_rotation.rs::pick_key_for()`：插件委托 provider 直接返回占位密钥 `"luter"`（不查池，内层只尝试 1 次），其余走 `get_next_key`。

### 更新健康状态

```rust
pub fn mark_key_invalid(&self, provider_id: &str, key_hash: &str) -> Result<()>
pub fn mark_key_low_quota(&self, provider_id: &str, key_hash: &str) -> Result<()>
pub fn record_key_success(&self, provider_id: &str, key_hash: &str, tokens_used: i64) -> Result<()>
```

**注意**: 参数名为 `key_hash`，实际是解密后的明文 key（与 `KeyEntry.key_hash` 匹配）。状态映射由 `key_rotation::update_key_health` 完成：401/403 → Red，402/429 → Yellow，2xx → Green 并累加 `total_requests/total_tokens`（写入 DB 的 `api_keys` 计数列）；5xx 与 400/404 等不改健康度。

### 轮询逻辑

1. 从该 provider 的 `current_index` 开始，最多扫描 `len` 个位置（round-robin）
2. `Red` 永不使用；`Yellow` 在 `now - last_error_time >= 300s` 后恢复可用；`Green/Unknown` 总是可用
3. 返回首个可用 key，`current_index = (idx + 1) % len`
4. **先释放 KeyPool 锁，再持久化指针**（见锁序）

### 持久化

```sql
-- settings 表
INSERT INTO settings (key, value) VALUES ('keypool_index_provider123', '5')
ON CONFLICT(key) DO UPDATE SET value = excluded.value;
```

## 约束
1. **纯内存状态**: `KeyStatus` 仅存内存，启动时全部初始化为 `Green`
2. **指针持久化**: `current_index` 持久化到 `settings` 表
3. **冷却期**: `Yellow` 状态 5 分钟后自动恢复
4. **轮询公平**: 所有 key 轮询一遍后才会重复
5. **锁顺序**（`keys/pool/mod.rs` 顶部注释，生死攸关）：
   - 加载：先收 DB 数据 → 释放 DB 锁 → 再拿 KeyPool 锁
   - 选取 / 记录：KeyPool 锁内只算结果 → 释放后再持久化指针
   - 健康标记：write lock 释放后再广播（broadcast 内部要读锁）
   违反任一都会与代理路径形成 ABBA 死锁（插件 keys_update 并发时触发）
6. **运行时同步**：`add_key` / `rename_key` / 删除 key 同步内存池并修正指针，不重建整个池

## 边界条件
| 场景 | 行为 |
|------|------|
| 所有 key 都 Red | 返回 `NoAvailableKeys` |
| 所有 key 都 Yellow 且冷却中 | 返回 `NoAvailableKeys` |
| Provider 不存在 | 返回 `KeyNotFound`（代理层映射为无可用密钥） |
| 持久化失败 | 记录 warn 日志，不影响运行 |

## 验收标准
1. **单元测试**: 轮询逻辑、状态转换、冷却期
2. **集成测试**: 持久化 + 恢复、多 Provider 并发
3. **边界测试**: 所有 key 都 Red、指针越界、持久化失败

## 完成定义
- `src-tauri/src/keys/pool/mod.rs` - 结构体定义
- `src-tauri/src/keys/pool/types.rs` - 类型定义
- `src-tauri/src/keys/pool/rotation.rs` - 轮询逻辑
- `src-tauri/src/keys/pool/health.rs` - 健康状态管理
- `src-tauri/src/keys/pool/persistence.rs` - 加载与持久化
- `src-tauri/src/api/proxy/key_rotation.rs` - `pick_key_for` + `update_key_health`（状态码 → 健康）

- [x] 轮询选取 key（跳过 Red/Yellow）
- [x] 健康状态管理（Green/Yellow/Red）
- [x] 冷却期（Yellow 5分钟）
- [x] 指针持久化（重启后继续）
- [x] 锁顺序正确（无死锁）
- [x] 通过所有单元测试
