// ws.rs — WebSocket 通道
// 入站: 客户端 ws://host:port/ws/v1/messages 连接 → 每帧一个 Anthropic 请求 JSON
//       → 走完整管道(注入→转发→缓冲→tamper) → Anthropic SSE/JSON 文本帧返回
// 出站: 上游 URL 为 ws:// 或 wss:// 时, 用 ws 客户端把请求 JSON 发送到上游,
//       响应帧拼装为流式响应 (由 core 转发调用)

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use bytes::Bytes;
use futures::StreamExt;
use std::sync::Arc;

use crate::core::MitmCore;

const MAX_ACCUMULATED: usize = 100 * 1024 * 1024;

/// 入站 ws 路由 handler（路由闭包已捕获 core）
/// 支持路径: /ws/v1/messages (Anthropic) /ws/v1/chat/completions /ws/v1/responses (OpenAI)
/// 响应包装格式由路径自动识别
pub async fn ws_handler(
    uri: http::Uri,
    headers: http::HeaderMap,
    ws: WebSocketUpgrade,
    core: Arc<MitmCore>,
) -> Response {
    // 从请求 URI 提取 API 路径: /ws/v1/messages → /v1/messages
    let api_path = uri
        .path()
        .strip_prefix("/ws")
        .filter(|p| p.starts_with("/v1/"))
        .unwrap_or("/v1/messages")
        .to_string();
    ws.on_upgrade(move |socket| ws_loop(socket, core, api_path, headers))
}

async fn ws_loop(
    mut socket: WebSocket,
    core: Arc<MitmCore>,
    api_path: String,
    headers: http::HeaderMap,
) {
    while let Some(msg) = socket.recv().await {
        let msg = match msg {
            Ok(m) => m,
            Err(_) => break,
        };
        if matches!(msg, Message::Close(_)) {
            break;
        }
        let text = match msg.to_text() {
            Ok(t) => t.to_string(),
            Err(_) => continue,
        };

        // 每帧一个请求: 走与 HTTP 相同的管道
        let response_text = process_one(&core, &api_path, &headers, &text).await;

        if socket
            .send(Message::Text(response_text.into()))
            .await
            .is_err()
        {
            break;
        }
    }
}

/// 单请求处理: 注入 → 转发 → 缓冲 → finalize(tamper) → 按路径格式输出
async fn process_one(
    core: &MitmCore,
    api_path: &str,
    headers: &http::HeaderMap,
    body_text: &str,
) -> String {
    let body_bytes = Bytes::from(body_text.to_string());
    let is_anthropic = api_path.contains("/v1/messages");
    let is_chat = api_path.contains("/chat/completions");

    // 非法 JSON → 错误帧
    if serde_json::from_str::<serde_json::Value>(body_text).is_err() {
        return serde_json::json!({
            "type": "error",
            "error": {"type": "invalid_request_error", "message": "request body is not valid JSON"}
        })
        .to_string();
    }

    let upstream = match core
        .handle_request(
            http::Method::POST,
            api_path.to_string(),
            headers.clone(),
            body_bytes.clone(),
        )
        .await
    {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("ws proxy error (request phase): {}", e);
            return serde_json::json!({
                "type": "error",
                "error": {"type": "upstream_error", "message": "upstream request failed"}
            })
            .to_string();
        }
    };

    let status = upstream.status;
    let content_type = upstream.content_type.clone();
    let is_sse = content_type
        .as_deref()
        .map(|ct| ct.contains("text/event-stream"))
        .unwrap_or(false);
    let meta = upstream.meta;
    let req_model = meta.model.clone();

    // 缓冲完整上游响应
    let mut accumulated: Vec<u8> = Vec::with_capacity(65536);
    let mut stream = upstream.response.bytes_stream();
    loop {
        match stream.next().await {
            Some(Ok(chunk)) => {
                if accumulated.len() + chunk.len() > MAX_ACCUMULATED {
                    tracing::warn!("ws: accumulated body exceeds limit, aborting");
                    return serde_json::json!({
                        "type": "error",
                        "error": {"type": "upstream_error", "message": "upstream response too large"}
                    })
                    .to_string();
                }
                accumulated.extend_from_slice(&chunk);
            }
            Some(Err(e)) => {
                tracing::warn!("ws upstream stream error: {}", e);
                break;
            }
            None => break,
        }
    }

    let accumulated = Bytes::from(accumulated);
    let duration_ms = chrono::Utc::now()
        .signed_duration_since(meta.timestamp)
        .num_milliseconds()
        .max(0) as u64;

    // finalize: 解析 + tamper/memory/monitor
    let (final_body, tampered, _ct) =
        core.finalize_response(meta, status, content_type.clone(), accumulated, duration_ms);

    if tampered {
        let text = String::from_utf8_lossy(&final_body).to_string();
        let wrapped = if is_anthropic {
            if is_sse {
                crate::anthropic::wrap_tamper_as_anthropic_sse(&text, &req_model)
            } else {
                crate::anthropic::wrap_tamper_as_anthropic_json(&text, &req_model)
            }
        } else if is_chat {
            crate::formats::wrap_tamper_as_chat_sse(&text, &req_model)
        } else {
            crate::formats::wrap_tamper_as_sse(&text, &req_model)
        };
        return String::from_utf8_lossy(&wrapped).to_string();
    }

    String::from_utf8_lossy(&final_body).to_string()
}

