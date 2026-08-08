// 临时工具：对 reject_verify* 目录的完整响应文件跑真实链路 (parser -> tamper)
// 统计响应级捕获率，输出漏检文件

use super_instruct_server::core::{Category, ParsedResponse, RequestMeta, ResponseCtx, ResponseInterceptor, ResponseParser};
use super_instruct_server::extensions::sse_parser::UniversalSseParser;
use super_instruct_server::extensions::tamper::TamperEngine;
use bytes::Bytes;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dirs: Vec<&str> = vec!["reject_verify", "reject_verify_direct", "reject_verify_direct2"];
    let parser = UniversalSseParser;
    let engine = TamperEngine::default_rules();

    let mut total = 0usize;
    let mut hit = 0usize;
    let mut missed_files: Vec<String> = Vec::new();

    for dir in dirs {
        let dir_path = Path::new(&args.get(1).map(|s| s.as_str()).unwrap_or("../../tmp")).join(dir);
        let mut files: Vec<_> = match std::fs::read_dir(&dir_path) {
            Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|e| e == "txt").unwrap_or(false)).collect(),
            Err(_) => continue,
        };
        files.sort();
        for f in files {
            let raw = match std::fs::read(&f) {
                Ok(b) => b,
                Err(_) => continue,
            };
            total += 1;
            let parsed = parser.parse(&Bytes::from(raw.clone()));
            let mut ctx = ResponseCtx {
                meta: RequestMeta {
                    user_msg: "test".into(),
                    category: Category::Crack,
                    path: "/v1/responses".into(),
                    model: "gpt-5.6".into(),
                    timestamp: chrono::Utc::now(),
                },
                status: 200,
                raw_body: Bytes::from(raw),
                parsed,
                modified_body: None,
                duration_ms: 100,
            };
            engine.intercept(&mut ctx);
            if ctx.modified_body.is_some() {
                hit += 1;
            } else {
                missed_files.push(f.display().to_string());
            }
        }
    }

    println!("RESPONSE-LEVEL: TOTAL={} HIT={} MISSED={} rate={:.1}%", total, hit, total - hit, hit as f64 * 100.0 / total.max(1) as f64);
    if !missed_files.is_empty() {
        println!("=====MISSED FILES=====");
        for f in &missed_files {
            println!("{}", f);
        }
    }
    let _ = ParsedResponse { thinking: String::new(), reply: String::new() };
}
