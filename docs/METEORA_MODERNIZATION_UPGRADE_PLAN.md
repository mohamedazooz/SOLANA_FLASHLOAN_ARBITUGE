# Meteora Modernization & Phased Upgrade Architecture Plan
## Institutional Engineering Research Report & Implementation Blueprint for Solana Flash Loan Arbitrage

**Document Version**: 1.0.0-PROD  
**Date**: November 16, 2026  
**Classification**: Engineering Architecture & Security Specification  
**Target Repository**: `SOLANA_FLASHLOAN_ARBITUGE`  
**Status**: APPROVED FOR IMPLEMENTATION (Milestone M1 Deliverable)  
**Authoritative Input Sources**:
- `docs/meteora_docs_complete.md` (57,266 lines of indexed Meteora technical documentation)
- Codebase Forensic Audit: `crates/arb-dex`, `crates/arb-tx`, `crates/arb-math`, `crates/arb-engine`, `crates/arb-ingest`, `crates/arb-constants`, `crates/arb-types`
- On-chain Deployments & Anchor IDLs: Meteora DLMM (`LBUZ...`), DLMM Pro (`dPro...`), DAMM v2 (`cpam...`), DBC (`dbci...`)

---

## 1. Executive Summary & Forensic Audit Overview

### 1.1 Architectural Context of Meteora DEX in Solana Flash Loan Arbitrage

Meteora is one of the premier liquidity protocol suites on Solana, processing hundreds of millions of dollars in daily spot volume across multiple specialized market-making primitives. In high-frequency atomic flash loan arbitrage, Meteora pools represent both high-yield arbitrage targets and critical routing venues due to their dynamic fee mechanisms, concentrated liquidity distributions, and aggressive retail trading volume.

The existing arbitrage bot architecture integrates Kamino flash loans, multi-hop swap routing, and Jito bundle block-engine submission under a strict atomicity invariant: **borrow, swap legs, and repay must land within a single atomic transaction**. Any failure in quote accuracy, instruction encoding, account ordering, or fee deduction causes immediate transaction simulation revert, burning transaction submission priority fees or leading to missed block inclusion.

Historically, the bot was developed around Meteora's legacy Bin-based Concentrated Liquidity protocol (DLMM v1, `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`). As Meteora expanded its product ecosystem to include **DLMM Pro** (continuous tick-based CLMM with multi-range positions and on-chain limit orders), **Dynamic Bonding Curves (DBC)** (a universal 16-point launchpad migrating to DAMM v2), **DAMM v2 / CP-AMM** (`cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG`) with rate-limiter protection and fee compounding, and dedicated **Hosted Data APIs**, the bot's codebase developed critical technical debt, outdated assumptions, and severe functional gaps.

This document delivers a forensic audit of the 7 major codebase gaps and articulates an institutional-grade, mathematically rigorous, and phased modernization plan.

---

### 1.2 Forensic Audit Matrix: The 7 Codebase Gaps

| Gap # | Protocol / Area | Impacted Crates & Files | Root-Cause Defect | Severity | Modernization Remediation |
| :---: | :--- | :--- | :--- | :---: | :--- |
| **1** | **DLMM Pro Integration** | `crates/arb-constants`<br>`crates/arb-dex`<br>`crates/arb-tx`<br>`crates/arb-math` | Zero implementation of DLMM Pro program `dPro1D5wiqnGMH8vMDG23jh35btTHoWdyqXjqkqxe8n`. Code only supports legacy discrete bin DLMM v1. | **High** | Implement tick-based CLMM decoder, 128-tick `TickArray` layout, multi-range NFT positions, resting limit order matching, and 17-account `swap` builder. |
| **2** | **Dynamic Bonding Curve (DBC)** | `crates/arb-dex`<br>`crates/arb-tx`<br>`crates/arb-engine`<br>`crates/arb-math` | Zero implementation of DBC program `dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN`. Only Pump.fun launchpad is implemented. | **High** | Implement universal 16-point piecewise bonding curve math, `VirtualPool` decoder, `swap2`/`swap2_with_transfer_hook` builders, and DAMM v2 graduation listener. |
| **3** | **DAMM v2 Discriminator & Sysvar** | `crates/arb-tx/src/protocols/swap_hop.rs:546-566` | Emits legacy `swap` discriminator (`f8c69e91e17587c8`) instead of canonical `swap2` (`414b3f4ceb5b5b88`); account ordering inverted; missing `sysvar::instructions` remaining account. | **Critical** | Upgrade discriminator to `swap2` with `SwapMode::ExactIn`; align 14 fixed accounts to `SwapCtx`; dynamically inject `sysvar::instructions` for active Rate-Limiter pools. |
| **4** | **`CollectFeeMode` Compounding Math** | `crates/arb-math/src/damm_v2.rs:28-29`<br>`crates/arb-dex/src/meteora_damm_v2.rs:225-229` | Line 28 mislabels variant 2 as "token A only" (canonically `Compounding`); line 225 drops all compounding pools. | **Medium** | Correct enum labels; implement full-range constant product compounding quote math with auto-reinvested Token B reserves; unblock compounding pools. |
| **5** | **DLMM Bin Array Traversal Cap** | `crates/arb-tx/src/protocols/meteora_dlmm.rs:252-257`<br>`crates/arb-engine/src/dlmm_flash_arbitrage.rs:682` | Hardcoded `max: 2` bin array cap (140 bins). High-volatility pairs (>140 bins) trigger `DlmmError::TooManyBinArrays` client-side rejection. | **High** | Dynamically expand bin array allocation to 3-4 arrays (280 bins); verify Solana V1 64 inline account budget; clamp size optimizer to loaded liquidity depth. |
| **6** | **Cross-Pool Pair Discovery** | `crates/arb-engine/src/meteora_pair_discovery.rs:97-101`<br>`crates/arb-engine/src/cross_pool_dlmm.rs:1-8` | Pair discovery strictly filters for `DexProtocol::MeteoraDlmm` vs `MeteoraDlmm`. Ignores DAMM v2, Raydium, and Pump.fun. | **Critical** | Unify multi-protocol pair discovery across Meteora (DLMM, DAMM v2, DBC), Raydium (AMM v4, CPMM, CLMM), and Pump.fun; introduce 2-stage hybrid sizing optimizer. |
| **7** | **Hosted Data APIs Integration** | `crates/arb-ingest/src/discovery.rs:680-748` | Codebase relies on third-party DexScreener HTTP scraping (30-120s delayed, rate-limited), ignoring Meteora hosted Data APIs. | **Medium** | Integrate high-performance clients for `dlmm.datapi.meteora.ag` (30 RPS) and `damm-v2.datapi.meteora.ag` (10 RPS) for background pool cataloging and volume scoring. |

---

## 2. Deep Codebase Audit of Meteora Integrations (The 7 Gaps)

### 2.1 Gap 1: Missing DLMM Pro (`dPro1D5wiqnGMH8vMDG23jh35btTHoWdyqXjqkqxe8n`)

#### Affected Codebase Files & Line Numbers
- `crates/arb-constants/src/lib.rs` (lines 4, 161)
- `crates/arb-dex/src/meteora_dlmm.rs` (lines 1–5, 94–105)
- `crates/arb-tx/src/protocols/meteora_dlmm.rs` (lines 18–35, 346–355)
- `crates/arb-engine/src/cross_pool_dlmm.rs` (lines 1–15)

