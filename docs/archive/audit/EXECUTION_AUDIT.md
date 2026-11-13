# Execution Audit

Status: source review of transaction construction, simulation, dispatch, and
landing. No live transaction was submitted. Runtime landing behavior is unproven.

## 1. Build path

`TxBuilder::assemble_flash_arbitrage` (and `assemble_split_flash_arbitrage`, per
protocol `build_swap_hop_instruction`):
- borrows the base mint from Kamino,
- emits one swap instruction per hop,
- repays the flash loan,
- adds compute-budget and tip instructions,
- optionally uses a v0 message with an address lookup table.

`build_swap_hop_instruction` supports Raydium v4, Raydium CPMM, Raydium CLMM,
Orca Whirlpool, and Meteora DLMM. Pump.fun and PumpSwap return
`TxError::UnsupportedProtocol` (fail closed). Jupiter and split routing are
disabled in the supervisor.

## 2. Kamino flash loan

- Borrow/repay instructions are built from the reserve and market state.
- The fee is `ceil(borrow * flash_loan_fee_sf / 2^64)`, read from
  `ReserveConfig.fees.flash_loan_fee_sf`. There is no hardcoded 5-bps fee on this
  path (`FIN-01` fixed).
- The decoder checks the klend discriminator, reserve length, mint, and active
  status. A wrong mint/discriminator/short account/inactive reserve is rejected.
- Repayment amount includes the fee. The transaction is atomic: borrow, swaps,
  and repay are in one transaction, so a failed swap reverts the borrow.

**Not proven:** the exact account list for every Kamino market and the
same-transaction profit-guard instruction semantics against a fork. This is
`14.1`/`14.3` open.

## 3. Min-out and profit floor

- The single-route builder computes min-out from the conservative output and
  includes principal, flash fee, DEX fees, network/priority fee, tip, and
  `min_net_profit_abs`.
- Slippage alone is not used for the final return hop.
- The split path is unreachable, so its previous first-hop reuse and
  slippage-only return hop are not live. The code remains and **must not** be
  re-enabled before per-leg re-quoting (`SPLIT-01` mitigated by disabling;
  capability gap remains).

## 4. Simulation

- `arb-sim` has an RPC simulator and an optional LiteSVM feature.
- No RPC URL is a hard error (`simulator_without_rpc_returns_error_instead_of_fake_success`).
- LiteSVM feature-off returns `success=false` with an explicit message; it does
  not fake success.
- In Live, any RPC error/timeout/`success=false` rejects. In shadow/simulate/
  canary, simulation is optional (`ENABLE_PREFLIGHT_SIMULATION=true`).
- A successful simulation is not landing proof and does not increment fills.

**Not proven:** coherent-bank simulation against the exact bundle bank across
all routes (`14.4` partial).

## 5. Dispatch and mode gate

- `supervisor::dispatch` matches on mode:
  - Canary: records a simulated edge, no submission.
  - Shadow/Simulate: records a simulated outcome, no submission.
  - Live: checks `live_execution_enabled()` before any network send.
- Live performs a 0.05 SOL reserve check and an optional cold-wallet sweep.
- MTU check rejects > 1232 bytes.
- Final guard re-checks stream connection, generation, current slot, and state
  age.
- A Jito bundle ID or SWQoS signature is recorded as accepted, not as a fill.

## 6. Landing and reconciliation

- `arb-land` submits bundles to multiple Jito regions, polls statuses, and tracks
  terminal states.
- `BundleTracker`/`poll_bundle_until_terminal` exist.
- The store records outcomes (`sent`, `simulated`, `canary`), but there is no
  durable, idempotent accepted→landed state machine and no tested ambiguous
  timeout recovery (`14.5` open). Until then, a crash after send can leave an
  untracked accepted bundle.

## 7. Findings status

| ID | Status | Evidence |
|---|---|---|
| GUARD-01 | **OPEN** | `quote_route_guarded`/`revalidate_before_dispatch` have no callers outside `arb-engine`; the supervisor still uses the direct quote path. |
| SPLIT-01 | **FIXED (disabled)** | `split = None`; the vulnerable branch is unreachable. Split capability is not implemented. |
| CANARY-01 | **FIXED** | Canary uses simulated metrics/outcome, not fills/realized PnL. |
| LIVE-03 | **FIXED** | `ARB_LIVE=0` no longer authorizes live; truthiness helper used at startup and dispatch. |
| FIN-01 | **FIXED** | Kamino fee from on-chain `flash_loan_fee_sf`. |
| Pump adapters | **DISABLED** | Builder returns `UnsupportedProtocol`; tests assert fail-closed. |

## 8. Residual execution risks

1. No fork simulation of the complete Kamino borrow → 2/3 hops → repay route.
2. No proof that the exact account metas match the on-chain program expectations
   for every protocol version.
3. No durable idempotent reconciliation; crash/timeout ambiguity remains.
4. The live path's simulation is mandatory, but shadow/ canary do not prove the
   same code would pass simulation; they only prove construction.
5. Auxiliary binaries are outside the mode gate.
