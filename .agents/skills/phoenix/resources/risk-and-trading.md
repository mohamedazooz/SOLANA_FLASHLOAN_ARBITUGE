# Phoenix Risk And Trading Notes

Phoenix is a perpetual futures venue. Agent integrations must treat every trade as a leveraged financial action.

## Pre-Trade Checklist

- Fetch current exchange metadata and market config.
- Confirm the market is active and tradeable.
- Fetch trader state and current subaccount collateral.
- Compute current notional, open orders, and position direction.
- Validate the new order against max position size, max leverage, max order size, and available collateral.
- Confirm whether the action increases or reduces risk.
- Simulate the transaction before sending.
- Show price, side, size, estimated notional, fees, and liquidation/risk impact to the user.

## Bot Controls

- Use a max notional per market.
- Use a max total notional across all markets.
- Use a max risk-increasing orders per minute.
- Stop trading when WebSocket data is stale.
- Stop trading when exchange sequence numbers skip until a fresh snapshot is loaded.
- Stop trading on repeated HTTP 429, 5xx, RPC, or simulation failures.
- Require explicit user confirmation for first live trading enablement.

## Numeric Handling

- Keep protocol integer fields as strings or bigint values.
- Use SDK packet builders for conversions whenever possible.
- Only format numbers for display at UI or logging boundaries.
- Be careful with stop-loss trigger prices because some helpers take tick values rather than human-readable USD strings.

## Flight

Flight is Phoenix builder routing. Treat it as beta until the official docs remove that warning.

- Make Flight opt-in.
- Show builder authority and fee bps in configuration.
- Verify wrapped instructions before shipping.
- Keep a non-Flight path available for fallback.