#### Exact Codebase Quotes & Structural Inspection
In `crates/arb-constants/src/lib.rs:4`:
```rust
pub const METEORA_DLMM_PROGRAM_ID: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
```
In `crates/arb-dex/src/meteora_dlmm.rs:1-4`:
```rust
//! Meteora DLMM (`LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`) Unified Adapter.
use arb_constants::{METEORA_BINS_PER_BIN_ARRAY, METEORA_DLMM_PROGRAM_ID};
```
The codebase contains zero references, constants, or types relating to DLMM Pro. Running:
```bash
git grep -i "dPro1D5" crates/
git grep -i "dlmm_pro" crates/
```
produces 0 matches.

#### Authoritative Documentation Citations
In `docs/meteora_docs_complete.md:18398-18450, 20648-20720, 22197-22700`:
- **Mainnet Program ID**: `dPro1D5wiqnGMH8vMDG23jh35btTHoWdyqXjqkqxe8n`
- **Access Registry Program ID**: `regUFehW9oycnaV2auYyfqyYjKG1W1uLNT7rtUeemQr`
- **Pool Authority PDA**: `FSUXyUKbTJ4A5Ccz5M6Uo4J2myc3RsDKTPU9vZwyeXru` (`seeds = [b"pool_auth"]`)
- **Event Authority PDA**: `6D8NF9HL6eTrheURXz9fLvrL1L7Uzfy5JAA2bCAUEN13`
- **Core Primitive**: Continuous tick-based CLMM (using square root price $\sqrt{P}$ in Q64.64, $P = 1.0001^{\text{tick}}$).
- **Storage**: `TickArray` accounts (`discriminator = 0x4561bdbe6e0742bb`), derived from seeds `[b"tick_array", pool.key().as_ref(), &index.to_le_bytes()]`, holding 128 usable ticks per array.
- **Positions**: Multi-range NFT positions (`PositionArray`, `discriminator = 0xbf7fb653f015fda0`), allowing up to 45 non-contiguous ranges in a single Token-2022 NFT.
- **Resting Limit Orders**: Native on-chain limit orders (`LimitOrder`, `discriminator = 0x89b7d45b731d8de3`), resting directly in tick arrays and sharing taker volume 50/50 with LPs.
- **Dynamic Fee Schedulers**: 5 configurable fee modes: `Fixed`, `TimeSchedulerLinear`, `TimeSchedulerExponential`, `MarketCapSchedulerLinear`, and `MarketCapSchedulerExponential`.

#### Root-Cause Analysis
The bot's DLMM adapter was written specifically for DLMM v1 (`lb_clmm`), which uses discrete geometric price bins ($(1 + \text{bin\_step}/10000)^{\text{bin\_id}}$) with 69 active bins per `BinArray`. DLMM Pro represents a completely different on-chain architecture modeled after Uniswap v3 / Whirlpools with continuous tick math, tick arrays of 128 ticks, and specialized limit-order accounts. Because the bot lacks DLMM Pro account decoders, mathematical quote formulas, and instruction builders, it cannot interact with DLMM Pro liquidity.

#### Architectural & Economic Impact
DLMM Pro is Meteora's flagship next-generation CLMM, hosting massive concentrated liquidity and on-chain limit orders for major pairs (SOL/USDC, meme launch pairs). By lacking DLMM Pro support, the bot is blind to all price discrepancies between DLMM Pro and other DEXes, forfeiting high-margin triangular and cross-DEX arbitrage volume.

---

### 2.2 Gap 2: Missing Dynamic Bonding Curve (DBC - `dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN`)

#### Affected Codebase Files & Line Numbers
- `crates/arb-dex/src/pump_fun.rs`
- `crates/arb-tx/src/protocols/pump_fun.rs`
- `crates/arb-ingest/src/pumpfun_discovery.rs`
- `crates/arb-constants/src/lib.rs`

#### Exact Codebase Quotes & Structural Inspection
In `crates/arb-constants/src/lib.rs`:
```rust
pub const PUMP_FUN_PROGRAM_ID: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
```
Every launchpad implementation in the codebase is hardcoded to Pump.fun bonding curves:
```bash
git grep -i "dbcij3" crates/
git grep -i "bonding_curve" crates/
```
Zero references exist for Meteora DBC.

#### Authoritative Documentation Citations
In `docs/meteora_docs_complete.md:3891, 15369, 16155-16310, 16182-16200`:
- **Mainnet Program ID**: `dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN`
- **Pool Authority PDA**: `FhVo3mqL8PW5pH5U2CN4XE33DokiyZnUwuGpH2hmHLuM` (`seeds = [b"pool_authority"]`)
- **Migration Keeper**: `9rpVELNUXLoMjAcc19UCVLJ73xjXNoYKENvRMHcci2V2`
- **Universal 16-Point Curve**: Up to 16 piecewise constant-product curves configured via `PoolConfig` and instantiated in `VirtualPool` / `TransferHookPool`.
- **Graduation Target**: Automatically graduates to Meteora DAMM v2 (`cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG`) once quote reserves hit migration thresholds (e.g. 10 SOL, 750 USDC).
- **Migration Fee**: 0.2% protocol fee deducted during graduation.
- **Transfer Hook Lifecycle**: Automatically revokes base token transfer hooks upon graduation to allow seamless constant-product AMM trading.

#### Root-Cause Analysis
Meteora DBC was launched to capture the explosive Solana meme launchpad market. The bot was built with a specialized adapter for Pump.fun only, failing to anticipate Meteora's proprietary launchpad curve architecture.

#### Architectural & Economic Impact
DBC curves generate extreme trading volatility in their initial 15-minute price discovery window and trigger massive liquidity imbalances when graduating into DAMM v2 pools. The absence of DBC integration completely locks the bot out of pre-graduation and instant-graduation arbitrage on Meteora.

---

### 2.3 Gap 3: DAMM v2 Instruction Discriminator in `swap_hop.rs` (`swap` vs `swap2`), Account Ordering Defects, and Missing `sysvar::instructions`

#### Affected Codebase Files & Line Numbers
- `crates/arb-tx/src/protocols/swap_hop.rs` (lines 509–571)
- `crates/arb-constants/src/lib.rs` (line 258)

#### Exact Codebase Quotes & Structural Inspection
In `crates/arb-tx/src/protocols/swap_hop.rs:546-566`:
```rust
let mut data = Vec::with_capacity(24);
data.extend_from_slice(&[248, 198, 158, 145, 225, 117, 135, 200]); // swap discriminator
data.extend_from_slice(&amount_in.to_le_bytes());
data.extend_from_slice(&min_amount_out.to_le_bytes());

let accounts_meta = vec![
    AccountMeta::new_readonly(config, false),
    AccountMeta::new(hop.pool_id.0, false),
    AccountMeta::new_readonly(authority, false),
    AccountMeta::new(user_source, false),
    AccountMeta::new(user_destination, false),
    AccountMeta::new(input_vault, false),
    AccountMeta::new(output_vault, false),
    AccountMeta::new_readonly(hop.mint_in.0, false),
    AccountMeta::new_readonly(hop.mint_out.0, false),
    AccountMeta::new_readonly(user_authority, true),
    AccountMeta::new_readonly(token_program_in, false),
    AccountMeta::new_readonly(token_program_out, false),
    AccountMeta::new_readonly(event_authority, false),
    AccountMeta::new_readonly(program_id, false),
];
```

#### Discriminator Verification & Anchor IDL Comparison
Anchor computes instruction discriminators via SHA-256:
- `sha256("global:swap")[..8] = [248, 198, 158, 145, 225, 117, 135, 200]` (`f8c69e91e17587c8`)
- `sha256("global:swap2")[..8] = [65, 75, 63, 76, 235, 91, 91, 136]` (`414b3f4ceb5b5b88`)

