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
