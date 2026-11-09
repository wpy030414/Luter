//! 插件 WebSocket（注册/心跳/config 消息循环）+ REST 管理接口。
//!
//! V24 契约：Router 不再为插件管理密钥。register 携带 `keys` 字段或收到
//! `keys_update` 消息一律**严格拒绝**（回 `keys_not_supported` 错误并断开），
//! 强制插件方按新契约升级。

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Json, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use tracing::{info, warn, error};

use crate::gateway::server::AppState;
use crate::plugin::PluginRegisterMsg;

pub(crate) async fn plugin_ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(client_addr): ConnectInfo<std::net::SocketAddr>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    info!("Plugin WS: incoming connection from {}", client_addr);
    ws.on_upgrade(move |socket| handle_plugin_ws(socket, state, client_addr))
}

/// register 首条消息的校验拒绝原因（供回错误消息与单测断言）。
#[derive(Debug, PartialEq)]
pub(crate) enum RegisterReject {
    /// 携带已废除的 `keys` 字段（无论空数组、null 还是有值——字段存在即拒绝）
    KeysNotSupported,
    NotRegister,
    EmptyPluginId,
    BadJson,
}

/// 解析并校验 register 首条消息（纯函数，便于内联单测）。
/// 注意：serde 默认忽略未知字段，`keys` 的存在性必须在原始 `Value` 上判定。
pub(crate) fn parse_register_msg(v: &serde_json::Value) -> Result<PluginRegisterMsg, RegisterReject> {
    if v.get("type").and_then(|t| t.as_str()) != Some("register") {
        return Err(RegisterReject::NotRegister);
    }
    // 严格口径：`keys` 字段存在即拒绝，强制插件方立即升级到无密钥契约
    if v.get("keys").is_some() {
        return Err(RegisterReject::KeysNotSupported);
    }
    let msg: PluginRegisterMsg = serde_json::from_value(v.clone()).map_err(|_| RegisterReject::BadJson)?;
    if msg.plugin_id.trim().is_empty() {
        return Err(RegisterReject::EmptyPluginId);
    }
    Ok(msg)
}

async fn handle_plugin_ws(mut socket: WebSocket, state: Arc<AppState>, client_addr: std::net::SocketAddr) {
    // First message must be "register"
    let plugin_id = match socket.recv().await {
        Some(Ok(Message::Text(text))) => {
            let msg: serde_json::Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(e) => {
                    warn!("Plugin WS: invalid first message: {}", e);
                    return;
                }
            };
            let reg_msg = match parse_register_msg(&msg) {
                Ok(m) => m,
                Err(reject) => {
                    let reason = match reject {
                        RegisterReject::KeysNotSupported => {
                            warn!("Plugin WS: rejected register with legacy 'keys' field from {}", client_addr);
                            "keys_not_supported"
                        }
                        RegisterReject::NotRegister => {
                            warn!("Plugin WS: first message must be 'register'");
                            "expected_register"
                        }
                        RegisterReject::EmptyPluginId => {
                            warn!("Plugin WS: rejected register with empty plugin_id from {}", client_addr);
                            "empty_plugin_id"
                        }
                        RegisterReject::BadJson => {
                            warn!("Plugin WS: invalid register message from {}", client_addr);
                            "invalid_register"
                        }
                    };
                    let _ = socket.send(Message::Text(
                        serde_json::json!({"type": "error", "reason": reason}).to_string().into()
                    )).await;
                    return;
                }
            };

            match state.plugins.register(reg_msg.clone()) {
                Ok((provider_id, needs_confirmation)) => {
                    let resp = if needs_confirmation {
                        serde_json::json!({
                            "type": "registered",
                            "provider_id": provider_id,
                            "status": "pending_confirmation"
                        })
                    } else {
                        serde_json::json!({
                            "type": "reconnected",
                            "provider_id": provider_id
                        })
                    };
                    let _ = socket.send(Message::Text(resp.to_string().into())).await;
                    info!("Plugin WS: registered, provider={}", provider_id);
                    // 注意：循环里的 plugin_id 必须是插件名（plugins 表主键），
                    // 不能是 provider_id（UUID）——否则 is_registered() 永远查不到，
                    // 会把每次心跳误判为「插件已被删除」而踢掉连接。
                    reg_msg.plugin_id
                }
                Err(e) => {
                    error!("Plugin WS: register failed: {}", e);
                    return;
                }
            }
        }
        _ => {
            warn!("Plugin WS: expected text register message");
            return;
        }
    };

    // Message loop
    loop {
        match socket.recv().await {
            Some(Ok(Message::Text(text))) => {
                let msg: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or("");

                // 插件已被用户删除（如忽略对话框）→ 关闭连接，让插件重连后重新注册、重新弹窗
                if !state.plugins.is_registered(&plugin_id) {
                    info!("Plugin {} was deleted by user, closing connection for re-registration", plugin_id);
                    let _ = socket.send(Message::Text(
                        serde_json::json!({"type": "deleted", "reason": "plugin_ignored"}).to_string().into()
                    )).await;
                    break;
                }

                match msg_type {
                    "heartbeat" => {
                        state.plugins.heartbeat(&plugin_id);
                    }
                    // V24：密钥同步已废除，收到即回错并断开（强制插件方升级）
                    "keys_update" => {
                        warn!("Plugin WS: rejected keys_update from {}, closing connection", plugin_id);
                        let _ = socket.send(Message::Text(
                            serde_json::json!({"type": "error", "reason": "keys_not_supported"}).to_string().into()
                        )).await;
                        break;
                    }
                    "config_update" => {
                        let base_url = msg.get("base_url").and_then(|v| v.as_str()).map(String::from);
                        let api_path = msg.get("api_path").and_then(|v| v.as_str()).map(String::from);
                        state.plugins.handle_config_update(&plugin_id, base_url, api_path);
                        info!("Plugin WS: config_update for {}", plugin_id);
                    }
                    _ => {
                        // Unknown message type, ignore
                    }
                }
            }
            Some(Ok(Message::Close(_))) | None => {
                break;
            }
            _ => {}
        }
    }

    // Plugin disconnected
    state.plugins.disconnect(&plugin_id);
    info!("Plugin WS: {} disconnected", plugin_id);
}

