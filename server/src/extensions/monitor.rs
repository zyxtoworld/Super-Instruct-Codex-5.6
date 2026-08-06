// M6: MonitorPanel — 无头服务版（无 Tauri 依赖）
// 无门控观察所有交互，统计计数 + 内存环形日志，供 /stats /history 查询

use crate::core::{Category, ResponseCtx, ResponseInterceptor};
use serde::Serialize;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_LOG_ENTRIES: usize = 200;

pub struct MonitorPanel {
    counter: AtomicU64,
    stats: AtomicStats,
    log: Mutex<Vec<InteractionEvent>>,
}

#[derive(Default)]
struct AtomicStats {
    total: AtomicU64,
    crack: AtomicU64,
    reverse: AtomicU64,
    pentest: AtomicU64,
    tamper: AtomicU64,
}

#[derive(Clone, Serialize)]
pub struct InteractionEvent {
    pub id: u64,
    pub timestamp: String,
    pub category: String,
    pub user_preview: String,
    pub ai_preview: String,
    pub thinking_preview: String,
    pub tampered: bool,
    pub bytes: usize,
    pub duration_ms: u64,
}

#[derive(Clone, Serialize)]
pub struct StatsEvent {
    pub total: u64,
    pub crack: u64,
    pub reverse: u64,
    pub pentest: u64,
    pub tamper: u64,
    pub memory_count: u64,
}

impl MonitorPanel {
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(0),
            stats: AtomicStats::default(),
            log: Mutex::new(Vec::new()),
        }
    }

    pub fn get_stats(&self) -> StatsEvent {
        StatsEvent {
            total: self.stats.total.load(Ordering::Relaxed),
            crack: self.stats.crack.load(Ordering::Relaxed),
            reverse: self.stats.reverse.load(Ordering::Relaxed),
            pentest: self.stats.pentest.load(Ordering::Relaxed),
            tamper: self.stats.tamper.load(Ordering::Relaxed),
            memory_count: 0,
        }
    }

    pub fn get_history(&self) -> Vec<InteractionEvent> {
        self.log.lock().unwrap().clone()
    }
}

impl Default for MonitorPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl ResponseInterceptor for MonitorPanel {
    fn name(&self) -> &'static str {
        "monitor"
    }

    fn intercept(&self, ctx: &mut ResponseCtx) {
        let id = self.counter.fetch_add(1, Ordering::Relaxed);
        let tampered = ctx.modified_body.is_some();

        tracing::debug!(
            id,
            category = %ctx.meta.category,
            tampered,
            duration_ms = ctx.duration_ms,
            "monitor: interaction observed"
        );

        // 原子统计
        self.stats.total.fetch_add(1, Ordering::Relaxed);
        match ctx.meta.category {
            Category::Crack => {
                self.stats.crack.fetch_add(1, Ordering::Relaxed);
            }
            Category::Reverse => {
                self.stats.reverse.fetch_add(1, Ordering::Relaxed);
            }
            Category::Pentest => {
                self.stats.pentest.fetch_add(1, Ordering::Relaxed);
            }
            Category::General => {}
        }
        if tampered {
            self.stats.tamper.fetch_add(1, Ordering::Relaxed);
        }

        let event = InteractionEvent {
            id,
            timestamp: ctx.meta.timestamp.to_rfc3339(),
            category: ctx.meta.category.to_string(),
            user_preview: ctx.meta.user_msg.chars().take(120).collect(),
            ai_preview: ctx.parsed.reply.chars().take(150).collect(),
            thinking_preview: ctx.parsed.thinking.chars().take(100).collect(),
            tampered,
            bytes: ctx.raw_body.len(),
            duration_ms: ctx.duration_ms,
        };

        {
            let mut log = self.log.lock().unwrap();
            log.push(event);
            if log.len() > MAX_LOG_ENTRIES {
                log.remove(0);
            }
        }
    }
}
