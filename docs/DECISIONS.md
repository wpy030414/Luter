# Architecture Decision Records

> 设计背后的历史原因。记录**解释现状为何如此**的决策：为什么选这条路、为什么不选另一条、愿意为此接受什么代价。实现细节见 `docs/specs/`，结构关系见 `docs/ARCHITECTURE.md`。

## ADR-001: FM 像素艺术桌面壁纸（WorkerW 劫持 + 引擎权威时钟）

**日期**: 2026-08-28 ｜ **状态**: 已接受
### 背景
用户希望右击 FM 像素艺术画面勾选「设置为桌面背景」后，桌面壁纸被劫持为与应用程序内像素艺术**严格同步**的动画（壁纸上不显示播放/暂停按钮与歌曲信息），暂停/切歌时两处画面一致。
### 决策
1. **渲染面 = 第二个 WebviewWindow**：注入 `__WALLPAPER_MODE__` 走 `WallpaperScene` 分支，复用整套前端像素渲染管线，不做 Rust 侧重绘。
2. **Windows = WorkerW 劫持（双布局探测）**：唤醒壁纸宿主后把窗口挂入 WorkerW，递归设点击穿透。取消勾选/进程退出即销毁窗口，WorkerW 自动重绘原壁纸——无需显式还原。
3. **macOS = `kCGDesktopIconWindowLevel`**：窗口层降到壁纸图与桌面图标之间，忽略鼠标事件。
4. **同步 = 引擎权威时钟**：动画时钟从本地累计改为引擎侧 `scene_t`（仅播放时按真实流逝累计、暂停冻结），两窗口采样同一值——不引入新广播事件，天然严格同步，且顺带修复了主窗口路由回到 /fm 时动画相位重置的问题。
5. **穿透方式 = 手动 Win32 样式**（非 `set_ignore_cursor_events`）：后者仅作用于顶层 HWND，不覆盖 WebView2 子窗口。
### 备选与权衡
- **定时截图刷新系统壁纸**（`SystemParametersInfo` 轮换 PNG）：刷新延迟与闪烁不可接受，且无法严格同步 → 否决。
- **全屏置底普通窗口**（不挂 WorkerW）：会被其他窗口覆盖，不是真壁纸 → 仅作 WorkerW 失效时的回退。
- **URL query 区分壁纸入口**：`WebviewUrl::App` 带 query 属未固定行为 → 改用 `initialization_script`。
- **`transparent` 窗口特性**：Windows 上 tauri 的 transparent 实为无效，且像素画本身铺满全屏 → 纯 CSS 黑底。
### 代价
- 仅 Windows / macOS 两平台实现；多显示器下 v1 仅主显示器，分辨率/DPI 变化后需重新勾选（已知限制）。
- 平台代码是 unsafe（Win32 / objc2），macOS 侧编译验证需在 macOS 上完成（CI macos-14 job 兜底）。

## ADR-002: Windows 壁纸挂载方案（WorkerW + 透明 WebView）

