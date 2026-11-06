# Spec: 冷启动动画（Splash）

## 要构建什么
在 Tauri `build()` 阻塞 WebView2 / WKWebView 初始化期间，用**纯 Rust 原生渲染**维持一个环形加载动画；React 首次渲染后关闭。不依赖 WebView，因此初始化卡顿期间动画仍然流畅。

## 行为
```
run()
  ├─ silent_start = args 含 --minimized
  ├─ splash = silent_start ? None : SplashScreen::new()   // 在 Builder::default() 之前创建
  ├─ Builder … .setup()
  │     └ splash.recenter_to_window(main)                 // 吸附到主窗口客户区中心
  └─ app.listen("app-ready")                              // 前端首帧后 emit
        └ run_on_main_thread(|| splash.close())           // 幂等关闭
```

- **静默启动不弹**：开机自启（`--minimized`）窗口隐藏到托盘，屏幕中央闪环反而突兀。
- **创建失败返回 `None`**，不影响正常启动。
- **关闭幂等**：`AtomicBool::swap` 防重复关闭；`Drop` 也会关闭。

| 平台 | 实现 | 关键点 |
|------|------|--------|
| 通用 | `splash/render.rs`（tiny-skia） | 49×49 画布绘制单帧圆环（内外半径 + 轨道色），预乘 alpha；Windows 输出前做 RGBA→BGRA 交换；转速 **360°/s**（与 shadcn/ui spinner 一致） |
| Windows | `splash/win.rs` | 分层窗口 + 独立后台线程消息泵；优先 **DirectComposition**：单张静态帧 + `IDCompositionRotateTransform` 线性无限旋转，DWM 合成器驱动，零 CPU 与刷新率同步；DComp 任一步失败回退 `WM_TIMER` + `UpdateLayeredWindow` 的 30fps 逐帧重绘（需销毁重建窗口） |
| macOS | `splash/macos.rs` | borderless floating `NSWindow` + `CALayer` `transform.rotation` 无限循环，Core Animation 渲染服务器驱动；主线程被 `build()`/setup 阻塞时照常旋转。不用 `CADisplayLink` / `NSTimer`（依赖主 run loop，最需要时失效） |

## 输入 / 输出
- 不适用：本模块无此项。

## 约束
1. **零 WebView 依赖**：动画必须在 WebView 初始化前和期间工作。
2. **主线程安全**：`app-ready` 回调线程不保证是主线程，关窗必须 `run_on_main_thread` 派发（macOS AppKit / Win32 限制）。
3. **对齐**：`"center": true` 只对装饰外框居中；Windows 去装饰后需按 `inner_position` / `inner_size` 重算，DPI / 任务栏 / 多屏免疫。
4. **不做交互**：纯展示，无按钮、无点击穿透需求。

## 边界条件
- 不适用：本模块无此项。

## 验收标准
- 不适用：本模块无此项。

## 完成定义
- `src-tauri/src/splash/mod.rs` — `SplashScreen` 包装（平台别名、关闭、recenter）
- `src-tauri/src/splash/render.rs` — 单帧渲染（tiny-skia）
- `src-tauri/src/splash/win.rs` / `macos.rs` — 平台窗口与动画驱动
- `src-tauri/src/lib.rs` — 创建时机、setup 吸附、`app-ready` 监听
- `src/App.tsx` / `src/main.tsx` — 首帧后发出 `app-ready`

- [x] 非静默启动弹出环形动画，`--minimized` 不弹
- [x] 动画在 WebView 初始化阻塞期间持续帧更新
- [x] 后端 `app-ready` 监听在主线程关闭，幂等
- [x] Windows DComp 优先 + 定时器回退；macOS Core Animation
