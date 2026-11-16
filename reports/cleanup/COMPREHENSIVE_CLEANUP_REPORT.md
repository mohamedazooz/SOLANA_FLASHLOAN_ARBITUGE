# Comprehensive Cleanup & Organization Report — Session 7

> Scope: dead-code removal, hardcoded-value inventory + unification, docs deduplication,
> file-tree organization. Companion machine-readable artifact: `cleanup_summary.json`.

## الملخص التنفيذي (عربي)

| البند | النتيجة |
|---|---|
| دوال ميتة محذوفة | **68 دالة عامة + 2 خاصة** (~1,725 سطر) — تحقق مزدوج (عدّ مرجعي + `rg -w`) |
| التحقق | clippy `-D warnings` صفر أخطاء؛ **1032 اختبار ناجح / 0 فاشل** (نفس العدد قبل الحذف + اختبارا الانحدار الجديدان) |
| تعارض عناوين جديد | **HC-1 (P0)**: سوق OpenBook لـ SOL/USDC كان له قيمتان مختلفتان — القيمة المصنّعة في `builder.rs` صُححت للقيمة المتحقق منها أونلاين |
| قيم غير مربوطة تم توحيدها | USDT mint (7 مواضع)، OpenBook program (4)، Raydium authority (4)، 9 حسابات pool-scoped، CPMM AmmConfig (2) — الكل الآن في `arb-constants` |
| مستندات | حذف `audit_extract/` (28 ملف مكرر byte-identical مع `docs/archive/`) و`audit_work/` (10 مكرر)؛ `dashboard/` حي (Dockerfile) — أُبقي |
| قرار مشغل معلّق | 6 أنظمة أمان مبنية غير مربوطة (KEEP set) — ربطها مسؤولية المشغل، انظر §5 |

**ملاحظة أمان**: لم يُضعف أي تحكم fail-closed. لم تُحذف أي أعمال مستخدم فريدة (كل ما حُذف كان مكررًا byte-identical أو كودًا ميتًا مؤكدًا).

---

## 1. Dead-Code Matrix

Method: (1) extract every `pub fn`, count `\bname\b` references across all workspace
`.rs` (lib + tests + benches + examples); (2) zero-reference candidates double-verified
with `rg -w` across the entire repo (`.rs/.ts/.js/.toml/.sh`, excluding
`target/`/`node_modules/`), excluding the definition line; (3) trait methods and
`#[test]` fns filtered as false positives; (4) brace-matching remover strips the fn
plus its `///` docs and `#[...]` attributes; (5) fixpoint iteration until no new
zero-ref functions appear.

Result: **68 pub fns + 2 orphaned private helpers** removed across 36 files.
Evidence of death: workspace test count unchanged (1030 → 1030 before this session's
new tests) — zero test coverage touched any deleted function.

Full per-function list: see commit `a317a29` diff (36 files, +17/−1,725).

## 2. Hardcoded-Value Inventory & Classification

Scan (non-test code, all crates): 109 distinct base58 address literals, 53 URLs,
32 distinct Duration literals, 7 denominator kinds (10_000 ×299, 1_000_000 ×173,
1_000_000_000 ×77, 1e9 ×48, 1e6 ×29, 1_000 ×82, 1_000_000_000_000 ×25).

Classification:

| Class | Definition | Disposition |
|---|---|---|
| A. Protocol invariant | program IDs, discriminators, mints, pool accounts, PDA seeds, bin-array layout | centralized in `arb-constants` (single authoritative source) |
| B. Operational config | timeouts, retry counts, worker counts, loss limits | `config/*.toml` (typed `arb-config`); stragglers listed in §2.1 |
| C. Dynamic market | tips, priority fees, SOL price, fees | read live (tip_oracle, fee-market observation); no static proxy |
| D. Secret | keys, admin tokens, API keys | env/secret-provider only; never in source (verified: no secrets in scan) |
| E. Math constant | 1e9 lamports/SOL, 1e6 µlamports/lamport, 10_000 bps, FEE_PRECISION | inline by nature; documented in `arb-math` |
| F. Test fixture | addresses/durations inside `#[cfg(test)]`/`tests/` | excluded from scan; kept |