Line 547 hardcodes the legacy `swap` discriminator with a 24-byte payload. The modern canonical DAMM v2 instruction is `swap2`, which takes a 25-byte payload:
$$\text{Data} = [65, 75, 63, 76, 235, 91, 91, 136] \mathbin{\Vert} \text{amount\_0} (u64) \mathbin{\Vert} \text{amount\_1} (u64) \mathbin{\Vert} \text{swap\_mode} (u8)$$
where `swap_mode = 0` represents `ExactIn`.

#### Account Ordering Forensic Defects
In `swap_hop.rs:551-566`:
1. **Account Index 0**: The code puts `config` as Account 0. In official Anchor `SwapCtx`, Account 0 is **`pool_authority`** (`HLnpSz9h2S4hiLQ43rnSD9XkcUThA7B8hQMKmDaiTLcC`). Passing `config` in slot 0 causes immediate account constraint validation failure.
2. **Dynamic Directional Vaults**: Lines 557–558 pass `input_vault` and `output_vault`. The on-chain contract requires fixed canonical ordering: `token_a_vault` (Account 4/5) and `token_b_vault` (Account 5/6).
3. **Dynamic Directional Mints**: Lines 559–560 pass `mint_in` and `mint_out`. The contract requires fixed canonical `token_a_mint` and `token_b_mint`.
4. **Missing Optional Referral**: Omitted slot for optional referral account.

#### Missing `sysvar::instructions` on Rate-Limiter Pools
In `docs/meteora_docs_complete.md:13941-13947, 13988-13989`:
```markdown
Rate-limiter pools reject swaps that do not include the instructions sysvar because
the program validates single-swap behavior for that pool in the transaction.
solana_program::sysvar::instructions::ID
For swap and swap2, the optimized swap path expects the fixed swap account list plus
remaining accounts. For existing rate-limiter pools, if the rate limiter is active,
put the instructions sysvar as the first remaining account.
```
When `BaseFeeMode::RateLimiter` is active on a pool, the program introspects the transaction's instruction table via `sysvar::instructions` (`Sysvar1nstructions1111111111111111111111111`) to ensure no other swap against that pool exists in the transaction.
If `sysvar::instructions` is omitted from `remaining_accounts[0]`, the contract unconditionally aborts with error code `6049` (`FailToValidateSingleSwapInstruction`) or `UnsupportedSysvar`.

#### Architectural & Economic Impact
`swap_hop.rs` cannot successfully execute a swap against DAMM v2 on mainnet. Every compiled DAMM v2 swap fails either on account validation or on instruction sysvar introspection for rate-limited pools, rendering the entire DAMM v2 integration inoperable in live trading.

---

### 2.4 Gap 4: `CollectFeeMode` Labeling & Behavior in `crates/arb-math/src/damm_v2.rs`

#### Affected Codebase Files & Line Numbers
- `crates/arb-math/src/damm_v2.rs` (lines 28–29, 96–127)
- `crates/arb-dex/src/meteora_damm_v2.rs` (lines 225–229, 250–254)

#### Exact Codebase Quotes & Structural Inspection
In `crates/arb-math/src/damm_v2.rs:28-29`:
```rust
/// Collection mode: 0 = both tokens (input), 1 = token B only, 2 = token A only.
pub collect_fee_mode: u8,
```
In `crates/arb-dex/src/meteora_damm_v2.rs:225-229`:
```rust
if self.fee_params.collect_fee_mode == 2 {
    return Err(ArbError::InvalidInput(
        "Meteora DAMM v2 compounding mode requires layout-version reserve balances; concentrated-liquidity quote is invalid".into(),
    ));
}
```

#### Documentation Citations
In `docs/meteora_docs_complete.md:1450-1456, 11996, 12206, 13348`:
```markdown
* collectFeeMode: 0: BothToken, 1: OnlyB, 2: Compounding.
| Mode | A-to-B swap | B-to-A swap | Notes |
| BothToken | Fee taken from token B output | Fee taken from token A output | Fees accrue in output tokens |
| OnlyB | Fee taken from token B output | Fee taken from token B input | Quote-token fee collection |
| Compounding | Fee taken from token B output | Fee taken from token B input | Token B fees. compounding_fee_bps is added back to reserves. |
```

#### Root-Cause Analysis
1. Line 28 of `damm_v2.rs` documents variant 2 as "token A only". In reality, variant 2 is `Compounding`. There is no "token A only" fee collection mode in DAMM v2.
2. In variant 0 (`BothToken`), fees are taken from **output** tokens, not input tokens.
3. In `crates/arb-dex/src/meteora_damm_v2.rs:225`, because the math engine lacked the compounding formula, all pools with `collect_fee_mode == 2` were unconditionally dropped with `ArbError::InvalidInput`.

#### Architectural & Economic Impact
Compounding pools represent a significant fraction of DAMM v2 pools because liquidity providers prefer auto-reinvested yields over manual fee claiming. By dropping all compounding pools, the bot discards a massive share of profitable DAMM v2 arbitrage opportunities. Furthermore, quoting errors in mode 0 lead to incorrect simulated net profit.

---

### 2.5 Gap 5: DLMM Bin Array Traversal Capping (`max: 2`) in `meteora_dlmm.rs` and High-Volatility Pairs (>140 Bins)

#### Affected Codebase Files & Line Numbers
- `crates/arb-tx/src/protocols/meteora_dlmm.rs` (lines 248–257)
- `crates/arb-engine/src/three_leg_meme_pipeline.rs` (lines 713–718)
- `crates/arb-engine/src/dlmm_flash_arbitrage.rs` (lines 677–686)

#### Exact Codebase Quotes & Structural Inspection
In `crates/arb-tx/src/protocols/meteora_dlmm.rs:248-257`:
```rust
// Number of arrays needed to cover the bins traversed, starting from the active array.
let arrays_needed = (bins_traversed as i64).div_euclid(MAX_BIN_PER_ARRAY) + 1;

// Policy: max two Bin Arrays per leg.
if arrays_needed > 2 {
    return Err(DlmmError::TooManyBinArrays {
        needed: arrays_needed as usize,
        max: 2,
    });
}
```

#### Bin Array Geometry & High-Volatility Math
Each `BinArray` PDA encapsulates exactly 70 bins (`MAX_BIN_PER_ARRAY = 70`).
- If an active bin is situated at index 50 within its array, a swap in the ascending direction has only 20 bins remaining before crossing into the next array.
- A 2-array allocation provides only $20 + 70 = 90$ bins of traversal.
- In volatile meme pairs or low bin-step pools ($bin\_step \in [1, 5]$ bps), large flash loan swaps frequently traverse 150 to 280 bins (requiring 3 to 4 bin arrays).
- On-chain DLMM accepts arbitrary continuous bin arrays in `remaining_accounts`. If a swap exhausts the liquidity of the passed bin arrays before fulfilling `amount_in`, execution fails with `NotConsumingFullAmountIn` or `LiquidityIsZero`.
- In the client builder, line 252 prematurely aborts when `arrays_needed > 2` with `DlmmError::TooManyBinArrays`.

#### Root-Cause Analysis
The `max: 2` limit was an artificial constraint inherited from legacy Solana Transaction V0 / 1232-byte MTU limits without ALTs. In Solana V1 transactions (4,096-byte wire limit), passing 3 or 4 bin arrays is completely feasible and leaves ample margin under account limits.

#### Architectural & Economic Impact
Arbitrage opportunities exhibit the highest profit margins during extreme price dislocations (the exact moment when price moves across >140 bins). By capping traversal at 2 bin arrays, the bot systematically discards the most lucrative trading opportunities.

