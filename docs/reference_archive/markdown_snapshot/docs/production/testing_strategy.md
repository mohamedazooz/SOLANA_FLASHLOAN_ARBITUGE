# Testing Strategy & Validation Matrix

## 1. Multi-Layer Testing Architecture

```
Layer 1: Unit Tests (Math, Primitive Types, Fixed-point, Bitmaps)
Layer 2: Differential Parity Tests (DLMM vs Meteora SDK, Kamino vs Klend)
Layer 3: Property & Fuzz Tests (Randomized Account Bytes, Boundary Bins)
Layer 4: Invariant Verification (Zero-Float, Non-Closing Routes, Sizing Bounds)
Layer 5: Local Preflight & LiteSVM (Deterministic Instruction Simulation)
Layer 6: Paper Trading Live Ingestion (Continuous Real-World Market Validation)
```

## 2. Automated Test Commands
- **Unit & Integration Suite**:
  ```bash
  cargo test --workspace --all-targets
  ```
- **Differential DLMM & PDA Verification**:
  ```bash
  cargo test -p arb-engine --test meteora_sdk_differential_tests
  cargo test -p arb-engine --test phase17_pda_differential
  cargo test -p arb-math --test dlmm_bin_array_boundary_tests
  ```
- **Kamino Flash Fee & V1 Serialization**:
  ```bash
  cargo test -p arb-tx --test v1_header_differential
  cargo test -p arb-tx --test v1_property_fuzz
  ```
- **Strict Clippy Audit**:
  ```bash
  cargo clippy --workspace --all-targets -- -D warnings
  ```
