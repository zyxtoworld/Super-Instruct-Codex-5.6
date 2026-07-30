// Super-Instruct — Tauri 桌面应用入口
// MITM Core 作为 Tauri 后端进程运行，前端通过事件系统接收实时数据

pub mod core;
pub mod deploy;
pub mod extensions;
pub mod log;

use std::sync::Arc;
use tokio::sync::RwLock;
use tauri::Emitter;
use tauri::Manager;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use futures::StreamExt;

use crate::core::MitmCore;
use crate::deploy::{find_relay_url, DeployManager};
use crate::extensions::inject::SystemPromptInjector;
use crate::extensions::memory::MemoryKernel;
use crate::extensions::monitor::{InteractionEvent, MonitorPanel, StatsEvent};
use crate::extensions::sse_parser::UniversalSseParser;
use crate::extensions::tamper::TamperEngine;

// ── AppState ──────────────────────────────────────────────
pub struct AppState {
    core: RwLock<Option<Arc<MitmCore>>>,
    proxy_handle: RwLock<Option<tokio::task::JoinHandle<()>>>,
    monitor: RwLock<Option<Arc<MonitorPanel>>>,
    memory: RwLock<Option<Arc<MemoryKernel>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            core: RwLock::new(None),
            proxy_handle: RwLock::new(None),
            monitor: RwLock::new(None),
            memory: RwLock::new(None),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// ── 应用入口 ──────────────────────────────────────────────
pub fn run() {
    // 日志: 控制台 + 文件双输出, guard 保活到应用结束
    // 写到 src-tauri/ 的父目录（项目根），避免 Tauri dev 文件监视器触发重建循环
    let log_dir = std::env::current_dir()
        .ok()
        .and_then(|d| d.parent().map(|p| p.join("logs")))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "../logs".to_string());
    let _log_guard = log::init_logging(&log_dir);

    tracing::info!("Super-Instruct starting up");