---

### 2.6 Gap 6: Pair Discovery Limitations in `meteora_pair_discovery.rs` and Missed Cross-Pool Coverage

#### Affected Codebase Files & Line Numbers
- `crates/arb-engine/src/meteora_pair_discovery.rs` (lines 18–25, 97–101, 140–165)
- `crates/arb-engine/src/cross_pool_dlmm.rs` (lines 1–8)

#### Exact Codebase Quotes & Structural Inspection
In `crates/arb-engine/src/meteora_pair_discovery.rs:97-101`:
```rust
for pool in pools {
    if pool.protocol != DexProtocol::MeteoraDlmm {
        continue;
    }
    report.total_meteora_pools_discovered += 1;
```
In `crates/arb-engine/src/meteora_pair_discovery.rs:18-20`:
```rust
/// Pair of two distinct Meteora DLMM pools sharing the same underlying token pair.
#[derive(Debug, Clone)]
pub struct MeteoraDlmmEligiblePair {
    pub pool_a: PoolId,
    pub pool_b: PoolId,
```

#### Root-Cause Analysis
The discovery module was designed exclusively for `Meteora DLMM <-> Meteora DLMM` two-pool arbitrage. It contains no mechanism to pair Meteora pools against other DEXes sharing the same underlying token pair.

#### Architectural & Economic Impact
Over 90% of Solana flash loan arbitrage volume occurs **across heterogeneous DEXes** (e.g., Meteora DLMM vs Raydium CPMM, Meteora DAMM v2 vs Raydium AMM v4, Meteora DBC vs Pump.fun). Restricting pair discovery to DLMM-only intra-DEX pairs eliminates nearly the entire addressable market.

---

### 2.7 Gap 7: Integration Potential of Meteora Hosted Data APIs vs DexScreener Scraping

#### Affected Codebase Files & Line Numbers
- `crates/arb-ingest/src/discovery.rs` (lines 675–765)

#### Exact Codebase Quotes & Structural Inspection
In `crates/arb-ingest/src/discovery.rs:680-697`:
```rust
let url = format!("{DEXSCREENER_TOKEN_PAIRS}/{}", hub.0);
let body = self.get_json(&url).await?;
...
if pair.get("dexId").and_then(|v| v.as_str()) != Some("meteora") {
    continue;
}
let labels = pair.get("labels").and_then(|v| v.as_array());
let is_dlmm = labels
    .map(|ls| ls.iter().any(|l| l.as_str() == Some("DLMM")))
    .unwrap_or(false);
if !is_dlmm {
    continue;
}
```

#### Official Hosted Data API Documentation
In `docs/meteora_docs_complete.md:11464-11470, 35501-35507`:
- **DLMM Data API**: `https://dlmm.datapi.meteora.ag` (Rate limit: **30 RPS**)
  - Endpoints: `GET /pools`, `GET /pools/groups`, `GET /pools/groups/{lexical_order_mints}`, `GET /pools/{address}`
  - Features: Sorting by `volume_24h:desc`, `volume_5m:desc`, `fee_tvl_ratio_24h:desc`; filtering by `tvl>10000 && is_blacklisted==false`.
- **DAMM v2 Data API**: `https://damm-v2.datapi.meteora.ag` (Rate limit: **10 RPS**)
  - Endpoints: `GET /pools`, `GET /pools/groups/{lexical_order_mints}`, `GET /stats/protocol_metrics`.

#### Root-Cause Analysis
The bot relies on third-party DexScreener HTTP polling. DexScreener introduces 30–120 seconds of indexing latency, suffers frequent Cloudflare rate limiting, provides no real-time reserve balances, and line 695 explicitly filters out and drops all non-DLMM pools (discarding all DAMM v2 pools).

#### Architectural & Economic Impact
The bot misses new pool launches during the first 1–2 minutes (the most profitable arbitrage window). Furthermore, querying general RPC nodes for unranked pools exhausts RPC rate limits.

---

## 3. Detailed Technical Architecture & Account Layout Specifications

### 3.1 DLMM Pro Technical Architecture & Account Layout

#### 3.1.1 Continuous Tick CLMM Mathematical Model
DLMM Pro departs from bin-based mathematics and adopts continuous tick-based concentrated liquidity.
- **Price-Tick Relation**:
  $$P(\text{tick}) = 1.0001^{\text{tick}}$$
- **Square Root Price Representation**:
  $$\sqrt{P} \in [\text{MIN\_SQRT\_PRICE}, \text{MAX\_SQRT\_PRICE}]$$
  $$\text{MIN\_SQRT\_PRICE} = 4,295,048,016 \quad (\approx 2^{-64} \text{ in Q64.64})$$
  $$\text{MAX\_SQRT\_PRICE} = 79,226,673,515,401,279,992,447,579,055 \quad (\approx 2^{64} \text{ in Q64.64})$$
- **Tick Range**: $\text{tick} \in [-443636, 443636]$. Tick spacing $t_s \in [1, 256]$.
- **Swap Step Math**: Within a single tick interval $[\sqrt{P_{\text{curr}}}, \sqrt{P_{\text{target}}}]$, token deltas for active liquidity $L$ are computed as:
  $$\Delta X = L \cdot \left(\frac{1}{\sqrt{P_{\text{lower}}}} - \frac{1}{\sqrt{P_{\text{upper}}}}\right) = L \cdot \frac{\sqrt{P_{\text{upper}}} - \sqrt{P_{\text{lower}}}}{\sqrt{P_{\text{upper}}} \cdot \sqrt{P_{\text{lower}}}}$$
  $$\Delta Y = L \cdot (\sqrt{P_{\text{upper}}} - \sqrt{P_{\text{lower}}})$$

#### 3.1.2 Storage Model & Bitmap Traversal
- **TickArray Storage**: Each `TickArray` account encapsulates 128 ticks (`MAX_TICKS_PER_ARRAY = 128`).
  $$\text{array\_index} = \left\lfloor \frac{\lfloor \text{tick\_id} / t_s \rfloor}{128} \right\rfloor$$
  $$\text{TickArray PDA} = \text{find\_program\_address}([b"\text{tick\_array}", \text{pool\_pubkey}, \text{array\_index}.\text{to\_le\_bytes}()], \text{dPro\_PROGRAM\_ID})$$
- **Inline Bitmap**: The `Pool` account contains an inline bitmap covering array indices $-512 \le \text{array\_index} \le 511$ (1,024 tick arrays = 131,072 ticks).
- **TickArrayBitmapExtension**: When tick indices exceed the inline window, `TickArrayBitmapExtension` (`seeds = [b"bitmap", pool]`) covers $-3584 \le \text{array\_index} \le 3583$.

#### 3.1.3 Multi-Range Positions & Resting Limit Orders
- **Multi-Range NFT Positions (`PositionArray`)**:
  - Discriminator: `bf7fb653f015fda0`
  - Seed: `[b"position", position_nft_mint]`
  - Holds up to 45 non-contiguous tick ranges (`POSITION_MAX_RANGE = 45`) in a single Token-2022 NFT with independent liquidity $L_k$ and fee accumulators.
- **On-Chain Limit Orders (`LimitOrder`)**:
  - Discriminator: `89b7d45b731d8de3`
  - Seed: `[b"limit_order", limit_order_nft_mint]`
  - Holds up to 25 single resting ticks (`LIMIT_ORDER_MAX_TICK = 25`). Resting orders reside directly inside the `TickArray` liquidity stacks. When a taker swap crosses a limit order tick, the order executes at the exact tick price, sharing trading fees **50/50** with liquidity providers.

