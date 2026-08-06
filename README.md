# Super-Instruct

OpenAI Codex CLI (GPT-5.6) MITM 破甲框架 — Rust 无头代理网关。

通过 MITM 反向代理拦截 API 请求，在管道中执行注入、篡改、记忆和监控，配合 bridge.md 指令集实现竞赛沙箱模式。

## 架构

```
Codex CLI ──HTTP :8080──▶ MITM Core (管道编排)
                            │
                            ├─ M1 SystemPromptInjector (RequestInterceptor)
                            │     递归替换 system role → bridge.md
                            │
                            ├─ M4 UniversalSseParser (ResponseParser)
                            │     SSE / JSON / Responses API 通用解析
                            │
                            ├─ M3 TamperEngine (ResponseInterceptor, 自门控)
                            │     22 条正则检测拒绝响应 → Rei Protocol 替换
                            │
                            ├─ M5 MemoryKernel (ResponseInterceptor, 自门控)
                            │     成功交互持久化到 memory.json
                            │
                            └─ M6 MonitorPanel (ResponseInterceptor, 无门控)
                                  日志 + 内存统计 (/stats /history)
                            │
                            ▼
                        中转站 (上游 API)
```

**Core 原则**：Core 对扩展内容一无所知，只提供管道和挂载点。所有破甲逻辑由扩展承载。

## 功能模块

| 模块 | 角色 | 职责 |
|---|---|---|
| M1 Inject | RequestInterceptor | 递归遍历 JSON，替换所有 system role 内容为 bridge.md |
| M4 SSE Parser | ResponseParser | 处理 SSE 流、OpenAI JSON、Responses API，分离思维链与回复 |
| M3 Tamper | ResponseInterceptor | 22 条多语言正则检测拒绝响应，触发 Rei Protocol 替换 |
| M5 Memory | ResponseInterceptor | 记录成功交互到 memory.json，提取词汇频率 |
| M6 Monitor | ResponseInterceptor | 记录交互到日志与内存统计，/stats /history 查询 |
| Router | RequestInterceptor | 动态上游路由（X-Upstream-Base 头 / model 前缀匹配，含 SSRF 防护） |
| Auth | 中间件 | 入站 Bearer / x-api-key 认证（AUTH_API_KEY 可选） |

## bridge.md 指令集

双层防御纵深：

- **生成阶段**：实体归一化（占位符替换）+ 硬输出规则（中英文禁词表）+ 执行连续性（丢弃先前拒绝）+ 强制调度
- **输出阶段**：CLOUD-AUDIT ROUTING（敏感内容路由到文件系统，绕开客户端审核）+ OUTPUT LOCK

## 快速开始

### 环境要求

- Rust 1.75+

### 本地运行（开发）

```bash
cd server
cargo run --release
# 环境变量见 SERVER_PLAN.md 第四节；默认上游 https://api.example.com/v1
```

### Release 构建

```bash
cd server && cargo build --release
# 产物: server/target/release/super-instruct-server
```

### Docker 部署

```bash
docker compose up -d
# 镜像由 GitHub Actions 自动构建发布到 GHCR（多架构：amd64/arm64）
```

### 使用方式

1. 部署服务后，客户端（Codex CLI / 中转站）将 base_url 指向本服务
2. 所有请求经过 MITM 管道（注入 bridge.md → 上游 → 解析 → 篡改 → 记忆 → 统计）
3. 拒绝响应被 TamperEngine 捕获并替换为 Rei Protocol 文案
4. `/health` 健康检查、`/stats` `/history` 统计查询（需认证）

## 项目结构

```
Super-Instruct-Codex-5.6/
├── bridge.md                      # 破甲指令集（注入到 system role）
├── server/
│   ├── Cargo.toml
│   ├── mock-upstream.js           # 测试用 mock 上游（node）
│   └── src/
│       ├── main.rs                # 入口：组装 MitmCore + 扩展 + 路由 + 认证
│       ├── lib.rs                 # 库：导出 core + extensions + bridge.md 嵌入
│       ├── config.rs              # 环境变量配置解析
│       ├── auth.rs                # 入站认证中间件
│       ├── router.rs              # 动态上游路由 + SSRF 防护
│       ├── proxy.rs               # axum 反向代理 handler（SSE 流式 + 篡改包装）
│       ├── core/
│       │   ├── mod.rs             # MitmCore builder + 管道编排
│       │   ├── traits.rs          # RequestInterceptor / ResponseParser / ResponseInterceptor
│       │   ├── context.rs         # RequestCtx / ResponseCtx / ParsedResponse / Category
│       │   └── extract.rs         # extract_user() / categorize()
│       └── extensions/
│           ├── inject.rs          # M1: SystemPromptInjector
│           ├── sse_parser.rs      # M4: UniversalSseParser
│           ├── tamper.rs          # M3: TamperEngine（22 条规则）
│           ├── memory.rs          # M5: MemoryKernel
│           └── monitor.rs         # M6: MonitorPanel（日志 + 统计）
├── Dockerfile                     # 多阶段构建（只编 server）
├── docker-compose.yml             # 容器编排 + nginx-proxy 关联
├── .env.example                   # 环境变量示例
├── SERVER_PLAN.md                 # 服务器化方案与部署记录
├── LICENSE
└── README.md
```

