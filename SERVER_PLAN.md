# Super-Instruct 服务器化（Docker 部署）— 实施方案与落地记录

> 目标：把当前 Tauri 桌面应用改造成可 Docker 部署在服务器上的**无头 OpenAI 兼容代理网关**，
> 入站可被 Codex CLI 或第三方中转站调用，出站可转发到中转站 / OpenAI 官方（支持按 model 动态路由）。

## 一、现状与可行性（结论）

- **核心破甲管道 `core/`（MitmCore）是纯 Rust（axum+reqwest），零 Tauri 依赖** —— 这是改造成功的关键。
- 只有 `extensions/monitor.rs`、`lib.rs`（窗口/命令）、`deploy.rs`（改写本机 Codex 配置）、`skills.rs` 依赖 Tauri。
- Tauri 在无头容器需 GTK/WebKit/显示器，**故抽离出独立无头 server crate，复用 core + 破甲扩展**。
- **已落地，非仅方案**：`server/` crate 已实现并本地验证通过。

## 二、目标架构（已实现，真实链路）

```
                    ┌────────────────────────── 容器 ──────────────────────────┐
 client ──────────▶│ relay.example.com/v1  (入口中转站, 配置了把请求转发到本服务)│
    (带自身 key)    │      │  入口中转站转发时在 Authorization 头带出站 key       │
                    │      ▼                                                    │
                    │  super-instruct-server (无头 axum, 0.0.0.0:8080)          │
                    │     ● M1 注入 bridge.md (替换 instructions/system)        │
                    │     ● 透传入站 Authorization (即出站 key)                  │
                    │     ● M4 解析 → M3 篡改 → M5 记忆 → M6 统计                │
                    └──────────┬────────────────────────────────────────────────┘
                               ▼
                    https://api.example.com/v1  (最终执行上游)
```

- **入站**：入口中转站按配置把请求转发到本服务 URL。
- **出站**：本服务改造后转发到 `api.example.com/v1`，**透传**入站 Authorization（出站 key）。
- **本服务的 URL** 配在入口中转站里；**出站上游的 URL** 配在本服务（`UPSTREAM_BASE_URL` / `X-Upstream-Base` 头）。
- 出站 URL 优先级：`X-Upstream-Base` 请求头 > 配置默认上游。

## 三、落地文件

```
server/
├── Cargo.toml            # 独立 binary crate super-instruct-server
├── mock-upstream.js      # 测试用 mock 上游（node），仅测试用
└── src/
    ├── main.rs           # 入口：组装 MitmCore+扩展+路由，启动 axum
    ├── config.rs         # env 配置解析（LISTEN_ADDR/UPSTREAMS/...）
    ├── router.rs         # 动态上游路由解析器（按 model 前缀匹配）
    ├── proxy.rs          # axum 反向代理 handler（SSE 流式 + tamper 包装）
    ├── lib.rs            # 核心库：导出 core + extensions
    ├── core/             # 核心管道（纯 Rust，加了动态上游/去重，源自早期桌面版）
    └── extensions/       # 平移 + monitor 去 Tauri 化
Dockerfile                # 多阶段构建（只编 server，不含 Tauri）
docker-compose.yml        # 容器编排
.dockerignore
```

> 核心管道源自早期桌面版：`core/`（管道）与 `extensions/inject.rs`、`sse_parser.rs`、`tamper.rs`、`memory.rs` 原样移植（桌面版源码已删除，本仓库仅保留服务端）；
> `extensions/monitor.rs` 去掉了 `AppHandle`/`Emitter`，改为日志 + 内存统计。

## 四、配置（全部环境变量）

| 变量 | 默认 | 说明 |
|---|---|---|
| `LISTEN_ADDR` | `0.0.0.0:8080` | 监听地址 |
| `UPSTREAM_BASE_URL` | `https://api.example.com/v1` | **出站（默认）上游**，本服务改造后转发到此 |
| `UPSTREAMS` | — | 多上游按 model 前缀路由（可选），`key=base_url;...` |
| `UPSTREAM_API_KEY` | (透传入站) | 仅当入站没有 Authorization 时兜底注入的出站 key |
| ~~`TRANSFORM_MODELS`~~ | (已废弃) | ~~允许改造的模型白名单~~ 门控已删除：**所有模型均注入 bridge.md + 篡改** |
| `BRIDGE_MD_PATH` | (嵌入 fallback) | bridge.md 路径，可挂 volume 热更新 |
| `MEMORY_PATH` | `memory.json` | 记忆持久化文件 |
| `LOG_DIR` | — | 设置则启用文件日志（滚动） |

