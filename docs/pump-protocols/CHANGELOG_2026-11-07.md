# Change log — Pump adapter increment (2026-11-07)

- Added `DexProtocol::PumpSwap` as a distinct protocol identity; it is not aliased to Pump.fun.
- Centralized PumpSwap program id, GlobalConfig address and Pool discriminator in `arb-constants`.
- Added PumpSwap Pool header decoder with optional appended fields and signed `virtual_quote_reserves` support.
- Added a quote-ready PumpSwap adapter that requires both vault balances and verified fee components.
- Added live FeeConfig tier selection using raw market-cap denomination and PPM fee components; rejects empty or unordered tier arrays.
- Applied PumpSwap quote-asset fee treatment directionally: fee deducted from quote output on sells and quote input on buys.
- Added DexScreener venue-name recognition and supervisor monitoring count for PumpSwap.
- Updated standard Pump.fun curve quote fee from obsolete 100 bps to 125 bps, and applied fee in quote-asset denomination (buy fee on quote input; sell fee on quote output).
- Rejected Pump.fun quotes that cross the remaining real token reserve until synthetic-migration quoting is implemented.
- Disabled generic live transaction building and execution capability for both Pump protocols; they remain shadow-only.
- Added F2 implementation plan for Yellowstone state joins, live fee decoding, exact quotes, current IDL transaction builders and verification gates.

## Validation

Source-level and archive checks only. Rust compiler, Cargo, rustfmt and Docker are not installed in this environment; no build, test, clippy or container success is claimed.
