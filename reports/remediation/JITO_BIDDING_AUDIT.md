# Jito Tip Pricing Engine & Statistical Model Forensic Audit

## 1. Executive Summary
The Jito Bidding Engine is responsible for dynamically pricing bundle tips submitted to the Jito Block Engine. This audit details the forensic remediation of the statistical model, the elimination of the $P_{50} = 50\%$ inclusion fallacy, and the surgical repair of the post-selection tip clipping defect.

---

## 2. Forensic Investigation of the Post-Selection Tip Clipping Defect

### 2.1 The Root Cause Defect
Prior to remediation, two severe post-selection tip clipping bugs existed in the execution pipeline:
1. **In `crates/arb-land/src/bidder.rs`**:
   The adaptive bidder evaluated candidate rungs ($P_{50} \dots P_{99}$) and selected a winning candidate (e.g. $P_{95}$ at 50,000 lamports with 95% inclusion probability).
   Then, the retry ladder escalation cap was applied:
   ```rust
   // OLD DEFECTIVE CODE:
   if winner.tip.0 > cap.0 {
       winner.tip = cap; // e.g. reduced to 16,000 lamports
       winner.expected_value = expected_value_lamports(winner.inclusion_p, edge.saturating_sub(cap));
       escalated = true;
   }
   ```
   **The flaw**: While `winner.tip` was clamped down to 16,000, `winner.inclusion_p` **remained 0.95**! A 16,000 lamport bid was falsely evaluated as having a 95% chance of inclusion, even though 16,000 was far below the $P_{90}$ market price (50,000 lamports). This generated artificially inflated Expected Value ($EV = P \times \text{margin}$) and submitted underpriced bids with false confidence.
2. **In `crates/arb-bot/src/supervisor/opportunity.rs`**:
   The supervisor computed its own surplus ceiling $t_{max}$ and clamped the approved tip:
   ```rust
   // OLD DEFECTIVE CODE:
   (Lamports(tip.as_u64().min(t_max)), Some(rung), false)
   ```
   If $t_{max}$ reduced the tip below the bidder's chosen price, the transaction proceeded without re-evaluating whether the clamped tip still yielded positive EV or met minimum inclusion probability.

### 2.2 Surgical Implementation & Mathematical Proof
We introduced `calibrated_inclusion_probability(tip, floor, ledger)` in `crates/arb-land/src/bidder.rs`:
```rust
pub fn calibrated_inclusion_probability(
    tip: Lamports,
    floor: &TipFloor,
    ledger: &LandingLedger,
) -> f64 {
    let p50_price = floor.at(Percentile::P50).0;
    if tip.0 < p50_price {
        let p50_coverage = ledger.coverage(Percentile::P50);
        return (p50_coverage * (tip.0 as f64 / p50_price.max(1) as f64)).clamp(0.0, 1.0);
    }

    let mut best_rung = Percentile::P50;
    let mut next_rung = None;

    for &rung in &Percentile::GRID {
        if floor.at(rung).0 <= tip.0 {
            best_rung = rung;
        } else {
            next_rung = Some(rung);
            break;
        }
    }

    let p_low = ledger.coverage(best_rung);
    let Some(higher) = next_rung else {
        return ledger.coverage(Percentile::P99);
    };

    let price_low = floor.at(best_rung).0;
    let price_high = floor.at(higher).0;
    if price_high <= price_low {
        return p_low;
    }

    let p_high = ledger.coverage(higher);
    let fraction = (tip.0.saturating_sub(price_low) as f64)
        / (price_high.saturating_sub(price_low) as f64);
    (p_low + fraction * (p_high - p_low)).clamp(0.0, 1.0)
}
```
**Invariants Enforced**:
- **Monotonic Calibration**: For any clamped bid $t$, its probability $p(t)$ is strictly bounded by the coverage of the rungs it actually funds: $p(t) \le p(R_{\text{higher}})$.
- **Immediate EV Recalculation**: Whenever a tip is scaled or clamped (in `bidder.rs` or `opportunity.rs`), $EV = p(t) \times (\text{edge} - t)$ is recomputed immediately. If $EV \le 0$, the opportunity fails closed and is dropped.
- **Automated Regression Proof**: Verified by unit test `bidder::tests::capped_tip_recalculates_landing_probability_and_ev`.

---

## 3. Statistical Model: Dismantling the Percentile Fallacy

### 3.1 Why $P_{50} \ne 50\%$ Probability
The Jito tip floor API reports the distribution of landed tips across all transactions included in prior blocks on Solana.
Conflating this historical percentile distribution with the conditional probability of landing a specific arbitrage transaction is a major statistical fallacy:
1. **Adversarial Contention**: Arbitrage trades compete for identical account write-locks (e.g. popular AMM pool state). If multiple searchers submit bundles for the same price dislocation, only the highest tip or earliest bundle in the block lands; the others fail or revert.
2. **Compute Unit Impact**: A 300,000 CU transaction consumes significantly more block space and scheduler budget than a simple 50,000 CU swap.
3. **Execution Freshness**: The value of an edge decays rapidly across slots (Chronos exponential decay). A stale bid submitted near the end of a slot window has a drastically lower inclusion rate than an early bid.

### 3.2 The Empirical Wilson Lower Bound Ledger
The `LandingLedger` does not rely on static assumptions. It records real on-chain landing outcomes for each price rung and computes the Wilson score lower bound at 95% confidence ($\alpha = 0.05$):
$$w = \frac{\hat{p} + \frac{z^2}{2n} - z \sqrt{\frac{\hat{p}(1-\hat{p})}{n} + \frac{z^2}{4n^2}}}{1 + \frac{z^2}{n}}$$
- Unproven rungs are never treated as certain.
- Even if an unmeasured rung has high theoretical percentile, empirical failures rapidly drive its coverage down.
- Terminal observation recording (`js.record`) records outcomes strictly once per terminal bundle state. `Pending` and `Invalid` statuses do not pollute the ledger.