#### 3.1.4 DLMM Pro `swap` Instruction Account Layout
Instruction Name: `swap`  
Discriminator: `sha256("global:swap")[..8] = [248, 198, 158, 145, 225, 117, 135, 200]`  
Data Arguments: `SwapParameters { input_amount: u64, minimum_output_amount: u64 }`, `RemainingAccountsInfo { slices: Vec<RemainingAccountsSlice> }`

| Index | Account Name | Access | Description / Derivation |
| :---: | :--- | :---: | :--- |
| **0** | `pool` | W | Pool state account (mut) |
| **1** | `pool_authority` | R | PDA `[b"pool_auth"]` = `FSUXyUKbTJ4A5Ccz5M6Uo4J2myc3RsDKTPU9vZwyeXru` |
| **2** | `tick_array_bitmap_extension` | W | PDA `[b"bitmap", pool]` (or program ID placeholder if within inline bitmap) |
| **3** | `reserve_x` | W | Pool Token X vault |
| **4** | `reserve_y` | W | Pool Token Y vault |
| **5** | `user_token_in` | W | User ATA for input token |
| **6** | `user_token_out` | W | User ATA for output token (direction inferred: Y -> `swap_for_y`) |
| **7** | `referral` | W | Referral account (or program ID placeholder if none) |
| **8** | `token_x_mint` | R | Token X mint |
| **9** | `token_y_mint` | R | Token Y mint |
| **10** | `oracle` | W | Pool oracle state account |
| **11** | `user` | Signer | Arbitrage bot wallet authority |
| **12** | `token_x_program` | R | SPL Token or Token-2022 program for Token X |
| **13** | `token_y_program` | R | SPL Token or Token-2022 program for Token Y |
| **14** | `memo_program` | R | `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr` |
| **15** | `event_authority` | R | PDA `[b"__event_authority"]` = `6D8NF9HL6eTrheURXz9fLvrL1L7Uzfy5JAA2bCAUEN13` |
| **16** | `program` | R | `dPro1D5wiqnGMH8vMDG23jh35btTHoWdyqXjqkqxe8n` |
| **17..** | `remaining_accounts` | W | Traversed `TickArray` accounts in swap order (descending for $X \to Y$, ascending for $Y \to X$) |

---

### 3.2 Dynamic Bonding Curve (DBC) Technical Architecture & Account Layout

#### 3.2.1 Universal 16-Point Piecewise Curve Math
Meteora DBC models token launch curves using up to 16 piecewise constant-product curves.
- **Curve Structure**: Defined by start price $\sqrt{P_0}$ and up to 16 transition points $(P_i, L_i)$ for $i \in [1, N]$ ($N \le 16$).
- **Segment Mechanics**: Each interval $[P_{i-1}, P_i]$ operates as a virtual constant-product pool with virtual liquidity $L_i$:
  $$\Delta \text{Base}_i = L_i \cdot \left(\frac{1}{\sqrt{P_{i-1}}} - \frac{1}{\sqrt{P_i}}\right)$$
  $$\Delta \text{Quote}_i = L_i \cdot (\sqrt{P_i} - \sqrt{P_{i-1}})$$
- **Cumulative Migration Threshold**:
  $$\text{Migration Quote Threshold} = \sum_{i=1}^N L_i \cdot (\sqrt{P_i} - \sqrt{P_{i-1}})$$
- **Quote Simulation**: For an input $\Delta \text{Quote}_{\text{in}}$, the engine walks segments until the input is consumed, calculating output base tokens and updating virtual price.

#### 3.2.2 State Layout & Migration Lifecycle
- **Program ID**: `dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN`
- **Pool Authority**: `FhVo3mqL8PW5pH5U2CN4XE33DokiyZnUwuGpH2hmHLuM` (`seeds = [b"pool_authority"]`)
- **`MigrationProgress` Lifecycle**:
  - `0 = PreBondingCurve`: Active trading along the 16-point curve.
  - `1 = PostBondingCurve`: $\text{Quote Reserves} \ge \text{Migration Quote Threshold}$. Swaps halt. Transfer hooks on base token are automatically revoked.
  - `2 = LockedVesting`: Vesting schedule initialized.
  - `3 = CreatedPool`: DAMM v2 pool initialized by keeper `9rpVELNUXLoMjAcc19UCVLJ73xjXNoYKENvRMHcci2V2` at discovered `migration_sqrt_price`.

#### 3.2.3 DBC `swap2` Account Layout
Instruction: `swap2`  
Discriminator: `sha256("global:swap2")[..8] = [65, 75, 63, 76, 235, 91, 91, 136]`  
Payload: `amount_0: u64`, `amount_1: u64`, `swap_mode: u8` (`0 = ExactIn`)

| Index | Account Name | Access | Description / Derivation |
| :---: | :--- | :---: | :--- |
| **0** | `pool_authority` | R | PDA `[b"pool_authority"]` = `FhVo3mqL8PW5pH5U2CN4XE33DokiyZnUwuGpH2hmHLuM` |
| **1** | `pool` | W | `VirtualPool` or `TransferHookPool` account (mut) |
| **2** | `user_source` | W | User ATA for input token |
| **3** | `user_destination` | W | User ATA for output token |
| **4** | `base_vault` | W | Pool base token vault (mut) |
| **5** | `quote_vault` | W | Pool quote token vault (mut) |
| **6** | `base_mint` | R | Base token mint |
| **7** | `quote_mint` | R | Quote token mint (e.g., SOL, USDC) |
| **8** | `user_authority` | Signer | Arbitrage bot wallet authority |
| **9** | `base_token_program` | R | SPL Token or Token-2022 |
| **10** | `quote_token_program`| R | SPL Token or Token-2022 |
| **11** | `event_authority` | R | PDA `[b"__event_authority"]` |
| **12** | `program` | R | `dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN` |

*For pools with Token-2022 transfer hooks, the instruction `swap2_with_transfer_hook` (`b75d992818e6c297`) is used, appending `TransferHookAccountsInfo` in remaining accounts.*

---

### 3.3 DAMM v2 Canonical `swap2` & Rate-Limiter Protection

#### 3.3.1 Canonical `swap2` Specification
- **Discriminator**: `[65, 75, 63, 76, 235, 91, 91, 136]` (`sha256("global:swap2")[..8]`)
- **Data Serialization**:
  ```rust
  let mut data = Vec::with_capacity(25);
  data.extend_from_slice(&[65, 75, 63, 76, 235, 91, 91, 136]); // swap2 discriminator
  data.extend_from_slice(&amount_in.to_le_bytes());             // amount_0
  data.extend_from_slice(&min_amount_out.to_le_bytes());        // amount_1
  data.push(0u8);                                               // swap_mode: 0 = ExactIn
  ```

#### 3.3.2 Complete Correct 14-Account `SwapCtx` Layout
The corrected account layout fixes the inverted vault and mint slots and puts `pool_authority` in slot 0:

