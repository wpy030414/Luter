# Luter — 产品需求文档

> 版本: 26.11.5（CalVer）· 更新日期: 2026-11-06 · 标识符 `im.xrl.router`

## 1. 产品目标

| 目标 | 衡量方式 |
|------|---------|
| 统一接入 | 通过单一端点访问所有 Provider |
| 零摩擦启动 | 从打开应用到发出第一个请求 < 3 分钟 |
| 可靠运行 | 单个 Key 失效不影响服务 |
| 透明可观测 | 所有请求的 token 用量、延迟、成功率可追踪 |

## 2. 用户画像与场景

### 2.1 主要用户：AI 开发者

- 熟悉 API 调用，了解 REST/HTTP
- 每天使用，作为日常开发基础设施
- 核心诉求：一个端点接入所有模型，密钥自动管理，本机运行并可共享给局域网

### 2.2 次要用户：Claude Code / AI IDE 用户

- 会用终端，不一定了解 API 细节
- 核心诉求：让 Claude Code 能用非 Anthropic 的模型

### 2.3 核心使用场景

**场景 A：首次配置**
1. 打开 Luter 桌面应用
2. 在「供应商」页面添加 Provider（选类型、填 URL、填 Key）
3. 创建 Service Key
4. 在 Claude Code 配置 base URL 和 Service Key
5. 开始使用

**场景 B：密钥故障恢复**
1. Provider 返回 401 → Key 自动标红
2. 系统自动切换到下一个可用 Key
3. 后续请求透明继续

**场景 C：插件自动发现**
1. 启动 luter-plugin-wukong
2. 插件 WS 连接到 Router → 发送注册信息
3. Router 弹出确认对话框
4. 用户确认 → 委托供应商自动激活

**场景 D：局域网设备快速接入**
1. 主机在「密钥管理」页创建密钥 → 弹窗显示分发链接
2. 设备浏览器打开链接 → 按平台显示一行命令
3. 设备复制命令到终端执行一次
4. 设备上的 Claude Code 直接通过主机网关使用所有模型

## 3. 功能需求

### 3.1 P0 — 核心功能 ✅

| ID | 功能 | 价值 |
|----|------|------|
| F-01 | Provider CRUD | 注册上游 LLM 供应商 |
| F-02 | API Key CRUD + 密钥池轮询 | 多 Key 自动轮换，单 Key 失效不影响服务 |
| F-03 | Service Key 认证（Argon2） | 客户端访问鉴权 |
| F-04 | LLM 流式代理 | 转发请求到上游，流式回传 |
| F-05 | 三协议 IR 转换 | Anthropic ↔ OpenAI ↔ Responses 协议互转 |
| F-06 | 模型别名 | 客户端用统一名称访问不同 Provider 的模型 |
| F-07 | 密钥健康监控（红绿灯） | 可视化 Key 状态 |
| F-08 | 桌面应用（Tauri 2） | 本地运行，轻量美观 |
| F-09 | 请求超时保护 | 防止长请求被误判挂死 |
| F-10 | 密钥轮询指针持久化 | 重启后从上次位置继续 |
| F-11 | AES-256-GCM 加密 Provider Key | 密钥安全存储 |
| F-12 | 令牌桶限流 | `/v1/*` 每 Service Key 128 req/min |

### 3.2 P1 — 重要功能 ✅

| ID | 功能 | 价值 |
|----|------|------|
| F-13 | 用量统计 | 可视化 token 用量、延迟、成功率 |
| F-14 | 模型注册 + 层级分类 | 管理可用模型 |
| F-15 | Provider 启用/禁用 | 临时停用某个供应商 |
| F-16 | 健康检查端点 | 监控网关状态 |
| F-17 | 缓存追踪 | 统计缓存命中 token |
| F-18 | WebSocket 实时推送 | 前端实时接收状态变更 |
| F-19 | Service Key 白名单 | 限制密钥可用模型 |
| F-20 | usage_log 自包含快照 | 删除实体不影响历史统计 |

### 3.3 P2 — 锦上添花 ✅

