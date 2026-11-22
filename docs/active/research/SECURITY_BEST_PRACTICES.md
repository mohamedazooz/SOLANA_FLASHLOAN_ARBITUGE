# Security Best Practices Research

**S3 verification date**: 2026-11-22.

## Verified / current-practice facts (supply chain, 2026 consensus)

| Item | Finding | Source |
|---|---|---|
| Scanner stack | Run **cargo-audit** (hard CI gate) + **cargo-deny** (policy: advisories, bans, licenses) — together; cargo-geiger as a quarterly manual aid; artifact-based auditing via `cargo auditable` builds is the "artifact as source of truth" pattern | [Pi Stack comparison 2026-08](https://www.pistack.xyz/posts/2026-08-19-rust-supply-chain-security-cargo-audit-deny-geiger-comparison/) (S3) |
| SBOM | CycloneDX generated on every release build; committed `Cargo.lock` is mandatory for reproducible builds and accurate SBOMs | [SoftwareSeni enterprise guide](https://www.softwareseni.com/rust-supply-chain-security-managing-crates-io-risk-in-an-enterprise-codebase/) (S3) |
| Review chain | `cargo-vet` records named-auditor certification for crates; imports are non-transitive | [awesome-rust-security-guide SUP-7 / Microsoft Rust training](https://microsoft.github.io/RustTraining/engineering-book/ch06-dependency-management-and-supply-chain-security.html) (S3) |
| Gitleaks licensing | Go binary remains MIT; only the `gitleaks-action` v2.0.0+ wrapper relicensed (proprietary EULA) — repo uses the binary + `.gitleaks.toml` directly, which stays MIT | [codevetter tooling survey 2026-08-30](https://codevetter.com/docs/knowledge/tooling-secrets-and-supply-chain/) (S3) |
| Dependency additions | exact-name check (typosquat), crates.io Security tab, `cargo deny check advisories` failing on vulnerable/unmaintained; `unsound = "all"` for transitive unsoundness | [awesome-rust-security-guide DEP-1](https://github.com/iAnonymous3000/awesome-rust-security-guide) (S3) |

## Existing implementation

- `deny.toml` + `audit.toml` present; CI runs clippy `-D warnings`; `.gitleaks.toml` with fixture allowlists (public on-chain pubkeys only, hardened in PR #120 line); `Cargo.lock` committed; toolchain pinned; release profile uses `overflow-checks = true`, `panic = "abort"`.
- Secrets only in git-ignored `.env`; `.env.example` carries placeholders; web API fails closed without `WEB_ADMIN_PASSWORD` and binds localhost (AGENTS.md invariants).

## Recommendations (REC)

1. Ensure CI's `cargo audit`/`cargo deny` steps are **blocking** (not advisory) and pin the RustSec DB fetch date in logs for reproducibility.
2. On each release build, emit a CycloneDX SBOM (`cargo cyclonedx` or auditable) and attach to the workflow artifact — needed for the later SBOM/provenance phase.
3. Keep `cargo-vet` out until a second maintainer exists (single-operator cost > benefit); revisit at team scale.

## Tests required

- CI gate test: intentionally add a vulnerable crate version on a scratch branch ⇒ `cargo deny check advisories` fails (one-off verification, not a permanent fixture).
- Secret hygiene: gitleaks must flag `.env` copies moved into `docs/` (fixture probes exist — keep).

## 2026-11-22 refresh (Phase 9 online sweep — verified citations)

| Principle | Source (URL, date) | Repo state | Gap / action |
|---|---|---|---|
| Env vars are process-visible; prefer a managed secret provider in production, inject only needed values | [OWASP-context analysis, blog.siebly.io (2026)](https://blog.siebly.io/crypto-exchange-api-key-security-best-practices/) | Secrets stored as env-var NAMES in TOML; provider abstraction ("env for local dev, secret manager in production") in `config/base.toml:1–8` | Production secret-manager binding is a deployment-time operator choice — documented in KEY_MANAGEMENT |
| Event-driven rotation > arbitrary periodic rotation (NIST position, Jul 2025) | [fxnx.com (2026)](https://fxnx.com/en/blog/trading-api-security-how-protect-your-keys-tokens) | Admin tokens compared constant-time (`main.rs:370–372`); rotation manual runbook | No automated rotation tooling — acceptable for single-operator scale |
| Production backend signing via a KMS/keychain library (Memory/Vault/AWS-KMS backends behind one interface) | [solana.com/docs/tools/security (canonical)](https://solana.com/docs/tools/security) | Local keypair file (`ARB_KEYPAIR_PATH`) | KMS backend not implemented; fine for shadow/simulation, required before multi-operator prod |
| Malware targets bot keystores/`.env` on VPS (documented 2025–2026 campaigns) | [tradoxvps.com (Jan 2026)](https://tradoxvps.com/how-to-secure-a-trading-vps/) | gitleaks CI + `.env` gitignored; kill switch persisted | VPS runbook must keep key custody + non-root service hardening front-and-center |
| Jito: tips are an auction market (floor 1,000 lamports; tip in a bundle member; bundleOnly=true revert protection; acceptance ≠ landing; ~12–20% of stake does not run Jito); **BAM (Block Assembly Marketplace) live on mainnet since 2025-09-25 — proposer-builder separation** | [docs.jito.wtf lowlatencytxnsend](https://docs.jito.wtf/lowlatencytxnsend/), [allenhark.com jito-bundles (2026)](https://allenhark.com/jito-bundles), [rpcfast.com (2026)](https://rpcfast.com/blog/jito-explained-bundles-tips-mev-solana), [onfinality.io](https://onfinality.io/en/learn/solana-jito-bundle-tip-transaction-sending) | Repo implements: bundleOnly=true; 8 tip accounts (verif); tip oracle REST+WS 5 s freshness; EV bidder with refuse-above-ceiling; proven-expiry lifecycle | **NEW: BAM/PBS transition unassessed** — registered as JTO-F-2026-11-22-N4 (P3); landing probability ceiling ≈ Jito-enabled stake share (document in Jito docs); alternative relays exist but adding a second relay is not justified without measured bundle-rejection telemetry |
| Tip accounts / API surface can change without notice | [onfinality.io](https://onfinality.io/en/learn/solana-jito-bundle-tip-transaction-sending) | Repo hard-codes the 8 canonical accounts + live `getTipAccounts` verify + CI drift test vs on-chain constant | Keep the live verify step in the boot sequence; drift CI already guards |
