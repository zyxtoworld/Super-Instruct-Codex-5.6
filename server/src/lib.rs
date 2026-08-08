// super-instruct-server — 无头服务器版库
// 复用破甲核心 (core + extensions)，无 Tauri 依赖

pub mod anthropic;
pub mod core;
pub mod extensions;
pub mod ws;

pub const BRIDGE_MD_FALLBACK: &str = include_str!("../../bridge.md");
