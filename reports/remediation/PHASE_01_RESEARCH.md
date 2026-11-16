# Phase One Research — Upstream Protocols & Official Execution Specifications

## 1. Upstream Official Documentation & Standards

### 1.1 Jito MEV Infrastructure
- **Official Docs**: `https://docs.jito.wtf/`
- **Low Latency Send**: `https://docs.jito.wtf/lowlatencytxnsend/`
- **Bundles & API**: `https://docs.jito.wtf/bundles/`
- **Searcher Examples**: `https://github.com/jito-labs/searcher-examples`

#### Exact Jito Product Distinctions:
1. **Classic Jito Block Engine (JSON-RPC / REST / WebSocket)**:
   - Methods: `sendBundle`, `getBundleStatuses`, `getInflightBundleStatuses`, `getTipAccounts`.
   - Bundles: 1 to 5 transactions executed sequentially and atomically within a single block.
   - Auction Floor: 1,000 lamports minimum tip accepted by the Block Engine.
   - Tip Transfer: Transfer to one of 8 verified on-chain Jito tip accounts.
2. **Jito gRPC Searcher API**:
   - High-throughput streaming interface for block-engine connectivity. Provides sub-millisecond bundle ingestion and real-time bundle state streaming.
3. **Direct SWQoS (Stake-Weighted Quality of Service)**:
   - Unbundled transaction forwarding via RPC nodes with delegated validator stake. Does NOT provide bundle atomicity or tip auction dynamics. Priority is determined purely by micro-lamports per CU and validator stake weight.
4. **Ordinary Solana RPC**:
   - Standard TPU forwarding. High drop rates during congested slots without priority fees or stake-weighting.

### 1.2 Solana Fee Structure & Compute Budget
- **Official Fee Docs**: `https://solana.com/docs/core/fees`
- **Compute Budget**: `https://solana.com/docs/core/fees/compute-budget`
- **Transactions V0 & V1**: `https://solana.com/docs/core/transactions`
- **SIMD Specifications**:
  - `SIMD-0160`: Compute budget configuration improvements.
  - `SIMD-0296`: Micro-lamport compute budget priority fees.
  - `SIMD-0385`: Transaction V1 wire format (discriminator `0x81`, 4,096-byte MTU, 64 inline accounts, message-level priority fee).

#### Protocol Fee Math Rules:
1. **Base Transaction Fee**:
   - 5,000 lamports per signature (`DEFAULT_BASE_FEE_LAMPORTS`).
   - Charged regardless of transaction success or execution revert.
2. **Priority Fee (Legacy & V0)**:
   - Configured via `ComputeBudgetInstruction::set_compute_unit_price(micro_lamports)`.
   - `PriorityFeeLamports = ceil(micro_lamports * requested_cu_limit / 1_000_000)`.
   - The fee is paid based on the **requested limit**, NOT actual units consumed.
3. **Priority Fee (Transaction V1)**:
   - Message-level configuration mask (`CONFIG_MASK_PRIORITY_FEE`).
   - The value encoded in the V1 message is an **absolute total in lamports**, not micro-lamports per CU.
4. **Runtime Limits**:
   - Protocol max CU per transaction: 1,400,000 units.
   - Application safety ceiling: 350,000 units (prevents runaway priority fee consumption on complex routes).

### 1.3 Statistical Modeling for MEV Inclusion
- **Percentile Distributions vs Inclusion Probability**:
  - The published Jito tip floor percentiles ($P_{50}, P_{75}, P_{90}, P_{95}, P_{99}$) represent the distribution of landed bids in previous blocks across the network.
  - Conflating $P_{50}$ with a 50% probability of landing an arbitrary arbitrage trade is mathematically flawed because MEV transactions compete for identical state locks (write-lock contention on hot AMM pools).
  - Proper statistical treatment:
    - Default prior for unobserved rungs should be bounded conservatively (e.g. 0.20 to 0.50 instead of 0.99).
    - As empirical observations accumulate, the Wilson score lower bound at 95% confidence ($\alpha = 0.05$) calibrates the true conditional landing rate:
      $$w = \frac{\hat{p} + \frac{z^2}{2n} - z \sqrt{\frac{\hat{p}(1-\hat{p})}{n} + \frac{z^2}{4n^2}}}{1 + \frac{z^2}{n}}$$
    - Whenever a bid is capped or clipped, the probability must be re-evaluated for the actual submitted bid rather than preserving the upper rung's probability.
