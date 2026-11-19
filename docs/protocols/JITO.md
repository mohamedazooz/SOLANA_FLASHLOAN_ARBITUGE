# PROTOCOL SPECIFICATION — JITO BLOCK ENGINE & MEV PROTECTION

## 1. Protocol Architecture & Purpose

The Jito Block Engine provides private bundle submission and MEV protection on Solana:
1. **Private Mempool:** Transactions submitted via Jito bundles do not traverse the public Solana gossip network, preventing sandwich attacks and predatory frontrunning.
2. **All-or-Nothing Inclusion:** A Jito bundle of up to 5 transactions is executed atomically by the validator leader. If any transaction in the bundle fails, the entire bundle is dropped.
3. **Tip Payment:** Tips are transferred to one of 8 verified Jito tip accounts via a native SOL transfer instruction appended to the transaction.

## 2. Verified Jito Tip Accounts

The 8 official Jito tip accounts hardcoded in `arb-constants`:
1. `96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5`
2. `HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe`
3. `Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY`
4. `ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49`
5. `DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh`
6. `ADuUkR4vqLUMWXxW9gh6D6L8pWHLnjvnwf7bzzcfjFr9`
7. `DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL`
8. `3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT`

- **Selection Logic:** Deterministically selected using slot-based hashing (`jito_tip_account_for_slot`) or round-robin selection.

## 3. Regional Block Engine Endpoints

- **Global:** `https://mainnet.block-engine.jito.wtf/api/v1/bundles`
- **Frankfurt (EU):** `https://frankfurt.mainnet.block-engine.jito.wtf/api/v1/bundles`
- **Amsterdam (EU):** `https://amsterdam.mainnet.block-engine.jito.wtf/api/v1/bundles`
- **New York (US East):** `https://ny.mainnet.block-engine.jito.wtf/api/v1/bundles`
- **Tokyo (Asia):** `https://tokyo.mainnet.block-engine.jito.wtf/api/v1/bundles`

## 4. Tip Pricing & Bidding Model

The tip is priced dynamically to win blockspace while preserving net profitability:
1. **Floor Tip:** Minimum 10,000 lamports (`MIN_JITO_TIP_LAMPORTS = 10_000`).
2. **Dynamic Ladder:** Scales with market contention percentiles (p50, p75, p95) reported by `TipOracle`.
3. **Hard Profit Cap:** The tip is clamped to a maximum share of gross arbitrage profit (typically $\le 50\%$). Under no circumstance may the tip exceed the expected profit of the trade.
$$\text{tip\_lamports} = \min\left(\text{profit} \times \text{max\_tip\_share\_bps} / 10000, \max(\text{floor}, \text{oracle\_p50})\right)$$
