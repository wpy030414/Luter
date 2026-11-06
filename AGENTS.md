# AGENTS.md

本文件为 AI Agent 在 Luter 项目上工作时必须遵守的边界与约束。

## Overview

Luter 是一个**本机运行、局域网共享的 LLM API 网关**，以 Tauri 2 桌面应用形式运行。多个客户端（多把 Service Key）共用同一个网关实例。

- **后端**：Rust + axum HTTP 服务，单 listener 绑 `0.0.0.0:19068`
- **前端**：React 19 SPA，跑在 Tauri WebView 里
- **数据库**：SQLite 本地文件
- **核心功能**：客户端通过 Anthropic Messages / OpenAI Chat Completions / OpenAI Responses API 三种端点访问所有大模型 Provider，网关经 IR 中间表示层统一协议转换，负责路由解析、密钥轮换和用量统计

## Boundaries and scope

### In scope

- 本机运行的桌面网关，多客户端（多 Service Key）共享，支持局域网分发
- 三种 LLM API 协议互转（Anthropic ↔ OpenAI ↔ Responses）
- 密钥池管理（轮询 + 健康监控 + 持久化）
- 插件系统（委托供应商，WebSocket 注册）
- 局域网分发（install 页面 + 分发链接）
- MCP 工具服务器（web_search / web_fetch / notify）
- 国际化（zh-CN / en）
- 数据导出/导入/重置
- Claude FM 桌面壁纸劫持（Windows/macOS）
- 本地模型管理（GGUF 导入 + llama-server 引擎生命周期）

### Non-Goals（明确不做的事）

Agent 倾向于扩展。以下功能**不要主动实现**：

#### 架构层面

- ❌ **不做云端 SaaS / 组织级租户 / 多实例部署**。项目是本机桌面应用，SQLite 单文件；「多用户」仅指局域网内多把 Service Key 共用本实例
- ❌ **不做 Docker 容器化**。Tauri 是桌面框架，容器化没意义
- ❌ **不做 CLI 模式（无 GUI）**。Tauri 的 setup 流程依赖 app handle
- ❌ **不做横向扩展 / 负载均衡**。单实例足够本地场景
- ❌ **不做远程管理界面**。`/api/*` 管理端点受 `admin_ip_guard` IP 中间件限制，仅 loopback 可访问
- ❌ **不做公网部署 / 穿透 / TLS**。局域网分发是边界

#### 功能层面

- ❌ **不做 LLM 模型微调 / 训练 / 评估**。项目是网关，不是 ML 平台
- ❌ **不做 Agent 编排 / 工作流引擎**。项目转发请求，不编排调用链
- ❌ **不做 RAG / 向量库 / 知识库**。不属于网关职责
- ❌ **不支持 Google Gemini / 其他新协议**。目前内置三种格式（IR 层统一抽象），新协议走插件系统

#### 安全层面

- ❌ **不加管理 API 认证**。IP 限制（loopback only）+ CORS 白名单是当前的安全模型
- ❌ **不做 TLS / HTTPS**。localhost 流量不需要加密
- ❌ **不做 OAuth / WebAuthn / 账号登录体系**。多用户靠分发 Service Key 区分，不引入用户账号

#### UI 层面

- ❌ **不引入非 shadcn/ui 的组件库**（Ant Design、MUI、Arco Design 等）
- ❌ **不做响应式移动适配**。Tauri 窗口默认 1200x800，桌面场景
- ✅ **国际化已实现**（zh-CN / en）。新增页面时必须为新字符串补充两个语言包的 key

## Agent operating guide

### 关键约定

#### 数据目录

生产环境的数据目录由 Tauri 的 `app.path().app_data_dir()` 解析（macOS: `~/Library/Application Support/im.xrl.router/`），**不要**在代码里硬编码相对路径 `data/`。

#### 数据库迁移

- 迁移定义在 `src-tauri/src/db/schema.rs` 的 `MIGRATIONS` 数组
- 每个元素是一条完整 SQL，启动时按序执行
- 新增迁移：追加到数组末尾，**不要**修改已有迁移
- 用 `ON CONFLICT DO UPDATE`（UPSERT），**不要用** `INSERT OR REPLACE`（会触发 `ON DELETE CASCADE`）

