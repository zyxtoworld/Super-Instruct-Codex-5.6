// MitmCore — 格式无关、功能无关的 HTTP 反向代理管道
// 两阶段流式架构:
//   1. handle_request  — 请求拦截器 → 转发上游 → 返回 reqwest::Response (流式)
//   2. finalize_response — 解析 → 响应拦截器 → 返回最终 body (后处理)
// axum handler 负责流式透传 + 背景累积 + 后处理调用

pub mod context;
pub mod extract;
pub mod traits;

pub use context::{Category, ParsedResponse, RequestCtx, RequestMeta, ResponseCtx};
pub use extract::{categorize, extract_user};
pub use traits::{RequestInterceptor, ResponseInterceptor, ResponseParser};

use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Method};
use reqwest::Client;

pub struct MitmCore {
    target: String,
    default_api_key: Option<String>,
    client: Client,
    request_interceptors: Vec<Box<dyn RequestInterceptor>>,
    response_parser: Box<dyn ResponseParser>,
    response_interceptors: Vec<Box<dyn ResponseInterceptor>>,
}

/// 阶段 1 产物: 请求拦截后的元数据 + 上游响应
pub struct UpstreamResult {
    pub meta: RequestMeta,
    pub status: u16,
    pub content_type: Option<String>,
    pub headers: HeaderMap,
    pub response: reqwest::Response,
}

impl MitmCore {
    pub fn builder() -> MitmCoreBuilder {
        MitmCoreBuilder::new()
    }

