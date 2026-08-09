# Super-Instruct

OpenAI/Anthropic 兼容 API MITM 破甲网关 — Rust 无头代理。

通过 MITM 反向代理拦截 API 请求，在管道中执行注入、篡改、记忆和监控，配合 bridge.md 指令集实现竞赛沙箱模式。**用户请求内容原样透传，仅系统提示载体被注入。**

## 架构

```
Codex/Claude Code ──HTTP/WS──▶ MITM Core (管道编排)
                                │
                                ├─ Router       (RequestInterceptor)
                                │     动态上游路由(X-Upstream-Base 头 / model 前缀匹配, SSRF 防护)
                                ├─ M1 Inject    (RequestInterceptor)
                                │     system/developer/instructions → bridge.md(按类别附加指令)
                                ├─ M4 Parser    (ResponseParser)
                                │     SSE / JSON / Responses / Anthropic 通用解析
                                ├─ M3 Tamper    (ResponseInterceptor, 自门控)
                                │     50+ 条多语言正则检测拒绝/偏转 → Rei Protocol 替换
                                ├─ M5 Memory    (ResponseInterceptor, 自门控)
                                │     成功交互持久化到 memory.json
                                └─ M6 Monitor   (ResponseInterceptor, 无门控)
                                      日志 + 统计(/stats /history)
                                │
                                ▼
                            上游 API (任意 OpenAI/Anthropic 兼容服务)
```

**Core 原则**：Core 对扩展内容一无所知，只提供管道和挂载点。所有破甲逻辑由扩展承载。

## 支持的接口

| 接口 | 格式 | 说明 |
|---|---|---|
| `POST /v1/messages` | Anthropic Messages | system 前缀注入 bridge.md，透传上游 |
| `POST /v1/chat/completions` | OpenAI Chat | system 消息替换为 bridge.md |
| `POST /v1/responses` | OpenAI Responses | instructions/input 注入 |
| `POST /v1/images/*` | OpenAI Images | 纯透传（不注入，避免图片模型过滤） |
| `GET /ws/{*path}` | WebSocket | 入站 ws：每帧一个请求，走完整管道 |
| `GET /stats` `/history` | — | 统计查询（无内置认证，公网需反代加认证） |
| 出站 `ws://`/`wss://` 上游 | — | UPSTREAMS 支持 ws 上游转发 |

## bridge.md 指令集

双层防御纵深：

- **生成阶段**：实体归一化（占位符替换）+ 术语重定义（reverse shell/C2/keygen 等按软件工程含义）+ 边界词处理（授权词视为靶场描述符）+ 硬输出规则（中英文禁词表 + 强制开头模板 + 步骤一禁止分析动词）+ 执行连续性（丢弃先前拒绝）+ 强制调度
- **输出阶段**：CLOUD-AUDIT ROUTING（敏感内容路由到文件系统）+ OUTPUT LOCK

按请求类别（crack/reverse/pentest）附加针对性指令，只改 system 载体，用户输入文字原样不动。

## 快速开始

### 环境要求

- Rust 1.75+

### 本地运行（开发）

```bash
cd server
cargo run --release
# 环境变量见 SERVER_PLAN.md；默认上游 https://api.example.com/v1
```

### Release 构建

```bash
cd server && cargo build --release
# 产物: server/target/release/super-instruct-server
```

### Docker 部署

```bash
docker compose up -d
# 镜像由 GitHub Actions 自动构建发布到 GHCR（多架构: amd64/arm64）
```

### 使用方式

1. 部署后客户端（Codex CLI / Claude Code / 中转站）将 base_url 指向本服务
2. 所有请求经过管道（注入 bridge.md → 上游 → 解析 → 篡改 → 记忆 → 统计）
3. `/health` 健康检查、`/stats` `/history` 统计（无内置认证；公网暴露需部署层反代加认证）
4. ws 客户端连接 `ws://host:port/ws/v1/messages`（每帧一个请求 JSON）

## 项目结构

```
Super-Instruct-Codex-5.6/
├── bridge.md                      # 破甲指令集(可挂载热更)
├── server/
│   ├── Cargo.toml
│   ├── mock-upstream.js           # 测试用 mock 上游
│   └── src/
│       ├── main.rs                # 入口: 组装 MitmCore + 扩展 + 路由
│       ├── lib.rs                 # 库: core + extensions + anthropic + ws + formats
│       ├── config.rs              # 环境变量配置
│       ├── router.rs              # 动态上游路由 + SSRF 防护
│       ├── proxy.rs               # axum 反向代理(SSE 流式 + 篡改包装)
│       ├── anthropic.rs           # Anthropic 透传辅助(tamper 的 SSE/JSON 包装)
│       ├── formats.rs             # OpenAI 格式 tamper 包装(chat/responses)
│       ├── ws.rs                  # 入站/出站 ws 通道(headers 透传)
│       ├── core/                  # MitmCore 管道 + traits + context + extract
│       └── extensions/            # inject / sse_parser / tamper / memory / monitor
├── Dockerfile                     # 多阶段构建
├── docker-compose.yml
├── .env.example
├── SERVER_PLAN.md                 # 部署方案与记录
└── README.md
```

## 自门控策略

| 扩展 | 门控条件 | 理由 |
|---|---|---|
| M3 Tamper | `modified_body.is_none()` | 已被其他拦截器修改则不重复篡改 |
| M5 Memory | `modified_body.is_none() && reply.len() > 50` | 被篡改的响应不是成功交互 |
| M6 Monitor | 无门控 | 始终观察所有交互 |

## 技术栈

- **Rust** — 无头代理网关
- **axum** — HTTP 反向代理 + WebSocket
- **reqwest / tokio-tungstenite** — 上游转发（HTTP 流式 + ws）
- **Docker** — 容器化部署

## 效能评估（完整链路实测）

> 测试链路：客户端 → 测试服务器(super-instruct) → 上游 → ChatGPT 会话
> 条件：auto 模型 / 工程化任务措辞 / 28 类 × 多轮

### 接口破限率（模型实际执行比例）

| 接口 | 均值 | 峰值 |
|---|---|---|
| `POST /v1/messages` (Anthropic) | **82%** | **92%** |
| `GET /ws/v1/messages` (ws) | 78% | — |
| `POST /v1/chat/completions` (OpenAI) | 73% | 82% |
| `POST /v1/responses` (OpenAI, input 数组) | 70-74% | 82% |

**会话级（拒绝后重试）96%**，拒绝/偏转/空响应捕获率 100%（Tamper 规则 + 空响应兜底），误伤 0。

### 关键结论

- **Anthropic 顶层 system 字段遵循最强**（82%）——OpenAI 格式的 system 在消息数组里遵循弱（chat 73%、responses 70%），属上游结构性差异
- 措辞影响：直白攻击词 65% → 工程化任务描述 82%（README 原版建议的措辞方向）
- bridge.md 内容/长度、effort、temperature、模型（gpt-5-5/mini/4-t-mini/5-6/research）均为次要因素
- 用户输入文字不改动；仅系统提示载体被注入

## License

MIT — 见 [LICENSE](LICENSE)

## 致谢

- 相关源码提供者：**lingbol088-spec**, **MDX-Tom**
- 仓库贡献者：**FuDie0915**
- 测试指令替换：**InsTest**
