// auth.rs — 简单入站认证中间件
// 支持 Authorization: Bearer <key> 或 x-api-key: <key>
// 未配置 AUTH_API_KEY 时放行（部署者务必在内网或加反代 TLS）

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::AppState;

pub async fn require_auth(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let Some(expected) = &state.auth_key else {
        return next.run(req).await;
    };

    let provided = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer ").map(str::trim).filter(|s| !s.is_empty()))
        .or_else(|| {
            req.headers()
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|s| !s.is_empty())
        });

    match provided {
        Some(k) if k == expected => next.run(req).await,
        _ => (StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
    }
}
