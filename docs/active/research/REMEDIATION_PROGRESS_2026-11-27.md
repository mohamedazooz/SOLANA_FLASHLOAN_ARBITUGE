# Protocol-Parity Remediation Progress

**Date:** 2026-11-27
**Working branch observed during handoff:** fix/protocol-parity-hardening-20261127
**Base HEAD observed:** 1412c65 — fix(routing): verify mint token programs before quotes

This is a progress checkpoint, not a declaration that the remediation pass is complete.

## 1. Kamino Reserve layout — golden fixture added

A confirmed Mainnet snapshot was captured using one public JSON-RPC getMultipleAccounts request so the Reserve and its supply vault share the same context slot.

- Context slot: 474341009
- Reserve: d4A2prbA2whesmvHaL88BH6Ewn5N4bTSU2Ze8P6Bc4Q
- Reserve owner: KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD
- Reserve data length: 8,624 bytes (8-byte discriminator + 8,616-byte payload)
- Supply vault: GafNuUXj9rxGLn4y79dPu6MHSuPWeJR6UtTWuexpGh3U
- Supply vault owner: TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA
- Supply vault data length: 165 bytes
- Status: 0 (active)
- Mint decimals: 9
- Flash-loan fee SF: 11,529,215,046,068
- Supply-vault amount: 321,999,152,973,336

Artifacts:
- crates/arb-tx/tests/fixtures/kamino_wsol_reserve_2026-11-27.bin
- crates/arb-tx/tests/fixtures/kamino_wsol_supply_vault_2026-11-27.bin
- crates/arb-tx/tests/fixtures/kamino_wsol_reserve_2026-11-27.json
- crates/arb-tx/tests/kamino_reserve_layout_fixtures.rs

The test command completed with exit code 0:
cargo test -p arb-tx --test kamino_reserve_layout_fixtures

The fixture test covers current account length/discriminator, expected mint, decimals, active status, flash fee, vault addresses, available liquidity, and truncation rejection. This is a pinned Mainnet golden-fixture test; it is **not** a direct SDK differential test and does not prove a live borrow/repay.

No klend-interface dependency was added. The crate's reviewed 0.6.0 license is BSL-1.1, which is not in the repository's deny.toml allowlist (BSL-1.0 is allowed). Do not override the license policy to add it. If a future legal review changes the allowlist, separately assess its Solana SDK compatibility before adoption.

The Kamino decoder offsets were not changed because the live account and official Reserve layout agreed at the reviewed offsets. A stale code comment was corrected to cite the official schema and current fixture evidence.

## 2. Raydium CLMM — active remediation / shared-worktree handoff

Official source reviewed:
- TickArrayState PDA uses the big-endian signed i32 start-index seed: https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/states/tick_array.rs
- Bitmap extension seed: https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/states/tick_array_bitmap_extension.rs
- Swap implementation: https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/instructions/swap.rs

Findings to address:
1. Swap has a fixed account prefix plus the first tick array and variable remaining accounts; do not model the full tick-array set as exactly three fixed entries.
2. Preserve the optional TickArrayBitmapExtension account in remaining accounts when it is required.
3. Derive its PDA using the official pool_tick_array_bitmap_extension seed, not a guessed seed.
4. Never pass a merely derived PDA as an executable account. Fetch it, validate owner/discriminator/length/pool relationship, and join it with tick arrays at a coherent slot first.
5. Do not duplicate the last tick-array key to fill missing positions.
6. Keep Raydium CLMM route eligibility gated until complete account hydration, quote parity, instruction parity, and simulation are demonstrated.

The hydration path was hardened to publish no Raydium CLMM tick-array/bitmap accounts until those accounts are actually fetched and validated. This is intentionally fail-closed.

A local coding agent is active in the same worktree and has switched to the branch named above; its Raydium CLMM and PumpSwap files are being edited. A targeted compile/test was attempted during that in-progress API change and failed because the builder signature and its callers/tests were temporarily out of sync. **Do not treat that transient failure as a final test result.** The active agent must finish the API migration, then rerun the Raydium CLMM builder tests, swap-hop tests, arb-ingest tests, and relevant workspace checks.

## 3. Shared-worktree coordination

- The local coding agent has been given the binding rules through root AGENTS.md and docs/agents/PROTOCOL_PARITY_RULES.md.
- Preserve its in-progress changes. Do not reset, stash, checkout, revert, or overwrite its files.
- Before editing a file, inspect git status and its current diff. If the agent is changing the same file, coordinate ownership rather than racing it.
- Do not push, open a PR, merge, or enable live execution as part of this checkpoint.
- After the agent reports completion, independently inspect the complete diff, rerun tests from a stable tree, and reconcile all acceptance criteria in the reference comparison dossier.
