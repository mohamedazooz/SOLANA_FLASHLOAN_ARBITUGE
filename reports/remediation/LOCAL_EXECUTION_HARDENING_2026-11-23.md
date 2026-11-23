# Local Execution Hardening Report — 2026-11-23

## Scope and safety boundary

- Repository: `mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE`
- Local branch: `fix/local-economic-execution-hardening`
- Base: `origin/main` at `b4700b989802e521a95e4838c0da78112d6fafce`
- Changes were made in the existing Windows working directory after creating a separate backup:
  `C:\Users\azoozz\Downloads\Compressed\SOLANA_FLASHLOAN_ARBITUGE-pre-fix-backup-20261123`
- The local `.env` file was used only for read-only RPC requests. Its contents, API keys, RPC URLs and wallet secrets were not printed or copied into this report. `.env` remains ignored and untracked.
- No private key was loaded for signing. No transaction was signed, simulated with wallet credentials, submitted or broadcast. No program was deployed or upgraded. No live-mode flag was enabled. Nothing was committed or pushed.

## Implemented changes

### 1. Complete WSOL transaction-cost floor

The canonical flash transaction builders now compute the final-hop minimum output with checked integer arithmetic:

`borrow principal + Kamino flash-loan fee + base transaction fee + priority fee + Jito tip + retained minimum profit`.

The priority fee is computed using the same effective CU limit that is encoded into the transaction. This applies to the generic atomic bundle, generic flash-arbitrage builder and non-flash arbitrage builder. The active flash path is WSOL-anchored, so lamports and WSOL raw units have a 1:1 conversion.

A cost-aware helper, `derive_swap_legs_with_costs`, owns this calculation. The legacy helper remains as a compatibility wrapper with zero network fee; production assemblers call the cost-aware helper.

### 2. Removed an undeployed program dependency from the active generic route

The standalone `profit_guard` instruction was removed from the active generic flash assemblers. Read-only `getAccountInfo` calls returned `null` for the configured executor placeholder:

`ArbExec1111111111111111111111111111111111111`

This result was observed through the configured RPC aliases SOLANA_RPC_PRIMARY, HELIUS_RPC_URL, SOLAMI_RPC_URL, SHYFT_RPC_URL and NODIT_SOLANA_RPC. The former ID in `programs/arb-executor/Anchor.toml` was malformed; the five Anchor cluster entries were made consistent with the Rust placeholder. This is configuration consistency only, not a real deployment.

The active generic route now relies on the final swap's cost-complete `min_amount_out` rather than an instruction to a missing custom program. The separate `arb-executor` CPI feature remains inactive and must not be activated until a genuine program key, deployed executable, binary identity and upgrade authority are verified.

### 3. Single CU safety buffer

The Supervisor already passes an effective CU limit after its safety buffer. The generic builders previously applied another dynamic buffer, potentially charging a larger priority fee than the economic gate evaluated. They now encode the supplied effective CU limit verbatim, and fail closed for zero or out-of-range values.

### 4. Explicit retained-profit policy input

`KaminoBundleParams` now carries `min_profit_raw` as an explicit policy input. `build_kamino_bundle` no longer substitutes `opportunity.net_profit` for the minimum retained-profit hurdle. Projected net profit and the operator's minimum-profit requirement are different values; conflating them could double-count costs. Regression coverage sets an intentionally high projected net profit while retaining a much lower explicit policy floor.

### 5. Three-leg candidate ranking includes all known costs

The three-leg Top-K size/route pipeline now ranks candidates after subtracting:
- Kamino flash fee;
- base transaction fee;
- candidate-specific priority fee, using the same profit-share cap and ceiling calculation as dispatch;
- the discovery Jito tip estimate.

Overflow fails closed. Final dispatch still rechecks the actual planned tip and current execution costs.

### 6. Tip discovery estimate

Discovery now uses a fresh landed-tip p50 when available, bounded by the configured maximum. If the oracle snapshot is unavailable, the conservative configured maximum remains in use rather than assuming a zero tip. Dispatch continues to recalculate and validate the actual planned bid.

### 7. Priority fee rounding in simulation

The Meteora fresh simulation pipeline now calls the canonical priority-fee helper, which uses ceiling division rather than floor division. This prevents fractional-lamport priority costs from being understated.

### 8. Split flash assembly fails closed

The legacy split flash assembler accepts prebuilt instructions and cannot prove that their aggregate return-hop minimum output covers every execution cost. It now returns an explicit error until that proof and supporting tests exist. The Supervisor already leaves split routing disabled; the builder itself now preserves that boundary.

## Verification results

All commands below were run locally with the process environment forced to shadow/dry-run:
`ARB_LIVE=0`, `DRY_RUN=true`, `ARB_MODE=shadow`.

| Check | Result |
|---|---|
| `cargo test -p arb-tx` | Passed; 166 unit tests and all enabled integration suites passed, 0 failures |
| `cargo test -p arb-engine` | Passed; all unit/integration suites passed, 0 failures |
| `cargo test -p arb-bot` | Passed; 95 unit tests, 6 binary tests, 8 cross-DEX tests and 5 shadow-boundary tests passed |
| `cargo test -p arb-sim` | Passed; enabled unit, LiteSVM, replay and non-broadcast simulation tests passed |
| `cargo check --workspace` | Passed; all workspace crates and the local executor program checked successfully |
| `cargo fmt --all -- --check` | Passed |
| `git diff --check` | Passed; Git emitted only Windows LF/CRLF normalization warnings |
| `.env` tracked status | Not tracked; ignored by `.gitignore` |

Explicit live-RPC diagnostic tests marked `ignored` in the test suite were not force-enabled. No Mainnet transaction was sent.

## Read-only RPC latency sample

The following are ten warm measured `getSlot` requests per provider (one warmup request excluded), issued sequentially from this laptop through the locally configured RPC credentials. These are end-to-end laptop request timings, not VPS measurements, not provider-side processing times, and not WebSocket subscription latency.

| Configured alias | Samples | Failures | P50 ms | P90 ms | Max ms | Average ms |
|---|---:|---:|---:|---:|---:|---:|
| SOLANA_RPC_PRIMARY | 10 | 0 | 555 | 687 | 790 | 585.4 |
| HELIUS_RPC_URL | 10 | 0 | 318 | 351 | 387 | 313.7 |
| SOLAMI_RPC_URL | 10 | 0 | 268 | 547 | 659 | 346.4 |
| SHYFT_RPC_URL | 10 | 0 | 383 | 486 | 522 | 400.9 |
| NODIT_SOLANA_RPC | 10 | 1 | 974 | 1,596 | 2,924 | 1,270.9 |

These small sequential samples are only diagnostic. They do not justify changing the production provider order or WebSocket failover policy. Measure again from the target VPS using the actual WebSocket stream, reconnect behavior, slot lag and sustained p50/p95 over a longer window before changing provider selection.

## Remaining production gates

1. The executor CPI program is still not deployed or verified. Do not activate that feature.
2. The local tests and workspace check do not prove a funded Mainnet trade will land or yield positive net PnL.
3. No 24-hour shadow soak was run as part of this work.
4. No VPS latency/failover benchmark was run.
5. No funded transaction, transaction landing or final settlement reconciliation was attempted.
6. Positive net profit cannot be guaranteed by a code change. The repairs make the economics/guards more consistent and reject costs that exceed the edge; they do not create arbitrage opportunities where the live market has none.

## Git state

All code and documentation edits remain uncommitted on `fix/local-economic-execution-hardening`. The `main` branch and GitHub remote were not changed. The local `.env` file remains ignored and untracked.
