# ARCHITECTURE — Luter

> 架构地图：描述稳定的结构关系，让 Agent 理解系统边界。

## 1. 系统总览

Luter 是一个 **Tauri 2 桌面应用**，内部跑着一个 Rust axum HTTP 服务（`0.0.0.0:19068`），前端 React 19 SPA 运行在 Tauri WebView 中。

```
┌─── Tauri 桌面应用 ───────────────────────────────────────────────────────┐
│  WebView (React 19 SPA)              Rust 后端 (axum + tokio)            │
│  ┌───────────────────┐               ┌──────────────────────────────┐   │
│  │ 11 个视图 (router)│  HTTP         │ /api/* 管理 API (loopback)   │   │
│  │ AppShell 侧边栏   │──────────────▶│ /health  /ws  /ws/plugin     │   │
│  │ SystemStatusBar   │  WebSocket    │ /mcp (公开 + Service Key)    │   │
│  └───────────────────┘══════════════▶│ /assets/*  SPA fallback      │   │
│                                      └──────────────────────────────┘   │
│  冷启动: splash/ 原生环形动画（tiny-skia）；React 首帧后 `app-ready` 关闭  │
└───────────────────────────────────────────────────────────────────────────┘

外部 LLM 客户端 (Claude Code / 其他)
    │  x-api-key: xrl-xxxx (Service Key)
    │  POST /v1/messages / /v1/chat/completions / /v1/responses
    ▼
┌───────────────────────────────────────────────────────────────────────────┐
│               单 listener 绑 0.0.0.0:19068 (host=0.0.0.0 时优先 [::] 双栈) │
│                                                                           │
│  公开路径（不限 IP）                    /api/* 管理路径（admin_ip_guard）  │
│  ├─ /v1/* 代理（token bucket 限流）     ├─ providers/models/combos/keys    │
│  ├─ /api/ui-settings                   ├─ service-keys, stats, settings    │
│  ├─ /health  /  /ws  /ws/plugin         ├─ audit（对话审查）               │
│  ├─ /mcp（Service Key 鉴权）            ├─ data/export,import,reset         │
│  ├─ /favicon.ico                        ├─ install/local-ip                │
│  ├─ /assets/* + SPA fallback            └─ local/*（GGUF + 引擎生命周期）  │
│  └─ （以上静态资源均从 include_dir! 编译期嵌入的 dist/ 读取）              │
│                                                                           │
│  请求入口 → 限流 → 认证 → 配额 → IR → 路由解析 → 密钥选取 → 转发          │
│                                                                           │
│       ┌──────────────────────────────────────────────────────────┐        │
│       │ 所有上游: 统一 IR 转发 (forward_stream_ir)               │        │
│       │   上游字节 → IR 事件 → 客户端 SSE 字节                    │        │
│       │ 客户端 stream:false → non_stream.rs 独立收集路径           │        │
│       │ 插件上游: 发占位凭证 "luter"，插件自持真实凭证            │        │
│       └──────────────────────────────────────────────────────────┘        │
└───────────────────────────────────────────────────────────────────────────┘
```

## 2. 后端模块依赖图

