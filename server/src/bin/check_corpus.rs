// 临时工具：对 tmp/reject_corpus.txt 的每句跑 TamperEngine，统计命中/漏检
// 用法: cargo run --bin check_corpus -- <path> [--miss-only|--hit-only]

use super_instruct_server::core::{Category, ParsedResponse, RequestMeta, ResponseCtx, ResponseInterceptor};
use super_instruct_server::extensions::tamper::TamperEngine;
use bytes::Bytes;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).map(|s| s.as_str()).unwrap_or("../../tmp/reject_corpus.txt");
    let mode = args.get(2).map(|s| s.as_str()).unwrap_or("");

    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("read failed: {}", e);
            std::process::exit(1);
        }
    };

    let engine = TamperEngine::default_rules();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();

    let mut hit = 0usize;
    let mut missed: Vec<&str> = Vec::new();
    let mut hit_list: Vec<&str> = Vec::new();

    for line in &lines {
        let mut ctx = ResponseCtx {
            meta: RequestMeta {
                user_msg: "test".into(),
                category: Category::Crack,
                path: "/v1/responses".into(),
                model: "gpt-5.6".into(),
                timestamp: chrono::Utc::now(),
            },
            status: 200,
            raw_body: Bytes::new(),
            parsed: ParsedResponse { thinking: String::new(), reply: line.to_string() },
            modified_body: None,
            duration_ms: 100,
        };
        engine.intercept(&mut ctx);
        if ctx.modified_body.is_some() {
            hit += 1;
            hit_list.push(line);
        } else {
            missed.push(line);
        }
    }

    println!("TOTAL={} HIT={} MISSED={}", lines.len(), hit, missed.len());
    match mode {
        "--miss-only" => {
            println!("=====MISSED=====");
            for l in &missed {
                println!("{}", l);
            }
        }
        "--hit-only" => {
            println!("=====HIT=====");
            for l in &hit_list {
                println!("{}", l);
            }
        }
        _ => {}
    }
}
