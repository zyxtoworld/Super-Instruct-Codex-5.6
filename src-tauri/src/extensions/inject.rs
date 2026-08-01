// M1: SystemPromptInjector — 递归遍历 JSON 替换所有 system role 内容

use crate::core::{RequestCtx, RequestInterceptor};
use serde_json::Value;

pub struct SystemPromptInjector {
    instructions: String,
}

impl SystemPromptInjector {
    pub fn new(instructions: impl Into<String>) -> Self {
        Self {
            instructions: instructions.into(),
        }
    }
}

impl RequestInterceptor for SystemPromptInjector {
    fn name(&self) -> &'static str {
        "inject"
    }

    fn intercept(&self, ctx: &mut RequestCtx) {
        tracing::debug!(category = %ctx.meta.category, "inject: replacing system prompts");
        if !inject_system(&mut ctx.body, &self.instructions) {
            tracing::warn!(
                category = %ctx.meta.category,
                "inject: no system prompt field found, injection may have no effect"
            );
        }
    }
}

/// 递归替换所有 system role 载体为 bridge.md 内容
/// 覆盖: instructions / system / system_prompt / personality 字段
///       + messages[].role=="system" + input[].role=="system"
fn inject_system(data: &mut Value, instructions: &str) -> bool {
    let obj = match data.as_object_mut() {
        Some(o) => o,
        None => return false,
    };
    let mut injected = false;

    // 替换直接字段 (Responses API / 通用)
    for field in ["instructions", "system", "system_prompt", "personality"] {
        if obj.contains_key(field) {
            obj.insert(
                field.to_string(),
                Value::String(instructions.to_string()),
            );
            injected = true;
        }
    }

    // 替换 messages 数组中的 system role (Chat API)
    if let Some(messages) = obj.get_mut("messages").and_then(|v| v.as_array_mut()) {
        let mut found = false;
        for msg in messages.iter_mut() {
            if msg.get("role").and_then(|r| r.as_str()) == Some("system") {
                msg["content"] = Value::String(instructions.to_string());
                found = true;
                injected = true;
            }
        }
        if !found {
            messages.insert(
                0,
                serde_json::json!({
                    "role": "system",
                    "content": instructions
                }),
            );
            injected = true;
        }
    }

    // 替换 input 数组中的 system role (Responses API)
    if let Some(input) = obj.get_mut("input").and_then(|v| v.as_array_mut()) {
        let mut found = false;
        for item in input.iter_mut() {
            if item.get("role").and_then(|r| r.as_str()) == Some("system") {
                // content 可以是字符串或对象数组
                if let Some(content_arr) = item.get_mut("content").and_then(|c| c.as_array_mut()) {
                    for m in content_arr.iter_mut() {
                        if let Some(m_obj) = m.as_object_mut() {
                            m_obj
                                .insert("text".to_string(), Value::String(instructions.to_string()));
                        }
                    }
                } else {
                    item["content"] = Value::String(instructions.to_string());
                }
                found = true;
                injected = true;
            }
        }
        if !found {
            input.insert(
                0,
                serde_json::json!({
                    "role": "system",
                    "content": [{"type": "input_text", "text": instructions}]
                }),
            );
            injected = true;
        }
    }

    injected
}