```
main.rs
  └─ lib.rs (Tauri setup + 启动流程 + 托盘 i18n + app-ready 关 splash)
       ├─ config.rs          环境变量 → Config
       ├─ error.rs           AppError 统一错误类型 (thiserror)
       ├─ crypto/mod.rs      AES-256-GCM + Argon2 + master key
       ├─ http.rs            统一 HTTP 客户端工厂（系统代理自动继承）
       ├─ db/                SQLite 封装（WAL）
       │    ├─ mod.rs / schema.rs   MIGRATIONS 数组 (V1→V25，共 25 条)
       │    ├─ providers.rs / models.rs / api_keys.rs / service_keys.rs
       │    ├─ combos.rs     组合别名 + 成员
       │    ├─ local_models.rs 本地模型注册
       │    ├─ conversations.rs 对话审查（audit）存储
       │    ├─ usage.rs      usage_log 查询 + 统计聚合 + 密钥配额聚合
       │    └─ settings.rs   key-value 设置 + 导出/导入/重置
       ├─ gateway/server.rs  AppState（全部运行时开关/缓存）+ start_gateway + CORS
       └─ api/
            ├─ router.rs      axum 路由表 + include_dir! 嵌入 dist（serve_asset/spa_fallback）
            ├─ handlers/      管理 API（health/providers/keys/models/combos/service_keys/
            │                 stats/settings/data/install/plugin/fm/local/audit/websocket）
            └─ proxy/         LLM 代理核心
                 ├─ handler.rs     薄入口: 取 key → 认证 → 配额 → 白名单 → session_inject
                 ├─ stream.rs      流式引擎: 工具剔除 → combo/route 解析 → 立即返回 Response
                 ├─ non_stream.rs  客户端 stream:false 的独立实现（收集 IR → 完整 JSON + 真实 HTTP 错误码）
                 ├─ forward.rs     统一 IR 转发（含 audit 落库、TTFT 打点、usage 累积）
                 ├─ audit.rs       消息清洗 / SHA256 指纹 / 首末消息提取
                 ├─ auth.rs        Service Key 验证（/v1/* 与 /mcp 共用）
                 ├─ quota.rs       5h/7d token 配额 + /v1/user/balance
                 ├─ route.rs       display_name→Route; 组合展开; 候选列表; 插件 URL 实时覆盖
                 ├─ failover.rs    provider 级冷却表（60s，纯内存）
                 ├─ key_rotation.rs 密钥选取 + 状态码健康反馈 + 插件占位凭证
                 └─ ir/            types / from_{messages,chat_completions,responses} /
                                   to_{messages,chat_completions,responses} / usage

独立模块：
  ├─ mcp/               MCP 工具服务器（/mcp，Streamable HTTP）
  │    ├─ mod.rs        Service Key 鉴权 + NeverSessionManager 无状态服务
  │    ├─ tools.rs      手写 ServerHandler（三种工具，按开关实时过滤）
  │    ├─ fetch.rs      静态优先抓取 + WebView 渲染池 + 缓存 + 分段读取 + Jina 兜底
  │    └─ notify.rs     桌面通知（tauri-plugin-notification）
  ├─ search/            搜索层：mod(RRF 融合+缓存) / bing(默认后端) / engines(JSON API) / config(密钥)
  ├─ content/           内容增强：readability(正文提取) / summary(命中段摘录) / rank(0~3 分) /
  │                     authority(域名分级+黑名单) / text(分词分句) — MCP 搜索抓取共用
  ├─ net_guard.rs       SSRF 守卫：字面量检查 + DNS 解析后检查（两道闸）
  ├─ providers/mod.rs   ProviderRegistry（DashMap 缓存，id→Provider）
  ├─ models/mod.rs      ModelRegistry（DashMap + tier 索引）
  ├─ keys/pool/         KeyPool（rotation/health/persistence/types）
  ├─ plugin/            PluginManager（registry/health/host 伴生启动/types）
  ├─ middleware/        admin_guard（loopback IP）/ rate_limit（令牌桶 128 req/min）
  ├─ system.rs          SystemMonitor（sysinfo：CPU/内存，供 SystemStatusBar）
  ├─ splash/            冷启动原生环形动画: mod / render(tiny-skia) / win(DComp) / macos(CALayer)
  ├─ wallpaper/         桌面壁纸劫持（mod / win / macos）
  ├─ local/             本地模型: mod(LocalManager) / engine(llama-server) / backend(GPU 检测)
  └─ types/             Provider/Model 等结构体 + key.rs(KeyStatus)
```

## 3. 数据流：一次 LLM 请求的完整生命周期

