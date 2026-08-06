# super-instruct-server — 无头代理网关
# 多阶段构建：只编 server binary（不编 Tauri），运行镜像无 WebKit/GTK 依赖

# ---- build 阶段 ----
FROM rust:1.95-bookworm AS builder
WORKDIR /app

# 只复制 server 相关，避免 COPY 整个 target/
COPY server/Cargo.toml ./server/Cargo.toml
# Cargo.lock 锁定依赖版本,保证 Docker 构建可复现(与本地/CI 一致)
COPY server/Cargo.lock ./server/Cargo.lock
COPY server/src ./server/src
COPY bridge.md ./bridge.md

# 预拉依赖（利用缓存）
RUN cd server && cargo fetch 2>/dev/null || true

RUN cd server && cargo build --release

# ---- run 阶段 ----
FROM debian:bookworm-slim
# CA 证书：reqwest 访问 https 上游必需
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/server/target/release/super-instruct-server /usr/local/bin/server

RUN mkdir -p /data
ENV LISTEN_ADDR=0.0.0.0:8080
ENV RUST_LOG=info

EXPOSE 8080
VOLUME ["/data"]

# 默认无上游则拒绝启动，见 config.rs
ENTRYPOINT ["/usr/local/bin/server"]