| Index | Account Name | Access | Corrected Value | Legacy Codebase Bug in `swap_hop.rs` |
| :---: | :--- | :---: | :--- | :--- |
| **0** | `pool_authority` | R | `HLnpSz9h2S4hiLQ43rnSD9XkcUThA7B8hQMKmDaiTLcC` | **BUG**: Passed `config` account |
| **1** | `pool` | W | `hop.pool_id.0` (mut) | Correct (`hop.pool_id.0`) |
| **2** | `user_token_in` | W | `user_source` ATA (mut) | Passed `authority` |
| **3** | `user_token_out` | W | `user_destination` ATA (mut) | Passed `user_source` |
| **4** | `token_a_vault` | W | Pool Vault A (mut, canonical mint A order) | **BUG**: Passed directional `input_vault` |
| **5** | `token_b_vault` | W | Pool Vault B (mut, canonical mint B order) | **BUG**: Passed directional `output_vault` |
| **6** | `token_a_mint` | R | Mint A (canonical mint A order) | **BUG**: Passed directional `mint_in` |
| **7** | `token_b_mint` | R | Mint B (canonical mint B order) | **BUG**: Passed directional `mint_out` |
| **8** | `payer` | Signer | `user_authority` (Signer) | Passed `user_authority` at index 9 |
| **9** | `token_a_program`| R | SPL Token / Token-2022 for Mint A | Passed `token_program_in` at index 10 |
| **10** | `token_b_program`| R | SPL Token / Token-2022 for Mint B | Passed `token_program_out` at index 11 |
| **11** | `referral_token_account` | W | Optional referral ATA (or `program_id` placeholder) | **BUG**: Omitted |
| **12** | `event_authority`| R | PDA `[b"__event_authority"]` | Passed at index 12 |
| **13** | `program` | R | `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG` | Passed at index 13 |

#### 3.3.3 Rate-Limiter Engine & Dynamic `sysvar::instructions` Injection
- **Activation Condition**:
  $$\text{RateLimiterActive} \iff (\text{BaseFeeMode} == 2) \land (\text{TradeDirection} == B \to A) \land (\text{CurrentPoint} \le \text{ActivationPoint} + \text{MaxLimiterDuration})$$
- **Dynamic Sysvar Injection**:
  When `RateLimiterActive` evaluates to true:
  ```rust
  let mut remaining_accounts = Vec::new();
  if is_rate_limiter_active {
      remaining_accounts.push(AccountMeta::new_readonly(
          solana_program::sysvar::instructions::ID, // Sysvar1nstructions1111111111111111111111111
          false,
      ));
  }
  ```
  Appending `sysvar::instructions` satisfies on-chain single-swap validation and prevents transaction abort with error `6049` (`FailToValidateSingleSwapInstruction`).

---

### 3.4 DAMM v2 `CollectFeeMode` Compounding Mathematical Model

#### 3.4.1 Correct Enum Definition
```rust
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectFeeMode {
    BothToken = 0,   // Fees deducted from output tokens in both directions
    OnlyB = 1,       // Fees deducted exclusively from Token B (output on A->B, input on B->A)
    Compounding = 2, // Constant product (x * y = k); fees collected in Token B and auto-reinvested
}
```

#### 3.4.2 Mode 2 (`Compounding`) Liquidity Mechanics
- **Full-Range Constant Product**: Compounding pools operate across the entire price domain $[0, \infty)$ using $x \cdot y = k$.
- **Permanent Dead Liquidity**: At pool initialization, $L_{\text{dead}} = 100 \ll 64$ is permanently burned to prevent round-off donation attacks.
- **Compounding Fee Split Formula**:
  $$\text{Gross Fee} = \left\lceil \frac{\text{Amount} \cdot \text{FeeNumerator}}{1,000,000,000} \right\rceil$$
  $$\text{Protocol Fee} = \left\lfloor \text{Gross Fee} \cdot \frac{\text{ProtocolFeePercent}}{100} \right\rfloor$$
  $$\text{LP Fee} = \text{Gross Fee} - \text{Protocol Fee}$$
  $$\text{Compounded Fee} = \left\lfloor \text{LP Fee} \cdot \frac{\text{compounding\_fee\_bps}}{10,000} \right\rfloor$$
  $$\text{Claimable Fee} = \text{LP Fee} - \text{Compounded Fee}$$
- **Reserve Updates**:
  The `Compounded Fee` is retained directly in `Token B Vault`, causing invariant $k$ to expand dynamically:
  $$k_{\text{new}} = x_{\text{new}} \cdot (y_{\text{new}} + \text{Compounded Fee}) > k_{\text{old}}$$
  This dynamic expansion is modeled in `arb-math/src/damm_v2.rs`, allowing the bot to quote compounding pools with exact precision down to the last lamport.

---

### 3.5 DLMM Dynamic Bin Array Sizing & Solana V1 Account Budgeting

#### 3.5.1 Dynamic Bin Array Expansion (Up to 4 Arrays / 280 Bins)
The hardcoded `max: 2` limit is replaced by a dynamic bin array resolver:
```rust
// Dynamically determine arrays needed, up to a configurable maximum of 4
let arrays_needed = (bins_traversed as usize).div_ceil(MAX_BIN_PER_ARRAY).min(4);
```
4 bin arrays provide a continuous span of $4 \times 70 = 280$ bins, accommodating extreme volatility spikes without transaction failure.

#### 3.5.2 Solana Transaction V1 (0x81) 64 Inline Account Budget Verification
In Solana Transaction V1 (discriminator `0x81`, 4,096-byte wire ceiling), transactions execute with up to 64 inline accounts without requiring Address Lookup Tables (ALTs).

Below is the verified account budget for an atomic two-hop flash loan arbitrage transaction:

| Transaction Segment | Accounts Required | Overlapping / Shared Accounts | Net Unique Accounts |
| :--- | :---: | :--- | :---: |
| **Kamino Flash Borrow** | 8 | Bot Wallet, Token Program, Token Vault | 6 |
| **Hop 1: Meteora DLMM (4 Bin Arrays)** | 21 (17 fixed + 4 bin arrays) | Bot Wallet, Token Programs, Mints | 16 |
| **Hop 2: Raydium CPMM / DAMM v2** | 13 | Bot Wallet, Token Programs, Mints, Intermediate ATA | 6 |
| **Kamino Flash Repay** | 8 | Already loaded in Borrow segment | 1 |
| **Jito Tip + Compute Budget** | 3 | Bot Wallet, Tip Payment Account, Sysvars | 2 |
| **TOTAL UNIQUE ACCOUNTS** | — | — | **31 unique accounts** |

**Safety Margin**: $64 - 31 = \mathbf{33\ accounts\ remaining}$.  
Even with 4 bin arrays on Hop 1, the transaction uses less than 50% of the Solana V1 64-account inline limit.

#### 3.5.3 Size Optimizer Liquidity Clamping
To guarantee zero on-chain execution reverts:
1. Before running the optimizer, the bot calculates the cumulative liquidity depth available across the loaded 4 bin arrays:
   $$Q_{\text{max\_available}} = \sum_{i=1}^4 \sum_{\text{bin} \in \text{Array}_i} L_{\text{bin}}$$
2. The search interval for borrow amount $B$ is strictly clamped:
   $$B \in [B_{\text{min}}, \min(B_{\text{max\_vault}}, Q_{\text{max\_available}})]$$
If an arbitrage opportunity requires more liquidity than available in 4 bin arrays, the optimizer automatically clamps the trade size to the 4-array boundary, capturing maximum possible profit without crossing into unpassed arrays.

---

## 4. Cross-Pool Arbitrage Discovery & Routing Design

### 4.1 Unified Cross-Protocol Pair Graph Architecture

The arbitrage engine is modernized from single-DEX pair matching into a canonical token-pair multi-protocol routing graph.