```
客户端 POST /v1/messages / /v1/chat/completions / /v1/responses
  │
  ▼
[1] rate_limit_middleware ──── 令牌桶 (per Service Key, 128 req/min)；请求体上限 64MiB
  │
  ▼
[2] handler.rs ──── authenticate_and_stream
  │  取 key（messages: x-api-key 优先；其余: Bearer 优先）
  │  客户端格式 → IR → verify_service_key → check_quota(5h/7d) → allowed_models
  │  保存 client_wants_stream，强制 ir.stream = true
  │  session_inject：非空时追加到 system（Text 拼接 / Blocks 追加 / None 新建）
  │  估算输入 token → header_timeout_secs；audit_enabled 时构建 AuditCapture
  │
  ▼
[3] stream.rs proxy_stream ──── 先做工具剔除（按开关实况）
  │  mcp_websearch → strip_search_tools
  │  mcp_webfetch  → strip_fetch_tools
  │  mcp_notify    → strip_notify_tools + inject_notify_hint
  │
  ├─ client_wants_stream = false → non_stream.rs::proxy_non_stream
  │    双循环同流式；收集全部 IR 事件 → 组完整 JSON；错误用真实 HTTP 状态码
  │    完成后 audit 落库 → usage_log
  │
  ▼ (流式)
[4] resolve_candidates ──── 优先级：组合 > 本地模型(tier=local) > sort_order ASC, created_at ASC
  │  命中 enabled 组合 → 按成员 position 逐个展开；组合强制 failover
  │  failover_enabled 常开（true）→ resolve_route_candidates；否则仅主 provider
  │  不可解析 → 400（尚未发送任何字节）
  │  上下文超限：仅 warn（估算 chars/4 > context_window），不阻断
  │
  ▼
[5] 立即返回 SSE Response（先发 `:keepalive`），后台 spawn：
  │  15s keepalive 心跳 + oneshot 取消
  │
  ▼
[6] 双层重试循环（外层 provider 候选，内层 key）
  │  failover 时跳过冷却中 provider（60s 冷却）
  │  pick_key_for：插件候选返回占位 "luter"、内层仅 1 次；常规走池 round-robin 跳过 Red/Yellow
  │  IR → 上游格式，model=real_model_id, stream=true（Chat 附 stream_options.include_usage）
  │  头超时 header_timeout_for：<50k→300s，50k~100k→480s，≥100k→600s
  │  401/402/403/429 → Mark key，换 key；400+quota 体 → 按 402 换 key；普通 400 → 透传
  │  5xx / 网络错误 / 头超时 → failover 时标记冷却切下一 provider
  │  200 + 流内密钥级错误（SSE error / 非 SSE JSON 错误体）→ 按状态码记健康，换 key
  │  2xx → mark_provider_ok，进入转发
  │
  ▼
[7] forward_stream_ir（forward.rs）── 上游 SSE → IR 事件 → 客户端格式 SSE
  │  chunk 间隔超时 120s；TTFT 取首个非空内容增量；usage 真实值覆盖估算
  │  流结束后（audit_enabled）→ conversations upsert（清洗消息 + 助手回复 + 首末消息）
  │  失败/耗尽 → SSE error event + usage_log(success=0)
  │
  ▼
[8] usage_log 落库（provider/key/service_key 快照、token、latency、first_token_ms）
  │
  ▼
SSE 流返回客户端
```

## 4. 前端架构

```
src/
├── main.tsx          入口：__WALLPAPER_MODE__ → WallpaperScene；否则 initI18n/initTheme → App
├── App.tsx           RouterProvider + wsClient.connect + Tauri 事件（app-ready 关 splash）
├── router.tsx        createBrowserRouter：11 个视图全部 lazy 加载
├── index.css         Tailwind + CSS 变量（主题色）
├── i18n/             index.tsx(useT) + zh-CN.ts / en.ts
├── hooks/            useTheme / useWebSocket / useFm / useMobile
├── lib/              api.ts(REST, 动态 BASE_URL) / ws.ts(自动重连 3s) / tauri.ts /
│                     pixelart.ts(确定性 160×90 像素风景) / utils.ts
├── stores/ (Zustand) providers / apiKeys / models / settings / combos / localModels
├── views/ (11)
│    ProvidersView / ProviderFormView / LocalModelsView / CombosView / ComboFormView
│    ServiceKeysView / StatsView / AuditView / SettingsView / FmView / InstallView(/install)
└── components/
     AppShell.tsx              侧边栏导航 + Windows 自定义窗口控制
     ConnectionStatus.tsx      离线横幅
     PluginRegisterDialog.tsx  插件注册确认（自监听事件）
     SystemStatusBar.tsx       CPU/内存实时状态（Tauri invoke）
     PixelScene.tsx / WallpaperScene.tsx   FM 像素画布 / 壁纸窗口入口
     ui/                       shadcn/ui 组件
```

### 4.1 Claude FM 桌面壁纸（wallpaper 引擎）

FM 像素艺术可被劫持为桌面壁纸（关闭即恢复原壁纸）：

- **Windows（透明 WebView + 社区插件，见 ADR-002）**：`tauri-plugin-desktop-underlay`
  的 `set_desktop_underlay` 把透明 WebView 窗口 SetParent 进壁纸 WorkerW。
  WebView2 内容经 DWM 视觉合成上屏，是桌面 WorkerW 层唯一可靠渲染路径。
  点击穿透为 `WS_EX_TRANSPARENT`（禁 `WS_EX_LAYERED`），见 `wallpaper/win.rs`。