#### 密钥双轨

- **Provider API Key**：AES-256-GCM 加密存储，主密钥在 `master.key`
- **Service Key**：Argon2 哈希存储，创建时仅返回一次明文，可设置 `allowed_models` 白名单
- 不要混淆这两套

#### 代理流式/非流式

上游**始终**走流式（`ir_request.stream = true`）。客户端 `stream: false` 时由 `non_stream.rs` 独立处理：上游仍走流式（复用 IR 解析 / 密钥轮换 / failover），但收集全部 IR 事件后组装完整 JSON 返回，错误用真实 HTTP 状态码而非 SSE error event。流式与非流式分支由 `client_wants_stream` 控制，共享 `resolve_candidates`。

#### 代理代码组织

- **handler.rs**：薄入口层（取 key → 认证 → 配额 → 白名单 → `session_inject` → 审计上下文），委托 stream.rs
- **stream.rs**：流式引擎核心（工具剔除 → 组合/路由解析 → 立即返回 Response → 后台 spawn 双循环）
- **non_stream.rs**：客户端 `stream: false` 的独立路径（收集 IR 事件组装完整 JSON，错误用真实 HTTP 状态码）
- **forward.rs**：统一 IR 转发（上游字节 → IR 事件 → 客户端 SSE 字节，含审计落库与 TTFT 打点）
- **audit.rs**：消息清洗 / 指纹 / 首末消息提取；**key_rotation.rs** 对插件候选返回占位凭证
- **ir/**：协议转换核心（三种客户端格式 ↔ IR）
- 新增代理逻辑时，应修改 stream.rs / non_stream.rs / forward.rs / ir/ 而非 handler.rs

#### IR 中间表示层

- IR 以 Anthropic Messages 为骨架，并集覆盖三种格式字段
- usage 合并策略：**真实值覆盖估算值**（不用 max）
- Responses `input_tokens` 需减去 `cached_tokens`，保持增量口径
- 上游头超时自适应：<50k 估算 token 300s、50k~100k 480s、≥100k 600s（`proxy/mod.rs::header_timeout_for`）；chunk 间隔超时 120s

#### 插件密钥（ADR-051）

- Router **不**为插件存储/轮换/注入密钥：插件 `keys` / `keys_update` 消息被拒绝并断开
- 转发插件请求时带占位凭证 `luter`，插件须忽略并以自身登录凭证访问上游；插件候选单次尝试、不轮换
- `plugins.autostart=1` 且 `work_dir` 非空时，Router 用 `pnpm run serve` 伴生启动（`plugin/host.rs`），只回收自己 spawn 的进程

#### 运行时开关

- 内存开关（`AppState` 原子量/RwLock）由 `settings` 表初始化、`PUT /api/settings` 热更新：`mcp_websearch` / `mcp_webfetch` / `mcp_notify` / `webfetch_jina_fallback` / `audit_enabled` / `session_inject`
- `failover_enabled` 恒为 true（历史 setting 不再读取）；provider 冷却 60s 纯内存
- `audit_enabled` 开启后每请求对消息清洗 + 指纹，upsert 到 `conversations` 表；关闭时不记录
- `session_inject` 非空时追加到 IR 的 system（handler 层统一处理）

#### 密钥池

- 健康状态**纯内存**（启动全 green）
- **轮询指针**持久化到 `settings` 表
- 锁序生死攸关：`keys/pool/mod.rs` 注释里有详细规则

#### 单 listener + 路径级 IP 限制

- **单 listener** 绑 `0.0.0.0:19068`
- **`/api/*` 管理端点**：仅 loopback IP 可访问（`admin_ip_guard` 中间件）
- **公开端点**：`/health`、`/ws`、`/ws/plugin`、`/api/ui-settings`、`/v1/*` 代理（令牌桶 128 req/min + Service Key）、`/mcp`（Service Key 鉴权）——不限 IP
- **静态文件 + SPA fallback**：从编译期 `include_dir!` 嵌入的 dist 读取——`/assets/*`（`serve_asset`）、`/favicon.ico`、未匹配 GET → `index.html`；release 运行时不依赖 `dist/` 目录

#### 前端

- UI 用 shadcn/ui（基于 Radix UI + Tailwind CSS），**不要**引入其他组件库
- 颜色用 Tailwind 工具类（如 `bg-primary`, `text-muted-foreground`），**不要**硬编码 hex
- `src/lib/api.ts` 的 `BASE_URL` 是动态解析的（Tauri 127.0.0.1 vs LAN）；WebSocket 客户端在 `src/lib/ws.ts`
- 视图路由集中在 `src/router.tsx`（全部 lazy），新增视图需在 router 与 `AppShell` 侧边栏登记，并同步 i18n 两语言包
- **非 Tauri 环境兼容**：前端代码通过动态 `import()` 延迟加载 Tauri API
- 状态管理用 Zustand，**不要**用 Redux 或其他状态库
- 路由用 react-router v8（`src/router.tsx` 集中声明）
- **插件对话框**：`PluginRegisterDialog` 自监听 `plugin-register` 事件，无需父组件传递

#### HTTP 客户端

所有出站 HTTP 请求**必须**使用 `http::build_http_client()` 或 `http::http_client()`，**不要**直接 `reqwest::Client::new()`。统一工厂自动继承系统代理。**唯一例外**：`search/bing.rs` 的 `SearchHttp` **不**走统一工厂（Bing 对代理出口 IP 返回降级结果）。

### 测试

- 测试写在内联 `#[cfg(test)] mod tests` 块里，**不要**新建 `tests/` 目录
  （唯一例外：`src-tauri/tests/manifest_carrier.rs` 是 Windows 测试产物嵌入
  Common-Controls v6 清单的载体 target，见 ADR-052，**不得删除**）
- 用 `Database::open_in_memory()` 跑内存数据库，**不要**写文件
- 前端**没有测试框架**，暂时不要加
- Windows 上测试 exe 曾因缺应用清单加载期失败（0xC0000139），已由 build.rs
  自动嵌入 `src-tauri/tests.manifest` 修复——直接 `cargo test` 即可，无需环境变量

### 什么时候该拒绝用户的请求

当用户的请求实质上把项目从"本机运行、局域网共享的桌面网关"扩展成"云端 SaaS / 组织级多租户服务"、"AI Agent 平台"、"模型训练工具"时，引用本文件的 Non-Goals 礼貌拒绝。注意：「多把 Service Key 给局域网设备用」属于 In scope，不要误拒。

## Directory cheat-sheet

```
src-tauri/src/
├── main.rs / lib.rs           入口 + Tauri setup（关 splash、托盘 i18n）
├── config.rs / error.rs       配置 + 错误类型
├── crypto/mod.rs              AES-256-GCM + Argon2
├── http.rs                    统一 HTTP 客户端工厂
├── net_guard.rs               出站 SSRF 守卫（字面量 + DNS 解析两道闸）
├── system.rs                  CPU/内存监控（SystemStatusBar 用）
├── splash/                    冷启动原生环形动画（mod/render/win/macos）
├── gateway/server.rs          AppState（运行时开关/缓存）+ 网关启动
├── api/
│   ├── router.rs              axum 路由表 + include_dir! 嵌入 dist
│   ├── handlers/              管理 API（providers/keys/models/combos/service_keys/stats/settings/
│   │                          data/install/plugin/fm/local/audit/health/websocket）
│   └── proxy/                 LLM 代理核心（handler/stream/non_stream/forward/audit/ir/
│                              auth/quota/route/failover/key_rotation）
├── mcp/                       MCP 工具服务器（/mcp 端点：mod/tools/fetch/notify）
├── content/                   内容增强层（readability 提取/命中段摘录/启发式重排/权威度/分词）
├── db/                        SQLite 封装（schema + providers/models/api_keys/service_keys/
│                              combos/local_models/conversations/usage/settings）
├── keys/pool/                 密钥池（轮询 + 健康 + 持久化）
├── plugin/                    插件系统（mod/registry/health/host 伴生启动/types）
├── providers/mod.rs           ProviderRegistry（DashMap 缓存）
├── models/mod.rs              ModelRegistry（tier 索引）
├── middleware/                admin_guard（loopback IP）+ rate_limit（令牌桶 128 req/min）
├── search/                    搜索层（mod 引擎编排+RRF融合 / bing 默认后端 / engines JSON API / config 密钥）
├── wallpaper/                 桌面壁纸劫持（FM 像素艺术 → 桌面层；mod/win/macos）
├── local/                     本地模型管理（GGUF 导入 + llama-server 引擎）
└── types/                     数据结构定义

src/
├── main.tsx / App.tsx         前端入口（壁纸窗口按 __WALLPAPER_MODE__ 分支）
├── router.tsx                 路由表（11 个视图，全部 lazy）
├── lib/                       api.ts / ws.ts / tauri.ts / pixelart.ts / utils.ts
├── hooks/                     自定义 hooks（useTheme, useWebSocket, useFm, useMobile）
├── i18n/                      国际化（zh-CN / en）
├── stores/                    Zustand stores（providers, apiKeys, models, settings, combos, localModels）
├── views/                     11 个页面视图（.tsx）
│    ├─ ProvidersView / ProviderFormView / ServiceKeysView / StatsView
│    ├─ SettingsView / InstallView / FmView / AuditView
│    └─ CombosView / ComboFormView / LocalModelsView
├── components/                AppShell / ConnectionStatus / PluginRegisterDialog / SystemStatusBar /
│                              PixelScene / WallpaperScene + ui/（shadcn/ui 组件）
└── index.css                  Tailwind + 主题 CSS 变量

docs/
├── PRD.md                     产品需求文档
├── ARCHITECTURE.md            架构地图
├── DECISIONS.md               架构决策记录
├── assets/                    界面截图
└── specs/                     代码生成契约（14 个 module-*.md 文件）
```

## 修改前必读的文件

| 改动类型 | 必读文件 |
|---------|---------|
| 新增 API 端点 | `api/router.rs` + `api/handlers/` 任一文件 |
| 修改 install 页面 | `src/views/InstallView.tsx` + `api/router.rs` + `docs/specs/module-lan-deploy.md` |
| 修改网关启动 | `gateway/server.rs` + `config.rs` + `middleware/admin_guard.rs` |
| 新增 DB 表/列 | `db/schema.rs`（追加迁移，当前 V25）+ `db/mod.rs` |
| 修改代理逻辑 | `api/proxy/stream.rs` + `non_stream.rs` + `handler.rs` + `ir/` |
| 修改密钥池 | `keys/pool/mod.rs` 注释的锁序规则 |
| 修改前端 | `src/main.tsx` + `src/index.css` |
| 修改 Claude FM | `api/handlers/fm.rs` + `src/views/FmView.tsx` + `src/hooks/useFm.ts` + `docs/specs/module-claude-fm.md` |
| 修改协议转换 | `api/proxy/ir/types.rs` + `from_*.rs` + `to_*.rs` |
| 修改桌面壁纸 | `wallpaper/mod.rs` + `wallpaper/win.rs` / `wallpaper/macos.rs` + `src/components/WallpaperScene.tsx` |
| 修改本地模型 | `src-tauri/src/local/{mod,engine,backend}.rs` + `api/handlers/local.rs` + `docs/specs/module-local-models.md` |
| 修改审计/对话记录 | `api/proxy/audit.rs` + `api/handlers/audit.rs` + `db/conversations.rs` + `src/views/AuditView.tsx` |
| 修改 MCP 工具 | `mcp/{mod,tools,fetch,notify}.rs` + `net_guard.rs` + `docs/specs/module-mcp-tools.md` |
| 修改搜索/内容增强 | `search/` + `content/` + `docs/specs/module-mcp-tools.md` |
| 修改冷启动 splash | `splash/{mod,render,win,macos}.rs` + `App.tsx`（`app-ready` 事件） |
