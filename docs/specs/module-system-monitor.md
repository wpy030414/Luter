# Spec: 系统资源监控

## 要构建什么
为前端提供本机实时 CPU / 内存占用，支撑本地模型页的状态条（运行 `llama-server` 时能直接看到资源压力）。

## 行为
1. 构造时 `System::new_all()` → 睡眠 **200ms** → `refresh_all()`：CPU 使用率需要两次采样间隔才有值，构造期先建立基线。
2. 每次调用 `refresh_cpu_all()` + `refresh_memory()`，读 `global_cpu_usage()`、`used_memory()`、`total_memory()`（sysinfo 单位换算为字节）。
3. 全局单实例，持 `Mutex<System>`；无后台轮询线程，**按需刷新**。

`src/components/SystemStatusBar.tsx` 挂载时立即拉取一次，之后每 **2 秒** `invoke('get_system_resources')` 刷新并渲染 CPU / 内存百分比；仅在 `AppShell` 的 `/local` 路由下挂载（`location.pathname === '/local'`）。文案 key `system.cpu` / `system.memory`（zh-CN / en）。

## 输入 / 输出
- **Tauri 命令**：`invoke("get_system_resources")` → `SystemResources`
- **托管**：`SystemMonitor` 以 `tauri::State<Mutex<SystemMonitor>>` 注册，命令内加锁取样
- **输出**：

```rust
pub struct SystemResources {
    pub cpu_usage: f32,   // 0–100，全局平均
    pub used_memory: u64, // 字节
    pub total_memory: u64,
}
```

## 约束
1. **不跨平台差异处理**：sysinfo 负责各平台采集，本项目不做显存统计（`SystemResources` 只有 CPU + 内存）。
2. **失败静默**：命令失败时前端只记 console 错误，不打断页面。
3. **不加后台推送**：不新增 WebSocket 事件，保持拉取模型。

## 边界条件
- 不适用：本模块无此项。

## 验收标准
- 不适用：本模块无此项。

## 完成定义
- `src-tauri/src/system.rs` — `SystemMonitor` / `SystemResources`
- `src-tauri/src/lib.rs` — `get_system_resources` 命令注册
- `src/components/SystemStatusBar.tsx` — 展示

- [x] `get_system_resources` 返回 CPU 百分比与内存字节数
- [x] 构造期建立 CPU 采样基线
- [x] 本地模型页展示实时状态