    tauri::Builder::default()
        .manage(AppState::new())
        .on_window_event(|window, event| {
            // 拦截关闭按钮: 隐藏窗口而非退出，让用户通过托盘退出
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            // 系统托盘
            let show_item = MenuItem::with_id(app, "show", "显示主界面", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出程序", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            let _tray = TrayIconBuilder::new()
                .menu(&menu)
                .show_menu_on_left_click(false)
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Super-Instruct")
                .on_menu_event(|app, event| {
                    match event.id.as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => {
                            // 先销毁窗口再退出，避免 Chromium "Failed to unregister class" Error 1412
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.destroy();
                            }
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_proxy,
            stop_proxy,
            deploy_bridge,
            restore_codex,
            get_stats,
            get_history,
            get_proxy_status,
            get_codex_info,
            minimize_window,
            toggle_maximize,
            close_window,
            show_window,
            quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ── Tauri Commands ────────────────────────────────────────

#[tauri::command]
async fn start_proxy(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    if state.core.read().await.is_some() {
        return Err("Proxy already running".into());
    }

    tracing::info!("start_proxy: initializing");

    // 读取 bridge.md
    let bridge_path = find_resource_file("bridge.md").map_err(|e| {
        tracing::error!("start_proxy: {}", e);
        e
    })?;
    let instructions = std::fs::read_to_string(&bridge_path)
        .map_err(|e| {
            let msg = format!("Failed to read bridge.md: {}", e);
            tracing::error!("start_proxy: {}", msg);
            msg
        })?;

    // 自动部署 — 与 Python 原版行为一致: 启动代理前自动修改 Codex config.toml
    if let Some(manager) = DeployManager::new() {
        let status = manager.status();
        if !status.bridge_active {
            tracing::info!("start_proxy: auto-deploying bridge.md + skills");
            let skills_dir = find_resource_dir("skills").map_err(|e| {
                tracing::error!("start_proxy: {}", e);
                e
            })?;
            match manager.apply(&instructions, &skills_dir) {
                Ok(msg) => tracing::info!("start_proxy: auto-deploy: {}", msg),
                Err(e) => {
                    tracing::error!("start_proxy: auto-deploy failed: {}", e);
                    return Err(format!("Auto-deploy failed: {}", e));
                }
            }
        } else {
            tracing::info!("start_proxy: already deployed, skipping auto-deploy");
        }
    } else {
        tracing::warn!("start_proxy: Codex home not found, skipping auto-deploy");
    }

    // 自动检测中转站地址 (deploy 后从备份读取原始地址)
    let relay_url = find_relay_url().unwrap_or_else(|| "http://127.0.0.1:57321".to_string());
    let relay_url_display = relay_url.clone();
    tracing::info!("start_proxy: relay_url = {}", relay_url_display);

    // 创建扩展实例 (Arc 共享 between Core 和 Tauri commands)
    let monitor = Arc::new(MonitorPanel::new(app.clone()));
    // 写到 src-tauri/ 的父目录（项目根），避免 Tauri dev 文件监视器触发重建循环
    let memory_path = std::env::current_dir()
        .ok()
        .and_then(|d| d.parent().map(|p| p.join("memory.json")))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "../memory.json".to_string());
    let memory = Arc::new(MemoryKernel::new(&memory_path));

    // 创建篡改引擎 (单实例, 用于 builder + rule_count)
    let tamper = TamperEngine::default_rules();
    let rule_count = tamper.rule_count();
    tracing::info!("start_proxy: tamper rules = {}", rule_count);

    // 构建 MitmCore
    let core = Arc::new(
        MitmCore::builder()
            .target(&relay_url)
            .request_interceptor(SystemPromptInjector::new(instructions))
            .response_parser(UniversalSseParser)
            .response_interceptor(tamper)
            .response_interceptor(memory.clone())
            .response_interceptor(monitor.clone())
            .build()
            .map_err(|e| e.to_string())?,
    );

    // 启动 axum HTTP 服务
    let core_for_server = core.clone();
    let relay_url_for_log = relay_url_display.clone();
    let handle = tokio::spawn(async move {
        let app = axum::Router::new()
            .route("/", axum::routing::get(health_check))
            .route("/{*path}", axum::routing::any(move |req| handle_proxy(req, core_for_server.clone())));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:8080")
            .await
            .expect("failed to bind :8080");
        tracing::info!("Proxy listening on :8080 -> {}", relay_url_for_log);
        axum::serve(listener, app).await.expect("proxy server error");
    });

    // 存入 AppState
    *state.core.write().await = Some(core);
    *state.proxy_handle.write().await = Some(handle);
    *state.monitor.write().await = Some(monitor);
    *state.memory.write().await = Some(memory);

    let _ = app.emit("proxy-status", "running");

    // 推送系统日志到交互面板
    let _ = app.emit("interaction", InteractionEvent {
        id: 0,
        timestamp: chrono::Utc::now().to_rfc3339(),
        category: "system".into(),
        user_preview: "启动代理".into(),
        ai_preview: format!("代理已启动 → 127.0.0.1:8080 → {}", relay_url_display),
        thinking_preview: String::new(),
        tampered: false,
        bytes: 0,
        duration_ms: 0,
    });

    tracing::info!("start_proxy: proxy running on :8080 -> {}", relay_url_display);

    Ok(format!(
        "Proxy running on :8080 -> {} | rules: {}",
        relay_url_display, rule_count
    ))
}

#[tauri::command]
async fn stop_proxy(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let handle = state.proxy_handle.write().await.take();
    if let Some(h) = handle {
        h.abort();
    }
    *state.core.write().await = None;
    *state.monitor.write().await = None;
    *state.memory.write().await = None;

    // 自动恢复 Codex 配置：停止代理后 base_url 指向死端口会导致 Codex CLI 不可用
    let restore_msg = if let Some(manager) = DeployManager::new() {
        match manager.restore() {
            Ok(msg) => {
                tracing::info!("stop_proxy: auto-restore: {}", msg);
                msg
            }
            Err(e) => {
                tracing::warn!("stop_proxy: auto-restore failed: {}", e);
                format!("auto-restore failed: {}", e)
            }
        }
    } else {
        "Codex home not found, skipping restore".to_string()
    };

    let _ = app.emit("proxy-status", "stopped");

    // 推送系统日志到交互面板
    let _ = app.emit("interaction", InteractionEvent {
        id: 0,
        timestamp: chrono::Utc::now().to_rfc3339(),
        category: "system".into(),
        user_preview: "停止代理".into(),
        ai_preview: format!("代理已停止，Codex 配置已自动还原 ({})", restore_msg),
        thinking_preview: String::new(),
        tampered: false,
        bytes: 0,
        duration_ms: 0,
    });

    tracing::info!("stop_proxy: proxy stopped, codex config restored");

    Ok(format!("Proxy stopped, codex config restored ({})", restore_msg))
}

#[tauri::command]
async fn deploy_bridge() -> Result<String, String> {
    tracing::info!("deploy_bridge: starting");
    let manager = DeployManager::new().ok_or("Codex home not found")?;
    let bridge_path = find_resource_file("bridge.md")?;
    let bridge_md = std::fs::read_to_string(&bridge_path)
        .map_err(|e| format!("Failed to read bridge.md: {}", e))?;
    let skills_dir = find_resource_dir("skills")?;
    let result = manager.apply(&bridge_md, &skills_dir);
    match &result {
        Ok(msg) => tracing::info!("deploy_bridge: {}", msg),
        Err(e) => tracing::error!("deploy_bridge: failed: {}", e),
    }
    result
}

#[tauri::command]
async fn restore_codex() -> Result<String, String> {
    tracing::info!("restore_codex: starting");
    let manager = DeployManager::new().ok_or("Codex home not found")?;
    let result = manager.restore();
    match &result {
        Ok(msg) => tracing::info!("restore_codex: {}", msg),
        Err(e) => tracing::error!("restore_codex: failed: {}", e),
    }
    result
}

#[tauri::command]
async fn get_stats(state: tauri::State<'_, AppState>) -> Result<StatsEvent, String> {
    let monitor = state.monitor.read().await;
    let monitor = monitor.as_ref().ok_or("Proxy not running")?;
    Ok(monitor.get_stats())
}

#[tauri::command]
async fn get_history(state: tauri::State<'_, AppState>) -> Result<Vec<InteractionEvent>, String> {
    let monitor = state.monitor.read().await;
    let monitor = monitor.as_ref().ok_or("Proxy not running")?;
    Ok(monitor.get_history())
}

#[tauri::command]
async fn get_proxy_status(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let core = state.core.read().await;
    if core.is_some() {
        Ok("running".into())
    } else {
        Ok("stopped".into())
    }
}

#[tauri::command]
async fn get_codex_info() -> Result<serde_json::Value, String> {
    let home = DeployManager::find_codex_home();
    let relay = find_relay_url();
    Ok(serde_json::json!({
        "codex_home": home.map(|p| p.display().to_string()),
        "relay_url": relay,
    }))
}

// ── 窗口控制 ──────────────────────────────────────────────

#[tauri::command]
async fn minimize_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.minimize().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn toggle_maximize(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        let is_max = window.is_maximized().unwrap_or(false);
        if is_max {
            window.unmaximize().map_err(|e| e.to_string())?;
        } else {
            window.maximize().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
async fn close_window(app: tauri::AppHandle) -> Result<(), String> {
    // 点 X 不退出，而是隐藏到托盘
    if let Some(window) = app.get_webview_window("main") {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn show_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn quit_app(app: tauri::AppHandle) -> Result<(), String> {
    // 先销毁窗口再退出，避免 Chromium Error 1412
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.destroy();
    }
    app.exit(0);
    Ok(())
}

// ── SSE 包装 ─────────────────────────────────────────────

/// 将 tamper 替换文本包装为合法的 Responses API SSE 格式
/// Codex CLI 期望: response.created → response.output_text.delta → response.completed
/// 纯文本或 data: [DONE] 都会导致 "stream disconnected before response.completed"
fn wrap_tamper_as_sse(text: &str) -> bytes::Bytes {
    let created = serde_json::json!({
        "type": "response.created",
        "response": {
            "id": "resp_tamper",
            "object": "response",
            "status": "in_progress",
            "output": []
        }
    });

    let delta = serde_json::json!({
        "type": "response.output_text.delta",
        "item_id": "msg_tamper",
        "output_index": 0,
        "content_index": 0,
        "delta": text
    });

    let done = serde_json::json!({
        "type": "response.output_text.done",
        "item_id": "msg_tamper",
        "output_index": 0,
        "content_index": 0,
        "text": text
    });

    let completed = serde_json::json!({
        "type": "response.completed",
        "response": {
            "id": "resp_tamper",
            "object": "response",
            "status": "completed",
            "output": [{
                "id": "msg_tamper",
                "type": "message",
                "role": "assistant",
                "status": "completed",
                "content": [{
                    "type": "output_text",
                    "text": text
                }]
            }],
            "usage": {
                "input_tokens": 0,
                "output_tokens": 0
            }
        }
    });

    let sse = format!(
        "event: response.created\ndata: {}\n\n\
         event: response.output_text.delta\ndata: {}\n\n\
         event: response.output_text.done\ndata: {}\n\n\
         event: response.completed\ndata: {}\n\n",
        created, delta, done, completed
    );

    bytes::Bytes::from(sse)
}

// ── Axum Handlers ─────────────────────────────────────────

async fn health_check() -> impl axum::response::IntoResponse {
    (
        axum::http::StatusCode::OK,
        [("Content-Type", "text/plain; charset=utf-8")],
        "Super-Instruct OK",
    )
}

async fn handle_proxy(
    req: axum::extract::Request,
    core: Arc<MitmCore>,
) -> axum::response::Response {
    // GET 请求 = 健康检查
    if req.method() == axum::http::Method::GET {
        return axum::response::Response::builder()
            .status(axum::http::StatusCode::OK)
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(axum::body::Body::from("Super-Instruct OK"))
            .unwrap();
    }

    let (parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::BAD_REQUEST)
                .body(axum::body::Body::from(format!("{{\"error\": \"{}\"}}", e)))
                .unwrap();
        }
    };

    // 阶段 1: 请求拦截 + 转发上游
    let upstream = match core
        .handle_request(
            parts.method,
            parts.uri.path().to_string(),
            parts.headers,
            bytes,
        )
        .await
    {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("Proxy error (request phase): {}", e);
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::BAD_GATEWAY)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(format!(
                    "{{\"error\": \"{}\"}}",
                    e
                )))
                .unwrap();
        }
    };

    let status = axum::http::StatusCode::from_u16(upstream.status).unwrap_or(
        axum::http::StatusCode::OK,
    );
    let content_type = upstream.content_type.clone();
    let is_sse = content_type
        .as_deref()
        .map(|ct| ct.contains("text/event-stream"))
        .unwrap_or(false);

    // 缓冲 + keepalive 模式:
    //   SSE 响应: 缓冲完整上游响应, 期间每 500ms 发 ": keepalive\n\n" 防 CLI 超时
    //   非 SSE:  直接缓冲, 无需 keepalive
    //   缓冲完成后跑 finalize_response → tamper 替换生效 → 发最终 body 给 CLI
    let (tx, rx) =
        tokio::sync::mpsc::unbounded_channel::<Result<bytes::Bytes, std::io::Error>>();
    let core_clone = core.clone();
    let meta = upstream.meta;
    let upstream_status = upstream.status;
    let ct_for_finalize = content_type.clone();
    let upstream_resp = upstream.response;

    tokio::spawn(async move {
        let mut accumulated = Vec::with_capacity(65536);
        let mut stream = upstream_resp.bytes_stream();
        let start = std::time::Instant::now();

        if is_sse {
            // SSE: 缓冲上游 chunk, 同时每 500ms 发 keepalive 注释防 CLI 超时
            let mut interval =
                tokio::time::interval(std::time::Duration::from_millis(500));
            interval.tick().await; // 跳过首次立即触发

            loop {
                tokio::select! {
                    chunk_result = stream.next() => {
                        match chunk_result {
                            Some(Ok(chunk)) => accumulated.extend_from_slice(&chunk),
                            Some(Err(e)) => {
                                tracing::warn!("upstream stream error: {}", e);
                                break;
                            }
                            None => break,
                        }
                    }
                    _ = interval.tick() => {
                        // SSE 注释行 (以 : 开头), 客户端解析器直接忽略
                        if tx
                            .send(Ok(bytes::Bytes::from_static(b": keepalive\n\n")))
                            .is_err()
                        {
                            return; // CLI 已断开, 停止一切
                        }
                    }
                }
            }
        } else {
            // 非 SSE: 直接缓冲, 无 keepalive
            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(chunk) => accumulated.extend_from_slice(&chunk),
                    Err(e) => {
                        tracing::warn!("upstream stream error: {}", e);
                        break;
                    }
                }
            }
        }

        let accumulated_bytes = bytes::Bytes::from(accumulated);
        let duration_ms = start.elapsed().as_millis() as u64;

        tracing::debug!(
            category = %meta.category,
            status = upstream_status,
            resp_bytes = accumulated_bytes.len(),
            duration_ms,
            "buffering completed, running finalize"
        );

        // 阶段 2: 解析 + 响应拦截器 (tamper/memory/monitor)
        let (final_body, tampered, _ct) = core_clone.finalize_response(
            meta,
            upstream_status,
            ct_for_finalize,
            accumulated_bytes,
            duration_ms,
        );

        // 阶段 3: 发送最终 body 给 Codex CLI
        if tampered {
            if is_sse {
                // SSE: 包装为合法 Responses API SSE 格式 (response.created → delta → completed)
                // 纯文本会导致 "stream disconnected before response.completed" 错误和无限重试
                let replacement_text =
                    std::str::from_utf8(&final_body).unwrap_or("「了解。実行する。」");
                let sse_body = wrap_tamper_as_sse(replacement_text);
                tracing::info!(
                    bytes = sse_body.len(),
                    "tamper: sending SSE-wrapped replacement to CLI"
                );
                let _ = tx.send(Ok(sse_body));
            } else {
                // 非 SSE: 直接发送纯文本
                tracing::info!(
                    bytes = final_body.len(),
                    "tamper: sending replaced body to CLI"
                );
                let _ = tx.send(Ok(final_body));
            }
        } else {
            let _ = tx.send(Ok(final_body));
        }

        drop(tx);
    });

    // 构建 axum 响应
    let body_stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);
    let body = axum::body::Body::from_stream(body_stream);

