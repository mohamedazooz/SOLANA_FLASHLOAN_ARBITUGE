# PR #7 + PR #8 Integration Baseline (2026-11-13)

Scope: unified integration audit of PR #7 (Unified Control Plane + Web
Dashboard) and PR #8 (Remediation R2/R3/R9/R10/R12 + execution pipeline).
**PR #9 is out of scope** and verified absent from both branches.

## 1. Exact SHAs and branch graph

| Ref | SHA | Relation |
|---|---|---|
| `origin/main` | `408c7b6a6c08b1217f82138d7477eff3debbd6ce` | integration target |
| `origin/feat/unified-pipeline-webapp` (PR #7 head) | `d3490ddc89e3cb2f2930d31bce29c515180cab54` | 9 commits ahead of main |
| `origin/feat/remediation-r2-r12` (PR #8 head) | `e2c965f36c3f2543dce85c41b361158706420d35` | 19 commits ahead of main |
| merge-base(main, PR #7) | `408c7b6` (= main) | PR #7 branches cleanly off main |
| merge-base(PR #7, PR #8) | `ded8e8c2a41983e412d4ecf3ca37c02c34ffd386` | PR #8 branched from the PR #7 branch **before** its final commit |

```
main 408c7b6 ──┬── a7533ff … ded8e8c (PR #7, 8 commits) ── d3490dd (PR #7 head)
               │                │
               │                └─ (PR #8 continues) 07c9306 … e2c965f (11 commits)
               │
               └─────────────── PR #8 diff vs main = PR #7's 8 commits + its own 11
```

## 2. Commit relationship — the one divergence

PR #8 was **branched from the PR #7 branch** at `ded8e8c`. PR #7's final
commit `d3490dd` ("serialize WEB_ADMIN_PASSWORD access with async
ENV_LOCK") landed on PR #7 **after** PR #8 forked.

- PR #7-only content: exactly **1 commit** (`d3490dd`), touching only
  `crates/arb-bot/src/web_api.rs` (+ENV_LOCK around 2 tests).
- PR #8 **independently supersedes** it: PR #8's `web_api.rs` contains 4
  ENV_LOCK usages — the same two tests plus the new ws-ticket test
  (added in `116d272`).
- Dry-run merge of PR #7 into PR #8's head: **clean auto-merge, zero
  conflicts** (`git merge --no-commit --no-ff` exit 0, only
  `web_api.rs` auto-merged, result identical in semantics to PR #8's
  version).
- No duplicated commits (different SHAs, but the only overlapping change
  is semantically identical and idempotent), no conflicting
  implementations, no accidental reintroductions detected.

## 3. Correct integration sequence

1. **Merge PR #7 first** (`d3490dd` → main). Human merge via GitHub UI —
   the automation token cannot merge.
2. **Then merge PR #8** (`e2c965f` → main). Because PR #8's history
   contains `ded8e8c`, GitHub will compute a minimal diff (its 11 own
   commits) and the `web_api.rs` merge resolves to PR #8's superset
   version (verified by the dry-run above).

Merging in the reverse order (PR #8 first) would also work content-wise
(PR #8 ⊇ PR #7's changes), but then PR #7 would show a near-empty diff
and its review record would be lost — sequence above preserves both
review trails.

## 4. PR #9 exclusion — verified

- `git merge-base --is-ancestor origin/feat/differential-quote-fixtures`
  against **both** PR #7 and PR #8 heads: **not an ancestor** of either.
- No file from PR #9 (`crates/arb-dex/tests/differential_fixtures.rs`,
  `fixtures/differential/`, `crates/arb-store/migrations/000[12]_*.sql`)
  exists on either branch.
- No cherry-picked PR #9 commits in either branch's log.

## 5. CI state at baseline

| PR | Check | Result |
|---|---|---|
| #7 | Format, check, test, clippy | **pass** (run 43344444628, 1m52s) |
| #7 | Web app build (React + Vite) | **pass** (25s) |
| #8 | Format, check, test, clippy | **pass** (run 43350685468, 1m47s) |
| #8 | Web app build (React + Vite) | **pass** (14s) |

## 6. Initial build/test state (local, this branch = PR #8 head)

- `cargo fmt --all -- --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — 0 warnings.
- `cargo test -q --workspace --all-targets` (CI-equivalent, no `.env`) —
  **951 passed / 0 failed**.
- `web`: `tsc --noEmit` + `npm run build` — clean.

## 7. Dependency relationship between the two PRs

- PR #8 **builds on** PR #7's control plane: `ControlService`,
  `web_api.rs`, `telegram_control.rs`, `web/` React app all originate in
  PR #7 and are hardened in PR #8 (single-use WS tickets, right-most XFF,
  body limit, security headers, fsync'd audit log).
- No file is owned by "both PRs with different logic": the only
  double-touched file (`web_api.rs`) converges as shown in §2.
- PR #8's execution pipeline (supervisor split into strategy/opportunity/
  recovery/dispatch modules, dispatch intents, recovery) is independent
  of PR #7's control plane except through the two intended coupling
  points: `ControlService` (mode/kill-switch/emergency state) and
  `WebState` (event fan-out) — both consumed, neither duplicated.

## 8. Known risks at baseline

| Risk | Severity | Note |
|---|---|---|
| Blockhash expiry classified by fixed slot arithmetic (`BLOCKHASH_VALIDITY_SLOTS = 160`) instead of persisted `lastValidBlockHeight` | HIGH (recovery correctness) | Remediated in this integration round — see PR8_EXECUTION_RECOVERY_AUDIT.md |
| Merge order must be PR #7 → PR #8 | INFO | Otherwise PR #7's review trail collapses into PR #8 |
| PR #7's `?token=` WS auth (superseded in PR #8 by single-use tickets) | — | Resolved by merge order; PR #8 wins |
| ≥24 h shadow soak not yet run | HIGH (readiness, not code) | Gates Shadow → Canary; see LIVE_READINESS_CHECKLIST |