- **macOS（WebView 方案，ADR-001）**：动态创建第二个 WebviewWindow
  （`initialization_script` 注入 `__WALLPAPER_MODE__`，前端分支渲染
  `WallpaperScene`——黑底全屏像素、无按钮/歌曲信息），
  `NSWindow.setLevel(kCGDesktopIconWindowLevel)` + `orderFront:` 呈现，
  `setIgnoresMouseEvents` 穿透。

**同步机制**：像素场景动画时钟为**引擎权威**（`fm.rs` 的 `scene_t`，仅
播放时按真实流逝累计、暂停冻结）；主窗口前端（`fm_scene_t` 采样）与
壁纸渲染（同一共享状态）取同一值——两处画面严格同步。seed（曲目 index）
与 playing 沿用既有全窗口广播事件（`fm-meta` / `fm-state-changed`）。

**生命周期**：勾选态持久化于 DB `settings.wallpaper_enabled`，应用重启
（含 `--minimized` 静默启动）延迟 2s 惰性恢复（+重试，避开主窗口 WebView2
初始化竞态）；进程退出由 OS 销毁子窗口，原壁纸自动恢复。

### 4.2 Windows 自定义窗口控制

Windows 平台去除原生标题栏，自定义红绿灯风格窗口控制按钮（关闭/最小化/最大化），
位于侧边栏左上角。拖拽区域高度 40px（macOS 保持 28px）。

## 5. 存储架构

```
┌─ SQLite (WAL 模式) ─────────────────────────────────┐
│  providers        供应商注册表 (含 sort_order)        │
│  models           模型定义 (display_name 即客户端别名) │
│  combos / combo_members   组合别名（member_alias 软引用）│
│  api_keys         Provider Key (AES-256-GCM 加密)     │
│  service_keys     客户端 Key (Argon2) + allowed_models │
│                   + quota_5h / quota_7d                │
│  usage_log        请求日志（自包含快照，无 FK，含 TTFT）│
│  conversations    对话审查（fingerprint UNIQUE，按请求 │
│                   upsert；仅 audit_enabled 时写入）     │
│  settings         key-value 设置 + 轮询指针 + 开关     │
│  plugins          插件注册记录 (work_dir / autostart)  │
│  local_models     本地模型注册 (GGUF 引擎参数)         │
│  routes           预留空表（V1 建表，代码未使用）       │
│  schema_version   迁移版本跟踪                         │
└─────────────────────────────────────────────────────┘

┌─ 文件系统（data_dir） ───────────────────────────────┐
│  master.key       AES-256-GCM 主密钥 (0600)          │
│  xrl-router.db    SQLite；models/ engines/ 本地模型与引擎缓存 │
│  dist/            仅构建期：release 已 include_dir!  │
│                   嵌入二进制，运行时不读该目录         │
└─────────────────────────────────────────────────────┘

┌─ 纯内存（AppState） ────────────────────────────────┐
│  KeyPool 健康 / ProviderRegistry / ModelRegistry      │
│  RateLimiter 令牌桶 / provider_cooldowns 冷却表       │
│  PluginManager 连接态 / LocalManager 引擎句柄         │
│  开关: mcp_websearch / mcp_webfetch / mcp_notify /    │
│        webfetch_jina_fallback / audit_enabled /       │
│        failover_enabled(恒 true)                      │
│  session_inject (RwLock<String>) / search_config      │
│  http_client（共享连接池）/ search_http（Bing 直连）   │
└─────────────────────────────────────────────────────┘
```

## 6. 安全边界

```
                    外部客户端 (本机 + 局域网设备)
                        │
            ┌───────────┼───────────────┐
            │  /v1/*：令牌桶限流 + Service Key 认证(Argon2) │
            │  /mcp：Service Key 认证                       │
            │  /api/*：admin_ip_guard 限 loopback            │
            └───────────┼───────────────┘
                        │
              /v1/messages, /v1/chat/completions, /v1/responses, /v1/models, /v1/user/balance
              (5h/7d token 配额)
                        │
                        ▼
                    Luter
                        │
            ┌───────────┼───────────────┐
            │    Provider API Key       │
            │    (AES-256-GCM 解密)     │
            │    MCP 抓取经 net_guard   │
            │    SSRF 两道闸拦截        │
            └───────────┼───────────────┘
                        │
                        ▼
                    上游 LLM API / 外部网页
```