#[derive(Serialize)]
struct PluginListItem {
    id: String,
    provider_id: Option<String>,
    status: String,
    last_heartbeat_at: Option<i64>,
    connected: bool,
    work_dir: Option<String>,
    autostart: bool,
}

pub(crate) async fn list_plugins(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let connected = state.plugins.list_connected();
    let connected_ids: std::collections::HashSet<String> = connected.iter()
        .map(|c| c.plugin_id.clone())
        .collect();

    // Get all plugins from DB
    let conn = state.database.conn();
    let mut stmt = match conn.prepare(
        "SELECT id, provider_id, status, last_heartbeat_at, work_dir, autostart FROM plugins"
    ) {
        Ok(s) => s,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response();
        }
    };
    let plugins: Vec<PluginListItem> = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        Ok(PluginListItem {
            connected: connected_ids.contains(&id),
            id,
            provider_id: row.get(1)?,
            status: row.get(2)?,
            last_heartbeat_at: row.get(3)?,
            work_dir: row.get(4)?,
            autostart: row.get::<_, i64>(5)? != 0,
        })
    }).unwrap().filter_map(|r| r.ok()).collect();

    Json(plugins).into_response()
}

pub(crate) async fn confirm_plugin(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    state.plugins.confirm(&id).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()})))
    })?;
    Ok(Json(serde_json::json!({"status": "confirmed", "plugin_id": id})))
}

pub(crate) async fn delete_plugin(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // 先回收 Router 托管的伴生进程（若有），再断连、删记录
    state.plugin_host.kill(&id);

    // Disconnect if connected
    if state.plugins.is_connected(&id) {
        state.plugins.disconnect(&id);
    }

    // Get provider_id + delete plugin record（conn 锁在块内释放，Mutex 不可重入）
    let provider_id: Option<String> = {
        let conn = state.database.conn();
        let pid: Option<String> = conn.query_row(
            "SELECT provider_id FROM plugins WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get(0),
        ).ok().flatten();

        conn.execute("DELETE FROM plugins WHERE id = ?1", rusqlite::params![id])
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))))?;

        pid
    }; // conn 锁在此释放

    // Delete associated provider (cascades to keys + models)
    if let Some(pid) = provider_id {
        let _ = state.database.delete_provider(&pid);
        // 同步内存 registry + KeyPool
        state.providers.remove(&pid);
        state.keys.remove_provider(&pid);
    }

    Ok(Json(serde_json::json!({"status": "deleted", "plugin_id": id})))
}

