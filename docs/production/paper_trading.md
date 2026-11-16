# Paper Trading Mode & Economic Validation

## 1. Operating Principle
Paper Trading ingests live mainnet Yellowstone gRPC and RPC data, discovers routes, sizes flash loans, prices dynamic Jito tips, evaluates the 20-point risk check, and simulates execution without signing or broadcasting real funds on-chain.

## 2. Telemetry & Observation Recording
Every observed opportunity is logged to SQLite with:
- `opportunity_id`: Unique UUID.
- `source_slot`: Exact slot observed.
- `route`: Pools, tokens, and direction.
- `input_amount`: Sized borrow principal.
- `gross_profit`: Output minus principal.
- `fees`: Flash fee, DEX fees, priority fee, base signature fee.
- `jito_tip`: Dynamic tip calculated from live tip floor.
- `expected_net_profit`: Net PnL.
- `subsequent_execution_state`: Verified on subsequent block commits whether the opportunity remained executable.

## 3. Viability & Statistical Confidence Metrics
- **Opportunity Decay Rate**: Fraction of opportunities consumed by competitor MEV bots within 1 slot.
- **Estimated vs Landable PnL**: Realized decay vs theoretical quote.
- **Calibration Ratio**: $P(\text{land})$ prediction calibration compared to actual block inclusion rates.
