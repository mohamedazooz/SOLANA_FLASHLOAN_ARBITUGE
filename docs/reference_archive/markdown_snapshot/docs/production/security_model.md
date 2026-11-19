# Application Security Model & Key Management

## 1. Threat Model & Key Security
- **Hot-Wallet Key Material**: Never stored in plain text in config files. The `KeypairSigner` debug formatting explicitly outputs `"[REDACTED_SECURE]"` and never leaks raw secret keys into logs or memory dumps.
- **Canary Balance Partitioning**: Hot wallet balance is strictly segregated from treasury funds. For Canary execution, the hot wallet is limited to a maximum balance of \$5-\$10 in SOL/USDC.
- **Fail-Closed Secrets Resolution**: Missing environment secrets (`HELIUS_API_KEY`, `WEB_ADMIN_PASSWORD`) fail at boot before any network listener opens.

## 2. Web API & Webhook Hardening
- **Localhost Binding**: Binds strictly to `127.0.0.1:8080`. External access requires a hardened reverse proxy (Caddy or Nginx) terminating TLS.
- **Session Authentication**: Single-operator admin authentication with constant-time password verification.
- **Single-Use WebSocket Tickets**: Short-lived (60s TTL) one-time tickets protect WebSocket subscriptions without leaking session tokens in URL query strings.
- **Strict Rate Limiting**: Per-IP fixed-window limiter enforces 10 logins/min and 60 mutations/min.
- **CORS Hardening**: Strict origin allowlists; wildcard `*` origins are rejected in production.

## 3. Supply Chain Security & Dependency Policy
- Pinned toolchain: Rust 1.99.1, Edition 2024.
- All workspace crates compile with `-D warnings` (zero warnings policy).
- All dependencies reviewed for sound unsafe blocks, memory safety, and provenance.