| ID | 功能 | 价值 |
|----|------|------|
| F-21 | 本地 MCP 工具服务器 | Streamable HTTP `/mcp`，web_search / web_fetch / notify 三工具，开关实时过滤 `tools/list`；搜索多后端融合 + 命中段摘录 + 0~3 参考分；抓取静态优先、按需渲染、输出干净 Markdown 与分段续读 |
| F-22 | 模型同步 | 从上游拉取可用模型列表 |
| F-23 | 系统托盘 | 关闭窗口后继续运行 |
| F-24 | 插件系统 | 扩展支持非标 API |
| F-25 | 插件伴生启动 | `autostart=1` 且上报 `work_dir` 的插件在 Router 启动时 `pnpm run serve`（需 node/pnpm，TCP 探测防重） |
| F-26 | 插件心跳监控 | 检测插件离线 |
| F-27 | 供应商拖拽排序 | 自定义优先级 |
| F-28 | 暗色模式 | 视觉舒适 |
| F-29 | 上游模型代理获取 | 绕过 CORS 限制 |
| F-30 | 应用设置 | MCP / 故障转移 / 对话审查 / 提示词注入 / Jina 兜底等开关，后端持久化 |
| F-31 | Token 配额（5h/7d） | 限制单密钥消费上限 |
| F-32 | 余额端点 | `GET /v1/user/balance` 返回配额已用量与重置时间 |
| F-33 | 系统代理自动继承 | 自动使用系统代理设置 |
| F-34 | ConnectionStatus 修复 | 离线状态正确显示 |
| F-35 | 局域网分发 | 快速接入局域网设备 |
| F-36 | 单 listener + IP 限制 | 管理 API 仅本机访问 |
| F-37 | 故障转移 | 上游故障自动切换 |
| F-38 | 请求日志分页 | `GET /api/stats/requests`，按时间范围与服务密钥过滤 |
| F-39 | 国际化 | 中英双语 |
| F-40 | 主题跟随系统 | 自动切换明暗模式 |
| F-41 | 开机静默启动 | 后台运行 |
| F-42 | 数据导出/导入/重置 | 备份恢复 |
| F-43 | Claude FM 播放器 | 内置氛围电台 |
| F-44 | OpenAI Responses API 支持 | 第三协议 |
| F-45 | usage 真实值覆盖估算 | 准确统计 |
| F-46 | 上下文超限预警 | 软警告不阻断 |
| F-47 | list_models 扩展 | 返回模型能力信息 |
| F-48 | provider kind 统一命名 | 规范化 |
| F-49 | Claude FM 桌面壁纸 | 右击像素画面勾选，桌面呈现与应用内严格同步的像素艺术动画（无按钮/歌曲信息） |
| F-50 | Bing 搜索策略升级 | 绕过降级 |
| F-51 | macOS 系统代理检测 | 自动检测代理 |
| F-52 | 主题色相滑块 | 自定义主题色 |
| F-53 | 统一 IR 转发 | 简化代码 |
| F-54 | Install 页面迁移 React SPA | 多消费端支持 |
| F-55 | UI 设置后端持久化 | 主题/语言同步 |
| F-56 | 动态 BASE_URL | LAN 浏览器同源访问 |
| F-57 | 组合别名 | 多模型捆绑回退 |
| F-58 | Windows 自定义窗口控制 | 红绿灯风格按钮（关闭/最小化/最大化） |
| F-59 | Service Key 创建时设置白名单 | 创建密钥时即可限定可用模型 |
| F-60 | 插件系统改进 | PluginRegisterDialog 自监听事件、支持编辑插件供应商 |
| F-61 | StatsView 数字翻动动画 | useAnimatedStats + easeOutSine 缓动 |
| F-62 | 本地模型管理 | GGUF 导入 + llama-server 引擎生命周期 |
| F-63 | 本地模型导入权重 | 文件选择器导入本地 GGUF 文件 |
| F-64 | 本地模型崩溃自重启 | 5s/15s/45s 退避，最多 3 次 |
| F-65 | 本地模型自启动 | 应用启动时自动拉起标记 autostart 的引擎 |
| F-66 | 冷启动原生动画 | Rust 原生环形加载动画，WebView 初始化期间保持流畅，前端 `app-ready` 后关闭 |
| F-67 | 系统状态栏 | 本地模型页底部展示 CPU 使用率与内存占用 |
| F-68 | IPv4/IPv6 双栈监听 | 通配 host 时绑定 `[::]:port`（`v6only=false`）单 socket 服务双栈，失败回落 IPv4 |
| F-69 | 系统提示词注入 | 设置项 `session_inject` 非空时前置到每次请求的系统提示词 |
| F-70 | 对话审查 | `audit_enabled` 开启后按会话记录请求与回复到 `conversations` 表，`/api/audit` 列表 / 详情 / 删除 |
| F-71 | 首 token 时延与输出速度 | 记录 `first_token_ms`，速度按生成窗口 `completion ÷ (latency − TTFT)` 展示 |
| F-72 | web_fetch 批量与 Jina 兜底 | `urls` 批量（≤8，并发 3）+ 静态与渲染均失败时的 Jina Reader 兜底（`webfetch_jina_fallback`，默认关） |
| F-73 | 出站 SSRF 守卫 | 协议白名单 + 私有段/主机名黑名单 + DNS 逐地址检查 + 重定向逐跳复查 |

