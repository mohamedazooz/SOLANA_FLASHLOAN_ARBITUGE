# Property-Based Fuzzing & Invariant Verification

This document details the property-based fuzzing architecture deployed across the arbitrage engine. Fuzzing validates mathematical stability, integer boundary constraints, deserialization memory safety, and transaction wire validity under adversarial and randomized inputs.

---

## 1. Fuzzing Test Suites Overview

The repository maintains five dedicated property-based fuzzing targets using the `proptest` framework:

| Test Suite File | Crate Target | Test Cases | Primary Invariants Verified |
|---|---|---|---|
| [fuzzing_decoders.rs](file:///c:/Users/azoozz/Downloads/my%20arbitrge/SOLANA_FLASHLOAN_ARBITUGE/crates/arb-dex/tests/fuzzing_decoders.rs) | `arb-dex` | 500 cases / target | Deserializer panic-freedom, truncated slice rejection, discriminator enforcement |
| [fuzzing_suite.rs](file:///c:/Users/azoozz/Downloads/my%20arbitrge/SOLANA_FLASHLOAN_ARBITUGE/crates/arb-math/tests/fuzzing_suite.rs) | `arb-math` | 500 cases / target | Constant-product invariant, Newton-Raphson `isqrt` convergence, tick-to-sqrt-price bounds |
| [fuzzing_economic_decimals.rs](file:///c:/Users/azoozz/Downloads/my%20arbitrge/SOLANA_FLASHLOAN_ARBITUGE/crates/arb-math/tests/fuzzing_economic_decimals.rs) | `arb-math` | 512 cases / target | Extreme decimal boundaries ($0 \le d \le 255$), checked fee subtraction, overflow fail-closed |
| [fuzzing_templates.rs](file:///c:/Users/azoozz/Downloads/my%20arbitrge/SOLANA_FLASHLOAN_ARBITUGE/crates/arb-tx/tests/fuzzing_templates.rs) | `arb-tx` | 256 cases / target | Prewarmed transaction template serialization, account index deduplication |
| [v1_property_fuzz.rs](file:///c:/Users/azoozz/Downloads/my%20arbitrge/SOLANA_FLASHLOAN_ARBITUGE/crates/arb-tx/tests/v1_property_fuzz.rs) | `arb-tx` | 500 cases / target | SIMD-0385 V1 wire boundaries, Kamino U68F60 fee monotonicity, MTU packet size limits |

---

## 2. Invariant Specifications

### 2.1 DEX Account Deserializer Invariants (`fuzzing_decoders.rs`)
- **Panic-Freedom**: For any arbitrary byte array $B \in [0, 1200]$ bytes, deserializers (`RaydiumV4Pool`, `OrcaWhirlpool`, `PumpFunBondingCurve`, `PumpSwapPoolHeader`) must never panic.
- **Fail-Closed Truncation**: When $\text{len}(B) < \text{LAYOUT\_SIZE}$, the parser must unconditionally return `Err(ArbError::InvalidAccountData)`.
- **Discriminator Integrity**: If the first 8 bytes do not match the expected Anchor or SPL discriminator, the parser immediately aborts without reading inner payload fields.

### 2.2 AMM Mathematics Invariants (`fuzzing_suite.rs`)
- **Output Boundedness**: For any valid reserves $x, y > 0$, trade size $\Delta x > 0$, and fee factor $\gamma \in (0, 10000]$:
  $$\Delta y = \text{cpmm\_out}(\Delta x, x, y, \gamma) < y$$
  The swap output can never drain or exceed the pool's total output reserves.
- **Strict Monotonicity**: Increasing the input amount $\Delta x_1 < \Delta x_2$ strictly increases the output amount $\Delta y_1 \le \Delta y_2$ until liquidity saturation.
- **Newton-Raphson Wide-Integer Square Root Invariant**: For all wide integers $N \in \text{U384}, \text{U512}, \text{U768}$:
  $$s = \text{isqrt}(N) \implies s^2 \le N < (s + 1)^2$$
  Tested against zero, powers of two, maximum integer limits, and random distributions.

### 2.3 Decimal & Economic Boundaries (`fuzzing_economic_decimals.rs`)
- **Decimal Cap Enforcement**: Token mints with decimals $d > 18$ are strictly rejected with `ArbError::InvalidInput`. The system refuses to process malformed synthetic mints.
- **Zero-Loss Conservation**: In `calculate_net_profit_raw`, if $\text{gross\_profit} < \text{total\_fees}$, net profit must strictly yield a negative value or fail with `ArbError::NegativeSpread`. It is mathematically impossible for negative spreads to underflow into positive values.

### 2.4 Transaction Wire Format & MTU Invariants (`v1_property_fuzz.rs`)
- **Packet Size Ceiling**: Every compiled Solana transaction envelope is checked against the network maximum transmission unit (MTU). Any payload exceeding 1232 bytes (Legacy/V0) or 4096 bytes (V1 SIMD-0385) returns a compile-time error.
- **Kamino Flash Fee Monotonicity**: For borrow amount $A$, fee $\phi(A)$:
  $$A_1 \le A_2 \implies \phi(A_1) \le \phi(A_2)$$
  $$\phi(A) \ge 1 \quad (\forall A > 0)$$
  Kamino flash-loan fees always impose a minimum floor of 1 lamport/token unit.

---

## 3. Running Property Fuzz Tests

Execute all property suites with verbose capture:

```bash
# Execute AMM mathematical fuzzing
cargo test --package arb-math --test fuzzing_suite -- --nocapture

# Execute economic decimal fuzzing
cargo test --package arb-math --test fuzzing_economic_decimals -- --nocapture

# Execute DEX account decoder fuzzing
cargo test --package arb-dex --test fuzzing_decoders -- --nocapture

# Execute transaction V1 property fuzzing
cargo test --package arb-tx --test v1_property_fuzz -- --nocapture
```

All fuzzing suites are integrated into the standard `cargo test --workspace` test suite.
