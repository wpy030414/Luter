# Spec: 本地模型管理（Local Models）

## 要构建什么
把开源 GGUF 模型变成「零配置私有 Provider」：用户导入本地 GGUF 文件 → 启动 `llama-server` → 以普通 Provider/Model/Key 身份注册进网关（`http://127.0.0.1:<port>`），与云端 Provider 一样参与路由、组合、密钥与统计体系。

## 行为
```
LocalModelsView.tsx ── REST /api/local/* ──▶ local/mod.rs   LocalManager（注册表 + 生命周期）
                                             ├─ engine.rs   llama-server 二进制下载/解压 + 健康检查 + 端口
                                             └─ backend.rs  GPU 后端检测（Metal/CUDA/Vulkan/ROCm/CPU）
```

- **引擎**：外部二进制 `llama-server`（`ggml-org/llama.cpp` releases，tag 见 `backend::LOCAL_LLAMA_TAG`），首次启动时按后端下载并解压到数据目录 `engines/`；每个模型一个进程。
- **启动参数**（`engine::spawn_llama_server`，固定传入）：`--model <gguf> --host 127.0.0.1 --port <port> --ctx-size --n-gpu-layers --reasoning on|off --api-key <随机> --parallel 1`；stdout/stderr 落 `engines/engine-{id}.log`。
- **端口**：`engine::free_port()` 绑定 `127.0.0.1:0` 由系统分配，启动时一次确定；stop 后释放。
- **注册语义**（`register_provider`）：
  - Provider：id `local-{lm-id}`，kind `chat_completions`，`base_url=http://127.0.0.1:<port>`，`api_path=/v1/chat/completions`，`sort_order` 取当前最大值之后
  - Model：id `local-m-{lm-id}`，`display_name = model_id`（用户命名），`tier="local"`，`context_window = ctx_size`，`capabilities=["text","tools"]`
  - Key：id `local-k-{lm-id}`，随机 32 位 hex，AES-256-GCM 入库并立即装载进 KeyPool（引擎的 `--api-key` 即此值）
- **状态**：`LocalManager` 持引擎句柄（`running: Mutex<HashMap<id, RunningEngine>>`），纯内存；状态经 `local_status` 事件广播，DB `local_models.status` 同步为 `downloaded|running|error`。

- **导入**：`POST` 携带 `local_path` → 校验存在且扩展名为 `.gguf`（格式必须 `gguf`）→ 名称冲突检查（`alias_taken` 同时查 models 与 combos）→ 生成 `lm-{uuid}` → 拷贝到 `models/{id}/model.gguf` → 记录 `file_size` → `status=downloaded`；拷贝或元数据失败即删除目标文件并报错，不产生半成品记录。`ctx_size` 下限 1024、`n_gpu_layers` 下限 0，`thinking` 新建时为 1。
- **后端解析**：`backend=auto` 时取检测列表中首个可用且非 CPU 的后端，否则 CPU；启动序列为「首选 + CPU 兜底」，逐个尝试，全部失败才报错（错误汇总各后端原因 + 日志路径）。`GET /api/local/backends` 返回候选及可用理由，进程内缓存（`OnceLock`）。
- **健康检查**：`wait_healthy` 轮询引擎 `GET /v1/models`（Bearer），超时 **120s**（`START_TIMEOUT`）。
- **崩溃重启**：watcher 每 500ms `try_wait`；进程退出 → 状态置 `error` 并广播 → 按 **5s / 15s / 45s** 退避重启，**最多 3 次**；期间被 stop 移除则放弃；3 次全败放弃（记 error 日志）。
- **停止**：从 running 表移除 → unix `kill -9 <pid>` / Windows `taskkill /F /PID` → 等 child 回收 → provider `enabled=false`（DB + 内存）→ 从 KeyPool 移除该 provider → 状态回 `downloaded`、`port=NULL`。
- **自启动**：`auto_start_all()` 在应用启动时（含 `--minimized`）顺序拉起 `autostart=1` 的模型。
- **思考模式**：`local_models.thinking`（V21，默认 1）总是显式映射为 `--reasoning on|off`；运行中修改下次启动生效（同 `ctx_size` / `n_gpu_layers`）。
- **路由优先级**：组合 > 私有（`tier='local'`）> 云。`resolve_route_candidates()` 把本地模型排前；组合先展开成员，成员内部同样遵循此优先级。
- **管理面隔离**：`/api/providers` 不返回 `local-*` provider；`/api/models` 与 `/api/keys` 仍返回全部记录（组合成员选择器与白名单需要看到私有模型）。

## 输入 / 输出
| 端点 | 功能 |
|------|------|
| `GET /api/local/models` | 列出本地模型（含运行状态/端口） |
| `POST /api/local/models` | 导入本地 GGUF（`local_path` 必填）+ 引擎参数 |
| `POST /api/local/models/:id/edit` | 编辑参数（别名、ctx、GPU 层、后端、自启动、思考模式） |
| `DELETE /api/local/models/:id` | 停止 → 删 provider（级联 models/keys）→ 删记录 → 可选删权重 |
| `POST /api/local/models/:id/start` / `/stop` | 引擎生命周期 |
| `GET /api/local/backends` | GPU 后端检测结果 |

- 引擎未就绪时请求该模型 → provider 不可用，走 failover 或返回错误（与普通上游不可达同语义）。
- `/v1/models` 中本地模型与云端模型统一呈现（tier=`local`）。

## 约束
- 不适用：本模块无此项。

## 边界条件
| 情况 | 行为 |
|------|------|
| 文件不存在 / 非 `.gguf` | 导入失败并返回原因 |
| 名称与现有模型或组合冲突 | 返回「模型名已被占用」 |
| 引擎二进制下载/解压失败 | 该后端启动失败，继续下一后端；全部失败返回错误 |
| 启动超时（120s） | 返回错误并附引擎日志路径 |
| 崩溃 | 退避重启 ≤3 次，失败后状态 `error` |

## 验收标准
- 不适用：本模块无此项。

## 完成定义
- Rust：`src-tauri/src/local/{mod,engine,backend}.rs` + `api/handlers/local.rs` + `db/local_models.rs`
- 前端：`src/views/LocalModelsView.tsx`、`src/stores/localModels.ts`
- 表结构：见 module-database（`local_models`，V20/V21）
- 下载走统一 `http::build_http_client()`

- [x] GGUF 导入（校验 + 拷贝 + 回滚），状态可见
- [x] 引擎下载、启动/停止、120s 健康检查、5s/15s/45s 崩溃重启（最多 3 次）
- [x] 自启动恢复
- [x] GPU 后端检测 + auto 降级 + 手动覆盖
- [x] `local-{id}` provider / `local-m-{id}` model / `local-k-{id}` key 注册，参与路由与统计