### 3.4 未实现（计划中）

| ID | 功能 | 计划版本 |
|----|------|---------|
| F-74 | 指数退避重试 | 待定 |
| F-75 | 更多 Provider 内置 | 待定 |
| F-76 | 自动更新机制 | 待定 |

> 管理 API 认证、路由规则引擎已由架构决策否决，不再列入计划（见 AGENTS.md Non-Goals；`routes` 表迁就保留但代码未使用）。

## 4. 协议转换规格

下游统一暴露 Anthropic Messages API、OpenAI Chat Completions API、OpenAI Responses API 三入口。所有客户端格式先经 IR（中间表示层）统一抽象，再渲染为目标上游格式。

### 4.1 必须支持的转换特性

- ✅ 文本消息 / 系统提示
- ✅ 工具调用 / 工具结果 / 工具选择
- ✅ 思考过程（含 Responses reasoning item 往返）
- ✅ 流式响应、缓存 token
- ✅ server-side web_search 工具归一化
- ✅ 请求体上限 64MiB（超长会话 + base64 截图）

### 4.2 usage 语义

| 项 | 规则 |
|----|------|
| 合并策略 | 真实值覆盖估算占位 |
| Responses input_tokens | 减去 cached_tokens，保持增量口径 |
| 上下文超限预警 | 仅 warn 日志，不阻断请求 |
| 输出速度 | `completion_tokens ÷ (latency_ms − first_token_ms)`，派生不落库 |

### 4.3 流式与非流式

上游始终走流式（`ir_request.stream = true`）。客户端 `stream: false` 时，引擎收集全部 SSE 事件后返回完整 JSON 响应。错误传达：流式路径优先经 SSE error 事件，无法建立流时（如上游返回 4xx/5xx）返回真实 HTTP 状态码；非流式始终返回真实 HTTP 状态码。

## 5. 密钥池规格

| 状态 | 颜色 | 触发条件 | 行为 |
|------|------|---------|------|
| 正常 | 🟢 绿 | 初始 / 请求成功 | 正常使用 |
| 低配额 | 🟡 黄 | 402 / 429 | 跳过，冷却 300 秒后自动恢复 |
| 失效 | 🔴 红 | 401 / 403 | 永久跳过 |

健康状态纯内存（启动全绿），轮询指针持久化到 `settings` 表。

## 6. Token 配额规格

每个 Service Key 可配置 5 小时和 7 天两个滚动窗口的 token 上限（0 表示不限）。只持久化上限，已用量按需从 `usage_log` 聚合。

| 项 | 规则 |
|----|------|
| 窗口定义 | 滚动窗口，按 Unix 时间戳整除窗口长度对齐（5h / 7d） |
| 用量口径 | prompt + completion + cache_read_input_tokens |
| 超限判定 | used >= limit 即 429；两窗口同时超限时优先报 7d |
| 恢复方式 | 窗口滚动重置后自动恢复 |
| 错误响应 | 429 + retry-after 头（距下一窗口边界秒数）+ `quota_error` 错误体 |

## 7. 故障转移规格

同一模型别名配置在多个 Provider 上时，网关按序尝试全部候选，上游故障自动切换。优先级：本地模型恒先于云端，其后按 `sort_order`、`created_at`；组合别名强制启用手动回退语义（不受全局开关影响）。

| 项 | 规则 |
|----|------|
| 开关 | `failover_enabled`（默认关闭） |
| 触发切换 | 上游 5xx / 网络错误 / 响应头超时 / 密钥轮换耗尽 |
| 冷却 | 失败 provider 冷却 60s，2xx 成功立即解除；纯内存不持久化 |
| 最终错误码 | 流式：SSE error 事件透传；非流式：真实 HTTP 状态码（缺省 502） |

## 8. 插件系统规格

外部服务通过 WebSocket 注册为「委托供应商」（契约见 docs/specs/module-plugin-system.md）。

| 职责 | Router | Plugin |
|------|--------|--------|
| 密钥管理 | ❌（register 携带 `keys` 直接拒绝并断开） | ✅ |
| 协议转换 | ✅ | ✅ 非标 → 标准 |
| 业务头注入 | ❌（仅注入合成占位凭证 `Luter`） | ✅ |
| 健康监控 | ✅ | ❌ |
| 用量统计 | ✅ | ❌ |
| 伴生启动（进程托管） | ✅ `pnpm run serve`，支持 `login` 登录流程 | — |

