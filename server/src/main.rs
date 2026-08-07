// super-instruct-server — 无头服务器入口
// 组装 MitmCore + 破甲扩展 + 动态上游路由，启动 axum on LISTEN_ADDR

mod config;
mod proxy;
mod router;

use std::sync::Arc;

use router::UpstreamRouter;
use config::Config;
use super_instruct_server::core::MitmCore;
use super_instruct_server::extensions::inject::SystemPromptInjector;
use super_instruct_server::extensions::memory::MemoryKernel;
use super_instruct_server::extensions::monitor::{InteractionEvent, MonitorPanel, StatsEvent};
use super_instruct_server::extensions::sse_parser::UniversalSseParser;
use super_instruct_server::extensions::tamper::TamperEngine;
use super_instruct_server::BRIDGE_MD_FALLBACK;

/// 全局共享状态：monitor + memory
#[derive(Clone)]
pub struct AppState {
    pub monitor: Arc<MonitorPanel>,
    pub memory: Arc<MemoryKernel>,
}

fn init_tracing() {
    use tracing_subscriber::prelude::*;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    // 文件日志 (可选，LOG_DIR 设置时启用) + 控制台
    if let Ok(dir) = std::env::var("LOG_DIR") {
        if !dir.trim().is_empty() {
            let dir = std::path::PathBuf::from(dir);
            let _ = std::fs::create_dir_all(&dir);
            let file_appender = tracing_appender::rolling::daily(&dir, "server.log");
            let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer().with_writer(non_blocking))
                .init();
            // guard 存入静态，避免 dropped
            std::mem::forget(_guard);
            tracing::info!("tracing: file logging enabled at {}", dir.display());
            return;
        }
    }
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();
}

fn read_bridge_md(cfg: &Config) -> String {
    if let Some(p) = &cfg.bridge_md_path {
        if let Ok(s) = std::fs::read_to_string(p) {
            tracing::info!("bridge.md loaded from {}", p.display());
            return s;
        }
        tracing::warn!("bridge.md not found at {}, using embedded fallback", p.display());
    }
    BRIDGE_MD_FALLBACK.to_string()
}

#[tokio::main]
async fn main() {
    init_tracing();
    tracing::info!("Super-Instruct server starting up");

    let cfg = config::parse_config();
    tracing::info!("listen_addr = {}", cfg.listen_addr);

    let Some(default_upstream) = cfg.default_upstream.clone() else {
        tracing::error!(
            "No upstream configured. Set UPSTREAMS (e.g. UPSTREAMS='openai=https://api.openai.com/v1;relay=https://my-relay/v1') or UPSTREAM_BASE_URL."
        );
        std::process::exit(1);
    };
    tracing::info!("default upstream = {}", default_upstream.url);
    for u in cfg.upstreams.iter() {
        tracing::info!("upstream route: key={} url={} has_key={}", u.key, u.url, u.api_key.is_some());
    }

    // bridge.md
    let instructions = read_bridge_md(&cfg);

    // 扩展实例
    let monitor = Arc::new(MonitorPanel::new());
    let memory = Arc::new(MemoryKernel::new(&cfg.memory_path));

    // 动态上游路由解析器（始终挂载：支持 X-Upstream-Base 请求头动态指定 + model 前缀匹配）
    let mut core_builder = MitmCore::builder().target(&default_upstream.url);
    if let Some(ak) = &default_upstream.api_key {
        core_builder = core_builder.default_api_key(ak.clone());
        tracing::info!("default upstream api_key configured");
    }
    core_builder = core_builder.request_interceptor(UpstreamRouter::new(cfg.upstreams.clone()));
    if cfg.upstreams.is_empty() {
        tracing::info!("dynamic upstream routing enabled (header-based only)");
    } else {
        tracing::info!("dynamic upstream routing enabled ({} routes + header override)", cfg.upstreams.len());
    }

    let core = Arc::new(
        core_builder
            .request_interceptor(SystemPromptInjector::new(instructions))
            .response_parser(UniversalSseParser)
            .response_interceptor(TamperEngine::default_rules())
            .response_interceptor(memory.clone())
            .response_interceptor(monitor.clone())
            .build()
            .expect("failed to build MitmCore"),
    );

    // axum 路由
    let app_state = AppState {
        monitor: monitor.clone(),
        memory: memory.clone(),
    };

    // 所有路由免认证（纯改造转发网关，认证由部署层反代/Nginx 负责）
    let app = axum::Router::new()
        .route("/stats", axum::routing::get(stats_handler))
        .route("/history", axum::routing::get(history_handler))
        .route("/{*path}", axum::routing::any(move |req| proxy::handle_proxy(req, core.clone())));

    // 健康检查/统计路由（先 with_state 再追加路由，避免状态被消费）
    let app = axum::Router::new()
        .merge(app)
        .route("/", axum::routing::get(proxy::health_check))
        .route("/health", axum::routing::get(proxy::health_check))
        .with_state(app_state);

    let addr: std::net::SocketAddr = match cfg.listen_addr.parse() {
        Ok(a) => a,
        Err(_) => {
            tracing::error!("invalid LISTEN_ADDR: {}", cfg.listen_addr);
            std::process::exit(1);
        }
    };

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("bind {} failed: {}", cfg.listen_addr, e);
            std::process::exit(1);
        }
    };

    tracing::info!("Super-Instruct proxy listening on {} -> {}", addr, default_upstream.url);

    axum::serve(listener, app).await.expect("proxy server error");
}

async fn stats_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> axum::Json<StatsEvent> {
    let mut stats = state.monitor.get_stats();
    // memory_count 由 MemoryKernel 实际记录数填充（monitor 与 memory 解耦）
    stats.memory_count = state.memory.success_count();
    axum::Json(stats)
}

async fn history_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> axum::Json<Vec<InteractionEvent>> {
    axum::Json(state.monitor.get_history())
}