**出站 URL 优先级**：请求头 `X-Upstream-Base` > model 匹配的 `UPSTREAMS` > `UPSTREAM_BASE_URL`。
**出站 key 策略（默认透传）**：
1. 请求头 `X-Upstream-Key` / model 匹配条目显式指定 → 覆盖；
2. 入站已有 `Authorization`（入口中转站带出站 key）→ 原样透传；
3. 入站无 `Authorization` 且配了 `UPSTREAM_API_KEY` → 用配置兜底。

## 五、Proxy 接口

- `GET /`、`GET /health` → 健康检查 `Super-Instruct OK`（**免认证**，供 Docker healthcheck / LB 探活）
- `GET /stats`、`GET /history` → 统计 / 交互历史（无内置认证；暴露公网请用反代加认证）
- `GET /v1/models` 与任意 `POST /*`（`/v1/chat/completions`、`/v1/responses` 等）→ 透传到匹配上游，走破甲管道
- SSE 流式透传（keepalive + tamper 的 Responses API SSE 包装）保持原逻辑

> 路径拼接自动去重：上游基址含 `/v1` 且路径也以 `/v1` 开头时只保留一次，兼容两种写法。

## 六、已验证结果（本地端到端）

- ✅ `cargo build` 通过（server binary 无 Tauri 依赖）
- ✅ 健康检查免认证返回 `Super-Instruct OK`
- ✅ `GET /stats` 未认证返回统计 JSON（内置认证已移除，暴露公网需反代加认证）
- ✅ 动态路由：`POST /v1/responses` 带 `model=mock-sol` → 路由到 mock 上游（URL 正确、`/v1` 去重生效）
- ✅ bridge.md 注入生效（mock 收到请求体内含注入标志 `injected=true`）
- ✅ 出站 key 覆盖：`UPSTREAM_API_KEY` 注入为 `Authorization: Bearer sk-upstream-key`
- ✅ SSE 流式响应完整透传

## 七、部署使用

```bash
# 1) 启动（镜像由 GitHub Actions 多架构构建发布到 GHCR,直接拉取）
docker compose up -d

# 2) 健康检查
curl http://<服务器IP>:8080/health   # Super-Instruct OK

# 3) 客户端（Codex）接入：base_url 指向服务器，任意 API key 占位即可（服务端不校验）
codex --config '{
  model_provider: openai,
  model: "gpt-5.6",
  base_url: "http://<服务器IP>:8080/v1",
  auth: { env_key: "OPENAI_API_KEY" }   # 值随意，服务端仅透传
}'

# 中转站聚合：把 http://<服务器IP>:8080/v1 加入其上游列表即可
```

`.env` 示例：
```
UPSTREAMS=openai=https://api.openai.com/v1;relay=https://my-relay.example.com/v1
UPSTREAM_API_KEY=sk-your-upstream-key
LISTEN_ADDR=0.0.0.0:8080
```

## 八、生产注意

1. **安全**：服务端无内置认证（纯改造转发），暴露公网必须用反向代理（nginx/caddy）加认证 + TLS。
2. **上游可达性**：容器需能访问上游（出网）与外部访问入站（入网 8080）。
3. **bridge.md 热更**：挂载 `./data:/data`，写入 `data/bridge.md` 即时生效（`BRIDGE_MD_PATH=/data/bridge.md`）。
4. **多实例**：横向扩展时 memory.json 建议用共享 volume 或外部 KV（当前单实例写文件）。
5. **服务端**：本仓库仅保留 `server/` 无头版（桌面版 `src-tauri/` 已删除），`server` 可独立构建部署；两者此前共用同一套核心。

## 九、后续增强（可选）
- 多用户 key / 配额管理
- Web 监控页（只读 Dashboard，复用 `/stats` `/history`）
- 上游故障转移 / 负载均衡
- TLS 终结内置
