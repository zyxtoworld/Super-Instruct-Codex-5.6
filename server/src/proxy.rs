// proxy.rs — axum 反向代理 handler
// 从桌面版 lib.rs handle_proxy 移植，去掉 Tauri 依赖。
// 支持流式透传（SSE keepalive + tamper SSE 包装）与非流式缓冲+后处理。

use super_instruct_server::core::MitmCore;
use futures::StreamExt;
use std::sync::Arc;

/// GET 请求 = 健康检查
pub async fn health_check() -> impl axum::response::IntoResponse {
    (
        axum::http::StatusCode::OK,
        [("Content-Type", "text/plain; charset=utf-8")],
        "Super-Instruct OK",
    )
}

pub async fn handle_proxy(
    req: axum::extract::Request,
    core: Arc<MitmCore>,
) -> axum::response::Response {
    let (parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::BAD_REQUEST)
                .body(axum::body::Body::from(
                    serde_json::json!({ "error": e.to_string() }).to_string(),
                ))
                .unwrap();
        }
    };

    // 保留 query string
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.to_string())
        .unwrap_or_else(|| parts.uri.path().to_string());
    // 入站 API 格式: /v1/chat/completions 是 Chat API,其余(responses 等)按 Responses API。
    // 篡改包装必须匹配入站格式,否则下游(中转站等)按错误格式解析会丢弃内容。
    // 用 uri.path() 而非 path_and_query,避免 query string 含 /chat/completions 时误判。
    let is_chat_api = parts.uri.path().contains("/chat/completions");
    // Anthropic Messages API: /v1/messages 透传原格式, 篡改用 Anthropic SSE/JSON 包装
    let is_anthropic = parts.uri.path().contains("/v1/messages");

    // 阶段 1: 请求拦截 + 转发上游
    let upstream = match core
        .handle_request(
            parts.method,
            path_and_query,
            parts.headers,
            bytes,
        )
        .await
    {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("Proxy error (request phase): {}", e);
            // 脱敏：不回显含 URL 的内部错误
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::BAD_GATEWAY)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(
                    serde_json::json!({ "error": "upstream request failed" }).to_string(),
                ))
                .unwrap();
        }
    };

    let status = axum::http::StatusCode::from_u16(upstream.status).unwrap_or(axum::http::StatusCode::OK);
    let content_type = upstream.content_type.clone();
    let is_sse = content_type
        .as_deref()
        .map(|ct| ct.contains("text/event-stream"))
        .unwrap_or(false);

    let meta = upstream.meta;
    let upstream_status = upstream.status;
    let ct_for_finalize = content_type.clone();
    let upstream_headers = upstream.headers;

    // 非 SSE：同步缓冲完整响应 → finalize（tamper/memory/monitor）→ 一次性构造响应，
    // 用 finalize 返回的 ct 作为 content-type（tamper 后为 application/json）
    if !is_sse {
        // 非 SSE：流式累积完整响应（带上限，防 OOM），再 finalize 一次性构造响应
        const MAX_ACCUMULATED: usize = 100 * 1024 * 1024;
        let mut accumulated: Vec<u8> = Vec::with_capacity(65536);
        let mut stream = upstream.response.bytes_stream();
        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    if accumulated.len() + chunk.len() > MAX_ACCUMULATED {
                        tracing::warn!("non-sse: accumulated body exceeds limit, aborting");
                        return axum::response::Response::builder()
                            .status(axum::http::StatusCode::BAD_GATEWAY)
                            .header("Content-Type", "application/json")
                            .body(axum::body::Body::from(
                                serde_json::json!({ "error": "upstream response too large" }).to_string(),
                            ))
                            .unwrap();
                    }
                    accumulated.extend_from_slice(&chunk);
                }
                Err(e) => {
                    tracing::warn!("non-sse upstream stream error: {}", e);
                    return axum::response::Response::builder()
                        .status(axum::http::StatusCode::BAD_GATEWAY)
                        .header("Content-Type", "application/json")
                        .body(axum::body::Body::from(
                            serde_json::json!({ "error": "upstream stream failed" }).to_string(),
                        ))
                        .unwrap();
                }
            }
        }
        let bytes = bytes::Bytes::from(accumulated);

        let duration_ms = chrono::Utc::now()
            .signed_duration_since(meta.timestamp)
            .num_milliseconds()
            .max(0) as u64;
        let req_model_anthropic = meta.model.clone();

        tracing::debug!(
            category = %meta.category,
            status = upstream_status,
            resp_bytes = bytes.len(),
            duration_ms,
            "non-sse: buffering completed, running finalize"
        );

        let (final_body, tampered, final_ct) = core.finalize_response(
            meta,
            upstream_status,
            ct_for_finalize,
            bytes,
            duration_ms,
        );

        // Anthropic 非流式: tamper 替换的纯文本需包装为 Anthropic JSON, 否则客户端解析失败
        let (final_body, final_ct) = if tampered && is_anthropic {
            let text = String::from_utf8_lossy(&final_body).to_string();
            let model = req_model_anthropic.clone();
            (
                super_instruct_server::anthropic::wrap_tamper_as_anthropic_json(&text, &model),
                Some("application/json; charset=utf-8".to_string()),
            )
        } else {
            (final_body, final_ct)
        };

        tracing::info!(
            %status,
            tampered,
            duration_ms,
            resp_bytes = final_body.len(),
            "non-sse: request completed"
        );

        let mut resp_builder = axum::response::Response::builder().status(status);
        for (name, value) in upstream_headers.iter() {
            let lower = name.as_str().to_lowercase();
            if is_response_hop_header(&lower) || lower == "content-type" {
                continue;
            }
            resp_builder = resp_builder.header(name, value);
        }
        // 篡改后 ct 由 finalize_response 统一给出(text/plain);未篡改则透传上游 ct
        let ct = final_ct.unwrap_or_else(|| "application/octet-stream".into());
        resp_builder = resp_builder.header("content-type", ct);
        return resp_builder
            .body(axum::body::Body::from(final_body))
            .unwrap();
    }

    // SSE：流式透传（keepalive）+ 缓冲后 finalize，tamper 时用 SSE 包装替换
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<bytes::Bytes, std::io::Error>>();
    let core_clone = core.clone();
    let upstream_resp = upstream.response;

    tokio::spawn(async move {
        // SSE 累积上限：防恶意上游无限灌流导致 OOM（100MB）
        const MAX_ACCUMULATED: usize = 100 * 1024 * 1024;
        let mut accumulated: Vec<u8> = Vec::with_capacity(65536);
        let mut stream = upstream_resp.bytes_stream();

        let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
        interval.tick().await;

        loop {
            tokio::select! {
                chunk_result = stream.next() => {
                    match chunk_result {
                        Some(Ok(chunk)) => {
                            tracing::trace!("sse: upstream chunk {} bytes", chunk.len());
                            if accumulated.len() + chunk.len() > MAX_ACCUMULATED {
                                tracing::warn!("sse: accumulated body exceeds limit, aborting");
                                break;
                            }
                            accumulated.extend_from_slice(&chunk);
                        }
                        Some(Err(e)) => { tracing::warn!("upstream stream error: {}", e); break; }
                        None => {
                            tracing::debug!("sse: upstream stream ended");
                            break;
                        }
                    }
                }
                _ = interval.tick() => {
                    if tx.send(Ok(bytes::Bytes::from_static(b": keepalive\n\n"))).is_err() {
                        return;
                    }
                }
            }
        }

        let accumulated_bytes = bytes::Bytes::from(accumulated);

        let duration_ms = chrono::Utc::now()
            .signed_duration_since(meta.timestamp)
            .num_milliseconds()
            .max(0) as u64;

        tracing::debug!(
            category = %meta.category,
            status = upstream_status,
            resp_bytes = accumulated_bytes.len(),
            duration_ms,
            "sse: buffering completed, running finalize"
        );

        // 阶段 2: 解析 + 响应拦截器 (tamper/memory/monitor)
        let req_model = meta.model.clone();
        let (final_body, tampered, _ct) = core_clone.finalize_response(
            meta,
            upstream_status,
            ct_for_finalize,
            accumulated_bytes,
            duration_ms,
        );

        // 阶段 3: 发送最终 body 给客户端
        if tampered {
            let replacement_text =
                std::str::from_utf8(&final_body).unwrap_or("「了解。実行する。」");
            let sse_body = if is_anthropic {
                super_instruct_server::anthropic::wrap_tamper_as_anthropic_sse(
                    replacement_text,
                    &req_model,
                )
            } else if is_chat_api {
                super_instruct_server::formats::wrap_tamper_as_chat_sse(replacement_text, &req_model)
            } else {
                super_instruct_server::formats::wrap_tamper_as_sse(replacement_text, &req_model)
            };
            tracing::info!(bytes = sse_body.len(), "tamper: sending SSE-wrapped replacement");
            let _ = tx.send(Ok(sse_body));
        } else {
            let _ = tx.send(Ok(final_body));
        }

        drop(tx);
    });

    // 构建 axum 响应（SSE）
    let body_stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);
    let body = axum::body::Body::from_stream(body_stream);

    let mut resp_builder = axum::response::Response::builder().status(status);

    for (name, value) in upstream_headers.iter() {
        let lower = name.as_str().to_lowercase();
        if is_response_hop_header(&lower) || lower == "content-type" {
            continue;
        }
        resp_builder = resp_builder.header(name, value);
    }

    resp_builder = resp_builder.header("content-type", "text/event-stream");
    resp_builder.body(body).unwrap()
}

fn is_response_hop_header(name: &str) -> bool {
    matches!(
        name,
        "connection" | "keep-alive" | "proxy-connection" | "te" | "trailer"
            | "transfer-encoding" | "upgrade" | "content-length" | "content-encoding"
    )
}

