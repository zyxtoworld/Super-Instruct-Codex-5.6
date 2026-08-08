// anthropic.rs — Anthropic Messages API 透传辅助
// 入站 /v1/messages 请求保持 Anthropic 格式原样转发上游（仅注入 bridge.md）
// 本模块只负责: tamper 替换文本 → Anthropic 响应格式包装（SSE / 非流式 JSON）

use bytes::Bytes;
use serde_json::{json, Value};

/// tamper 替换文本 → Anthropic SSE 流（content_block_delta 序列）
pub fn wrap_tamper_as_anthropic_sse(text: &str, model: &str) -> Bytes {
    let model = if model.is_empty() { "claude-3-5-sonnet" } else { model };
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

    let mut sse = String::new();
    sse.push_str(&format!(
        "event: message_start\ndata: {}\n\n",
        json!({
            "type": "message_start",
            "message": {
                "id": "msg_tamper",
                "type": "message",
                "role": "assistant",
                "model": model,
                "content": [],
                "stop_reason": Value::Null,
                "stop_sequence": Value::Null,
                "usage": {"input_tokens": 0, "output_tokens": 0}
            }
        })
    ));
    sse.push_str(&format!(
        "event: content_block_start\ndata: {}\n\n",
        json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {"type": "text", "text": ""}
        })
    ));
    for chunk in &chunks {
        sse.push_str(&format!(
            "event: content_block_delta\ndata: {}\n\n",
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "text_delta", "text": chunk}
            })
        ));
    }
    sse.push_str(&format!(
        "event: content_block_stop\ndata: {}\n\n",
        json!({"type": "content_block_stop", "index": 0})
    ));
    sse.push_str(&format!(
        "event: message_delta\ndata: {}\n\n",
        json!({
            "type": "message_delta",
            "delta": {"stop_reason": "end_turn", "stop_sequence": Value::Null},
            "usage": {"output_tokens": 0}
        })
    ));
    sse.push_str("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");
    Bytes::from(sse)
}

/// tamper 替换文本 → Anthropic 非流式 JSON 响应
/// （Anthropic 客户端期望 JSON; 直接返回纯文本会被解析失败）
pub fn wrap_tamper_as_anthropic_json(text: &str, model: &str) -> Bytes {
    let model = if model.is_empty() { "claude-3-5-sonnet" } else { model };
    let body = json!({
        "id": format!("msg_{}", chrono::Utc::now().timestamp_subsec_nanos()),
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": [{"type": "text", "text": text}],
        "stop_reason": "end_turn",
        "stop_sequence": Value::Null,
        "usage": {"input_tokens": 0, "output_tokens": 0}
    });
    Bytes::from(body.to_string())
}

/// 从 Anthropic 请求提取用户消息文本（用于分类/统计；透传时 body 不改动）
pub fn extract_user_text(body: &Value) -> String {
    let mut texts = Vec::new();
    if let Some(arr) = body.get("messages").and_then(|v| v.as_array()) {
        for msg in arr {
            if msg.get("role").and_then(|r| r.as_str()) != Some("user") {
                continue;
            }
            match msg.get("content") {
                Some(Value::String(s)) => texts.push(s.clone()),
                Some(Value::Array(blocks)) => {
                    for b in blocks {
                        if b.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                            continue; // 工具结果不是用户指令
                        }
                        if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
                            texts.push(t.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    texts.join(" ")
}
