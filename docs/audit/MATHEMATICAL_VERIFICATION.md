# PHASE 4 — Mathematical Verification

**Independent oracle:** `audit/math_oracle/oracle.py` (Python 3, arbitrary-precision integers).
**Vectors:** `audit/test_vectors/cpmm_vectors.json` (13), `audit/test_vectors/dlmm_single_bin_vectors.json` (6).
**Differential harness:** `crates/arb-math/tests/python_oracle_differential.rs` — **19/19 MATCH**.

The oracle was derived from protocol specifications, NOT from the Rust code. During
derivation, two errors in the oracle itself were caught and corrected by evidence:
(1) fee-added-on-top must be `ceil(a·r/(1e9−r))` (SDK `getIncludedFeeAmount`), not
`ceil(a·r/1e9)`; (2) for X→Y, `net_in = ceil(out·2^64/P)`, not `ceil(out·P/2^64)`.
The Rust implementation matched the specification in both cases.

## Equation registry

### E1. CPMM output, upfront ceil fee — VERIFIED (oracle, 13 vectors)

- Definition: `fee = ceil(dx·fee_num/fee_den)`; `net = dx − fee`;
  `out = floor(net·y/(x+net))`; invariant `x·y = k`.
- Protocol source: Raydium AMM v4 / Raydium CPMM / Meteora DAMM v2 accounting.
- Rust: `arb_math::cpmm::cpmm_out_upfront_fee_ceil` (crates/arb-math/src/cpmm.rs:41).
- Units: raw token (input-side) in, raw token (output-side) out. Rounding: fee CEIL,
  output FLOOR (pool-favorable, prevents 1-lamport dust reverts).
- Overflow: U256 intermediates; `u64::try_from` fail-closed on output.

### E2. CPMM output, gamma-folded fee — VERIFIED (oracle, same 13 vectors)

- Definition: `out = floor(γ·dx·y / (D·x + γ·dx))`, `γ = D − fee`.
- Rust: `arb_math::cpmm::cpmm_out_fee` (cpmm.rs:73).
- Cross-check: E1 and E2 agree on every vector (algebraically identical; both floor).

### E3. DLMM bin price — VERIFIED structurally, NOT oracle-covered

- Definition: `P(i) = (1 + bin_step/10^4)^i` in Q64.64.
- Rust: exponentiation-by-squaring in Q64.64 (dlmm.rs:250–265 region).
- Oracle coverage: **ABSENT** — the Q64.64 rounding chain of the power series is
  implementation-specific; oracle uses `price_q64` as an input instead. Marked
  **NOT INDEPENDENTLY VERIFIED**; mitigation: E4/E5 take P as input, so price errors
  cannot hide inside fee/consumption math.

### E4. DLMM single-bin consumption, fee-on-input — VERIFIED (oracle, 6 vectors)

- Definition: `net_in_max = ceil(out_max·2^64/P)` (X→Y) or `ceil(out_max·P/2^64)` (Y→X);
  `max_fee = ceil(net_in_max·r/(1e9−r))`; bin drains iff `amount_in ≥ net_in_max + max_fee`;
  non-drain: `fee = ceil(amount_in·r/1e9)`, `out = floor(net_in·P/2^64)` (X→Y).
- Rust: `consume_bin_liquidity` fee_on_input branch (dlmm.rs:649+).
- Includes drain boundary (v3), dust (v5: 1-raw unit), large amounts (v6: 2^63).

### E5. DLMM single-bin consumption, fee-on-output (OnlyY) — VERIFIED (oracle, 6 vectors)

- Definition: pool consumes gross input; `fee = ceil(gross_out·r/1e9)` removed from
  outbound token; `user_out = gross_out − fee`.
- Rust: `consume_bin_liquidity` fee_on_output branch. This is the branch whose
  `collect_fee_mode` forwarding was fixed in commit `152ad88`.

### E6. DLMM dynamic fees — VERIFIED against official formula (structural + live)

- Base: `base_fee = base_factor × bin_step × 10 × 10^base_fee_power_factor` (1e9 units).
  Rust `decode_dlmm_fee_bps` (registry.rs:2039) matches exactly.
- Variable: `⌈vfc × (VA × bin_step)² / 1e11⌉`; `compute_variable_fee` (arb-math) matches.
- Live feed: `quote_dlmm_hop` consumes real volatility params from ingested LbPair bytes.

### E7. CLMM tick math — PARTIALLY VERIFIED

- sqrt-price/tick relations use `orca_whirlpools_core 2.1.1` (official crate, pinned `=`).
- Virtual reserves `x_virt = (L<<64)/√P`, `y_virt = (L·√P)>>64` — verified structurally
  (`compute_clmm_virtual_reserves`, registry.rs:2009).
- Oracle coverage: **ABSENT** (tick-crossing oracle = backlog item).

### E8. Net profit equation — VERIFIED structurally, NOT live-proven

- `net = final_out − principal − flash_fee − swap_fees(in outputs) − priority_fee − tip − lamport_fees`.
- `guarded.rs` gates on chained `amount_out` (fees already inside swap outputs — no
  double counting; verified after the `fee_paid` denomination fix `152ad88`).
- Integer-only arithmetic end-to-end; no floating point in token amounts (verified by grep).

## Honest gaps

| Item | Status |
|---|---|
| CLMM tick-crossing oracle | NOT DONE (backlog P1) |
| DLMM bin-price power series oracle | NOT DONE (inputs verified instead) |
| Multi-bin traversal differential (E4/E5 chained) | NOT DONE — single-bin verified; traversal reviewed line-by-line only |
| Flash-loan fee oracle | Kamino fee taken from on-chain reserve data (verified discriminator + layout earlier session); oracle vectorization pending |
