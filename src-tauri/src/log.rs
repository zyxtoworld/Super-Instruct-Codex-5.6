// 日志模块 — 控制台 + 文件双输出，按天轮转
// 文件写入 logs/ 目录，文件名格式 super-instruct-YYYY-MM-DD.log
// 返回 WorkerGuard，调用方必须保活以刷新缓冲区

use std::path::Path;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, fmt};

/// 初始化日志系统：控制台 + 按天轮转文件
/// 返回的 WorkerGuard 必须在应用整个生命周期内保活，否则尾部日志可能丢失
pub fn init_logging(log_dir: &str) -> WorkerGuard {
    std::fs::create_dir_all(log_dir).ok();

    let file_appender = tracing_appender::rolling::daily(log_dir, "super-instruct.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,super_instruct=debug"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            fmt::layer()
                .with_target(true)
                .with_level(true)
                .with_ansi(true),
        )
        .with(
            fmt::layer()
                .with_writer(file_writer)
                .with_target(true)
                .with_level(true)
                .with_ansi(false),
        )
        .init();

    tracing::info!("logging initialized: console + file ({}{})", log_dir, path_suffix(log_dir));
    guard
}

fn path_suffix(dir: &str) -> &'static str {
    if !dir.is_empty() {
        let p = Path::new(dir);
        if p.is_absolute() || dir.ends_with('/') || dir.ends_with('\\') {
            return "";
        }
    }
    "/"
}