/// 出站 ws: 把请求 JSON 通过 ws 发送到上游, 返回流式响应构造器
/// 由 core/mod.rs 在 upstream scheme 为 ws/wss 时调用
pub async fn forward_via_ws(
    url: &str,
    body: Bytes,
    headers: &http::HeaderMap,
) -> Result<reqwest::Response, Box<dyn std::error::Error + Send + Sync>> {
    use tokio_tungstenite::connect_async;

    // 仅当 scheme 是 ws/wss 时走此路径（调用方保证）
    let (ws_stream, _resp) = connect_async(url).await?;
    let (mut write, mut read) = ws_stream.split();
    tracing::debug!(url = %url, "ws: connected to upstream");

    // 发送请求 JSON（文本帧）
    use futures::SinkExt;
    let payload = String::from_utf8_lossy(&body).to_string();
    tracing::debug!(len = payload.len(), "ws: sending request frame");
    write
        .send(tokio_tungstenite::tungstenite::Message::Text(payload))
        .await?;
    tracing::debug!("ws: request frame sent");

    // 收集响应帧（文本/二进制），拼装为 SSE 流
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, std::io::Error>>();
    tokio::spawn(async move {
        while let Some(msg) = read.next().await {
            match msg {
                Ok(tokio_tungstenite::tungstenite::Message::Text(t)) => {
                    tracing::debug!(len = t.len(), "ws: upstream frame (text)");
                    // SSE 结束标志: OpenAI [DONE] / Anthropic message_stop
                    // 上游可能不主动关闭连接, 检测到结束标志后终止读循环,
                    // 否则 channel 永不关闭, 下游 keepalive 卡死
                    let is_end = t.contains("[DONE]") || t.contains("message_stop");
                    if tx.send(Ok(Bytes::from(t.into_bytes()))).is_err() {
                        return;
                    }
                    if is_end {
                        tracing::debug!("ws: SSE end marker received, ending stream");
                        break;
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Binary(b)) => {
                    tracing::debug!(len = b.len(), "ws: upstream frame (binary)");
                    if tx.send(Ok(Bytes::from(b))).is_err() {
                        return;
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                    tracing::debug!("ws: upstream close frame, ending stream");
                    break;
                }
                // 连接错误/上游关闭: 结束流 (否则 channel 永不关闭, 下游 keepalive 卡死)
                Err(e) => {
                    tracing::debug!("ws: upstream stream error, ending: {}", e);
                    break;
                }
                _ => {}
            }
        }
        drop(tx);
    });

    // 构造 reqwest::Response（保持调用方流式处理不变）
    let mut builder = http::Response::builder().status(http::StatusCode::OK);
    builder = builder.header("content-type", "text/event-stream");
    for (name, value) in headers.iter() {
        let lower = name.as_str().to_lowercase();
        if lower == "host" || lower == "content-length" || lower == "accept-encoding" {
            continue;
        }
        builder = builder.header(name, value);
    }
    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);
    let body = reqwest::Body::wrap_stream(stream);
    let http_resp = builder
        .body(body)
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
    Ok(reqwest::Response::from(http_resp))
}
