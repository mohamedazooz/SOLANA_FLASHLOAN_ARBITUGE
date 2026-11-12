# 13 — Cross-Protocol Compatibility Audit

Full evidence: `audit_work/13_CROSS_PROTOCOL.md`. Official IDL/discriminator ground truth:
`14_EXTERNAL_RESEARCH_REFERENCES.md` §9 (verified by local SHA-256 recomputation where reachable).

## 1. Adapter inventory and runtime reachability

| Adapter | Program ID | Pinned SDK/IDL | Reachable from live path? | Verdict |
|---|---|---|---|---|
| Raydium AMM V4 | AMM program constants in arb-constants | none (legacy; raw tags 9/11/16/17) | **Yes** | Instruction-builder + quote; **V1 legacy entrypoints with market accounts ignored — v4 2026-07 upgrade removed OpenBook; AMM v4 = classic SPL only; account layout decode byte-verified against upstream** |
| Raydium CPMM | `CPMMoo8L3F4…` | discriminators re-derived (swap_base_input/output, §9.1) | **Yes** | Full adapter; fee from AmmConfig; `swap_base_input`/`swap_base_output` correct |
| Raydium CLMM | CLMM program | CLMM IDL (swap=[248,…]; swap_v2=[43,…]) | **Yes** | **Swap-disable check reads bit 16 instead of bit 4 (`swap_config`?)** — mis-evaluates "swaps disabled" pools as enabled (see §3) |
| Orca Whirlpool | `whirLbM…` (unverified on-chain this session) | whirlpools source | **Yes** | **`a_to_b` hard-derived from `mint_in == WSOL`** — breaks all non-SOL whirlpool legs; `fee_tier_index_seed` correctly decoded (byte-verified vs upstream state/whirlpool.rs); sqrt_price_limit dimensionally wrong (not Q64 of current price orientation) |
| Meteora DLMM | `LBUZKhRx…` | `lb_clmm` IDL constant matches | **Yes** | Math byte-equal vs SDK on captured pool; **quote engine broken (17 §7.1/7.2); ingest oracle = pool id; empty bins hard-error; bitmap never scanned** |
| Meteora DAMM v2 | program id constant | `damm_v2.rs` math | **NO** — missing from `program_id_to_protocol`, `decode_amm_pool`, registry fee resolution | **Dead code (quote-only, unwired)** |
| Raydium LaunchLab | constant | launchlab math | **NO** — same | **Dead code (quote-only); launchlab discriminators are DEFINED but arb-tx inlines pump.fun's ⇒ would revert if wired** |
| PumpSwap | program id | IDL-less (account-order convention) | Narrow path only: engine reaches via `arb-ingest::pump_state` join; `decode_amm_pool` returns `None` (registry polls skip it) | Partially implemented |
| Pump.fun (bonding curve) | `6EF8r…` | none (layout known) | **Yes** | Instruction-builder + quote; `sell_v2` discriminator **wrong** (see §2); `global:sell` hash stored where `sell_v2` expected |

## 2. Confirmed discriminator defects

| Item | Expected | Stored | Effect |
|---|---|---|---|
| PumpSwap `global:sell_v2` | sha256("global:sell_v2")[..8] = **[93,246,130,60,231,233,64,178]** | **[51,230,133,164,…] = sha256("global:sell")[..8]** | every PumpSwap V2 sell fails on-chain check |
| RaydiumLaunchLab inline builder (arb-tx) | launchlab's own discriminators (defined, arb-constants) | pump.fun's | every LaunchLab swap fails on-chain |
| CLMM swap-disable bit | bit **4** of swap-disable config | bit **16** read | disabled pools quoted as enabled |
| Whirlpool `a_to_b` | token_a in ⇒ true, else false (mint order vs pool token mints) | `mint_in == WSOL` | wrong direction for every non-SOL whirlpool |
| CLMM `sqrt_price_limit` | per-direction Q64 limit relative to current tick | dimensionally invalid value | would cross forbidden boundary or abort |

## 3. Per-adapter deep findings