## 9. 局域网分发规格

把本机网关能力延伸到局域网设备。

| 项 | 规则 |
|----|------|
| 单 listener + IP 限制 | `/api/*` 管理端点仅 loopback（`admin_ip_guard`）；`/v1/*`、`/mcp`、`/install` 对外开放 |
| 监听 | 通配 `0.0.0.0` 时以 IPv6 双栈 socket 服务 IPv4 + IPv6 |
| 分发链接 | `http://<本机IP>:19068/install?t=<明文key>` |
| 消费端选择 | Claude Code + ChatGPT/Codex |
| 安全边界 | 密钥明文嵌入 URL，仅限可信设备 |

## 10. 本地模型规格

在本机导入并运行 GGUF 模型，自动注册为网关内 Provider，对外暴露方式与云端 Provider 一致；路由时本地模型恒优先于云端。

### 10.1 模型获取

| 项 | 规则 |
|----|------|
| 来源 | 用户自行从 HuggingFace 或其他渠道下载 GGUF 文件，通过文件选择器导入 |
| 格式 | 仅 GGUF |
| 存储 | 应用数据目录下按模型 id 分目录，导入即拷贝权重 |
| 导入失败 | 校验失败或拷贝错误时不产生半成品记录 |

### 10.2 引擎生命周期

| 项 | 规则 |
|----|------|
| 引擎 | llama-server（自动下载对应平台二进制） |
| GPU 后端 | Metal / CUDA / Vulkan / ROCm / CPU（自动检测 + 手动选择，`auto` 取首个可用非 CPU） |
| 启动序列 | 首选后端 → CPU 兜底 |
| 健康检查 | 启动后 120s 内等待 `/health` 返回 200 |
| 端口 | 自动分配空闲端口 |
| 崩溃重启 | 5s / 15s / 45s 退避，最多 3 次 |
| 自启动 | `autostart=1` 的模型在应用启动时自动拉起 |

### 10.3 注册为 Provider

| 项 | 规则 |
|----|------|
| Provider ID | `local-{model_id}` |
| Provider Kind | `chat_completions` |
| Base URL | `http://127.0.0.1:{port}` |
| API Key | 启动时随机生成，AES 加密入库 + 同步进 KeyPool |
| Model | `display_name` = 用户命名（别名），`tier` = `local`，`context_window` = `ctx_size` |

## 11. 非功能需求

### 11.1 性能（设计目标，未做系统性实测）

| 指标 | 目标 |
|------|------|
| 冷启动 | 原生加载动画遮盖 WebView 初始化，前端首屏 `app-ready` 后关闭 |
| 代理额外延迟（转换） | 目标 ≤ 20ms |
| 内存占用（空闲） | 目标 ≤ 100MB |

### 11.2 安全

| 要求 | 实现 |
|------|------|
| Service Key 存储 | Argon2 哈希 |
| Provider Key 存储 | AES-256-GCM 加密 |
| 管理 API | IP 中间件限 loopback（管理面仅本机；多用户经 Service Key 区分，不加账号体系） |
| CORS | origin 白名单（为空时放开） |
| 频率限制 | `/v1/*` 令牌桶 128 req/min / Service Key |
| 出站 SSRF | `net_guard.rs` 拦截私有地址与元数据端点 |

### 11.3 兼容性

| 维度 | 要求 |
|------|------|
| Anthropic API | 兼容 `2023-06-01` |
| OpenAI API | 兼容 Chat Completions v1 + Responses API |
| MCP | Streamable HTTP，协议修订 `2026-07-28` |
| 操作系统 | macOS（主要）、Windows（原生 splash / 窗口控制）、Linux |

## 12. 成功指标

### 12.1 北极星指标

> **成功发出首个代理请求的时间（Time to First Request）** ≤ 3 分钟

### 12.2 功能指标

| 指标 | 目标 |
|------|------|
| Provider 接入成功率（设计目标） | ≥ 99% |
| 协议转换正确率（设计目标） | 100% |
| Key 故障自动切换 | 100% |

## 13. 风险

| 风险 | 概率 | 缓解 |
|------|------|------|
| 上游 API 格式变更 | 中 | 版本锁定 + 兼容性测试 |
| SQLite 高并发瓶颈 | 低 | WAL 模式 + 异步批量写入 |
| 协议转换丢失特性 | 中 | 不兼容特性显式报错 |
| 上游挂起网关卡死 | 低 | 独立超时保护 |
| 密钥泄露 | 低 | AES-256-GCM + Argon2 |
| 提示注入诱导 web_fetch 读取本机端点 | 低 | SSRF 守卫 + 私有网段拦截 |
