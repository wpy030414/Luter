# Luter

> 本机运行、局域网共享的多 Provider LLM API 网关 — Tauri 2 桌面应用

Luter 是一个运行在本地的 LLM API 统一网关。客户端通过 Anthropic Messages、OpenAI Chat Completions 或 OpenAI Responses API 三种端点访问所有大模型 Provider，网关负责路由解析、密钥轮换和用量统计。

## 界面预览

<table>
  <tr>
    <td width="50%">
      <img src="docs/assets/fm.png" alt="FM">
    </td>
    <td width="50%">
      <img src="docs/assets/cloud.ai.png" alt="Cloud AI">
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="docs/assets/combos.png" alt="Combos">
    </td>
    <td width="50%">
      <img src="docs/assets/secrets.png" alt="Secrets">
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="docs/assets/stat.png" alt="Stats">
    </td>
  </tr>
</table>

## 这是什么？

- **定位**：本机运行、局域网共享的 LLM API 网关桌面应用；可为每台设备 / 每个用途签发独立 Service Key
- **核心问题**：LLM 生态协议碎片化（Anthropic/OpenAI 等 API 格式不兼容），开发者需维护多套客户端代码，密钥散落各处

## 为什么存在？

- **统一接入**：通过单一端点访问所有 Provider，客户端零配置
- **数据自持**：数据存在本机 SQLite，不经第三方，隐私安全
- **轻量美观**：Tauri 2 框架，安装包 < 10MB，内存占用 < 100MB
- **现有方案不足**：OpenRouter 是云端 SaaS、LiteLLM 是 Python 服务、one-api 需要 Docker 部署，本地开发体验差

## 如何安装和运行？

### 前置要求

- **Rust**（stable toolchain，edition 2021）
- **Node.js** >= 20 + **pnpm**
- **Tauri CLI** 已包含在 devDependencies 中（`pnpm dev` / `pnpm build` 直接调用）
- 平台：macOS / Windows。Linux 未纳入构建矩阵

### 安装与运行

```bash
# 前端依赖
pnpm install

# 开发模式（前端 :5173 + 后端 :19068）
pnpm dev

# 生产构建（macOS .dmg / Windows .msi）
pnpm build
```

### 接入客户端

- **Base URL**：`http://127.0.0.1:19068`
- **API Key**：在应用内「密钥管理」页创建的 Service Key
- **模型**：使用应用内配置的模型别名（网关负责路由到真实上游）

### 局域网分发

把本机变成局域网 API 网关：在「密钥管理」页创建密钥后，弹窗里复制「分发链接」，发给局域网设备打开。Install 页面支持 Claude Code 和 ChatGPT/Codex 两种消费端，按平台生成单行命令。详见 [docs/specs/module-lan-deploy.md](docs/specs/module-lan-deploy.md)。

## 当前状态

- **阶段**：stable（核心功能已完成，版本号为 CalVer，见 `src-tauri/tauri.conf.json`）
- **已知限制**：
  - 仅支持三种内建协议（Anthropic Messages / OpenAI Chat Completions / OpenAI Responses），新协议走插件系统
  - 多用户靠「多 Service Key + 局域网分发」实现，各自独立配额与模型白名单；管理 API 无认证，仅靠 loopback IP 限制。不做云端 SaaS / 公网部署 / Docker
  - Claude FM 桌面壁纸劫持仅支持 Windows 11 / macOS，且仅主显示器（分辨率/DPI 变化后需重新勾选恢复）
  - 本地模型依赖外部 `llama-server` 引擎二进制，需 GPU 后端受支持

## 核心技术

- **后端**：Rust + Tauri 2 + axum 0.7 + tokio
- **数据库**：SQLite 3 (WAL 模式)
- **前端**：React 19 + Zustand + shadcn/ui (Radix UI + Tailwind CSS)
- **协议转换**：IR（中间表示层）统一 Anthropic Messages / OpenAI Chat Completions / OpenAI Responses API 三种格式
- **安全**：AES-256-GCM 加密 Provider Key + Argon2 哈希 Service Key；`/api/*` 仅 loopback 可访问；出站 URL 过 SSRF 守卫
- **内置能力**：MCP 工具服务器（web_search / web_fetch / notify）、本地 GGUF 模型管理、Claude FM

## 项目文档

```
AGENTS.md            — Agent 工作边界与约定（本项目的最高操作约束）
docs/
├── PRD.md           — 产品需求文档（功能存在的意义）
├── ARCHITECTURE.md  — 架构地图（稳定的结构关系）
├── DECISIONS.md     — 架构决策记录（历史原因）
└── specs/           — 模块规格（要做什么、做到什么程度算完成），索引见 specs/README.md
```

## CI

push 到 main 自动构建 macOS arm64 (.dmg) 和 Windows amd64 (.msi) 安装包，发布到 GitHub Releases。

> README 不承载项目的完整文档职责，详细信息见 [docs/](docs/)。