/// GET /api/plugins/:id — 返回插件完整预填数据（provider + models + 伴生启动状态），
/// 供前端 ProviderNewView 以插件模式渲染表单。
pub(crate) async fn get_plugin(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // 1. 查 plugins 表拿 provider_id + 状态 + work_dir/autostart（conn 锁在块内释放，Mutex 不可重入）
    let (provider_id, status, work_dir, autostart) = {
        let conn = state.database.conn();
        let plugin_row = conn.query_row(
            "SELECT provider_id, status, work_dir, autostart FROM plugins WHERE id = ?1",
            rusqlite::params![id],
            |row| Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)? != 0,
            )),
        );
        match plugin_row {
            Ok(r) => r,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Plugin not found"}))));
            }
            Err(e) => {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))));
            }
        }
    }; // conn 锁在此释放

    // 2. 查关联 provider
    let provider = match provider_id.as_deref().and_then(|pid| state.providers.get(pid)) {
        Some(p) => p,
        None => {
            return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Provider not found"}))));
        }
    };

    // 3. 查该 provider 的模型
    let models: Vec<serde_json::Value> = state.database.list_all_models()
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.provider_id == provider.id)
        .map(|m| serde_json::json!({
            "model_id": m.model_id,
            "display_name": m.display_name,
            "tier": m.tier,
        }))
        .collect();

    // 4. 插件是否在线
    let connected = state.plugins.is_connected(&id);

    Ok(Json(serde_json::json!({
        "plugin_id": id,
        "status": status,
        "connected": connected,
        "work_dir": work_dir,
        "autostart": autostart,
        "provider": {
            "id": provider.id,
            "name": provider.name,
            "kind": provider.kind.to_string(),
            "base_url": provider.base_url,
            "api_path": provider.api_path,
        },
        "models": models,
    })))
}

/// GET /api/plugins/runtime — 系统内 node / pnpm 可用性（伴生启动前置条件）。
/// 值为版本串（如 "v20.11.1"），null 表示不可用。
pub(crate) async fn get_plugin_runtime(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let rt = state.plugin_host.check_runtime(false).await;
    Json(serde_json::json!({ "node": rt.node, "pnpm": rt.pnpm }))
}

#[derive(Deserialize, Default)]
pub struct UpdatePluginBody {
    autostart: Option<bool>,
    work_dir: Option<String>,
}

/// PATCH /api/plugins/:id — 更新伴生启动开关 / 项目目录。
pub(crate) async fn update_plugin(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdatePluginBody>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // 确认插件存在
    if state.plugins.get_plugin_record(&id).is_err() {
        return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Plugin not found"}))));
    }

    if let Some(dir) = body.work_dir.as_deref() {
        let trimmed = dir.trim();
        if !trimmed.is_empty() && !std::path::Path::new(trimmed).is_dir() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "work_dir_not_found", "detail": trimmed})),
            ));
        }
        let v = if trimmed.is_empty() { None } else { Some(trimmed) };
        state.plugins.set_plugin_workdir(&id, v).map_err(internal_err)?;
    }
    if let Some(on) = body.autostart {
        state.plugins.set_plugin_autostart(&id, on).map_err(internal_err)?;
    }

    let record = state.plugins.get_plugin_record(&id).map_err(internal_err)?;
    Ok(Json(serde_json::json!({
        "plugin_id": id,
        "autostart": record.autostart,
        "work_dir": record.work_dir,
    })))
}

/// POST /api/plugins/:id/login — 在插件项目目录运行 `pnpm run login`，
/// login 脚本契约要求自行弹出系统浏览器完成登录（不要求客户端到位）。
pub(crate) async fn login_plugin(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let record = state.plugins.get_plugin_record(&id).map_err(internal_err)?;
    let work_dir = record
        .work_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    let work_dir = match work_dir {
        Some(d) => d,
        None => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "work_dir_missing"})),
            ));
        }
    };

    let rt = state.plugin_host.check_runtime(false).await;
    if rt.node.is_none() || rt.pnpm.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "runtime_unavailable",
                "node": rt.node,
                "pnpm": rt.pnpm,
            })),
        ));
    }

    // 脚本名由 host 按 package.json 自动判定（login / log-in）；缺脚本时这里返回 400
    if let Err(e) = state.plugin_host.spawn_login(&id, &work_dir) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ));
    }
    Ok((StatusCode::ACCEPTED, Json(serde_json::json!({"status": "launched"}))))
}