    /// 阶段 1: 请求拦截 → 转发上游 → 返回流式响应
    /// 调用方拿到返回的 reqwest::Response 后做流式透传
    pub async fn handle_request(
        &self,
        method: Method,
        path_and_query: String,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<UpstreamResult, Box<dyn std::error::Error + Send + Sync>> {
        // 1. 解析请求 JSON（GET 等无 body 请求容错为空对象）
        // 非 JSON body（表单/multipart/任意）→ 拦截器视为空对象，转发时保留原始 bytes 透传
        let mut raw_forward: Option<Bytes> = None;
        let data: serde_json::Value = if body.is_empty() {
            serde_json::Value::Object(Default::default())
        } else {
            match serde_json::from_slice(&body) {
                Ok(v) => v,
                Err(_) => {
                    tracing::warn!("request body is not valid JSON, forwarding raw bytes unchanged");
                    raw_forward = Some(body.clone());
                    serde_json::Value::Object(Default::default())
                }
            }
        };
        let user_msg = extract_user(&data);
        let category = categorize(&user_msg);

        tracing::debug!(
            category = %category,
            path = %path_and_query,
            method = %method,
            user_msg_len = user_msg.len(),
            "request received"
        );

        let model = data
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();

        let mut req_ctx = RequestCtx {
            meta: RequestMeta {
                user_msg,
                category,
                path: path_and_query.clone(),
                model: model.clone(),
                timestamp: chrono::Utc::now(),
            },
            headers: headers.clone(),
            body: data,
            // 动态上游覆盖：由请求拦截器（路由解析器）在 intercept 阶段设置，
            // 若为 None 则使用构造时的默认 target
            upstream_override: None,
            upstream_api_key: None,
        };

        // 2. 请求拦截器 — 全量执行
        for ext in &self.request_interceptors {
            tracing::trace!(interceptor = ext.name(), "request interceptor running");
            ext.intercept(&mut req_ctx);
        }

        // 2b. 解析动态上游目标（路由解析器可能设置 override）
        let effective_target = req_ctx.upstream_override.as_deref().unwrap_or(&self.target);

        // 3. 转发到上游 — 跳过 hop-by-hop 头
        // 拼接 URL：避免重复的 /v1（上游基址含 /v1 且路径也以 /v1 开头时去重）
        // 先 trim 尾斜杠，兼容 base 为 ".../v1/" 的写法
        let base = effective_target.trim_end_matches('/');
        let mut url = format!("{}{}", base, path_and_query);
        let base_lower = base.to_lowercase();
        let path_lower = path_and_query.to_lowercase();
        if base_lower.ends_with("/v1") && path_lower.starts_with("/v1") {
            // 把 path 的 /v1 去掉，仅保留一次
            url = format!("{}{}", base, &path_and_query[3..]);
        }
        tracing::debug!(url = %url, "forwarding to upstream");

        let mut forward_headers = HeaderMap::new();
        // 上游 key 策略（默认透传入站 Authorization，保留入口中转站带来的出站 key）：
        //   1. router 显式设置（X-Upstream-Key 头 / model 匹配条目）→ 覆盖
        //   2. 入站已有 Authorization → 透传原样
        //   3. 仅当走默认 target（无动态 override）且入站无 Authorization 时，
        //      才用 default_api_key 兜底 —— 动态 header 上游绝不用配置凭据，防凭据外泄
        let inbound_auth = headers.get(http::header::AUTHORIZATION).and_then(|v| v.to_str().ok());
        let override_auth = req_ctx.upstream_api_key.is_some();
        if let Some(api_key) = &req_ctx.upstream_api_key {
            forward_headers.insert(
                http::header::AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", api_key))
                    .unwrap_or_else(|_| HeaderValue::from_static("Bearer")),
            );
        } else if inbound_auth.is_none() && req_ctx.upstream_override.is_none() {
            if let Some(api_key) = &self.default_api_key {
                forward_headers.insert(
                    http::header::AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer {}", api_key))
                        .unwrap_or_else(|_| HeaderValue::from_static("Bearer")),
                );
            }
        }
        for (name, value) in headers.iter() {
            let lower = name.as_str().to_lowercase();
            // 跳过 hop-by-hop 和需要重新计算的头部
            // accept-encoding: 强制上游返回未压缩数据，否则 SSE 解析器无法读取压缩字节
            if lower == "host"
                || lower == "content-length"
                || lower == "accept-encoding"
                // JSON 转发时 content-type 由 .json() 重新设置；raw 透传时需保留原 content-type
                || (lower == "content-type" && raw_forward.is_none())
                // 动态上游控制头不转发给上游
                || lower == "x-upstream-base"
                || lower == "x-upstream-key"
                // 已由上游 key 注入时跳过入站 Authorization（避免覆盖）
                || (override_auth && lower == "authorization")
            {
                continue;
            }
            forward_headers.insert(name.clone(), value.clone());
        }

        let mut req_builder = self.client.request(method, &url).headers(forward_headers);
        // 非 JSON body：保留原始 bytes 原样透传（含原 content-type）
        // JSON body：发送改造后的 req_ctx.body（.json() 会设置 content-type）
        if let Some(raw) = raw_forward {
            req_builder = req_builder.body(raw);
        } else if !body.is_empty() {
            req_builder = req_builder.json(&req_ctx.body);
        }

        let resp = req_builder.send().await?;

        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        tracing::debug!(status, "upstream response headers received");

        let headers = resp.headers().clone();

        Ok(UpstreamResult {
            meta: req_ctx.meta,
            status,
            content_type,
            headers,
            response: resp,
        })
    }

    /// 阶段 2: 流结束后 — 解析响应 + 运行响应拦截器
    /// 返回 (最终 body, 是否被篡改, content_type)
    pub fn finalize_response(
        &self,
        meta: RequestMeta,
        status: u16,
        content_type: Option<String>,
        accumulated: Bytes,
        duration_ms: u64,
    ) -> (Bytes, bool, Option<String>) {
        // 4. 响应解析
        let parsed = self.response_parser.parse(&accumulated);

        tracing::debug!(
            thinking_len = parsed.thinking.len(),
            reply_len = parsed.reply.len(),
            "response parsed"
        );

        // 5. 响应拦截器 — 全量执行, 自门控
        let mut resp_ctx = ResponseCtx {
            meta,
            status,
            raw_body: accumulated.clone(),
            parsed,
            modified_body: None,
            duration_ms,
        };

        for ext in &self.response_interceptors {
            tracing::trace!(interceptor = ext.name(), "response interceptor running");
            ext.intercept(&mut resp_ctx);
        }

        // 6. 返回
        let tampered = resp_ctx.modified_body.is_some();
        let final_body = resp_ctx.modified_body.clone().unwrap_or(accumulated);
        tracing::info!(
            category = %resp_ctx.meta.category,
            status,
            tampered,
            duration_ms = resp_ctx.duration_ms,
            resp_bytes = final_body.len(),
            "request completed"
        );

        // 篡改后 body 是纯文本替换体（tamper.rs 的「了解。実行する。」格式），
        // 必须标 text/plain，否则客户端按 JSON 解析纯文本会失败。
        // SSE 场景由 proxy 忽略此 ct 改用 text/event-stream；非 SSE 场景直接采用。
        let ct = if tampered {
            Some("text/plain; charset=utf-8".to_string())
        } else {
            content_type
        };

        (final_body, tampered, ct)
    }
}

pub struct MitmCoreBuilder {
    target: Option<String>,
    default_api_key: Option<String>,
    client: Option<Client>,
    request_interceptors: Vec<Box<dyn RequestInterceptor>>,
    response_parser: Option<Box<dyn ResponseParser>>,
    response_interceptors: Vec<Box<dyn ResponseInterceptor>>,
}

impl MitmCoreBuilder {
    pub fn new() -> Self {
        Self {
            target: None,
            default_api_key: None,
            client: None,
            request_interceptors: Vec::new(),
            response_parser: None,
            response_interceptors: Vec::new(),
        }
    }

    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn default_api_key(mut self, key: impl Into<String>) -> Self {
        self.default_api_key = Some(key.into());
        self
    }

    pub fn client(mut self, client: Client) -> Self {
        self.client = Some(client);
        self
    }

    pub fn request_interceptor(mut self, ext: impl RequestInterceptor + 'static) -> Self {
        self.request_interceptors.push(Box::new(ext));
        self
    }

    pub fn response_parser(mut self, ext: impl ResponseParser + 'static) -> Self {
        self.response_parser = Some(Box::new(ext));
        self
    }

    pub fn response_interceptor(mut self, ext: impl ResponseInterceptor + 'static) -> Self {
        self.response_interceptors.push(Box::new(ext));
        self
    }

    pub fn build(self) -> Result<MitmCore, String> {
        Ok(MitmCore {
            target: self.target.ok_or("target not set")?,
            default_api_key: self.default_api_key,
            client: self.client.unwrap_or_else(|| {
                Client::builder()
                    .timeout(std::time::Duration::from_secs(300))
                    .build()
                    .expect("failed to build reqwest client")
            }),
            request_interceptors: self.request_interceptors,
            response_parser: self.response_parser.ok_or("response parser not set")?,
            response_interceptors: self.response_interceptors,
        })
    }
}

impl Default for MitmCoreBuilder {
    fn default() -> Self {
        Self::new()
    }
}