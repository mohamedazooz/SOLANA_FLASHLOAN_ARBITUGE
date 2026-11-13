# PHASE 9 — Security & Dependency Audit (supersedes 2026-11-13T04:54 version)

## Automated dependency analysis (this audit, real runs)

- `cargo audit` (v0.23.0, fresh install): **0 vulnerabilities**; 5 pre-allowed
  unmaintained-dependency warnings (ansi_term, bincode, derivative, libsecp256k1, paste)
  — all transitive, no fixed upgrade available, already accepted in repo config.
- `cargo-deny` / `cargo-machete`: NOT INSTALLED in environment — **NOT RUN**
  (blocker documented; recommended as CI addition).
- CodeQL / Semgrep: no workflow present — **NOT RUN**.

## Key management

- `shadow-only` feature: compile-time removal of the broadcast path + signer loading
  disabled (main.rs:318–329, dispatch.rs:77/93/162) — strong containment, verified.
- `ARB_LIVE` gating: requires explicit truthy value; refused twice (main.rs:349,
  control_service.rs:535); audit-logged (`mode_live_rejected`).
- Test `get_funded_signer` reads `SOLANA_PRIVATE_KEY` from env/.env — live-mainnet
  integration tests only; never broadcast (non-broadcast simulation). CI must never set
  that variable (currently does not).
- **Incident note (this audit)**: a keypair JSON was uploaded to the working chat by the
  operator. It was NOT read, NOT committed, NOT used. Recommendation: treat that key as
  exposed and rotate; never attach key material to chats or repos.

## Secret exposure scan

- Logs redact credentialed endpoints (`[REDACTED ALCHEMY MAINNET]` observed in test output).
- No secrets in repo tree; `.env` gitignored (verified).

## Arithmetic safety

- Token math: integer-only, U256 intermediates, checked ops, fail-closed errors
  (`DlmmError::MathOverflow`). No `saturating_sub` in profit/balance accounting paths
  (grep-verified; display contexts only).

## Account validation

- WS seed: owner-mismatch fail-closed (helius_ws.rs, commit `228c330`).
- Decoders: default-pubkey and length guards everywhere (probe-verified offsets).
- Execution accounts: derived from on-chain bytes, not catalog.

## Open security items

| # | Item | Severity |
|---|---|---|
| S-1 | Rotate uploaded keypair (operator action) | P0 (operator-side, outside repo) |
| S-2 | Add cargo-deny + cargo-machete to CI | P3 |
| S-3 | swap_hop `unwrap_or_default` zero-pubkey hazard (P1-1) | P1 |
