// router.rs — 动态上游路由解析器（作为请求拦截器挂载）
// 上游选择的优先级：
//   1. 请求头 `X-Upstream-Base` 显式指定上游基址（可选 `X-Upstream-Key` 带 key）
//   2. 请求 body 的 model 前缀匹配已配置的 UPSTREAMS 条目
//   3. 以上均不命中 → 保持默认 target（配置的默认上游）
// 即：留配置做默认兜底，其余参数尽量从请求里拿。

use crate::config::Upstream;
use super_instruct_server::core::{RequestCtx, RequestInterceptor};

const HDR_BASE: &str = "x-upstream-base";
const HDR_KEY: &str = "x-upstream-key";

pub struct UpstreamRouter {
    upstreams: Vec<Upstream>,
}

impl UpstreamRouter {
    pub fn new(upstreams: Vec<Upstream>) -> Self {
        Self { upstreams }
    }
}

impl RequestInterceptor for UpstreamRouter {
    fn name(&self) -> &'static str {
        "router"
    }

    fn intercept(&self, ctx: &mut RequestCtx) {
        // 1. 请求头显式指定上游基址（最高优先级）
        if let Some(base) = ctx
            .headers
            .get(HDR_BASE)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            // SSRF 加固：仅接受 http/https，拒绝回环/链路本地/云元数据地址
            if !is_safe_upstream_url(base) {
                tracing::warn!(base = %base, "router: X-Upstream-Base rejected (unsafe scheme/host)");
                return;
            }
            tracing::info!(base = %base, "router: upstream base from request header");
            ctx.upstream_override = Some(base.to_string());
            // 可选的出站 key 头
            if let Some(key) = ctx
                .headers
                .get(HDR_KEY)
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                ctx.upstream_api_key = Some(key.to_string());
            }
            return;
        }

        // 2. 按 model 前缀匹配配置的 UPSTREAMS
        let model = ctx
            .body
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_lowercase();

        if model.is_empty() {
            return;
        }

        for u in &self.upstreams {
            if model == u.key.to_lowercase()
                || (u.key.len() >= 2 && model.starts_with(&u.key.to_lowercase()))
            {
                tracing::debug!(
                    model = %model,
                    upstream = %u.key,
                    "router: matched dynamic upstream"
                );
                ctx.upstream_override = Some(u.url.clone());
                if let Some(ak) = &u.api_key {
                    ctx.upstream_api_key = Some(ak.clone());
                }
                return;
            }
        }

        tracing::trace!(model = %model, "router: no dynamic upstream matched, using default");
    }
}

