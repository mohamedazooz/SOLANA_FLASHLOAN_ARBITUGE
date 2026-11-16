# syntax=docker/dockerfile:1

# ============================================================
# Stage 1: Builder
#   The workspace uses edition 2024 plus let-chains, and the
#   validated toolchain is Rust 1.99.1 (see rust-toolchain.toml).
#   Pin the same image here so local, CI, and release builds agree.
# ============================================================
FROM rust:1.99.1-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy manifests first so dependency compilation is cached independently of
# source edits. `--locked` makes the build fail on an out-of-date lockfile.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY config ./config

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked -p arb-bot --bin arb-bot --bin alchemy-webhook

# ============================================================
# Stage 2: Runtime
#   Minimal Debian image - only ca-certificates for TLS.
# ============================================================
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -u 1000 arb

WORKDIR /app

COPY --from=builder /app/target/release/arb-bot /usr/local/bin/arb-bot
COPY --from=builder /app/target/release/alchemy-webhook /usr/local/bin/alchemy-webhook

# The repository ships config/config.example.toml only; real deployments mount
# a populated config at /app/config/config.toml (compose does this read-only).
# Shipping the example as the default makes a misconfigured container fail with
# a clear "missing endpoint" error instead of "file not found", and never
# contains credentials.
COPY config/config.example.toml /app/config/config.toml

# Runtime state (sqlite fallback, landing ledger, kill-switch marker) and logs
# live under /app/data, which deployments should mount as a volume.
RUN mkdir -p /app/data && chown -R arb:arb /app

ENV RUST_LOG=info
USER arb

# Liveness: the Prometheus metrics endpoint must answer.
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -fsS http://127.0.0.1:9100/metrics >/dev/null || exit 1

STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/arb-bot"]
CMD ["--config", "/app/config/config.toml", "--mode", "shadow"]