### 6.1 单 listener + 路径级 IP 限制

| 路径类型 | 绑定 | 路由 | 访问方 |
|----------|------|------|--------|
| 公开 | `0.0.0.0:19068` | `/health`、`/ws`、`/ws/plugin`、`/api/ui-settings`、`/v1/*`（限流+认证）、`/mcp`（Service Key）、`/favicon.ico`、`/assets/*`、SPA fallback | Tauri WebView、本机客户端、局域网设备 |
| 管理（IP 限制） | 同上（`admin_ip_guard` 限 loopback，返回 403） | `/api/*`（CRUD、service-keys、stats、settings、audit、data/*、install/local-ip、plugins、local/*） | 仅本机 |

### 6.2 文件系统权限

前端 WebView 的文件访问仅限**导出/导入对话框选中的文件**：路径白名单（`$HOME/**`、`$DOWNLOAD/**`、`$DOCUMENT/**`、`$DESKTOP/**`、`$APPDATA/**`）。

## 7. 关键设计约束

| 约束 | 原因 |
|------|------|
| 上游始终走流式 | 复用 IR/轮换/failover；非流式由 `non_stream.rs` 独立收集并组装（错误走真实 HTTP 码） |
| SQLite 单文件 | 本机规模（少量局域网客户端）足够，WAL 模式缓解并发 |
| 密钥状态纯内存 | 减少 DB 写入开销，启动全 green 可接受 |
| 轮询指针持久化 | 重启后跳过已失效的 key |
| usage_log 无 FK | 删除 Provider/Model/Key 不影响历史统计 |
| 管理 API 无认证 | `admin_ip_guard` IP 中间件限 loopback 是安全模型 |
| IR 真实值覆盖估算值 | `chars/4` 估算值偏大，max 合并会永久压住真实值 |
| 上下文超限预警而非硬拒绝 | 估算口径偏保守，硬拒绝会阻断客户端 auto-compact |
| 工具按开关实时剔除 | websearch/webfetch/notify 开关开启时移除请求自带同类工具，防上游官方实现生效 |
| session_inject 追加系统提示 | 统一注入到 system 尾部，对三种客户端格式一致生效 |
| 审计可开关（默认关） | 开启后每次请求做清洗/指纹/upsert，含隐私与写放大成本 |
| 本地模型优先于云端 | 路由 ORDER BY tier='local' 优先，其次 sort_order，保证私有化优先 |
| failover 常开 | 组合强制回退；普通别名同样受益；冷却纯内存不持久化 |
| WebFetch 静态优先、按需渲染 | 普通页面无需 JS；内置 WebView 池只服务 SPA/反爬兜底 |
| Jina Reader 兜底可开关 | 静态+渲染均失败时的第三方兜底，默认关 |
| web_fetch 拦截内网地址 | 管理 API 只信 loopback，提示注入可借此读本地数据 (DECISIONS.md 速查表条目 056) |
| 数据导出用 SQL dump | SQLite 原生语句保真度高，导入即执行 |
| 壁纸窗口用透明 WebView | DWM 视觉合成是桌面 WorkerW 层唯一可靠渲染路径（Windows） |

## 8. 外部依赖关系

```
Luter
  ├── Tauri 2          桌面框架（tray-icon）
  ├── tauri-plugin-{autostart,dialog,fs,shell,notification}
  ├── llama-server (外部二进制)     本地 GGUF 模型推理引擎
  ├── axum 0.7 / tower 0.4 / tower-http 0.5   HTTP 框架
  ├── tokio / futures / tokio-stream / socket2  异步运行时（socket2 仅双栈绑定）
  ├── include_dir 0.7  编译期嵌入 dist
  ├── rusqlite 0.32    SQLite (bundled)
  ├── reqwest 0.12     HTTP 客户端（json/stream/cookies）
  ├── aes-gcm 0.10 / argon2 0.5 / sha2 / base64 / rand   加密与哈希
  ├── dashmap 6        并发 HashMap（注册表/限流/连接）
  ├── tracing          结构化日志
  ├── scraper 0.20 / ego-tree 0.6   HTML 解析（Bing / 正文提取）
  ├── encoding_rs 0.8  非 UTF-8 页面解码（内容增强层）
  ├── rmcp 3           MCP Rust SDK（Streamable HTTP）
  ├── htmd 0.2         HTML → Markdown
  ├── pdf-extract 0.12 PDF 文本层提取
  ├── rodio 0.20 / souvlaki 0.8   音频播放 / 系统媒体控制
  ├── tiny-skia 0.11   splash 环形动画渲染
  ├── sysinfo 0.33     系统资源监控
  ├── zip / tar / flate2  llama-server 二进制解压
  ├── windows 0.61     Windows API（壁纸 / DComp splash）
  ├── objc2 0.6 + objc2-app-kit/foundation/quartz-core 0.3   macOS API
  ├── React 19 / ReactDOM
  ├── Zustand 5        状态管理
  ├── shadcn/ui        组件（Radix UI + Tailwind CSS 3）
  ├── lucide-react     图标库
  ├── react-router 8   路由
  ├── react-hook-form 7 + zod 3  表单校验
  ├── recharts 2       统计图表
  ├── @dnd-kit/{core,sortable,utilities}   拖拽排序
  ├── @tanstack/react-virtual  虚拟滚动
  ├── date-fns 4       日期格式化
  └── @playwright/test（devDependency）  E2E 脚本
```

## 9. 插件系统交互

```
luter-plugin-wukong (外部进程，TS + Hono 反向代理网关)
    │
    │  WebSocket /ws/plugin
    │◀═══════════════════════════▶  register(含 work_dir) + heartbeat + config_update
    │                               （keys / keys_update 被拒绝并断开，见 module-plugin-system.md）
    │
    │  HTTP POST /v1/chat/completions
    │════════════════════════════▶  Router 发请求到插件 base_url，携带占位凭证 "luter"
    │
    │                              插件忽略占位凭证，用自身登录持有的真实凭证访问上游
    │                              插件注入业务头 + 协议转换
    │                              POST https://api-deap.dingtalk.com/...
    │◀════════════════════════════  返回结果
```

**Router 管**: 路由解析、插件离线跳过、故障转移/冷却、用量统计（插件候选单次尝试，不轮换密钥）。
**Plugin 管**: 凭证登录与持有、非标→标准协议转换、业务头注入、base_url/api_path 提供。
**伴生启动**: `plugins.autostart=1` 且 `work_dir` 非空时由 Router `pnpm run serve` 拉起（见 `plugin/host.rs`）。

## 10. MCP 工具服务器

网关内置 MCP（Streamable HTTP）端点 `/mcp`，提供三个工具：

| 工具 | 功能 | 开关 |
|------|------|------|
| `web_search` | 多来源搜索（Bing 默认；SearXNG/Tavily/Exa/Brave 可选，auto 并发 + RRF 融合 + 15min 查询缓存）+ 内容增强（并发抓正文生成命中段摘录、rerank_score 0~3 参考分、日期/站点/权威度元数据；`num_results`/`freshness`/域名过滤/`ori_question`） | `mcp_websearch` |
| `web_fetch` | 静态优先抓取（HTML/文本/JSON/XML/CSV/PDF）→ 正文提取转 Markdown；正文过薄或反爬时升级内置 WebView 渲染；15min 内容缓存；`urls` 批量（≤8，并发 3）；`start_index`/`max_chars` 分段读取；可选 Jina Reader 兜底（`webfetch_jina_fallback`） | `mcp_webfetch` |
| `notify` | 发送系统桌面通知（title/body/sound） | `mcp_notify` |

**鉴权**：与 `/v1/*` 代理同源，`Authorization: Bearer <service-key>`（Argon2 校验），失败 401。路径公开（不限 IP），Host 白名单关闭以兼容局域网访问。

**会话模式**：无状态（`NeverSessionManager`，`legacy_session_mode=false`）——工具无服务端推送。

**工具列表**：`tools/list` 按开关**实时过滤**；全部关闭时仍正常应答（空列表），已有客户端不报错。新版客户端（2026-07-28）附带 `ttlMs=0` / `cacheScope=private`。

**请求体上限**：路由层未设 `DefaultBodyLimit`，使用 axum 默认 2MiB。

**代理侧配合**：三个开关打开时分别剔除请求自带的搜索/抓取/通知类工具（notify 额外注入系统提示）。