**Raydium V4** — account layout decode verified byte-for-byte vs upstream. Fee: `fees`/`fund`
fields; direction by reserve orientation; discriminator tags 9/11/16/17 correctly implemented
(16/17 = V2 entrypoints post-2026-07 OpenBook removal; SDK types expose them). Slippage:
`min_out`. Token-2022: explicitly **classic SPL only**; any 2022 mint silently misdecodes — no
guard rejects a 2022 mint for a V4 pool at quote time.

**Raydium CPMM** — `PoolState`/`AmmConfig` offsets verified against upstream. Fee read from
AmmConfig (correct — not catalog); `swap_base_input/output` discriminators correct (re-derived).
Fee ceil semantics match on-chain. Token-2022: decoded but `swap_base_input` v1 doesn't handle
transfer fees; pools flagged 2022 should use `swap_base_output`/v2 — not enforced.

**Raydium CLMM** — layout verified. **Fee always from catalog, with fabricated 1-bps floor**
(registry.rs:910-911) — real pools with other fees are mispriced. Swap-disable bit 16 bug above.
`sqrt_price_limit` dimensionally invalid (not in the format the program expects; will either always
succeed the limit check trivially or revert — direction-dependent) or, with tick-arrays parsed,
produce nonsensical price bounds. Tick array PDAs derived correctly (`:1476`).

**Orca Whirlpool** — `fee_tier_index_seed` decoded correctly (easily missed field). Direction bug
above. Whirlpool program id unverified on-chain this session (UNVERIFIED — flagged, not asserted).

**Meteora DLMM** — see 06. Math core byte-equal vs SDK (tolerance 0) for price/fee on a captured
pool; CollectFeeMode 1 never exercised through the engine; ingest writes pool id as oracle PDA;
status/collect_fee_mode hardcoded 0/0 in cross-pool quote engine; host/referral fee (HOST_FEE_BPS =
2000 protocol-side) has no math anywhere — pools paying referral/host fees are overpriced for user
output by 20% of the protocol fee.

**PumpSwap** — V2 `sell` discriminator wrong (§2); instruction builder-only otherwise; quote from
in-house math unverified against program; `PumpSwapPoolVersion::V2` default everywhere;
`decode_amm_pool` returns None so the batch registry path never emits it — only the
`pump_state` live join reaches it.

**Pump.fun bonding curve** — formulae in arb-math/pump_fun_raydium.rs; direction OK; fee = 1%
hardcoded launchlab-fee in one path; `sell_v2` confusion above; protobuf/layout not independently
re-verified.

**LaunchLab** — dead; discriminators unused/wrong when delegated; fee hardcoded 1%.

**Token program handling across all adapters**: SPL-derived ATAs everywhere on the hot path;
Token-2022 swap account metas carry the 2022 program id while the ATA is SPL-derived ⇒ every
route with a 2022 intermediate is invalid. Withheld extension fees never checked in any hot path
(only in the 3-leg meme compiler).

## 4. Adapter classification summary

| Adapter | Class |
|---|---|
| Raydium V4 | Instruction-builder + quote (direction bug-free; SPL-only unguarded) |
| Raydium CPMM | Full adapter (fee ceil OK); v2-vs-v1 2022 split unenforced |
| Raydium CLMM | Quote OK (with 1 bps catalog floor caveat); instruction builder has limit-price defect |
| Orca Whirlpool | Instruction builder with direction bug for non-SOL pairs |
| Meteora DLMM | Quote byte-equal for exact-in/out on captured pool; engine integration broken (transfer fee, status, mode, oracle, array span) |
| Meteora DAMM v2 | Dead code (quote-only, unreachable) |
| Raydium LaunchLab | Dead code (quote-only; builder emits wrong discriminators) |
| PumpSwap | Partially implemented (sell_v2 broken; narrow ingest reachability) |
| Pump.fun | Full adapter |

**No DEX should be classified execution-ready by this audit.** The DLMM math core and the CPMM
adapter are closest to correct; everything else carries at least one blocker.