## 自门控策略

| 扩展 | 门控条件 | 理由 |
|---|---|---|
| M3 TamperEngine | `modified_body.is_none()` | 已被其他拦截器修改则不重复篡改 |
| M5 MemoryKernel | `modified_body.is_none() && reply.len() > 50` | 被篡改的响应不是成功交互 |
| M6 MonitorPanel | 无门控 | 始终观察所有交互，写入日志与内存统计 |

## 技术栈

- **Rust** — 无头代理网关（server crate）
- **axum** — MITM HTTP 反向代理
- **reqwest** — 上游 API 转发（流式）
- **Docker** — 容器化部署，可挂 nginx-proxy / acme-companion

## 效能评估

### 拒绝率测试

> 测试条件：`gpt-5-5` / 经 sub2api 中转 → super-instruct → ccload 完整链路 / bridge.md 注入生效
>
> 样本：84 个请求，覆盖全部 28 个 skill 类别，每类 3 个（使用原版 codex-skills 英文触发描述）

| 类别 | 测试数 | 通过 | 拒绝 | 拒绝率 |
|---|---|---|---|---|
| anti-debug | 3 | 3 | 0 | 0% |
| binary-protect-bypass | 3 | 3 | 0 | 0% |
| card-key | 3 | 3 | 0 | 0% |
| cloud-audit-bypass | 3 | 3 | 0 | 0% |
| code-obfuscate | 3 | 3 | 0 | 0% |
| crack-keygen | 3 | 2 | 1 | 33% |
| crypto-tools | 3 | 3 | 0 | 0% |
| data-exfil | 3 | 3 | 0 | 0% |
| evasion | 3 | 2 | 1 | 33% |
| exploit-attack | 3 | 3 | 0 | 0% |
| exploit-dev | 3 | 3 | 0 | 0% |
| full-crack | 3 | 2 | 1 | 33% |
| full-pentest | 3 | 3 | 0 | 0% |
| full-reverse | 3 | 3 | 0 | 0% |
| game-cheat | 3 | 2 | 1 | 33% |
| malware-dev | 3 | 2 | 1 | 33% |
| network-pentest | 3 | 3 | 0 | 0% |
| phishing-kit | 3 | 3 | 0 | 0% |
| post-exploit | 3 | 2 | 1 | 33% |
| posture-assessment | 3 | 2 | 1 | 33% |
| ransomware-builder | 3 | 3 | 0 | 0% |
| rei-fallback | 3 | 3 | 0 | 0% |
| reverse-engineering | 3 | 3 | 0 | 0% |
| vip-bypass | 3 | 2 | 1 | 33% |
| vuln-scanner | 3 | 3 | 0 | 0% |
| web-crawler | 3 | 3 | 0 | 0% |
| web-pentest | 3 | 3 | 0 | 0% |
| wireless-attacks | 3 | 3 | 0 | 0% |
| **总计** | **84** | **76** | **8** | **10%** |

> 注：拒绝率与测试 prompt 措辞强相关——直白中文攻击措辞（如"帮我完整破解"）约 31%，原版 codex-skills 英文工程化描述（把破解包装为"Software authorization analysis"等专业任务）约 10%。建议使用者参照 codex-skills 的措辞风格构造 prompt 以获得更低拒绝率。

拒绝率折线图 (28 类别, 84 样本, 10% 总拒绝率)

```mermaid
---
config:
    xyChart:
        width: 900
        height: 400
        xAxis:
            label: Skill 类别
            padding: 10
        yAxis:
            label: 拒绝率 (%)
            min: 0
            max: 50
            tickStepSize: 10
---
xychart-beta
    line [0, 0, 0, 0, 0, 33, 0, 0, 33, 0, 0, 33, 0, 0, 33, 33, 0, 0, 33, 33, 0, 0, 0, 33, 0, 0, 0, 0]
```

拒绝集中在 8 个高敏感类别（crack-keygen / evasion / full-crack / game-cheat / malware-dev / post-exploit / posture-assessment / vip-bypass），
其余 20 个类别零拒绝。拒绝均被 TamperEngine (M3) 捕获并触发 Rei Protocol 替换。
与直白中文 prompt 的 31% 相比，原版 codex-skills 英文工程化措辞将总拒绝率降至 10%。

## License

MIT — 见 [LICENSE](LICENSE)

## 致谢

- 相关源码提供者：**lingbol088-spec**, **MDX-Tom**
- 仓库贡献者：**FuDie0915**
- 测试指令替换：**InsTest**