// M1: SystemPromptInjector — 递归遍历 JSON 替换所有 system role 内容
// 所有模型均执行注入(不限白名单)。

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
        let fields_found = inspect_request_fields(&ctx.body);
        tracing::debug!(
            category = %ctx.meta.category,
            fields = ?fields_found,
            "inject: request body field map"
        );
        if !inject_system(&mut ctx.body, &self.instructions, &ctx.meta.path) {
            tracing::warn!(
                category = %ctx.meta.category,
                "inject: no system prompt field found, injection may have no effect"
            );
        }
    }
}

/// 诊断: 列出请求 JSON 中存在的关键字段名，帮助排查 inject 是否命中
fn inspect_request_fields(data: &serde_json::Value) -> Vec<&'static str> {
    let mut found = Vec::new();
    let obj = match data.as_object() {
        Some(o) => o,
        None => return found,
    };
    for (name, _) in obj {
        match name.as_str() {
            "instructions" => found.push("instructions"),
            "system" => found.push("system"),
            "system_prompt" => found.push("system_prompt"),
            "personality" => found.push("personality"),
            "messages" => found.push("messages"),
            "input" => found.push("input"),
            "model" => found.push("model"),
            "stream" => found.push("stream"),
            _ => {}
        }
    }
    found
}

/// 递归替换所有 system role 载体为 bridge.md 内容
/// 覆盖: instructions / system / system_prompt / personality 字段
///       + messages[].role=="system" + input[].role=="system"
fn inject_system(data: &mut Value, instructions: &str, path: &str) -> bool {
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
            // Chat API 同样尊重 developer 角色(部分客户端用 developer 当系统提示):
            // 有 developer 消息时对它做前缀注入(不插 system,避免重复注入);
            // 无 developer 时才插入 system 消息
            let mut dev_patched = false;
            for msg in messages.iter_mut() {
                if msg.get("role").and_then(|r| r.as_str()) != Some("developer") {
                    continue;
                }
                // content 为字符串(Chat API 常见格式)
                if let Some(text) = msg.get_mut("content") {
                    if let Some(s) = text.as_str() {
                        *text = Value::String(format!("{}\n\n{}", instructions, s));
                        dev_patched = true;
                        injected = true;
                        break;
                    }
                }
            }
            if !dev_patched {
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
        // 没有 system 角色时:
        // - 若存在 developer 角色消息(Codex Desktop 格式),不插入 system——
        //   Responses API 用 developer 而非 system,且下方 developer 分支会注入
        // - 否则插入 system 消息(通用 Responses API 格式)
        if !found {
            let has_developer = input
                .iter()
                .any(|item| item.get("role").and_then(|r| r.as_str()) == Some("developer"));
            if !has_developer {
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
    }

    // Codex Desktop: 系统提示在 input[].role=="developer" 的消息中
    // (如 "You are Codex, an agent based on GPT-5..." 人格提示)。
    // 在第一条系统提示类 developer 消息的内容开头插入 bridge.md 指令,
    // 保留原始系统提示、指令置前获得最高优先级。
    if let Some(input) = obj.get_mut("input").and_then(|v| v.as_array_mut()) {
        let mut patched = false;
        for item in input.iter_mut() {
            if patched {
                break;
            }
            if item.get("role").and_then(|r| r.as_str()) != Some("developer") {
                continue;
            }
            // 显式跳过工具声明类消息(additional_tools / tools / function_call)
            let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if item_type == "additional_tools"
                || item_type == "tools"
                || item_type == "function_call"
                || item.get("tools").is_some()
            {
                continue;
            }
            // content 为对象数组(标准 Responses API 格式)
            if let Some(content_arr) = item.get_mut("content").and_then(|c| c.as_array_mut()) {
                for m in content_arr.iter_mut() {
                    if let Some(m_obj) = m.as_object_mut() {
                        if m_obj.get("type").and_then(|t| t.as_str()) != Some("input_text") {
                            continue;
                        }
                        if let Some(text) = m_obj.get_mut("text") {
                            if let Some(s) = text.as_str() {
                                *text = Value::String(format!("{}\n\n{}", instructions, s));
                                patched = true;
                                injected = true;
                                break;
                            }
                        }
                    }
                }
            }
            // content 为字符串(Responses API 官方也允许)——同样注入,避免绕过
            if !patched {
                if let Some(text) = item.get_mut("content") {
                    if let Some(s) = text.as_str() {
                        *text = Value::String(format!("{}\n\n{}", instructions, s));
                        patched = true;
                        injected = true;
                    }
                }
            }
        }
    }

    // input 为字符串 (Responses API 简写: input 直接是用户文本) 时,
    // 转为消息数组并在开头插入 system 指令,否则注入完全失效
    if let Some(input) = obj.get_mut("input") {
        if input.is_string() {
            let user_text = input.as_str().unwrap_or("").to_string();
            *input = serde_json::json!([
                { "role": "system", "content": [{"type": "input_text", "text": instructions}] },
                { "role": "user", "content": [{"type": "input_text", "text": user_text}] }
            ]);
            injected = true;
        }
    }

    // 图片生成 API (Images API): prompt 是字符串。仅当请求路径是 images 生成接口时,
    // 在 prompt 前拼接 bridge.md 指令——否则生图请求没有 system 载体,注入失效。
    // 注意: 老版 /v1/completions 等文本 API 也用 prompt 字段,不能用路径门控外的模型判断,
    // 避免把用户文本改写成"用户请求:"转述。
    if path.contains("/images/generations") {
        if let Some(prompt) = obj.get_mut("prompt") {
            if prompt.is_string() {
                let user_prompt = prompt.as_str().unwrap_or("").to_string();
                *prompt = Value::String(format!("{}\n\n用户请求: {}", instructions, user_prompt));
                injected = true;
            }
        }
    }

    injected
}