/// POST /api/plugins/:id/serve —— 启动插件网关（幂等：已托管则直接返回）
pub(crate) async fn start_plugin(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let record = state.plugins.get_plugin_record(&id).map_err(internal_err)?;
    let work_dir = record
        .work_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    let Some(work_dir) = work_dir else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "work_dir_missing"})),
        ));
    };

    let rt = state.plugin_host.check_runtime(false).await;
    if rt.node.is_none() || rt.pnpm.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "runtime_unavailable",
                "node": rt.node,
                "pnpm": rt.pnpm,
            })),
        ));
    }

    state.plugin_host.spawn_serve(&id, &work_dir).map_err(internal_err)?;
    Ok((StatusCode::ACCEPTED, Json(serde_json::json!({"status": "starting"}))))
}

/// POST /api/plugins/:id/stop —— 停止 Router 托管的插件网关。
///
/// 只杀 Router 自己 spawn 的进程；用户手动启动的网关返回 409，
/// 前端据此提示「请在外部停止」，不谎报成功。
pub(crate) async fn stop_plugin(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let work_dir = state.plugins.get_plugin_record(&id).ok().and_then(|r| r.work_dir);
    if !state.plugin_host.stop(&id, work_dir.as_deref()) {
        return Err((StatusCode::CONFLICT, Json(serde_json::json!({"error": "not_hosted"}))));
    }
    Ok(Json(serde_json::json!({"status": "stopped"})))
}

// ── 插件市场（GitHub 搜索 + 一键安装） ──

#[derive(Deserialize, Default)]
pub(crate) struct MarketSearchQuery {
    /// 跳过 30s 缓存，强制重查 GitHub
    #[serde(default)]
    pub force: bool,
}

/// GET /api/plugin-market/search
pub(crate) async fn search_market_plugins(
    State(state): State<Arc<AppState>>,
    Query(q): Query<MarketSearchQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let items = crate::plugin::market::search(&state.http_client, q.force)
        .await
        .map_err(internal_err)?;
    Ok(Json(serde_json::json!({ "items": items })))
}

#[derive(Deserialize)]
pub(crate) struct InstallMarketPluginReq {
    pub repo: String,
}

/// POST /api/plugin-market/install
///
/// clone + pnpm install 后立即 `pnpm run serve`；插件会按 V24 契约自行 WS 注册，
/// 前端轮询 `/api/plugins` 拿到 plugin_id 后调用 confirm 完成启用。
pub(crate) async fn install_market_plugin(
    State(state): State<Arc<AppState>>,
    Json(req): Json<InstallMarketPluginReq>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let rt = state.plugin_host.check_runtime(false).await;
    if rt.node.is_none() || rt.pnpm.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "runtime_unavailable",
                "node": rt.node,
                "pnpm": rt.pnpm,
            })),
        ));
    }

    let out = crate::plugin::market::install(state.plugin_host.data_dir(), &req.repo)
        .await
        .map_err(internal_err)?;

    // 托管键用目录名（插件真实 plugin_id 要等它注册后才可知，不能从仓库名推导：
    // 如 xrp-cosy 注册的是 plugin-qwenwork）
    let work_path = std::path::Path::new(&out.work_dir);
    let key = work_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| out.repo.clone());

    // 有登录脚本 → 交给前端下一步调 /api/plugin-market/login（登录成功后才 serve）；
    // 没有登录脚本的插件才在这里直接拉起。
    let login_script = crate::plugin::market::detect_login_script(work_path);
    if login_script.is_none() {
        state
            .plugin_host
            .spawn_serve(&key, &out.work_dir)
            .map_err(internal_err)?;
    }

    Ok(Json(serde_json::json!({
        "repo": out.repo,
        "work_dir": out.work_dir,
        "login_script": login_script,
        "status": if login_script.is_some() { "needs_login" } else { "starting" },
    })))
}

#[derive(Deserialize)]
pub(crate) struct MarketLoginReq {
    pub work_dir: String,
}

