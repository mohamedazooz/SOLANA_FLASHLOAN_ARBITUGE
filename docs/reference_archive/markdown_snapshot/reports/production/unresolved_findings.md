# Unresolved Findings & External Operational Blockers

## External Operational Blockers (Requiring Operator Action)
1. **Live Operator Key Injection**: Live on-chain execution requires the operator to provide the designated Canary hot wallet keypair with micro-funds ($5-$10 SOL/USDC) and export `ARB_LIVE=1`.
2. **Dedicated Yellowstone gRPC Credential**: High-frequency streaming on mainnet-beta requires an authenticated Helius/Triton gRPC endpoint token injected into `.env`.
3. **SIMD-0385 Mainnet Cluster Feature**: Transaction V1 (0x81 prefix) is fully compiled and tested, but will operate through universal V0 Address Lookup Table fallback until Agave validators on mainnet activate the SIMD-0385 feature gate.

## Zero Code-Level Blockers
- **Compiler Status**: 0 errors across 14 workspace crates.
- **Linter Status**: 0 warnings under `cargo clippy --workspace --all-targets -- -D warnings`.
- **Test Matrix**: 0 test failures across 700+ automated tests.
- **Fail-Closed Safety**: Invariants verified; default mode remains strictly simulation/shadow.