/// SSRF 加固：校验 X-Upstream-Base 是否安全
/// 用 url::Url 规范解析（处理数字 IP、IPv6 方括号、userinfo 等混淆），
/// 仅接受 http/https；host 不得为回环(127.x/localhost)、链路本地(169.254.x)、
/// 云元数据(169.254.169.254)、0.x、IPv6 回环(::1)/链路本地(fe80::/10)。
/// 私网段(10/172.16-31/192.168)允许 —— Docker 内网服务互相访问常用私网地址。
fn is_safe_upstream_url(base: &str) -> bool {
    let parsed = match url::Url::parse(base.trim()) {
        Ok(u) => u,
        Err(_) => return false,
    };

    // 仅 http/https
    match parsed.scheme() {
        "http" | "https" => {}
        _ => return false,
    }

    // 规范 host（url crate 已解码 userinfo/IPv6 括号/数字 IP）
    let host = match parsed.host_str() {
        Some(h) => h.to_lowercase(),
        None => return false,
    };

    // 域名形式：localhost 拒绝（先去除尾点，如 "localhost."）
    let host_nodot = host.trim_end_matches('.');
    if host_nodot == "localhost" || host_nodot.ends_with(".localhost") {
        return false;
    }

    // IP 形式判断（IPv6 可能带方括号如 "[::1]"，先剥离）
    let ip_host = if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        inner.to_string()
    } else {
        host.clone()
    };

    // 判定 host 是否为「IP 形态」（仅数字/点/冒号/百分号/字母x-a-f 的十六进制）：
    // 域名（含字母）不在此列，解析失败正常放行；IP 形态解析失败则 fail-closed 拒绝
    fn looks_like_ip(h: &str) -> bool {
        !h.is_empty()
            && h.chars()
                .all(|c| c.is_ascii_hexdigit() || matches!(c, '.' | ':' | '%'))
    }

    if looks_like_ip(&ip_host) {
        let ip = ip_host
            .parse::<std::net::IpAddr>()
            .map_err(|_| ()) // fail-closed：IP 形态但无法解析（如 zone id）→ 拒绝
            .ok();
        let ip = match ip {
            Some(ip) => ip,
            None => return false,
        };

        if ip.is_loopback() {
            return false;
        }
        match ip {
            std::net::IpAddr::V4(v4) => {
                let octets = v4.octets();
                // 链路本地 169.254.x.x（含云元数据 169.254.169.254）与 0.x.x.x 拒绝
                if octets[0] == 0 || (octets[0] == 169 && octets[1] == 254) {
                    return false;
                }
            }
            std::net::IpAddr::V6(v6) => {
                // IPv4-mapped IPv6（如 ::ffff:127.0.0.1 / ::ffff:169.254.169.254）→ 复用完整 IPv4 检查
                if let Some(mapped) = v6.to_ipv4_mapped() {
                    if mapped.is_loopback() {
                        return false;
                    }
                    let octets = mapped.octets();
                    if octets[0] == 0 || (octets[0] == 169 && octets[1] == 254) {
                        return false;
                    }
                }
                // IPv4-compatible IPv6（::a.b.c.d，旧式兼容地址）→ 复用 IPv4 检查
                if let Some(compat) = v6.to_ipv4() {
                    if compat.is_loopback() {
                        return false;
                    }
                    let octets = compat.octets();
                    if octets[0] == 0 || (octets[0] == 169 && octets[1] == 254) {
                        return false;
                    }
                }
                // NAT64 前缀 64:ff9b::/96（IPv4 嵌入）→ 静态无法判定目标，拒绝
                if v6.segments()[0] == 0x0064 && v6.segments()[1] == 0xff9b {
                    return false;
                }
                // 链路本地 fe80::/10
                if v6.segments()[0] & 0xffc0 == 0xfe80 {
                    return false;
                }
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::is_safe_upstream_url;

    #[test]
    fn rejects_ssrf_vectors() {
        // 回环 / 域名 / 数字 IP / 链路本地 / 元数据 / userinfo / 0.x / IPv6 回环 / IPv6 链路本地 / 非 http scheme
        for url in [
            "http://127.0.0.1:9/x",
            "http://localhost:9/x",
            "http://2852039166/x",        // 169.254.169.254 十进制
            "http://169.254.169.254/latest",
            "http://user:pass@127.0.0.1:9/x",
            "http://0.0.0.0:9/x",
            "http://[::1]:9/x",
            "http://[fe80::1]/x",
            "http://127.1/x",
            "ftp://x.com/f",
            "gopher://169.254.169.254/x",
            "http://0x7f000001/x",        // 127.0.0.1 hex
            "http://[::ffff:127.0.0.1]/x",          // IPv4-mapped 回环
            "http://[::ffff:169.254.169.254]/x",    // IPv4-mapped 元数据
            "http://[::ffff:0.0.0.0]/x",
            "http://[fe80::1%25eth0]/x",            // zone id 链路本地
            "http://localhost./x",                  // 尾点 localhost
            "http://foo.localhost./x",
            "http://[64:ff9b::7f00:1]/x",           // NAT64 → 127.0.0.1
            "http://[64:ff9b::a9fe:a9fe]/x",        // NAT64 → 169.254.169.254
            "http://[::127.0.0.1]/x",               // IPv4-compatible 回环
            "http://[::169.254.169.254]/x",         // IPv4-compatible 元数据
        ] {
            assert!(!is_safe_upstream_url(url), "should reject: {}", url);
        }
    }

    #[test]
    fn allows_safe_urls() {
        // 公网域名 / 私网段（Docker 内网）/ 无 scheme 拒绝
        for url in [
            "https://relay.example.com/v1",
            "https://api.openai.com/v1",
            "http://192.168.31.228:8080/v1",
            "http://10.0.0.5/v1",
            "http://172.16.1.2/v1",
        ] {
            assert!(is_safe_upstream_url(url), "should allow: {}", url);
        }
        assert!(!is_safe_upstream_url("relay.example.com/v1"));
        assert!(!is_safe_upstream_url(""));
    }
}
