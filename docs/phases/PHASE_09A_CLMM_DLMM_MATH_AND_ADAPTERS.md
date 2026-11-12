# Phase 09A — CLMM / DLMM mathematical correctness and native adapters

**Priority:** first implementation phase.  
**Acceptance mode:** offline fixtures + deterministic replay + differential quotes. Live execution remains disabled.

## Scope separation

CLMM and DLMM are not variants of CPMM and must not share a constant-product approximation.

- **Orca Whirlpool:** CLMM with Q64.64 sqrt price, initialized ticks, Whirlpool TickArray accounts, tick crossing and liquidity-net updates.
- **Raydium CLMM:** separate CLMM implementation. It has its own PoolState, AmmConfig, TickArrayState, bitmap-extension account and Token-2022 V2 account requirements. Do not reuse the Orca account parser or PDA derivation.
- **Meteora DLMM:** discrete liquidity bins with per-bin reserves/prices, BinArray accounts, volatility reference/accumulator and dynamic fee calculation. It is not a continuous tick CLMM.

## Current code findings

### CLMM

1. `arb-math::clmm` contains Q64.64 math and an 88-tick parser matching the Orca-style layout; this is not sufficient evidence for Raydium's distinct TickArray layout.
2. `arb-ingest::registry` derives only the current, previous and next Orca TickArray PDAs. That bounded set is not a guarantee that a large swap can be fully quoted or that every required initialized tick is available.
3. The old `OrcaWhirlpool::quote` used a fixed eight-spacing target and substituted a one-percent price movement when tick conversion failed. That is not a protocol quote. It has been removed; the adapter now requires supplied decoded arrays and calls the multi-tick quote path.
4. `compute_swap` previously used saturating totals and silently stopped after 256 iterations. It now returns typed errors for no progress, iteration limit and incomplete input consumption.
5. `compute_swap_step` previously swallowed math errors and fabricated a target price with zero output. It now propagates `ClmmMathError`.
6. `div_ceil_u64` no longer saturates its increment.
7. `ClmmState` must carry initialized tick arrays from the same coherent account snapshot as the pool state.

### DLMM

1. `arb-math::dlmm` already models base fee, variable fee, protocol share, volatility reference/accumulator, bin traversal and exact-in quote output.
2. `pow_q64` previously replaced overflow/underflow with zero, one or `u128::MAX`. Those substitutions can create phantom prices. It now returns `MathOverflow` when the result is not representable.
3. Engine conversion of bin reserves and input amounts previously clamped values into `u64`; it now rejects out-of-domain values.
4. `quote_dlmm_hop` intentionally advances a route-local mutable pool view so repeated use of the same pool within one route is sequential. Do not confuse that with mutating the global market cache.
5. `quote_dlmm_view` still copies the route-local bin window for a non-mutating quote. The global pool cache must never be cloned for a single account update; that is a separate state-update concern.
6. Current ingestion fetches three BinArray accounts around the active bin. This is not enough to guarantee full quote coverage for all input sizes.

## Mathematical contracts to enforce

### CLMM

Let `S = sqrt_price_x64 / 2^64` and `L` be active liquidity. For a price interval from `S_a` to `S_b`:

- Token0 delta magnitude: `L * |S_b - S_a| / (S_a * S_b)`.
- Token1 delta magnitude: `L * |S_b - S_a|`.
- Convert fixed-point scaling exactly with wide integers; round required input upward and output downward.
- Exact-in swaps repeatedly compute the next reachable sqrt price, consume the current interval, cross the next initialized tick, apply signed `liquidity_net` according to direction, and continue until input is consumed or the supplied state/account coverage is insufficient.
- A quote is valid only if it consumed the requested input, respected the directional sqrt-price limit, crossed only verified initialized ticks, and has no arithmetic overflow or zero-progress loop.

### DLMM

For active bin `i` and bin step `b`:

- Price is represented as `P_i = (1 + b/10_000)^i` in Q64.64, subject to the exact protocol integer implementation and token orientation.
- Exact-in traversal visits bins in protocol direction, applies the per-bin fee based on base fee and current volatility accumulator, computes output using protocol rounding, and advances volatility state exactly as the deployed program.
- The quote must report consumed input, output, total fee, protocol fee, bins crossed, end active ID and end volatility state.
- Missing BinArray coverage, invalid status, impossible fee parameters, price overflow or unconsumed input must reject the quote.

## Implementation sequence

### 09A.1 — protocol account contracts

- Add versioned account decoders for Orca Whirlpool, Raydium CLMM and Meteora LbPair/BinArray.
- Validate owner program, discriminator, account length, pool reference, PDA derivation, tick/bin index and slot/write-version metadata.
- Keep protocol-specific account layouts isolated. Shared math primitives are allowed; shared binary parsers are not unless layouts are proven identical.

### 09A.2 — complete state coverage

- Derive the next required TickArray/BinArray from the current simulated price/bin and direction.
- Fetch/subscribe dynamically until the requested input is consumed or the protocol's valid boundary is reached.
- Require all state accounts to meet the same snapshot watermark policy. Do not combine pool state from slot N with a tick/bin array from an incompatible fork/slot.
- Track per-array coverage and missing-account reasons. Never infer empty liquidity from a missing account.

### 09A.3 — math parity

- Differential-test every CLMM step against official protocol SDK/program math.
- Differential-test DLMM bin quotes, dynamic fees, fee rounding and volatility updates against the official Meteora SDK/program.
- Cover exact boundary, one atom below/above, tick/bin crossing, empty bins, partial fill, max fee, fee rounding, negative ticks/bin IDs, large amounts, Token-2022 transfer fee, and insufficient array coverage.
- Use property tests for monotonicity, conservation, no negative reserves, no phantom liquidity, and exact-in input accounting.

### 09A.4 — instruction parity

- Generate the exact account list required for the chosen path, including Raydium bitmap extension when crossed, all required tick arrays, Token-2022 mint accounts and memo program when required.
- Verify the route builder consumes the exact same `PoolLegQuote` and state version as RiskGate.
- Keep CLMM/DLMM execution disabled until quote and instruction parity are proven.

## Acceptance gates

| Gate | Required evidence |
|---|---|
| Decoder correctness | Golden raw account fixtures + owner/PDA/discriminator tests |
| State coherence | Slot/write-version/fork consistency tests |
| Math correctness | Differential SDK/program quote suite with exact integer amounts |
| Traversal completeness | Multi-tick/multi-bin fixtures beyond the first neighboring array |
| Failure behavior | Missing/invalid/stale state rejects; no synthetic price or partial quote |
| Execution parity | Instruction accounts and amount limits match the validated quote |
| Performance | Criterion benchmarks for quote hot path and update-only pool mutation |

**No CLMM/DLMM live execution is accepted until all gates pass.**