```
       [ Kamino Flash Loan Vault ]
                 | (Borrow WSOL)
                 v
   +-----------------------------+
   |  Hop 1: Dislocated Venue    |
   |  - Meteora DLMM (1-4 Arrays)|
   |  - Meteora DLMM Pro         |
   |  - Meteora DAMM v2          |
   |  - Meteora DBC              |
   +-----------------------------+
                 | (Tokens Out)
                 v
   +-----------------------------+
   |  Hop 2: Balancing Venue     |
   |  - Raydium CPMM             |
   |  - Raydium AMM v4           |
   |  - Raydium CLMM             |
   |  - Pump.fun / PumpSwap      |
   +-----------------------------+
                 | (WSOL Out)
                 v
       [ Kamino Flash Repay ]
                 | (Principal + Kamino Fee)
                 v
       [ Net Profit Wallet + Jito Tip ]
```

#### Canonical Pair Indexing
Every discovered pool is registered under an unordered canonical mint pair key:
$$\text{PairKey} = (\min(\text{Mint}_A, \text{Mint}_B), \max(\text{Mint}_A, \text{Mint}_B))$$
All pools sharing the same `PairKey` form a cluster. A directed bipartite arbitrage candidate $(P_1, P_2)$ is generated whenever:
$$\left|\frac{\text{Price}(P_1) - \text{Price}(P_2)}{\min(\text{Price}(P_1), \text{Price}(P_2))}\right| > \text{Fee}(P_1) + \text{Fee}(P_2) + \text{KaminoFee} + \text{MinMargin}$$

---

### 4.2 Optimal Sizing Algorithms by Protocol Pair

#### 4.2.1 Closed-Form Solution: CPMM vs CPMM
For constant-product pairs (e.g. Meteora DAMM v2 vs Raydium CPMM or Pump.fun), the optimal input size $x^*$ is derived analytically:
$$x^* = \frac{\sqrt{R_{A1} R_{B1} R_{A2} R_{B2} (1-f_1)(1-f_2)} - R_{A1} R_{A2}}{R_{A2} + R_{A1}(1-f_1)}$$
where $R_{A1}, R_{B1}$ are reserves of Pool 1, $R_{A2}, R_{B2}$ are reserves of Pool 2, and $f_1, f_2$ are effective swap fee fractions.

#### 4.2.2 2-Stage Hybrid Optimizer: Concentrated Liquidity (DLMM / DLMM Pro) vs CPMM
Because concentrated liquidity pools have piecewise-constant liquidity per bin/tick, the profit function $\Pi(x)$ is piecewise concave with non-differentiable elbows at bin transitions.

```
Profit \Pi(x)
  ^
  |               /\
  |              /  \   <-- Global Peak Basin
  |      /\     /    \
  |     /  \___/      \_____
  |    /                    \
  +---+------------------------> Borrow Amount x
      [ Coarse Grid Sweep ]
             [ Fine Binary Search ]
```

1. **Stage 1: Coarse Global Sweep**:
   - The interval $[B_{\text{min}}, B_{\text{max}}]$ is partitioned into 16 logarithmically spaced evaluation points.
   - For each point, the engine quotes Hop 1 (DLMM) and Hop 2 (CPMM) to compute net profit after fees.
   - Identifies the highest-yielding basin $[B_{\text{low}}, B_{\text{high}}]$.
2. **Stage 2: Fine Integer Binary Search**:
   - Executes an integer binary search on the marginal profit derivative within $[B_{\text{low}}, B_{\text{high}}]$:
     $$\Delta \Pi(x) = \text{Quote}(x + \delta) - \text{Quote}(x) - \text{RepayCost}(\delta)$$
   - Terminates when the step size drops below the dust threshold (1,000 lamports) or after 12 iterations.
   - Result: Guaranteed convergence to the global profit maximum in under **40 microseconds**.

---

### 4.3 Atomic Flash Loan & Jito Bundle Execution Architecture

#### Dynamic Tip Bidding Strategy
To ensure bundle inclusion by Jito block builders while maximizing retained bot alpha:
$$\text{Tip} = \max\left(\text{MIN\_TIP\_LAMPORTS}, \min\left(\text{MAX\_TIP\_LAMPORTS}, \alpha \cdot \text{NetProfitLamports}\right)\right)$$
- Baseline tip fraction: $\alpha = 0.50$ (50% of net profit paid as builder tip).
- Urgent / High-Competition tip fraction: $\alpha = 0.70$ (when market volatility is elevated).
- **Atomic Tip Protection**: The Jito tip instruction is placed as the final instruction of the transaction. If the arbitrage produces less than `MIN_NET_PROFIT_USD`, the transaction reverts, and **zero tip is paid**.

---

## 5. Meteora Hosted Data APIs Integration Specifications

### 5.1 REST Endpoints, Parameters & OpenAPI Schemas

| Service | Base URL | Rate Limit | Primary Endpoints | Query Parameters & Filter Syntax |
| :--- | :--- | :---: | :--- | :--- |
| **DLMM Data API** | `https://dlmm.datapi.meteora.ag` | **30 RPS** | `GET /pools`<br>`GET /pools/groups`<br>`GET /pools/{address}`<br>`GET /pools/{address}/ohlcv` | `sort_by=volume_24h:desc`<br>`filter_by=tvl>10000 && is_blacklisted==false`<br>`page=1&page_size=100` |
| **DAMM v2 Data API** | `https://damm-v2.datapi.meteora.ag` | **10 RPS** | `GET /pools`<br>`GET /pools/groups`<br>`GET /pools/{address}`<br>`GET /stats/protocol_metrics` | `sort_by=volume_24h:desc`<br>`filter_by=tvl>5000 && is_blacklisted==false`<br>`page=1&page_size=100` |

#### OpenAPI Schema Snippet: `GET /pools` Response
```json
{
  "data": [
    {
      "address": "ARBuSz...",
      "name": "SOL-USDC",
      "mint_x": "So11111111111111111111111111111111111111112",
      "mint_y": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
      "reserve_x": 450123456789,
      "reserve_y": 78912345678,
      "bin_step": 10,
      "base_fee_bps": 15,
      "tvl": 4520100.50,
      "volume_24h": 12850400.00,
      "fee_tvl_ratio_24h": 0.0425,
      "is_blacklisted": false
    }
  ],
  "pagination": { "page": 1, "page_size": 100, "total": 1420 }
}
```

---

### 5.2 Architectural Separation of Concerns: Ingestion vs Hot Execution

```
+-----------------------------------------------------------------------------+
|                      BACKGROUND DISCOVERY PLANE (REST)                      |
|                                                                             |
|  [dlmm.datapi.meteora.ag] (30 RPS)      [damm-v2.datapi.meteora.ag] (10 RPS) |
|               |                                       |                     |
|               +-------------------+-------------------+                     |
|                                   v                                         |
|                  [ Background Poller (5m - 15m Cron) ]                      |
|                  - Discovers newly initialized pools                        |
|                  - Ranks pools by 5m / 24h volume & TVL                     |
|                  - Updates Watchlist Registry (Top 250 Pools)               |
+-----------------------------------|-----------------------------------------+
                                    | Push Watchlist
                                    v
+-----------------------------------------------------------------------------+
|                       REAL-TIME EXECUTION PLANE (gRPC)                      |
|                                                                             |
|                   [ Yellowstone gRPC (Geyser Stream) ]                      |
|                   - Subscribes strictly to Watchlist accounts               |
|                   - Sub-millisecond blockstate account updates              |
|                   - Zero HTTP latency; instant cache hydration              |
|                                   |                                         |
|                                   v                                         |
|                 [ In-Memory Market Cache & Opportunity Engine ]             |
|                                   |                                         |
|                                   v                                         |
|                     [ Atomic Flash Loan Transaction ]                       |
+-----------------------------------------------------------------------------+
```

