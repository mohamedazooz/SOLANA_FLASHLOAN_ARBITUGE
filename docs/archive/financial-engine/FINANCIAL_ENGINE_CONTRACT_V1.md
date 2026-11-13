# Unified Financial Engine — Contract V1

## Domain boundaries

- `RawAmount<Mint>`: unsigned amount of one exact mint, in atomic units.
- `BaseAssetAmount`: unsigned amount of the chosen cycle base mint (initially WSOL / lamports).
- `SignedBasePnl`: signed profit/loss in base asset atomic units.
- `UsdMicros`: valuation/reporting/risk-policy unit only; never substituted for raw token amounts.
- `Lamports`: SOL network costs only.
- `ComputeUnits`: resource usage, never a currency.
- `Slot` + provider/account write-version: provenance/freshness metadata.

No function may accept an unqualified `u64` for an economic quantity if the mint, decimals, fee denominator, or unit is needed to interpret it.

## Quote contract

Every DEX adapter returns:

- protocol and pool ID
- input mint + raw input amount
- output mint + raw output amount
- expected output and enforceable minimum output
- exact fee components in their native quote asset
- whether each fee is already reflected in output
- transfer-fee / Token-2022 treatment
- price impact / end price
- required writable accounts (TickArrays / BinArrays / bitmap extensions)
- state slot, write version, account-set hash, and decoder version
- exact/partial-fill status and consumed input amount

Adapter errors are typed. Missing coverage, stale state, unsupported mint extensions, arithmetic overflow, incomplete route, or unsupported instruction version is a hard quote failure.

## Economic evaluation contract

The central engine accepts a fully quoted cycle and a cost ledger. It must:

1. Prove first and final mint are identical.
2. Prove every hop's output mint equals the next hop's input mint.
3. Prove the flash-loan asset equals the cycle base asset.
4. Prove all amounts are in raw units of the correct mint.
5. Aggregate only costs not already embedded in adapter net outputs.
6. Convert SOL liabilities to base-asset raw units with checked **ceiling** conversion.
7. Calculate expected and minimum PnL as signed values.
8. Apply profit floor to minimum PnL for execution authorization.
9. Emit an immutable evaluation ID + calculation version + input state provenance.

## Cost accounting rules

| Cost | Before execution | After execution |
|---|---|---|
| AMM / protocol / creator / LP / dynamic fee | Adapter quote, once | Reconcile via token deltas / program events |
| Token-2022 transfer fee | Adapter quote, once | Reconcile withheld fee / transfer deltas |
| Flash-loan premium | Exact lender state + lender formula | Repay instruction and actual account delta |
| Base signature fee | Signature count + current fee schedule estimate | Transaction metadata `fee` |
| Priority fee | `ceil(price * CU_limit / 1e6)` | Transaction metadata `fee` / message budget |
| Jito tip | Planned tip in bundle success scenario only | Actual tip-account transfer if landed |
| ATA/account rent | Required cash balance | Separate recoverable rent asset vs irrecoverable expense |
| Failed attempt | Scenario-specific expected cost | Actual metadata if transaction landed/failed |

## Execution outcomes

- `Prepared`: local validation complete; not sent.
- `Submitted`: transport accepted payload; not evidence of landing.
- `BundleAccepted`: Jito returned bundle ID; not evidence of landing.
- `Processed`: observed in a processed bank; may be forked out.
- `Confirmed`: supermajority voted; operational confirmation.
- `Finalized`: rooted; final settlement state.
- `LandedFailed`: transaction landed with `meta.err`; state changes rolled back, fees may still be charged.
- `LandedSucceeded`: landed with no error; reconcile actual deltas.
- `ExpiredNotLanded`: blockhash expired and no landing evidence after the prescribed search window.
- `Unknown`: not enough evidence; never report as failed or successful prematurely.

Bundle ID and transaction signatures are separate typed identifiers. A bundle acceptance response never increments landed counters.

## Golden tests required before integration

1. WSOL base: decimals=9, 1 SOL = 1,000,000,000 lamports.
2. USDC base: decimals from mint account, never assume 6; verify official mint and Token Program.
3. Cost conversion: rational rates with non-zero remainder round liabilities upward.
4. Zero conversion denominator rejects.
5. `minimum_final < principal + costs` yields negative signed PnL, not arithmetic error.
6. `expected_final > principal` but `minimum_final <= principal + costs` must fail execution gate.
7. Adapter fee already included in output is not deducted twice.
8. Token-2022 transfer fee reduces spendable net output exactly once.
9. Pump curve 1.25% current SOL/USDC schedule; PumpSwap canonical tier and non-canonical flat schedule are distinct.
10. PumpSwap fee charged in quote asset; fee config state slot must be no older than the pool quote state policy.
11. Jito tip is charged only in the bundle outcome where its transfer commits.
12. Failed landed transaction charges actual metadata fee; unlanded bundle does not get treated as a charged transaction.
13. Flash borrow and repay instructions are in the same transaction; repay amount is checked principal + lender fee.
14. Overflow at every conversion and sum rejects; no saturation/clamping in economic truth.
15. Multi-hop route validates mint continuity and returns to original base mint.
