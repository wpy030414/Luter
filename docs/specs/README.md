# SPECS — 模块规格索引

每个模块一份 spec，说明「要做什么、做到什么程度算完成」。规格定稿后不再保留悬而未决的讨论。

## 登记规则
- 一个模块一个文件，命名 `module-<kebab-name>.md`（如 `module-auth-gateway.md`）。
- 模块名必须与 `docs/ARCHITECTURE.md` 的「核心模块」表一致，不得出现架构里没有的模块。
- 状态变更时只改索引行的状态，不在 spec 正文里写进度日志。

## 条目
| 模块 | 文件 | 一句话职责 | 状态 | 更新日期 |
|------|------|------------|------|----------|
| 认证 | [module-auth.md](module-auth.md) | Service Key（Argon2）/ Provider Key（AES-256-GCM）双层认证 | 已实现 | 2026-11-06 |
| 密钥池 | [module-key-pool.md](module-key-pool.md) | Provider 密钥轮询、红黄绿健康状态、指针持久化 | 已实现 | 2026-11-06 |
| Token 配额 | [module-token-quota.md](module-token-quota.md) | 5h/7d 滚动窗口 token 限额检查 + `/v1/user/balance` | 已实现 | 2026-11-06 |
| 代理处理器 | [module-proxy-handler.md](module-proxy-handler.md) | `/v1/*` 五端点：限流、路由、failover、流式/非流式转发 | 已实现 | 2026-11-06 |
| 协议转换 | [module-protocol-translation.md](module-protocol-translation.md) | IR 中间表示层，三种协议双向转换 | 已实现 | 2026-11-06 |
| 用量统计 | [module-usage-tracking.md](module-usage-tracking.md) | usage_log 记录、聚合统计、请求日志分页与输出速度 | 已实现 | 2026-11-06 |
| 组合别名 | [module-combos.md](module-combos.md) | 多个模型别名捆绑为组合，按序回退 | 已实现 | 2026-11-06 |
| 数据库 | [module-database.md](module-database.md) | SQLite schema 与 V1→V25 迁移、导出/导入/重置 | 已实现 | 2026-11-06 |
| 对话审查 | [module-conversation-audit.md](module-conversation-audit.md) | `audit_enabled` 开启时按指纹归并存储对话 | 已实现 | 2026-11-06 |
| 插件系统 | [module-plugin-system.md](module-plugin-system.md) | WebSocket 委托供应商：占位凭证、伴生启动 | 已实现 | 2026-11-06 |
| MCP 工具 | [module-mcp-tools.md](module-mcp-tools.md) | `/mcp` 端点：web_search / web_fetch / notify + SSRF 守卫 | 已实现 | 2026-11-06 |
| 本地模型 | [module-local-models.md](module-local-models.md) | GGUF 导入 + llama-server 引擎生命周期 + Provider 注册 | 已实现 | 2026-11-06 |
| 局域网分发 | [module-lan-deploy.md](module-lan-deploy.md) | install 页面与分发链接、单 listener 公开/管理路径隔离 | 已实现 | 2026-11-06 |
| Claude FM | [module-claude-fm.md](module-claude-fm.md) | 内置氛围电台播放、墙钟种子、scene_t 引擎时钟 | 已实现 | 2026-11-06 |
| 像素壁纸 | [module-pixel-wallpaper.md](module-pixel-wallpaper.md) | FM 像素艺术劫持为桌面壁纸（Windows/macOS） | 已实现 | 2026-11-06 |
| 冷启动动画 | [module-cold-start-splash.md](module-cold-start-splash.md) | WebView 初始化期间的原生环形加载动画 | 已实现 | 2026-11-06 |
| 系统资源监控 | [module-system-monitor.md](module-system-monitor.md) | 本机 CPU / 内存实时占用（Tauri 命令） | 已实现 | 2026-11-06 |