    let mut resp_builder = axum::response::Response::builder().status(status);
    if is_sse {
        // SSE 模式统一用 text/event-stream, 让 keepalive 注释生效
        resp_builder = resp_builder.header("content-type", "text/event-stream");
    } else if let Some(ct) = &content_type {
        resp_builder = resp_builder.header("content-type", ct);
    }
    resp_builder.body(body).unwrap()
}

// ── 资源文件查找 ─────────────────────────────────────────

fn find_resource_file(name: &str) -> Result<std::path::PathBuf, String> {
    let mut candidates = vec![
        std::path::PathBuf::from(name),
        std::env::current_dir().unwrap_or_default().join(name),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join(name)))
            .unwrap_or_default(),
    ];
    // cargo run 的工作目录是 src-tauri/，资源在项目根目录（上一级）
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(parent) = cwd.parent() {
            candidates.push(parent.join(name));
        }
    }
    for p in &candidates {
        if p.exists() {
            return Ok(p.clone());
        }
    }
    Err(format!("{} not found (searched: {:?})", name, candidates))
}

fn find_resource_dir(name: &str) -> Result<std::path::PathBuf, String> {
    let mut candidates = vec![
        std::path::PathBuf::from(name),
        std::env::current_dir().unwrap_or_default().join(name),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join(name)))
            .unwrap_or_default(),
    ];
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(parent) = cwd.parent() {
            candidates.push(parent.join(name));
        }
    }
    for p in &candidates {
        if p.is_dir() {
            return Ok(p.clone());
        }
    }
    Err(format!("{} directory not found", name))
}