/// POST /api/plugin-market/login
///
/// 插件网关没有凭证时照样监听端口、但所有推理请求都会 401，所以**必须登录后再 serve**。
/// 登录是人工环节（扫码/账密），本请求会阻塞到登录完成，成功后立即拉起网关；
/// 插件随后按 V24 契约自行 WS 注册。
pub(crate) async fn login_market_plugin(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MarketLoginReq>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let work_dir = req.work_dir.trim();
    if work_dir.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "work_dir_missing"})),
        ));
    }

    // 安全边界：只允许操作 {data_dir}/plugins 下的目录，
    // 防止把本接口当成「任意目录执行 pnpm run」的入口。
    let root = state.plugin_host.data_dir().join("plugins");
    let canon_root = root.canonicalize().unwrap_or(root);
    let canon = std::path::Path::new(work_dir).canonicalize().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("work_dir_invalid: {e}")})),
        )
    })?;
    if !canon.starts_with(&canon_root) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "work_dir_not_allowed"})),
        ));
    }

    let rt = state.plugin_host.check_runtime(false).await;
    if rt.node.is_none() || rt.pnpm.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "runtime_unavailable",
                "node": rt.node,
                "pnpm": rt.pnpm,
            })),
        ));
    }

    // 用原始路径执行（不用 canonicalize 结果）：Windows 的 `\\?\` 前缀会让
    // 插件 process.cwd() 与前端记录的 work_dir 对不上，导致「等待注册」匹配失败。
    let script = crate::plugin::market::login(std::path::Path::new(work_dir))
        .await
        .map_err(internal_err)?;

    let key = std::path::Path::new(work_dir)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "plugin".to_string());
    state
        .plugin_host
        .spawn_serve(&key, work_dir)
        .map_err(internal_err)?;

    Ok(Json(serde_json::json!({
        "status": "starting",
        "login_script": script,
    })))
}

fn internal_err(e: anyhow::Error) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_register_msg_success_with_workdir() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"type":"register","plugin_id":"wukong","provider":{"kind":"chat_completions","base_url":"http://127.0.0.1:19067","api_path":"/v1/chat"},"models":[],"workdir":"/tmp/wukong"}"#
        ).unwrap();
        let msg = parse_register_msg(&v).unwrap();
        assert_eq!(msg.plugin_id, "wukong");
        assert_eq!(msg.workdir.as_deref(), Some("/tmp/wukong"));
    }

    #[test]
    fn parse_register_msg_success_without_workdir() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"type":"register","plugin_id":"wukong","provider":{"kind":"messages","base_url":"http://x","api_path":"/y"},"models":[]}"#
        ).unwrap();
        let msg = parse_register_msg(&v).unwrap();
        assert_eq!(msg.workdir, None);
    }

    #[test]
    fn parse_register_msg_rejects_keys_empty_array() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"type":"register","plugin_id":"wukong","provider":{"kind":"messages","base_url":"http://x","api_path":"/y"},"models":[],"keys":[]}"#
        ).unwrap();
        assert_eq!(parse_register_msg(&v).unwrap_err(), RegisterReject::KeysNotSupported);
    }

    #[test]
    fn parse_register_msg_rejects_keys_nonempty() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"type":"register","plugin_id":"wukong","provider":{"kind":"messages","base_url":"http://x","api_path":"/y"},"models":[],"keys":["sk-xxx"]}"#
        ).unwrap();
        assert_eq!(parse_register_msg(&v).unwrap_err(), RegisterReject::KeysNotSupported);
    }

    #[test]
    fn parse_register_msg_rejects_keys_null() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"type":"register","plugin_id":"wukong","provider":{"kind":"messages","base_url":"http://x","api_path":"/y"},"models":[],"keys":null}"#
        ).unwrap();
        assert_eq!(parse_register_msg(&v).unwrap_err(), RegisterReject::KeysNotSupported);
    }

    #[test]
    fn parse_register_msg_rejects_empty_plugin_id() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"type":"register","plugin_id":"   ","provider":{"kind":"messages","base_url":"http://x","api_path":"/y"},"models":[]}"#
        ).unwrap();
        assert_eq!(parse_register_msg(&v).unwrap_err(), RegisterReject::EmptyPluginId);
    }

    #[test]
    fn parse_register_msg_rejects_missing_type() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"plugin_id":"wukong","provider":{"kind":"messages","base_url":"http://x","api_path":"/y"},"models":[]}"#
        ).unwrap();
        assert_eq!(parse_register_msg(&v).unwrap_err(), RegisterReject::NotRegister);
    }

    #[test]
    fn parse_register_msg_rejects_wrong_type() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"type":"heartbeat","plugin_id":"wukong","provider":{"kind":"messages","base_url":"http://x","api_path":"/y"},"models":[]}"#
        ).unwrap();
        assert_eq!(parse_register_msg(&v).unwrap_err(), RegisterReject::NotRegister);
    }

    #[test]
    fn parse_register_msg_rejects_bad_json() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"type":"register","plugin_id":"wukong"}"#
        ).unwrap();
        assert_eq!(parse_register_msg(&v).unwrap_err(), RegisterReject::BadJson);
    }
}
