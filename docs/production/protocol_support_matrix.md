# Protocol Support & Capability Matrix

## Supported Protocols & Verified Capabilities

| Protocol | Program ID | Supported Pools | Quote Model | Account Order Verified | Differential Parity |
|---|---|---|---|---|---|
| **Meteora DLMM** | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` | Concentrated Bin Arrays | Exact Bin Traversal (Limit Orders Folded) | YES (`lb_clmm` 0.12.0) | PASS vs Official SDK Fixtures |
| **Kamino Flash Loans** | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` | Main Market Reserves | `U68F60` Integer klend fee math | YES (12 Anchor Metas) | PASS (monotonic & bounds checked) |
| **Raydium CPMM** | `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C` | Standard AMM Pairs | Exact Constant Product with Fee Deductions | YES | PASS |
| **Raydium CLMM** | `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK` | Concentrated Liquidity | Tick Array Walk | YES | PASS |
| **Raydium V4** | `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8` | Legacy OpenBook Hybrid | OpenBook Orderbook & Reserve Ratios | YES | PASS |
| **Orca Whirlpools** | `whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc` | Concentrated Pools | Tick Array Walk | YES | PASS |

## Unsupported Protocols (Fail-Closed)
- Any protocol not listed above is rejected at the graph discovery stage.
- Tokens with unverified or unsupported transfer fees or non-standard token extensions are rejected fail-closed.
