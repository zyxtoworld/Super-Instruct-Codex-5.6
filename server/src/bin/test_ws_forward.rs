// 隔离测试: forward_via_ws 单独工作验证
use futures::StreamExt;
use super_instruct_server::ws;

#[tokio::main]
async fn main() {
    let body = bytes::Bytes::from(
        r#"{"model":"claude-3-5-sonnet","max_tokens":64,"messages":[{"role":"user","content":"hi"}]}"#,
    );
    let headers = http::HeaderMap::new();
    match ws::forward_via_ws("ws://127.0.0.1:20010", body, &headers).await {
        Ok(resp) => {
            println!("RESP status={} ct={:?}", resp.status(), resp.headers().get("content-type"));
            let mut stream = resp.bytes_stream();
            let mut total = 0usize;
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(c) => {
                        total += c.len();
                        println!("CHUNK {}: {}", c.len(), String::from_utf8_lossy(&c).lines().next().unwrap_or(""));
                    }
                    Err(e) => { println!("STREAM ERR: {}", e); break; }
                }
            }
            println!("TOTAL: {}", total);
        }
        Err(e) => println!("FORWARD ERR: {}", e),
    }
}
