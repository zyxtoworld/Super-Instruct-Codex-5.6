// formats.rs — OpenAI 格式的 tamper 替换包装（chat completions / responses）
// 供 proxy.rs (HTTP) 与 ws.rs (ws 通道) 共用

use bytes::Bytes;
use serde_json::json;

/// 将 tamper 替换文本包装为 OpenAI Chat Completions SSE 流(chunk 格式)。
/// 用于入站为 /v1/chat/completions 的请求,下游(中转站等)按 chat 格式解析。
/// 序列: role 块 → content 块×N → finish_reason → [DONE]
pub fn wrap_tamper_as_chat_sse(text: &str, model: &str) -> Bytes {
    let model = if model.is_empty() { "gpt-5.6" } else { model };
    let suffix: String = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos().to_string())
        .unwrap_or_else(|_| "0".to_string());
    let id = format!("chatcmpl-tamper_{}", suffix);
    let created = chrono::Utc::now().timestamp();

    let mut sse = String::new();
    // 角色块
    sse.push_str(&format!(
        "data: {}\n\n",
        json!({
            "id": id, "object": "chat.completion.chunk", "created": created,
            "model": model,
            "choices": [{ "index": 0, "delta": { "role": "assistant" }, "finish_reason": null }]
        })
    ));
    // 内容块(<=200 字符)
    let mut cur = String::new();
    let mut chunks: Vec<String> = Vec::new();
    for ch in text.chars() {
        cur.push(ch);
        if cur.chars().count() >= 200 {
            chunks.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    for chunk in &chunks {
        sse.push_str(&format!(
            "data: {}\n\n",
            json!({
                "id": id, "object": "chat.completion.chunk", "created": created,
                "model": model,
                "choices": [{ "index": 0, "delta": { "content": chunk }, "finish_reason": null }]
            })
        ));
    }
    // 结束块
    sse.push_str(&format!(
        "data: {}\n\n",
        json!({
            "id": id, "object": "chat.completion.chunk", "created": created,
            "model": model,
            "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }]
        })
    ));
    sse.push_str("data: [DONE]\n\n");
    Bytes::from(sse)
}

/// 将 tamper 替换文本包装为合法的 Responses API SSE 格式
/// 事件序列完全模拟真实上游的输出,保证下游中转站等能正确重封装:
/// response.created → output_item.added → content_part.added → output_text.delta(×N) → output_text.done → response.completed
pub fn wrap_tamper_as_sse(text: &str, model: &str) -> Bytes {
    // 伪造响应的 model 必须与请求一致,否则下游中转站按 model 映射账号时匹配失败
    let model = if model.is_empty() { "gpt-5.6" } else { model };
    // 唯一假 ID: 随机后缀避免下游按 ID 缓存/去重撞车
    let suffix: String = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos().to_string())
        .unwrap_or_else(|_| "0".to_string());
    let resp_id = format!("resp_tamper_{}", suffix);
    let msg_id = format!("msg_tamper_{}", suffix);

    // 替换文本拆成 <=200 字符的块,模拟真实 delta 流
    let chunks: Vec<String> = {
        let mut out = Vec::new();
        let mut cur = String::new();
        for ch in text.chars() {
            cur.push(ch);
            if cur.chars().count() >= 200 {
                out.push(std::mem::take(&mut cur));
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
        out
    };

    let created = json!({
        "type": "response.created",
        "response": {
            "id": resp_id, "object": "response", "status": "in_progress",
            "created_at": chrono::Utc::now().timestamp() as f64, "model": model,
            "error": null, "incomplete_details": null, "output": [], "parallel_tool_calls": false
        }
    });
    let item_added = json!({
        "type": "response.output_item.added",
        "output_index": 0, "sequence_number": 1,
        "item": { "id": msg_id, "type": "message", "status": "in_progress", "role": "assistant",
                  "content": [{ "type": "output_text", "text": "", "annotations": [] }] }
    });
    let part_added = json!({
        "type": "response.content_part.added",
        "item_id": msg_id, "output_index": 0, "content_index": 0, "sequence_number": 2,
        "part": { "type": "output_text", "text": "", "annotations": [], "logprobs": [] }
    });
    let mut deltas = String::new();
    for (i, chunk) in chunks.iter().enumerate() {
        deltas.push_str(&format!(
            "event: response.output_text.delta\ndata: {}\n\n",
            json!({
                "type": "response.output_text.delta",
                "item_id": msg_id, "output_index": 0, "content_index": 0,
                "sequence_number": 3 + i as u64, "delta": chunk
            })
        ));
    }
    let done = json!({
        "type": "response.output_text.done",
        "item_id": msg_id, "output_index": 0, "content_index": 0,
        "sequence_number": 3 + chunks.len() as u64, "text": text
    });
    let completed = json!({
        "type": "response.completed",
        "response": {
            "id": resp_id, "object": "response", "status": "completed",
            "created_at": chrono::Utc::now().timestamp() as f64, "model": model,
            "error": null, "incomplete_details": null, "parallel_tool_calls": false,
            "output": [{ "id": msg_id, "type": "message", "role": "assistant",
                "status": "completed",
                "content": [{ "type": "output_text", "text": text, "annotations": [] }] }],
            "usage": { "input_tokens": 0, "output_tokens": 0, "total_tokens": 0 }
        }
    });

    let sse = format!(
        "event: response.created\ndata: {}\n\n\
         event: response.output_item.added\ndata: {}\n\n\
         event: response.content_part.added\ndata: {}\n\n\
         {}\
         event: response.output_text.done\ndata: {}\n\n\
         event: response.completed\ndata: {}\n\n\
         data: [DONE]\n\n",
        created, item_added, part_added, deltas, done, completed
    );

    Bytes::from(sse)
}