### 2.1 Resolved this session

**HC-1 (P0) — OpenBook SOL/USDC market conflict.**
- `crates/arb-tx/src/protocols/raydium_v4.rs`: `8BnEgHoWFysVcuFFX7QztDmzuH8r5ZFvyP3sYwn1XTh6` (VERIFIED — openserum.io market page; Solana StackExchange Q19480/Q11993 both reference it as the Raydium SOL/USDC OpenBook market bound to pool `58oQChx4…YQo2`)
- `crates/arb-tx/src/primitives/builder.rs:969`: `8BnEgHoWFysVcuFFX7QBpDmvu3xei5qGE9KitUfqWQH9` (FABRICATED — shares the real market's 20-char prefix, deviates after)
- Resolution: builder.rs now uses the verified constant; regression tests
  `raydium_v4_mainnet_sol_usdc_uses_verified_openbook_market` +
  `fabricated_openbook_market_absent_from_all_workspace_sources` added.

### 2.2 Unlinked values centralized (linking pass)

All of these were hardcoded inline in multiple files; each now has exactly one
authoritative definition in `arb-constants/src/lib.rs` and every site imports it:

| Constant | Former inline sites |
|---|---|
| `USDT_MINT` | opportunity.rs, flash/compiler.rs, master_alt.rs (local dup const), discovery.rs ×2, registry.rs |
| `OPENBOOK_PROGRAM_ID` | arb-dex/raydium_v4.rs, arb-tx/raydium_v4.rs, builder.rs, registry.rs |
| `RAYDIUM_V4_AUTHORITY` | raydium_v4.rs, builder.rs, latency_bench.rs, live_scanner.rs (existed in arb-constants but was not imported) |
| `RAYDIUM_SOL_USDC_OPENBOOK_MARKET` + OPEN_ORDERS + TARGET_ORDERS + SERUM_BIDS/ASKS/EVENT_QUEUE/COIN_VAULT/PC_VAULT/VAULT_SIGNER | raydium_v4.rs `mainnet_sol_usdc()`, builder.rs |
| `RAYDIUM_CPMM_SOL_USDC_AMM_CONFIG` | raydium_cpmm.rs ×2 (pool-scoped 0.25% fee tier; per Raydium docs fee tiers are on-chain AmmConfig accounts — rates must be read from the account, never assumed) |

Previously resolved conflicts (verified intact, guarded by `address_regression_guards.rs`):
`KAMINO_USDC_RESERVE` (fabricated `…KUvxCoXepEAbW39zWCuYAZBpqUg6xK25` rejected) and
`RAYDIUM_SOL_VAULT` (fabricated `…XYZujhgwgCRiaACVKAfdA3kh2v7iqWw5jz` rejected).

### 2.3 Remaining accepted hardcoded values (no action needed)

- 53 URLs: provider endpoints (Helius/Alchemy/Shyft/RPCfast/Solami/Jito/Telegram/
  Bitquery/Dexscreener/discovery APIs) — provider URLs are deployment configuration
  with env overrides in `arb-config`; the literals are only **fallback defaults**.
- 32 Duration literals: operational timeouts (blockhash TTL 3s, telegram poll 15s,
  provider health 2s, …) — mostly local to their subsystem; a full migration into
  `config/*.toml` is tracked as follow-up work, not a defect.
- `arb-ingest/src/registry.rs` bootstrap pool seeds: fixture-like bootstrap entries
  replaced by live discovery at startup; documented as such in the file.

## 3. Docs Organization (conflict resolution)

551 markdown files existed. Duplication audit (byte-level `diff -rq`):

| Path | Status | Action |
|---|---|---|
| `audit_extract/audit_reports/` (19) | byte-identical to `docs/archive/audit_reports/` | **deleted** (content preserved in docs/archive) |
| `audit_extract/audit_work/` (9+1) | byte-identical to `docs/archive/audit_work/` | **deleted** |
| `audit_work/` (untracked, 10) | byte-identical to the above | **deleted** (zero unique content) |
| `dashboard/` | referenced by `Dockerfile:22` (`COPY dashboard ./dashboard`) | **kept — LIVE** |
| `docs/audit/` (22) vs `docs/audits/` (8) | different generations of audit docs, not duplicates | kept; both indexed in `docs/README.md`; historical generation flagged below |
| `docs/archive/` | historical audit generations (audit2/3/4, audit_reports, financial-engine, math…) | kept as archive |
| `audit/` (math_oracle, test_vectors) | LIVE test fixtures used by differential tests | kept |

Stale-reference check: searched all `.md` for the 68 deleted function names —
only 3 hits, all in historical audit records (`audit_work/01_ARCHITECTURE_INVENTORY.md`
duplicates and `docs/archive/audit4.md`), which are point-in-time snapshots and
correctly historical. **No live doc references deleted code.**

Known doc-conflict class (flagged, not silent): status documents from different
campaigns (`docs/audit/*PR7/PR8*`, `docs/audits/PR7_PR8_*`, `docs/RELEASE_READINESS.md`)
carry gate verdicts against older baselines. Authoritative current status is
`docs/PROJECT_STATE.md` + `docs/quality/PRODUCTION_READINESS_2026-11-13.md`;
all others must be read as historical. This is now stated in `docs/README.md`.

## 4. Organized File Tree

Full annotated tree: [`docs/architecture/CODE_TREE.md`](../../docs/architecture/CODE_TREE.md)
(updated this session). Top level after cleanup:

```
SOLANA_FLASHLOAN_ARBITUGE/
├── crates/               13 crates (arb-types … arb-bot) + webhook-service
├── config/               base/development/staging/production/risk/providers/
│                         execution/protocols/webhook .toml + config.schema.json
├── docs/                 SINGLE documentation root
│   ├── adr/ architecture/ audit/ audits/ archive/ configuration/ economics/
│   ├── operations/ production/ quality/ reference/
│   └── ARCHITECTURE.md PROJECT_STATE.md IMPLEMENTATION_PROGRESS.md …
├── audit/                LIVE differential-test fixtures (math_oracle, test_vectors)
├── reports/              generated evidence (phase reports, cleanup/, production/)
├── dashboard/            LIVE (Dockerfile COPY)
├── scripts/ deploy/      operational tooling
└── SAFETY_AND_HARDCODED_AUDIT.md  root safety baseline
```

Deleted from top level: `audit_extract/`, `audit_work/` (duplicates — see §3).

## 5. KEEP Set — Unwired Subsystems (operator decision required)

Built, tested, but not called from the live path. Wiring any of these into the
execution path is execution-adjacent and requires explicit operator authorization
(قرار مشغل). **Not deleted, not silently wired.**

| Subsystem | Functions | Recommendation |
|---|---|---|
| `arb-risk` activation | `activate_from_signal`, `reset_losses` | wire into dispatch loss-recording path before canary |
| `jito_service` landing | `verify_landing`, `poll_until_terminal`, `ledger_json` | wire into landing tracker for realized-PnL reconciliation |
| `capability` | `validate_execution_prerequisites` | evaluate as a pre-dispatch gate addition |
| `dependency_manifest` | `from_registered_pool` | evaluate for pool account-coverage manifests |
| `dual_ingest` | `record_decode_error`, `record_resync`, `run_comparison_loop` | enable as ingest-integrity monitor |
| `rpc_reconcile` | `fetch_and_publish` | enable as RPC health reconciler |

## 6. Verification

| Check | Result |
|---|---|
| `cargo fmt --all` | applied, clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| `cargo test --workspace` | **1032 passed / 0 failed** |
| new regression tests | `raydium_v4_mainnet_sol_usdc_uses_verified_openbook_market`, `fabricated_openbook_market_absent_from_all_workspace_sources` — pass |
| stale-doc scan vs deleted fns | 0 live-doc hits |
