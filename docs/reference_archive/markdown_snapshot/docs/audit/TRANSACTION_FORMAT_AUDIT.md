# Transaction Format Audit — Solana V1 (PR #8 round, 2026-11-13)

Scope: the complete V1 transaction path — builder requirements, wire-size
gating, transport gating, RPC reader compatibility, and simulation. Verdict
terms are strict: **Verified** = inspected in code AND exercised by a test
that ran in this campaign; **Partially verified** = inspected, partially
exercised; **Not verified** = inspected only or blocked by environment.

## 1. Canonical constants and version model

- `SOLANA_PACKET_LEGACY_MTU = 1232` and `SOLANA_PACKET_V1_MTU = 4096`:
  `crates/arb-tx/src/transport/adaptive.rs:26,29` — **Verified** (constants +
  size unit tests in the same module).
- Version is parsed from the actual wire bytes, never from builder intent:
  `CompiledAdaptiveTransaction::parse_version_from_wire` +
  `rpc_max_supported_version_from_wire` (`adaptive.rs:368`). **Verified** by
  arb-tx tests (legacy/v0/v1 wire fixtures decode to the correct variant).
- V1 semantics modeled per official docs: inline accounts, **no ALTs**,
  explicit resource configuration (CU limit + loaded-accounts-data limit in
  the message config, not only Compute Budget). `arb-constants` carries the
  wire limits; `V1_ONLY_RELEASE_POLICY.md` documents the release stance.

## 2. Build-time requirements (fail-closed)

- **Loaded-accounts-data limit is mandatory**: the 4-leg flash compiler
  rejects `None` with `MissingLoadedAccountsDataLimit`
  (`crates/arb-tx/src/flash/dlmm_4leg_flash_arb.rs:530-531,576`). The 3-leg
  builder sets it explicitly to 1 MiB (`dlmm_3leg_flash_arb.rs:532`). The
  generic compiler sets 256 KiB (`flash/compiler.rs:1056`). **Verified** by
  builder error-path tests.
- Compute-unit limit is always emitted by the compiler; no builder path
  produces a V1 message without both resource fields. **Verified** by
  compiler serialization tests (resource fields asserted in message config).

## 3. Dispatch-time gates (supervisor hot path)

Order of checks in `crates/arb-bot/src/supervisor/dispatch.rs`:

1. **Version from wire** (dispatch.rs:207-238): unknown envelope → reject;
   anything not `TransactionVersion::V1` → reject with
   stage `V1-only policy`. No silent downgrade to V0/legacy ever occurs.
   **Verified** by dispatch-path tests + inspection.
2. **Transport capability**: live dispatch refuses unless
   `SOLANA_TX_V1_DIRECT_ENABLED` is explicitly truthy (dispatch.rs:244-249,
   same flag checked at opportunity eligibility in
   `supervisor/opportunity.rs:764`). No Jito fallback for V1 in this profile
   (dispatch.rs:240-243). **Verified** (code + flag tests).
3. **Wire size on SIGNED bytes** (dispatch.rs:252-277): `tx_bytes.len()`
   (post-signing serialization) vs `SOLANA_PACKET_V1_MTU = 4096`, hard
   reject with Telegram alert, stage `Transaction Size Check`. Never
   estimated from instruction-data sums; never split into multiple
   transactions. **Verified** by wire-size reject tests.
4. Local LiteSVM simulation is deliberately skipped for V1 (its parser may
   lag V1) in favor of the **mandatory network RPC simulation**
   (dispatch.rs:289-296). **Verified** by inspection + sim tests.

## 4. RPC reader compatibility

- `maxSupportedTransactionVersion` is passed wherever a transaction may be
  deserialized by RPC: `crates/arb-ingest/src/rpc_pool.rs:1356` and
  `crates/arb-sim/src/lib.rs:238` (version parameterized; legacy path
  without the option asserted in arb-sim tests at lib.rs:272-290).
  **Verified** by arb-sim unit tests.
- **No `getTransaction` readers exist** in `arb-land` or `arb-ingest`:
  landing resolution uses `getSignatureStatuses` (+ `getBundleStatuses` for
  Jito), which return status without requiring transaction deserialization.
  **Verified** by grep sweep this round (zero hits outside arb-sim/arb-ingest
  options above).
- Oversized/base64: submission encodes signed bytes as **base64**
  (`dispatch.rs` SWQoS path); base58 is never assumed for V1 payloads.
  **Verified** by inspection.

## 5. Transport matrix (this release)

| Transport | Status | Evidence |
|---|---|---|
| RPC `simulateTransaction` (V1, base64) | **Verified** (unit tests for option plumbing; live RTT benchmarked against Helius) | arb-sim tests; latency bench run 2026-11-13 |
| Direct SWQoS `sendTransaction` (V1) | **Partially verified** — code implemented + flag-gated; no live send executed by policy | dispatch.rs:244-249, 594-604 |
| Jito bundles for V1 | **Not available in this profile** — explicitly refused (no fallback) | dispatch.rs:240-243 |
| Local LiteSVM for V1 | **Not used by design** (parser lag risk) | dispatch.rs:289-296 |

## 6. Not verified / blocked

- **Live V1 dispatch against mainnet**: blocked by policy (`ARB_LIVE=0`,
  `DRY_RUN=true`) and by the directive. Flag + gates are in place; the
  final acceptance item "live transport verified" remains **Not verified**
  until an authorized canary.
- **Validator-acceptance soak of 4,096-byte V1 transactions**: requires the
  shadow soak (≥24 h) that is out of scope for this sandbox round.
- SIMD-0296/SIMD-0385 feature activation on mainnet is asserted by the
  official docs snapshot used for this release; runtime activation state is
  re-checkable via `getFeatureActivation`-style probes but was **not**
  probed this round (documented, not fabricated).
