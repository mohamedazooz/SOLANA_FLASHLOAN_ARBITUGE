# 05 — Route Direction Audit

Direction is **derived from mint equality against decoded pool mints** — never from symbol names
or host ordering. Evidence below shows this is done consistently in mint-terms, and inconsistently
in reserve-terms, and once fatally in account selection.

## 1. Direction derivation sites

| Site | Code | Direction semantics |
|---|---|---|
| Cross-pool quote engine | `meteora_quote_engine.rs:109-115` | `swap_for_y = (base==from && quote==to)` |
| DLMM adapter quote | arb-dex/src/meteora_dlmm.rs:426,475 | same predicate; reserves normalized by mint comparison (registry.rs:1986) |
| Meteora quote hop (adapters.rs:581) | `quote_dlmm_multi_bin` directional | direction from `mint_in == x` |
| CPMM engine | arb-engine/src/lib.rs:68→cpmm_out | direction from reserve match at PoolState level |
| Orca Whirlpool instruction | arb-tx/lib.rs:2181 → `build_orca_whirlpool_swap_instruction_11` (:947) | **`a_to_b` derived from `mint_in == WSOL`** — breaks every non-SOL whirlpool leg |
| Atomic two-swap builder | `meteora_atomic_builder.rs:198,234` | `swap_for_y = (first_pool.base == token_in / second_pool.base == token_mid)` — mint equality ✓ (hop 1 implicitly Quote→Base; no independent check for hop 2 input, guarded only by equality) |
| DLMM instruction arrays | `meteora_bin_array_indices` (lib.rs:1059-1070), `derive_directional_bin_arrays` (meteora_dlmm.rs:239) | X→Y ⇒ arrays step **down**; Y→X ⇒ step **up** |
| CLMM tick arrays | arb-tx/lib.rs:2336 → `build_raydium_clmm_swap_instruction` (:1586) | `a_to_b = (amount of token_a in?)` via mint equality (engine-side); instruction-side re-derives |
| PumpSwap / PumpFun | pump_fun.rs, pump_swap.rs; `build_pump_fun_swap_instruction` (:1693), v2 (:1803) | mint equality vs bonding curve/global config |
| Raydium V4 | raydium_v4.rs; `build_raydium_v4_swap_instruction` (:732) | direction by account ordering vs mint match |
| LaunchLab | raydium_launchlab.rs | same |

## 2. X→Y / Y→X semantics vs implementation (Meteora DLMM)

Official (docs.meteora.ag / IDL): `swap_for_y = true` ⇒ token X in (Y out); direction derives
from account ordering — there is no `swap_for_y` byte in current `swap`/`swap2` instruction data.
Verified against `lb_clmm` IDL (see `14_EXTERNAL_RESEARCH_REFERENCES.md` §6).

| Check | Implementation | Result |
|---|---|---|
| swap_for_y definition | `true` = X→Y (price falls, bins decrease) — `meteora_dlmm.rs:63-69`, dlmm.rs:642-644 | ✓ matches |
| Input mint side | from `mint_in == pool.base_mint` equality; base==token_x in decode | ✓ |
| Traversal | X→Y decrements active bin (`advance_active_id`, dlmm.rs:558-563); Y→X increments | ✓ matches |
| BinArray arrays step | X→Y: `[active, active-1, …]`; Y→X: `[active, active+1, …]` (lib.rs:1065; meteora_dlmm.rs:248-251) | ✓ |
| `swap_for_y` as instruction arg | not present (correctly absent) | ✓ |
| Event semantics | not decoded by engine | n/a |

## 3. Route matrix (mint-order dependent cases from source, both directions)

| Route | Pool | Input mint | Output mint | Dir inferred | Expected | Actual | Result |
|---|---|---|---|---|---|---|---|
| WSOL→token X pool X/Y=WSOL/USDC | DLMM | WSOL (X) | USDC (Y) | true | X→Y | true | OK |
| USDC→WSOL same pool | DLMM | USDC (Y) | WSOL (X) | false | Y→X | false | OK |
| uncommon: pool catalog says base=USDC, decode gave token_x=USDC — OK | DLMM | | | | | | OK |
| **pool with misordered catalog/decode (base=token_y)** | DLMM | WSOL | USDC | `pool.base==from` ⇒ swap_for_y **true** (X→Y assumed) | depends on layout | quote/clamps/fees placed for X→Y regardless | **FAIL — well-formed wrong quote** |
| Reverse of above | DLMM | USDC | WSOL | false | | | same class of failure |
| Token-2022 leg | any | — | — | — | — | priced fee-free, ATA derived SPL | **FAIL** |
| Whirlpool non-SOL leg | Orca | USDC | BONK | a_to_b = (mint_in == WSOL) = false | depends on token_a | wrong direction hard-coded for non-SOL pairs | **FAIL** |
| LaunchLab leg | LaunchLab | — | — | mint equality OK in builder | — | — | direction OK; **discriminators wrong** (see 13) |
| LaunchLab via arb-tx inline builder | LaunchLab | — | — | — | — | launchlab discriminators defined but pump.fun's are emitted | **FAIL (tx would revert)** |
| Pump.fun leg | PumpFun | SOL→meme | — | bonding curve direction by mint match | — | OK direction | OK |

## 4. BinArray selection vs direction (see 06 for full audit)

- Quote path loads PDAs symmetrically (`bin_array_pdas_around_active`, arb-dex:100-115; ingest
  :1508; pipeline `depth → 2d+1`) — **not direction-aware**.
- Instruction path loads direction-aware (`meteora_bin_array_indices`, `derive_directional_bin_arrays`)
  — **offset-unaware under-count** (see 06 §4). The two sets never match.

## 5. Reverse-route account reuse check

- Bin arrays: re-derived per hop in the instruction builder (direction-dependent indices) ✓.
- Wrong-direction bin array reuse is caught by nothing: no test asserts that arrays ordered for
  the opposite direction are rejected; the VM would revert (fail-closed, but a wasted dispatch).
- Token program metas per hop are placed from `prog_in/prog_out = prog_mid/token_x_program`
  (meteora_atomic_builder.rs:217-218,251-252) — for the atomic builder this is consistent; for the
  hot path the SPL-only derivation breaks 2022 (see 03/13).
- Bitmap extension: **always `None` in the atomic builder** (meteora_atomic_builder.rs:222,256)
  ⇒ far-from-parity pools can never trade and nothing alerts; dex adapters always emit
  `Some(writable PDA)` even when the account doesn't exist (arb-dex meteora_dlmm.rs:282,
  registry.rs:1056) ⇒ any consumer that wires it in reverts; `derive_directional_bin_arrays`
  computes `bitmap_extension_required` (meteora_dlmm.rs:281) but is unused by the atomic path.
- **Oracle account = pool address** (registry.rs:1054) instead of `[b"oracle", lb_pair]` PDA.

## 6. Verdicts

Direction *logic* is mint-based and correct for CPMM/CLMM/DLMM in the common case; it is **not
correct** for misordered DLMM pools, non-SOL Whirlpools, and is unusable for Token-2022 and
LaunchLab legs. Reverse routes do not reuse wrong *accounts* structurally, but quote and
instruction arrays diverge systematically (06).
