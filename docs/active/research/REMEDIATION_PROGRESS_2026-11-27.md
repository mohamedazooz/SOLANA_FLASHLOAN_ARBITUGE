# Protocol-Parity Remediation Progress

**Date:** 2026-11-27
**Implementation branch:** `fix/protocol-parity-hardening-20261127`
**Base commit:** `1412c65` — verify mint token programs before quotes

This record separates local code/test evidence from missing live hydration, differential, simulation, and production evidence.

## 1. Kamino Reserve layout — pinned Mainnet fixture

A confirmed Mainnet snapshot was captured using one `getMultipleAccounts` request so the Reserve and its supply vault share the same context slot.

- Context slot: 474341009
- Reserve: `d4A2prbA2whesmvHaL88BH6Ewn5N4bTSU2Ze8P6Bc4Q`
- Reserve owner: KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD
- Reserve data length: 8,624 bytes (8-byte discriminator + 8,616-byte payload)
- Supply vault: `GafNuUXj9rxGLn4y79dPu6MHSuPWeJR6UtTWuexpGh3U`
- Supply vault owner: TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA
- Supply vault data length: 165 bytes
- Status: 0 (active); mint decimals: 9
- Flash-loan fee SF: 11,529,215,046,068
- Supply-vault amount: 321,999,152,973,336

Fixtures and regression test:
- `crates/arb-tx/tests/fixtures/kamino_wsol_reserve_2026-11-27.bin`
- `crates/arb-tx/tests/fixtures/kamino_wsol_supply_vault_2026-11-27.bin`
- `crates/arb-tx/tests/fixtures/kamino_wsol_reserve_2026-11-27.json`
- `crates/arb-tx/tests/kamino_reserve_layout_fixtures.rs`

The test validates account length/discriminator, mint, decimals, active status, fee SF, vault addresses, available liquidity, and truncation rejection. It is a pinned Mainnet golden-fixture test, **not** an official-SDK differential test and does not prove a live borrow/repay.

No `klend-interface` dependency was added: the reviewed 0.6.0 crate is BSL-1.1, outside the repository's `deny.toml` allowlist. Do not weaken the license policy to import it.

The Reserve offsets were not changed: the fixture agrees with the reviewed official Reserve schema at the existing offsets. The stale discriminator/layout comment was corrected.

## 2. Raydium CLMM — builder parity improved; runtime remains gated

Official source reviewed:
- TickArrayState PDA seed/start-index encoding: https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/states/tick_array.rs
- Bitmap extension seed: https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/states/tick_array_bitmap_extension.rs
- Swap account parsing: https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/instructions/swap.rs

Implemented locally:
1. The transaction account model accepts the first required tick array, optional bitmap extension, and variable additional tick arrays; it no longer treats three arrays as a universal fixed account set.
2. The bitmap-extension PDA helper uses the official `pool_tick_array_bitmap_extension` seed.
3. The builder rejects empty, default, or duplicate tick-array/bitmap keys.
4. Direction is derived from the pool's on-chain `token_mint_0`, not lexicographic mint order.
5. Hydration does **not** publish derived tick-array/bitmap PDAs as if they were fetched executable accounts. Raydium CLMM fee state is still not joined from the live AmmConfig/dynamic-fee dependencies; it remains unpriceable and unexecutable.

Not completed:
- Fetch and validate AmmConfig, tick arrays, optional bitmap extension, pool relation, owner/discriminator/length, and same-slot coherence.
- Exact quote parity, dynamic-fee modelling, end-to-end simulation, shadow testing, or production execution evidence.

The builder's structural unit tests do not imply the venue is executable.

## 3. PumpSwap V2 — official fixed account matrix

The official PumpSwap V2 trade interface uses a fixed 17-account matrix and does not append the previously inferred `+3 buy / +2 sell` remaining accounts. The builder now derives the user-volume-accumulator PDA, uses a published buyback-fee recipient ATA, rejects V1 modes in the V2 builder, and keeps the legacy V1 builder separate.

The current V2 builder only accepts WSOL or USDC quote mints until arbitrary quote-mint buyback ATA existence/rent handling is implemented; other quote mints fail closed. The golden test checks account order, mutability/signer flags, the three V2 discriminators, and mode-specific argument order. This verifies builder structure, not a live PumpSwap simulation or settlement.

Official references:
- https://github.com/pump-fun/pump-public-docs/blob/main/docs/PUMP_SWAP_TRADE_V2.md
- https://github.com/pump-fun/pump-public-docs/blob/main/docs/FEE_RECIPIENTS.md

## 4. Orca Whirlpool adaptive-fee gate

The decoder reads the Whirlpool `fee_tier_index_seed` and accepts the classic fee tier only when it equals `tick_spacing`. Adaptive-fee pools are rejected from this classic-fee quote path rather than being priced with a static rate. Regression tests cover classic acceptance and adaptive rejection.

Adaptive-fee support itself is **not** implemented: oracle-aware fee state/math and its complete execution dependency set would be required before enabling such pools.

Official source:
https://github.com/orca-so/whirlpools/blob/main/programs/whirlpool/src/state/whirlpool.rs

## 5. Verification on this branch

The following local Windows/toolchain runs completed successfully:

- `cargo fmt --all -- --check`
- `cargo test -p arb-tx --test p0_golden_vectors --no-fail-fast`: 9 passed, 0 failed
- `cargo test -p arb-tx --test kamino_reserve_layout_fixtures -- --nocapture`: 2 passed, 0 failed
- `cargo test -p arb-ingest --lib`: 186 passed, 0 failed
- `cargo test -p arb-tx --lib`: 189 passed, 0 failed
- `cargo test -p arb-tx --test swap_hop_direction_tests`: 19 passed, 0 failed
- `cargo clippy --workspace --all-targets -- -D warnings`: passed
- `git diff --check`: passed before final documentation reconciliation

A full `cargo test --workspace --all-targets` run is still required for this branch. No mainnet transaction was submitted; no new RPC simulation, Jito ingress canary, shadow soak, landing, or realized-PnL evidence is claimed.

## 6. Safety and PR workflow

- Preserve `ARB_LIVE=0`, `DRY_RUN=true`, and the existing quote/execution safety gates.
- Do not merge the resulting PR automatically.
- PR #184 already contains the economic-accounting work on `fix/economic-fee-accounting-20261127`. The protocol-parity changes should be submitted as a separate stacked PR based on that branch, so the PR diff contains only this remediation commit and does not duplicate #184.
- The operator explicitly requested completion and PR creation on 2026-11-27. That request supersedes the earlier temporary handoff note that prohibited PR creation. It does not authorize merging or enabling live execution.