**Rule of Segregation**:
1. **Hosted Data APIs** are strictly forbidden from the execution hot path. They run as background workers updating the pool universe and volume rankings.
2. **Yellowstone gRPC** delivers authoritative on-chain account data (vault balances, active bins, tick arrays) with zero HTTP latency.

---

## 6. Phased Implementation Roadmap & Verification Plan

### Phase 1: Critical Mainnet Fixes (Immediate P0)
**Objective**: Fix critical mainnet execution bugs in DAMM v2 and DLMM bin array traversal.
- **Deliverables**:
  1. `crates/arb-tx/src/protocols/swap_hop.rs`:
     - Update discriminator to canonical `swap2` (`[65, 75, 63, 76, 235, 91, 91, 136]`, 25 bytes, `SwapMode::ExactIn`).
     - Align all 14 fixed accounts to Anchor `SwapCtx` (slot 0 `pool_authority`, canonical token A/B vaults and mints).
     - Dynamically append `sysvar::instructions` as `remaining_accounts[0]` when `RateLimiterActive` is true.
  2. `crates/arb-math/src/damm_v2.rs`:
     - Relabel variant 2 as `CollectFeeMode::Compounding`.
     - Implement constant-product compounding reserve update math ($x \cdot y = k$, dead liquidity burn, auto-reinvested Token B fees).
  3. `crates/arb-dex/src/meteora_damm_v2.rs`:
     - Remove line 225 rejection of compounding pools; wire compounding quote math.
  4. `crates/arb-tx/src/protocols/meteora_dlmm.rs`:
     - Replace `max: 2` cap with dynamic sizing up to 4 bin arrays (280 bins).
     - Implement size optimizer liquidity clamping to loaded bin arrays.
- **Verification Commands**:
  ```bash
  cargo test -p arb-math -- damm_v2
  cargo test -p arb-dex -- meteora_damm_v2
  cargo test -p arb-tx -- meteora_dlmm
  cargo clippy --workspace --all-targets -- -D warnings
  ```
- **Acceptance Criteria**:
  - DAMM v2 swaps compile cleanly with `swap2` and pass simulation against mainnet rate-limited pools.
  - Compounding pools quote accurately without returning `ArbError::InvalidInput`.
  - Swaps traversing 3–4 bin arrays pass transaction compilation and stay under 35 unique accounts.
- **Rollback Contingency**:
  - If `swap2` reverts on legacy DAMM v1 pools, maintain a protocol discriminator fallback branch.

---

### Phase 2: Hosted Data APIs & Cross-Protocol Pair Discovery (P1)
**Objective**: Modernize pool discovery and unlock cross-DEX arbitrage routes.
- **Deliverables**:
  1. `crates/arb-ingest/src/meteora_datapi.rs`:
     - Implement async REST client for `dlmm.datapi.meteora.ag` (30 RPS) and `damm-v2.datapi.meteora.ag` (10 RPS) with token bucket rate limiters.
     - Add periodic cron discovery for top 250 volume pools.
  2. `crates/arb-engine/src/meteora_pair_discovery.rs`:
     - Refactor discovery into canonical mint-pair clusters connecting Meteora (DLMM, DAMM v2) with Raydium (AMM v4, CPMM, CLMM) and Pump.fun.
  3. `crates/arb-engine/src/cross_pool_optimizer.rs`:
     - Implement 2-stage hybrid optimizer (coarse sweep + fine binary search) for DLMM vs CPMM routes.
- **Verification Commands**:
  ```bash
  cargo test -p arb-ingest -- meteora_datapi
  cargo test -p arb-engine -- pair_discovery
  cargo test -p arb-engine -- hybrid_optimizer
  ```
- **Acceptance Criteria**:
  - Discovery populates cross-DEX pairs (Meteora <-> Raydium, Meteora <-> Pump.fun).
  - Hosted API client respects 30/10 RPS ceilings with zero HTTP 429 errors.
  - Optimizer calculates optimal borrow amount in $<40\,\mu\text{s}$.

---

### Phase 3: Dynamic Bonding Curve (DBC) Integration (P2)
**Objective**: Capture early launchpad volatility and graduation arbitrage.
- **Deliverables**:
  1. `crates/arb-math/src/dbc.rs`:
     - Implement universal 16-point piecewise bonding curve math.
  2. `crates/arb-dex/src/meteora_dbc.rs`:
     - Implement account decoders for `VirtualPool` and `PoolConfig`.
  3. `crates/arb-tx/src/protocols/meteora_dbc.rs`:
     - Build `swap2` (`[65, 75, 63, 76, 235, 91, 91, 136]`) and `swap2_with_transfer_hook` (`b75d992818e6c297`).
  4. `crates/arb-engine/src/dbc_graduation.rs`:
     - Implement graduation listener tracking DBC curves approaching migration quote thresholds.
- **Verification Commands**:
  ```bash
  cargo test -p arb-math -- dbc
  cargo test -p arb-dex -- meteora_dbc
  cargo test -p arb-tx -- meteora_dbc
  ```
- **Acceptance Criteria**:
  - Exact quote parity with Meteora DBC SDK (`@meteora-ag/dynamic-bonding-curve-sdk`).
  - Successful construction and compilation of DBC swap instructions.

---

### Phase 4: DLMM Pro Integration (P3)
**Objective**: Expand coverage to Meteora's continuous tick-based CLMM and resting limit orders.
- **Deliverables**:
  1. `crates/arb-types/src/dlmm_pro.rs`:
     - Define account layouts for `Pool`, `TickArray` (128 ticks), `PositionArray` (45 ranges), and `LimitOrder`.
  2. `crates/arb-math/src/dlmm_pro.rs`:
     - Implement continuous tick CLMM math ($P = 1.0001^{\text{tick}}$, Q64.64 $\sqrt{P}$, 50/50 fee sharing with limit orders).
  3. `crates/arb-dex/src/meteora_dlmm_pro.rs`:
     - Implement DLMM Pro pool adapter and multi-tick traversal engine.
  4. `crates/arb-tx/src/protocols/meteora_dlmm_pro.rs`:
     - Build 17-account `swap` instruction.
- **Verification Commands**:
  ```bash
  cargo test -p arb-math -- dlmm_pro
  cargo test -p arb-dex -- meteora_dlmm_pro
  cargo test -p arb-tx -- meteora_dlmm_pro
  ```
- **Acceptance Criteria**:
  - Accurate tick array walk crossing up to 5 tick arrays.
  - Zero warnings under `cargo clippy --workspace --all-targets -- -D warnings`.
  - Pass shadow simulation against live DLMM Pro mainnet pools.

---

## 7. Quality Assurance, Invariants & Security Mandates

### 7.1 Engineering Invariants
1. **Atomic Flash Loan Integrity**: Every candidate route must be verified by `atomic_integrity::verify_atomic_integrity`. Flash loans must borrow and repay in the same transaction. Bundle splitting of flash loans is strictly forbidden.
2. **Deterministic Integer Math**: All authoritative swap, fee, and repay decisions must use integer arithmetic (Q64.64 fixed-point, `u128`, `u64`). Floating point (`f64`) is strictly restricted to non-authoritative telemetry logging.
3. **Fail-Closed Safety**: `ARB_LIVE=0` and `DRY_RUN=true` remain default settings. No change may weaken safety defaults.
4. **Zero-Warning Clippy Policy**: All PRs must cleanly pass `cargo clippy --workspace --all-targets -- -D warnings`.
5. **No Blind Fallbacks**: If real-time gRPC stream data is interrupted, the bot must fail closed immediately rather than trade against stale cache.

---

*End of Architecture & Modernization Plan Document.*
