# Meteora DLMM Bitmap Extension Validation

## Scope

This change hardens DLMM bin-array discovery and execution-account hydration. The inline `LbPair.bin_array_bitmap` covers indices `-512..=511`; the `BinArrayBitmapExtension` provides twelve 512-bit pages on each signed side. The extension PDA is `['bitmap', lb_pair]`.

## Implemented safeguards

- Decode the inline bitmap only after validating the `LbPair` discriminator and minimum account length.
- Decode the extension only after checking Meteora program ownership, discriminator, exact account length, and its embedded `lb_pair` key.
- Map positive pages from `512` upward and negative pages from `-513` downward, preserving page and bit order.
- Search the inline bitmap and extension in swap direction, stopping at the signed boundary and resuming in the other bitmap without skipping nearer initialized arrays.
- Select only bitmap-proven initialized BinArray PDAs. Empty gaps are represented as zero-liquidity bins locally; nonexistent writable accounts are never synthesized.
- Fetch up to five initialized arrays per swap direction. Five is the worst-case coverage for the 260-bin protocol traversal limit when the active bin is at an arbitrary offset in its 70-bin array.
- Enforce the protocol bin-step range `1..=400` in pool validation, Q64.64 price calculation, and base/variable fee calculation. Bin price remains `(1 + bin_step / 10_000)^bin_id`; base fee and volatility fee retain the protocol's `1e9` precision and 10% cap.
- Require an actually fetched and validated extension whenever the active array or traversed arrays leave the inline range.
- Reject incomplete, invalid, or incoherent dependency snapshots rather than publishing executable DLMM state.
- The atomic transaction builder now filters its remaining accounts from the hydrated bitmap-proven PDA set, checks the 260-bin traversal ceiling, and forwards the validated extension only when the quote span leaves the inline bitmap range.
- The legacy `MeteoraDlmmPool` adapter no longer publishes execution accounts from geometric PDA guesses; only the owner-verified ingestion snapshot can authorize an executable route.

## Validation

Unit and package tests cover signed boundary traversal, empty active arrays, directionality, resuming from extension into the inline bitmap, account owner/discriminator/pool linkage, and all twelve extension pages including the outermost positive and negative indices.

The official Meteora documentation describes the inline range, extension PDA, and the SDK helper `get_bin_array_pubkeys_for_swap`:

- https://github.com/MeteoraAg/docs/blob/main/developer-guides/dlmm/program/accounts.mdx
- https://github.com/MeteoraAg/docs/blob/main/developer-guides/dlmm/rust-integration/library.mdx

## Explicit limitation

Passing local unit tests is not a live SDK differential. Before enabling any live execution path that depends on overflow arrays, run the read-only Mainnet differential probe against the official SDK for pools whose active/traversed arrays are outside `-512..=511`. Compare selected indices, PDA order, and account presence in both swap directions at a coherent slot. Until that evidence is recorded, keep the execution path fail-closed for missing or unverified extension state.
