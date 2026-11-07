# syntax=docker/dockerfile:1

# ============================================================
# Stage 1: Builder
#   Compile only the binaries used by the production services. rust:1.85
#   supports edition 2024 used by the workspace.
# ============================================================
FROM rust:1.85-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

# --locked honours Cargo.lock (reproducible).
RUN cargo build --release --locked -p arb-bot --bin arb-bot --bin alchemy-webhook

# ============================================================
# Stage 2: Runtime
#   Minimal Debian image — only ca-certificates for TLS.
# ============================================================
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -u 1000 arb

WORKDIR /app

COPY --from=builder /app/target/release/arb-bot /usr/local/bin/arb-bot
COPY --from=builder /app/target/release/alchemy-webhook /usr/local/bin/alchemy-webhook
COPY --from=builder /app/config /app/config

# Runtime state (sqlite fallback, landing ledger) and logs live under /app/data.
RUN mkdir -p /app/data && chown -R arb:arb /app

ENV RUST_LOG=info
USER arb

# Liveness: the Prometheus metrics endpoint must answer.
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -fsS http://127.0.0.1:9100/metrics >/dev/null || exit 1

ENTRYPOINT ["/usr/local/bin/arb-bot"]
CMD ["--config", "/app/config/config.toml", "--mode", "shadow"]
