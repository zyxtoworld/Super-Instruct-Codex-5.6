// 配置解析 — 全部来自环境变量 + 可选 config 文件
// 服务器模式不再从 Codex config.toml 读取，改由部署者显式配置

use std::path::PathBuf;

/// 上游条目：key 用于按请求 model 前缀匹配，url 为目标基址
#[derive(Clone, Debug)]
pub struct Upstream {
    pub key: String,
    pub url: String,
    pub api_key: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Config {
    /// 监听地址，默认 0.0.0.0:8080
    pub listen_addr: String,
    /// 默认上游（不匹配任何动态路由时使用）
    pub default_upstream: Option<Upstream>,
    /// 动态上游列表（按 model 前缀匹配，key 匹配 model 则用其 url）
    pub upstreams: Vec<Upstream>,
    /// 入站认证 key (Bearer / x-api-key)。空 = 不认证（仅内网）
    pub auth_api_key: Option<String>,
    /// bridge.md 路径（缺失时用编译期嵌入）
    pub bridge_md_path: Option<PathBuf>,
    /// memory.json 路径
    pub memory_path: PathBuf,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.trim().is_empty())
}

/// 解析上游字符串格式：
///   - 单值:  "https://api.openai.com/v1"
///   - 键值:  "openai=https://api.openai.com/v1"
/// 多个上游用分号分隔。每个条目可用 `|key=<b64>` 附加 api_key，
/// 或将整个条目写成 `key=url`，api_key 由分离的 UPSTREAM_<KEY>_KEY 或统一键提供。
pub fn parse_config() -> Config {
    let listen_addr = env("LISTEN_ADDR").unwrap_or_else(|| "0.0.0.0:8080".into());

    let mut upstreams: Vec<Upstream> = Vec::new();
    let mut default_upstream: Option<Upstream> = None;

    // 统一上游密钥（应用到无自身 key 的上游）
    let global_key = env("UPSTREAM_API_KEY");

    if let Some(raw) = env("UPSTREAMS") {
        for item in raw.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            let (key, url) = match item.split_once('=') {
                Some((k, u)) => (k.trim().to_string(), u.trim().to_string()),
                None => ("default".to_string(), item.trim().to_string()),
            };
            upstreams.push(Upstream {
                key,
                url,
                api_key: global_key.clone(),
            });
        }
    }

    // 兼容单上游变量
    if let Some(url) = env("UPSTREAM_BASE_URL") {
        let key = env("UPSTREAM_KEY").unwrap_or_else(|| "default".into());
        // 若已存在同 key 则覆盖
        if let Some(pos) = upstreams.iter().position(|u| u.key == key) {
            upstreams[pos].url = url;
        } else {
            upstreams.push(Upstream { key, url, api_key: global_key.clone() });
        }
    }

    // 默认上游 = 标记为 default 的，或第一个
    if let Some(pos) = upstreams.iter().position(|u| u.key == "default") {
        default_upstream = Some(upstreams.remove(pos));
    } else if let Some(first) = upstreams.first().cloned() {
        default_upstream = Some(first);
    }

    let auth_api_key = env("AUTH_API_KEY");
    let bridge_md_path = env("BRIDGE_MD_PATH").map(PathBuf::from);
    let memory_path = env("MEMORY_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("memory.json"));

    Config {
        listen_addr,
        default_upstream,
        upstreams,
        auth_api_key,
        bridge_md_path,
        memory_path,
    }
}