**日期**: 2026-08-28 ｜ **状态**: 已接受（Windows 部分）；取代已废弃的 Rust GDI 直绘方案
### 背景
先前的 Rust GDI 直绘方案在真机上也未能上屏（窗口结构完全正确——
WS_CHILD、可见、父链正确、WM_PAINT 正规绘制——桌面依旧无变化）。
检索社区 [tauri-plugin-desktop-underlay](https://github.com/Charlie-XIAO/tauri-plugin-desktop-underlay)
（把窗口挂到桌面 WorkerW 层的 Tauri 插件）发现其真实用户能看到 WebView
内容，而我们的 WebView 尝试（多轮配置）看不见。核心差异：**窗口透明度**。
### 决策
1. **Windows = 透明 WebView 窗口 + 社区插件挂载**：
   - `transparent(true)` 的 WebviewWindow 走 **DWM 视觉合成**渲染——桌面 WorkerW
     层唯一可靠的内容上屏路径（经典 GDI/重定向表面在桌面层不被 DWM 呈现，这同时
     解释了 GDI 直绘版为何「什么都对但看不见」）；
   - `set_desktop_underlay(true)`（插件）负责 SetParent 进 WorkerW，不再自研挂载；
   - WebView2 需显式关闭 `CalculateNativeWinOcclusion`（WorkerW 子窗会被判「完全
     遮挡」而暂停合成），并使用独立 user data directory（否则与主窗口环境参数冲突）；
   - 点击穿透用 `WS_EX_TRANSPARENT`（顶层 + 递归子窗）；**全程严禁 `WS_EX_LAYERED`**
     ——分层窗口不调 SetLayeredWindowAttributes 即不显示内容，这是 WebView 版与
     GDI 版两次「看不见」的公共元凶。
2. **GDI 直绘退役**：`pixelart.rs` 移植与画家线程删除（保留在 git 历史）；
   前端 `WallpaperScene` / `sampleT` 引擎时钟方案复用（macOS 同款）。
3. **macOS 不变**（objc2 层级方案；后续真机验证）。
### 代价
- WebView 方案的固有成本（一个浏览器进程）重新回来；
- 依赖社区插件的 Windows 实现（其维护节奏不可控，但配方公开可自行顶替）。

## ADR-003: Windows 壁纸 Z 序锚定（防盖桌面图标）

**日期**: 2026-08-29 ｜ **状态**: 已接受
### 背景
ADR-002 的透明 WebView + WorkerW 挂载真机反馈：**壁纸窗口盖住桌面图标**。挂载链本身正确（`SetParent` 成功、子窗枚举验证通过），问题全在 Z 序——Win8+ 的 `SetParent` 对 popup 风格窗口只重设父链，窗口仍留在顶层 Z 序，于是在宿主内盖住 `SHELLDLL_DefView`（桌面图标层）；`fit_monitor` 的 `SetWindowPos` 未带 `SWP_NOZORDER`，会把壁纸窗顶到宿主上层，进一步加重。
### 决策
1. **Z 序锚定**：挂载后手动转 `WS_CHILD`（清 `WS_POPUP`）使窗口真正参与宿主子窗 Z 序，再 `SetWindowPos(HWND_BOTTOM, SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE)` 沉到宿主最底。
2. **验证 = `GW_HWNDNEXT`**：壁纸窗之下无兄弟窗即锚定生效；有则说明系统仍在重排窗口树，由调用方重试。
3. **后续不扰 Z 序**：`fit_monitor` 的 `SetWindowPos` 补 `SWP_NOZORDER`，否则默认 HWND_TOP 会撤销锚定。
4. **show 后复查**：show / WebView2 初始化可能重排宿主子窗，故 show 后延迟复查一次并幂等重锚。
### 备选与权衡
- **建窗即 `WS_CHILD` 子窗**（GDI 版旧做法）：tauri builder 不暴露该样式，仍需运行时补——与现方案等价，无收益。
- **照社区插件只做 SetParent**：已核实 `tauri-plugin-desktop-underlay` 源码不做任何 Z 序处理，在部分真机上即复现盖图标 → 否决。
### 代价
- 挂载链多两次重试 + 一次延迟复查的主线程 dispatch，代码复杂度上升。
- 锚定依赖宿主子窗 Z 序语义，未来 Windows 布局变化需重新验证（与 ADR-001/002 的 WorkerW 配方同命运）。

## ADR-004: 选择 Tauri 2 作为桌面框架

**日期**: 2026-07-28 ｜ **状态**: 已接受
### 背景
需要一个轻量级桌面应用来运行本地 LLM API 网关。考虑过 Electron、原生应用、纯 CLI。
### 决策
采用 Tauri 2：Rust 后端 + WebView 前端。
### 原因
1. **轻量**: 安装包 < 10MB，内存占用 < 100MB（Electron 通常 > 200MB）
2. **性能**: Rust 后端处理高并发代理请求
3. **安全**: Rust 内存安全 + 系统级加密库原生支持
4. **跨平台**: 一套代码编译 macOS/Windows/Linux
### 代价
- 需要 Rust 工具链（学习曲线）
- WebView 渲染在某些系统可能不一致

## ADR-005: 仅支持流式代理（stream=true）

**日期**: 2026-07-28 ｜ **状态**: 已接受（已修订）
### 背景
LLM API 支持流式（SSE）和非流式两种模式。完整支持需要两套代码路径。
### 决策
上游**始终**走流式（`ir_request.stream = true`）。客户端 `stream: false` 时，引擎收集全部 SSE 事件后返回完整 JSON 响应（`client_wants_stream` 标志控制输出格式）。
### 原因
1. **简化实现**: 上游只需一套流式处理逻辑
2. **用户体验**: Claude Code 等主流客户端都默认流式
3. **资源效率**: 流式可以边生成边传输，不需要缓存完整响应
4. **兼容性**: 客户端非流式请求也能正确响应（收集 SSE → 合成 JSON）
### 代价
- 非流式客户端需等待全部 token 生成完毕才能收到响应（与原生非流式语义一致）

> **2026-08-28 修订**：新增客户端非流式支持（`c090c7b`）。上游仍强制流式，但客户端可发送 `stream: false`，引擎收集全部事件后返回完整 JSON。原始决策（上游仅走流式）仍然有效。

## ADR-006: 密钥健康状态纯内存存储

**日期**: 2026-07-28 ｜ **状态**: 已接受
### 背景
密钥健康状态（green/yellow/red）需要持久化还是仅内存？
### 决策
健康状态仅存内存，启动时全部初始化为 green。只有轮询指针（`current_index`）持久化到 `settings` 表。
### 原因
1. **启动恢复**: 重启后从上次轮询位置继续
2. **减少 IO**: 每次健康状态变更都写 DB 会产生大量小事务
3. **语义合理**: 健康状态是运行时概念，重启后重新探测更合理
### 代价
- 重启后无法看到历史健康状态

## ADR-007: 管理 API 无认证，绑定 loopback

**日期**: 2026-07-28 ｜ **状态**: 已接受
### 背景
管理 API（`/api/providers`、`/api/keys` 等）是否需要认证？
### 决策
管理 API 无认证，仅通过绑定 loopback IP + CORS 白名单保护。
### 原因
1. **本地场景**: 桌面应用运行在用户本机，威胁模型是"本机其他进程"
2. **简化使用**: 无需登录/Token，打开应用即用
3. **CORS 保护**: 浏览器端恶意网页无法跨域调用
### 代价与威胁模型
- **已防护**：远程攻击（绑定 loopback）、浏览器跨域攻击（CORS）
- **未防护**：**本机恶意进程可以访问管理 API**——这是明确接受的风险
- **接受理由**：桌面应用场景，本机安全由用户保证

## ADR-008: 统一 HTTP 客户端工厂 + 系统代理自动继承

**日期**: 2026-08-03 ｜ **状态**: 已接受
### 背景
项目有多处出站 HTTP 请求，各自独立构建。国内网络下需走代理才能连通。
### 决策
新增 `http.rs` 模块作为唯一 HTTP 客户端工厂：

1. `system_proxy()`: 解析系统代理（环境变量 → Windows 注册表 → macOS scutil），OnceLock 缓存
2. `build_http_client() -> ClientBuilder`: 返回带系统代理的 builder
3. NO_PROXY 默认豁免 `localhost`、`127.0.0.1`、`[::1]`

所有出站 HTTP 请求必须使用工厂方法。
### 原因
1. **统一代理**：所有调用点自动继承系统代理
2. **零配置**：Windows 用户配 Clash 系统代理后自动继承
3. **性能**：OnceLock 缓存代理解析结果
### 代价
- 代理在应用运行期间不可变

## ADR-009: 单 listener + 路径级 IP 限制

**日期**: 2026-08-06 ｜ **状态**: 已接受
### 背景
旧方案用两个 listener 分离：admin 绑 `127.0.0.1:19068`，public 绑 `0.0.0.0:19069`。
### 决策
合并为单 listener 绑 `0.0.0.0:19068`，通过路径级 IP 中间件控制访问权限：

| 路径类型 | 限制方式 | 路由 |
|----------|----------|------|
| 公开 | 不限 IP | `/health`、`/ws`、`/v1/*` 代理 |
| 管理 | `admin_ip_guard` 中间件限 loopback | `/api/*` CRUD |
### 原因
1. **单端口简化运维**：防火墙只需放行 19068
2. **IP 中间件而非双 listener**：路径级限制比端口级更灵活
3. **保留 loopback 安全模型**：`admin_ip_guard` 确保 `/api/*` 管理端点永不对外开放
### 代价
- `0.0.0.0:19068` 向局域网暴露：任何人可调 `/v1/*`（需有效 key）

## ADR-010: 故障转移（Provider Failover）

**日期**: 2026-08-03 ｜ **状态**: 已接受
### 背景
同一模型别名配置在多个 Provider（官方 + 代理镜像）时，上游故障会让整个会话失败。
### 决策
1. **候选解析**：`resolve_route_candidates()` 返回同 `display_name` 下全部候选（`sort_order ASC, created_at ASC`，按 provider_id 去重）
2. **双层循环**：外层遍历 provider 候选，内层遍历 key 池
3. **冷却表纯内存**：失败 provider 冷却 60s
4. **开关默认关闭**：`failover_enabled` 存 settings 表
5. **错误码语义**：网络错误 502、响应头超时 504、key 4xx 耗尽透传最后一次上游失败响应、无可用 key 503
### 原因
1. 按 provider_id 去重避免同一上游被重复尝试
2. 冷却 60s 而非指数退避：场景是「上游暂时不可用」，固定冷却足够
3. 开关默认关闭符合「不主动改变既有行为」原则
### 代价
- 双循环复杂度集中在 handler.rs

## ADR-011: 引入 IR 中间表示层统一三协议转换

**日期**: 2026-08-09 ｜ **状态**: 已接受
### 背景
项目最初只有 Anthropic Messages ↔ OpenAI Chat Completions 双向转换。随着 OpenAI Responses API 支持需求出现，三协议互转的组合爆炸问题凸显。
### 决策
1. **新建 `api/proxy/ir/` 模块**（Intermediate Representation，中间表示层）
2. **IR 以 Anthropic Messages 为骨架**：`IrContentBlock` 覆盖 Text/Image/Thinking/ToolUse/ToolResult 五种内容块
3. **单向转换取代双向转换**：所有客户端格式 → IR（`from_*`），IR → 所有客户端格式（`to_*`）
4. **`IrStreamEvent` 6 种变体**：MessageStart → ContentBlockStart → ContentBlockDelta → ContentBlockStop → MessageDelta → MessageStop
5. **`IrUsage` 统一 token 统计**：input_tokens / output_tokens / cache_read_input_tokens 等
6. **usage 真实值覆盖估算占位**：上游返回真实 usage 时直接覆盖估算值（不用 `max()`）
7. **上下文超限预警（软警告）**：仅记录 warn 日志，不返回 400 错误（避免阻断客户端 auto-compact）
### 原因
1. **组合爆炸收敛**：N 个协议只需 2N 个转换模块，而非 N×(N-1) 个双向模块
2. **内部工具解耦**：websearch 劫持、usage 追踪等内部工具只操作 IR 类型
3. **扩展性**：新增协议只需新增一对 `from_*` / `to_*` 模块
### 代价
- 转换路径变长：客户端格式 → IR → 客户端格式（两步）

## ADR-012: WebSearch/WebFetch 迁移到本地 MCP 端点

**日期**: 2026-08-24 ｜ **状态**: 已接受
### 背景
旧方案由代理跑 server-side 劫持循环：注入 `web_search` 工具 → 缓冲所有中间搜索轮次 → 本地 Bing 搜索回传 → 收尾合成响应。
### 决策
1. **删除劫持循环**，改为网关内置 **MCP（Streamable HTTP）端点 `/mcp`**，提供 `web_search`、`web_fetch`、`web_vision` 三个工具
2. **Bing 搜索策略**：HTTP 浏览器头 + cookie 复用 + 懒预热 + 双域名 fallback + 绕过代理直连
3. **WebFetch 渲染**：Tauri 内置 WebView（懒创建隐藏窗口）+ 静态抓取回退
4. **web_vision**：设置页指定「视觉专用模型」，网关取图后调该模型生成描述
5. **开关语义**：`mcp_websearch` / `mcp_webfetch` / `mcp_vision` 三个开关独立控制
6. **代理侧只保留「剔除」逻辑**：开关开启时剔除请求自带搜索工具（防上游官方搜索生效）
### 原因
1. **标准协议 > 私有序列**：MCP 是客户端原生支持的协议，工具调用可见、可审批
2. **性能**：模型直接调用工具，无中间轮缓冲
3. **删除大于新增**：净删 ~1600 行，新增 ~600 行
### 代价
- 客户端需一次性注册 MCP（设置页提供可复制的命令）

> **2026-08-28 变更**：`web_vision` 随 MCP Vision 功能整体移除（设置页「路由」Tab 不再提供视觉能力开关与视觉模型配置，`mcp/vision.rs` 删除，代理侧不再剥离图片内容块）。本 ADR 其余决策仍然有效。

## ADR-013: 组合别名（Combo）

**日期**: 2026-08-24 ｜ **状态**: 已接受
### 背景
用户需要把多个模型别名捆绑成一个新别名：客户端用组合名连接时，路由「不断尝试列表中的所有模型直到找到可用模型」。
### 决策
1. **组合 = 解析层展开**：`resolve_combo` 把组合按成员 `position` 逐个展开成候选列表，交给现有双循环执行
2. **组合强制回退**：组合命中后 `failover = global_failover || is_combo`——成员间回退不受全局开关影响
3. **仅供应商级失败换成员**：网络错误、头超时、5xx、401/402/403/429 → 换下一个成员；普通 400（非配额）立即透传
4. **成员只能是叶子模型别名**：不允许嵌套组合 → 天然无环
5. **命名双向冲突校验**：组合名不得撞 `models.display_name`
6. **白名单按组合名授予**：授予组合名 = 授予全部成员
7. **统计归因到实际成员**：`usage_log` 零改动
### 原因
1. **复用双循环**：组合的全部回退语义自动继承
2. **叶子成员最简**：嵌套组合需要运行时环检测 + UI 复杂度翻倍
3. **400 透传优于全量尝试**：请求本身非法时换模型是白跑延迟
### 代价
- 组合解析是 N+1 查询（1 次组合 + 每成员 1 次候选查询）

## ADR-014: 插件系统契约升级——密钥职责剥离 + 伴生启动

**日期**: 2026-09-22 ｜ **状态**: 已采纳
### 背景
原契约（V11）中插件经 WS 推送 keys，Router 入密钥池并注入上游请求的鉴权头。这导致：

1. 密钥双写（插件侧 + Router 密钥池），安全边界复杂
2. 插件更新凭证需主动 keys_update，易漂移
3. 插件进程生命周期不受 Router 管理，用户需手动启动
### 决策
1. **密钥职责完全剥离**：Router 不再为插件存储/轮换/注入密钥。register 消息的 `keys` 字段与 `keys_update` 消息严格拒绝（回 `keys_not_supported` 并断开），强制插件方升级。
2. **虚拟占位密钥**：代理层对插件候选恒返回合成 `PickedKey { key_hash: "Luter" }`，鉴权头注入代码零改动；`update_key_health` 对合成 key 天然 no-op（无红色黏性）。usage_log.key_id 存伪 id（V12 起已无 FK 约束，安全入库）。
3. **反向代理实现契约**：每个插件实现必须是 TypeScript + Hono 网关，package.json 提供 `login`（弹系统浏览器网页登录，不要求客户端到位）与 `serve`（启动网关 + WS 注册；幂等；Router 未就绪时重试）脚本。
4. **伴生启动（进程托管）**：插件 register 上报 `workdir`，Router 启动时 `pnpm run serve` 拉起（已启动不重复拉起，TCP 探测防重）。Windows 用 `pnpm.cmd` + `CREATE_NO_WINDOW` + `taskkill /T /F`（杀祖孙进程树）；unix 用 `process_group(0)` + `kill -9 -- -pgid`（杀进程组）。只杀 Router 自己 spawn 的进程。
### 代价
- 旧版插件必须升级到无密钥契约，否则被 Router 断开并拒绝重连
- 占位凭证 `Luter` 会出现在插件的上游请求日志中（无害，但需插件忽略）
- 伴生启动要求系统内 node + pnpm 可用，缺失时 UI 入口禁用

## ADR-015: 本地模型管理（GGUF 导入 + llama-server）

**日期**: 2026-08-28 ｜ **状态**: 已接受
### 背景
用户希望在本地运行开源模型（Llama、Qwen 等），不经过云端 API。需要一个从模型导入、引擎启动到自动注册为 Provider 的完整流程。
### 决策
1. **引擎 = llama-server（GGUF）**：自动下载对应平台/后端的预编译二进制，不捆绑在应用内
2. **模型来源 = 用户自行导入 GGUF 文件**：用户从 HuggingFace 或其他来源下载 GGUF 文件后，通过文件选择器导入到应用中
3. **自动注册 Provider**：每台本地模型注册一个 Chat Completions provider（id = `local-{model_id}`）+ 一条模型 + 一条随机密钥（AES 加密入库、同步进 KeyPool）
4. **崩溃自重启**：120s 启动健康检查；退出后按 5s/15s/45s 退避重启，最多 3 次
5. **自启动**：`autostart=1` 的模型在应用启动时自动拉起
6. **GPU 后端检测**：Metal/CUDA/Vulkan/ROCm/CPU 自动检测 + 手动选择，启动序列首选后端 → CPU 兜底
7. **事件广播**：状态变更（`local_status`）经 `AppState.events_tx` 推送前端
### 原因
1. **零配置体验**：用户不需要手动安装 llama.cpp 或配置 Provider
2. **统一接入**：本地模型与云端 Provider 共用同一套路由/密钥/统计管线
3. **GGUF 生态成熟**：HuggingFace 上 GGUF 量化模型丰富，llama-server 是事实标准
4. **不捆绑引擎**：应用安装包保持 < 10MB，引擎按需下载
5. **不内建 HuggingFace 集成**：简化代码、避免网络依赖和镜像维护，用户自行下载 GGUF 文件后导入
### 代价
- 引擎二进制由社区维护，版本更新不可控
- 首次启动需下载引擎（~50MB），需要网络连接
- GPU 后端依赖系统驱动（CUDA/Vulkan/ROCm 需预装）
- 用户需自行获取 GGUF 文件（无法在应用内直接搜索下载）

## ADR-016: 自研内容提取与重排层（content/）

**日期**: 2026-10-19 ｜ **状态**: 已采纳
### 背景
MCP 的 `web_search` / `web_fetch` 输出信息量不足，喂给模型的上下文质量差：`web_search` 只回 Bing 的机械截断 snippet，模型只能靠一句摘要判断要不要点进去；`web_fetch` 把**整页** outerHTML 丢给 `htmd` 转 Markdown，导航/侧栏/页脚/推荐位全部进入上下文，既浪费 token 又稀释正文信号；且无任何垃圾过滤，文档倒卖站与 SEO 农场结果和官方文档同权重呈现。

对标 DeepSeek Search & Fetch API 的能力语义（长摘要 + 0~3 语义重排 + `ori_question` 意图注入 + 干净全文 + 垃圾清洗），但**不接入该外部服务**：按量计费的外部搜索 API 与「本地直连、零外部依赖」的既有设计冲突，且会新增一个必须联网鉴权的故障点。
### 决策
新增 `src-tauri/src/content/` 内容增强层，**纯本地启发式算法，零 LLM 调用、零外部服务、零配置项**：
1. **正文提取**：trafilatura-lite 自研实现，复用已有 `scraper` 依赖而非引入 readability 类 crate。
2. **抽取式摘要**：按查询词覆盖度选句，**按原文顺序**拼接以保持行文连贯。
3. **语义重排**：`rerank_score` 0~3，由相关度、域名权威、新鲜度、内容质量四个分量合成。
4. **垃圾清洗**：硬编码域名分级 + 黑名单（命中整条丢弃）。后缀匹配按段比较，`github.com.evil.com` 不会继承 `github.com` 的权威度。
5. **意图注入**：`web_search` 新增可选参数 `ori_question`，其词项以更低权重并入查询词，引导重排与摘要选取。
6. **抓取策略与降级链**：预排选出候选后并发静态抓取，整体设预算而非逐页串行，超时保留已得结果；enrichment 全失败退化为 snippet，`web_fetch` 提取质量过低退回整页转换兜底——**永不失败**。
7. **`spawn_blocking` 纪律**：大页面解析是 CPU 密集且 `scraper::Html` 非 `Send`，解析整体放 blocking 线程池，绝不跨 await 持有——否则会拖累本网关的 SSE 流式转发延迟。
### 代价
- `web_search` 最坏延迟显著上升（搜索超时链 + enrichment 预算）；正常情况因并发抓取与缓存，增量约 2~5s。
- 反爬站点静态抓取拿不到正文，永远只有 snippet——设计内降级，实测知乎、百度百科对完整浏览器头仍返回 403，属站点侧风控。
- 抽取式摘要质量上限低于生成式：不会改写概括，只做句子选取。这是「零 token 成本 + 零额外延迟 + 行为确定可测」的自觉取舍。
- 域名分级表与黑名单需人工维护，误判风险存在——但权威度只占单因子权重，错判不致命。
- 新增两个直接依赖：`ego-tree`（scraper 未 re-export DOM 树类型）、`encoding_rs`（reqwest 只认响应头 charset，GBK 老站会乱码）。

## ADR-017: 搜索层多后端融合与命中段摘录

**日期**: 2026-10-24 ｜ **状态**: 已采纳
### 背景
ADR-053 的内容增强层解决了「只有 snippet」的问题，但搜索侧仍是单引擎裸爬 Bing：
Bing 风控降级时无备选；查询只有 `q` 一个参数（条数/时间/站点全部靠模型在自然语言
里猜）；每次搜索并发抓 5 页生成 600~800 字**抽取式摘要**——句子按词频挑选后拼接，
上下文断裂，表格/代码/数字常被筛掉，对模型是高噪声 token。对照社区（Exa highlights、
Tavily 清洗片段、Open WebUI 的 search engine / web loader 两层抽象），方向明确：
**搜索与抓取分层、多引擎融合、给命中段落而不是句子合集。**
### 决策
1. **引擎抽象（`search/`）**：`bing` 为零配置默认（保留 cookie 预热与降级检测）；
   `engines` 承载 SearXNG / Tavily / Exa / Brave 的 JSON API。`auto` 模式并发执行
   已配置来源，RRF（k=60）融合名次 + URL 归一化去重，全失败回退 Bing；显式指定后端
   时不融合。
2. **查询参数**：`num_results`、`freshness`、`include_domains` / `exclude_domains`。
   无原生域名参数的引擎用 `site:` / `-site:` 表达，结果侧再统一过滤。
3. **查询缓存**：按 `(backend, query, 参数组合)`，TTL 15min、容量 64。
4. **命中段摘录**：`summary` 从「贪心句子合集」改为「以最高分句为锚点的连续窗口」，
   保留上下文衔接——这是「给命中段落而不是句子合集」的落地。
5. **配置面**：搜索来源密钥与 Provider 密钥分离存放（搜索服务不在 Provider 注册表内），
   且 AES-256-GCM 加密，`/api/settings` 只回显「是否已配置」，明文永不回传。
6. **`rerank_score` 降级为参考分**：分数仍输出，但由模型自行判断，不再要求「忽略接近 0 的结果」。
### 代价
- 配置面变大：多后端意味着用户可能填错地址/密钥。错误信息按来源聚合返回，便于定位。
- `auto` 模式会同时打多个外部接口；有缓存兜底，且仅对**已配置**来源并发。`qft` 与
  `site:` 属半官方行为，Bing 改版时可能失效。
- 摘录比旧的「800 字合集」短，单条信息量下降，但噪声显著减少；需要全文时模型应
  调用 `web_fetch`（这也是两层工具分工的本意）。
- 搜索引擎密钥与 Provider 密钥分离存放（搜索服务不在 Provider 注册表内）。

## ADR-018: web_fetch 静态优先 + SSRF 守卫 + 分段读取

**日期**: 2026-10-24 ｜ **状态**: 已采纳
### 背景
`web_fetch` 原实现「一律隐藏 WebView 渲染 + 全局串行锁 + 60K 字符硬截断」，
在实际使用中暴露三类问题：

1. **慢**：普通静态页也付 3~10 秒渲染成本；`FETCH_LOCK` 跨 await 持有，
   模型并行抓多个 URL 时串行排队（每页最多 30s 超时叠加）；
2. **不可续读**：超过 60K 字符后只剩一句「已截断 N 字符」，模型无法取回后半篇；
3. **安全**：URL 不做任何检查，`http://127.0.0.1:19068/api/*` 这类本机管理端点
   可被提示注入后当作普通网页读入上下文；只认 HTML，PDF/JSON/纯文本全部失败；
   重复抓取同一 URL 每次都重新渲染。
### 决策
1. **静态优先，按需渲染**：先用 reqwest 抓取；仅在静态失败且状态码值得重试，
   或正文过薄且页面带 SPA 特征时，才升级到 WebView。404/410 不做无谓重试——
   宁可少渲染，也不把每页拖慢数秒。
2. **渲染池替代串行锁**：隐藏窗口池 + `Semaphore` 限流，窗口 Drop 归还；
   静态路径不受渲染池影响，可任意并发。
3. **内容缓存**：归一化 URL（去 fragment 与跟踪参数）为键，TTL 15 分钟、容量 64。
4. **分段读取**：`start_index` / `max_chars`；输出头写明总长度与本次区间，尾部给出
   续读参数。缓存存全文，切片在输出阶段做。
5. **内容类型分支**：HTML / text / JSON / XML / CSV / PDF；其他二进制明确报错。
6. **SSRF 守卫（`net_guard.rs`）**：协议白名单 + 字面量私有段/主机名黑名单 +
   DNS 解析后逐地址检查；静态 client 的重定向策略与渲染窗口的 `on_navigation`
   逐跳复查。`web_search` 富化抓取同样接入。
7. **输出框架**：标题 + 元数据头（URL / 抓取方式 / 长度区间）+ 正文；降级到静态
   内容时在头部注明「可能不含 JS 渲染结果」。
### 代价
- 极少数「静态返回空壳但无 SPA 特征」的页面会停在降级内容上；判据保守，宁可少渲染
  也不把每页拖慢数秒。
- SSRF 检查与真正连接之间存在 DNS rebinding 窗口；单用户桌面场景不引入
  「解析并钉死 IP」的复杂度（那需要自建连接器，绕过 reqwest 的连接池）。
- 局域网地址被拦截：`web_fetch` 不承担内网探测职责；确有内网抓取需求应由用户在
  客户端侧完成。另新增 `pdf-extract` 依赖（纯 Rust，数百 KB）。
## 已沉淀的单点技术选择

结论已完整落在对应 spec 中、且当时不存在有分量替代方案的决策，不再单独成篇。列在此仅为说明「这些事已经定过了」。

| 决策 | 结论 | 详见 |
|------|------|------|
| 密钥双轨 | Provider Key 用 AES-256-GCM（主密钥存 `master.key`，丢失则全部不可恢复）；Service Key 用 Argon2id + salt（校验需逐条遍历） | `module-auth.md` |
| 数据库 UPSERT | `INSERT ... ON CONFLICT DO UPDATE`，禁用 `INSERT OR REPLACE`（会触发级联删除） | `module-database.md` |
| usage_log 快照 | 不用外键，冗余存 provider/model/key 名称快照；每行多约 100 字节 | `module-usage-tracking.md` |
| Token 配额窗口 | `now % window_secs` 对齐，非自然日/自然小时；用量按需聚合，不维护计数器 | `module-token-quota.md` |
| 模型白名单 | 创建 Service Key 时即可设 `allowed_models`，按 `display_name` 匹配 | `module-auth.md` |
| 输出速度口径 | `first_token_ms` + 生成窗口；迁移前历史日志无速度，NULL 表示未知 | `module-usage-tracking.md` |
| 对话审查 | `audit_enabled` 开关 + 指纹归并；关闭时不记录任何内容 | `module-conversation-audit.md` |
| web_fetch 批量与兜底 | `urls` 批量 ≤8 并发 3；Jina Reader 为第三方服务，默认关闭 | `module-mcp-tools.md` |
| Windows 窗口控制 | 去原生标题栏，红绿灯按钮仅 Windows 渲染 | `ARCHITECTURE.md` §4.2 |
| 测试清单嵌入 | `tests.manifest` 载体 target 解决 0xC0000139（**不得删除**）；runner 包装器补齐对真实执行的覆盖 | `AGENTS.md`（测试） |
| 移除 MCP Vision | 视觉能力改由客户端原生承担，网关不再中转图片 | `module-mcp-tools.md` |
| 插件对话框 | `PluginRegisterDialog` 自监听 `plugin-register` 事件 | `module-plugin-system.md` |
