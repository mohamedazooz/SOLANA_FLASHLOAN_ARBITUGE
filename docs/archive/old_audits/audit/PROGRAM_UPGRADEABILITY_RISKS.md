> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Program Upgradeability Risks — SOLANA_FLASHLOAN_ARBITUGE

> Checklist item **609**. Risk register for every on-chain program on the execution path.
> Rationale: programs that are upgradeable (admin/upgrade authority active) can change their
> fee model, account layout, or validation logic at any time without notice. For a bot that
> hardcodes account offsets and fee math, a silent upgrade is a correctness and capital-safety risk.

## How a program upgrade breaks this system

| Program change | Impact on this repo | Detection |
|---|---|---|
| Account layout / offsets change | Silent decode corruption (wrong reserves/liquidity → wrong quotes → false profitability) | Decode guards + owner validation fail closed |
| New fee tier or fee account | Net-profit model undercounts cost → margin turns negative after landing | Quote-vs-landed reconciliation (checklist 605) |
| New required remaining accounts | CPI/swap instruction fails simulation | Simulation + bundle-rejection telemetry (Domain 15) |
| New discriminator/version | Pool rejected at decode → discovery shrink | Per-protocol discovery counters |
| Upgrade authority rotated to new owner | All of the above at once, statically verified assumptions void | Off-chain: periodic re-verification of this register |

## Register — execution-path programs

| Program | ID (`crates/arb-constants/src/lib.rs`) | Known upgrade authority | Risk level | Note |
|---|---|---|---|---|
| Kamino (Klend) | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` | Kamino team multisig (active) | **High** — flash-loan provider | Fee schedule and reserve config are mutable; flash-loan fee is read from reserve config at quote time (not hardcoded) |
| Meteora DLMM | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` | Meteora team (active) | High — busiest DEX | Live fee params on LB-pair config accounts; decode guards re-check discriminators |
| Meteora DLMM Pro | `dPro1D5wiqnGMH8vMDG23jh35btTHoWdyqXjqkqxe8n` | Meteora team (active) | High | Newer program, layout drift risk highest |
| Meteora DAMM v2 | `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG` | Meteora team (active) | Medium-High | Dynamic fee modes tested (10 tests, G11) |
| Meteora DBC | `dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN` | Meteora team (active) | Medium-High | Bonding-curve migration points |
| Raydium CLMM | `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK` | Raydium team (active) | High | Tick-array dependency planning mandatory |
| Raydium CPMM | `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C` | Raydium team (active) | Medium-High | |
| Raydium AMM v4 | `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8` | Raydium team (active) | Medium-High | Legacy layout, frozen since 2021 but authority still active |
| Raydium LaunchLab | `LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj` | Raydium team (active) | Medium-High | Early-stage curve economics |
| Orca Whirlpool | `whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc` | Orca team (active) | Medium-High | |
| Pump.fun | `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P` | Pump.fun team (active) | Medium-High | Fee-account migration history (2024) |
| PumpSwap | `pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA` | Pump.fun team (active) | Medium-High | V2 remaining-accounts contract under audit (checklist 61/661) |
| Pump fees | `pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ` | Pump.fun team (active) | Low — fee-only | |
| Token / Token-2022 | `TokenkegQ…` / `TokenzQd…` | Spl Gov (frozen semantics; transfer-fee extensions mutable per-mint) | Medium | Per-mint transfer-fee config must be re-read at quote time |
| ALT program | `AddressLookupTab1e…` | Native | Low | |

## Special case — our own executor program

`programs/arb-executor` declares `ARB_EXECUTOR_PROGRAM_ID = ArbExec1111111111111111111111111111111111111`
(`programs/arb-executor/src/lib.rs:27`).

* **Deployment status: NOT DEPLOYED to the governance-controlled mainnet path in this session.**
  The source builds and has validation/CPI modules and tests, but there is no retained deployment
  evidence (no confirmed on-chain program invocation, no authority record). It is therefore
  classified **UNVERIFIED** for production; dispatch paths that require the executor must stay in
  shadow mode (D-18 enforces no silent re-dispatch; S-1 rotation recommendation stands).
* If deployed later: the upgrade authority must be a multisig, and this register must record the
  authority address + deployment slot recorded here per `docs/audit/ECONOMIC_MODEL_AUDIT.md` rules.
* Custom-program risk is the same as third-party: an upgradeable executor can be made исполнения
  losing swaps. Long-term mitigations: freeze authority after deployment, or multisig with timelock.

## Standing mitigations (already enforced in code)

1. Owner + discriminator validation on every hydrated account (StateIntegrity, D-02..D-04).
2. Fee parameters always read from live config accounts — never hardcoded, so fee-model upgrades
   change quotes rather than invent profit (checked at quote time).
3. Simulation + bundle-rejection classification catches new required accounts before capital
   exposure (Domain 15 mock tests).
4. Quote-vs-landed reconciliation flags systematic cost undercounts (checklist 605).
5. Fail-closed on unsupported layout/version — no stale fallback (test: `stream_route…`, workstream 4 folders).

## Operational duty

Before any live (signing-enabled) run, re-verify this register against the live programs:
`getProgramAccounts`-free method — check each program's upgrade authority via
`getAccountInfo` on the program address (executable-data carries the upgrade field) and compare
deployment slot vs. the last recorded one. A change ⇒ re-run the affected protocol's differential
suite (docs/testing/DIFFERENTIAL_TESTING.md) before resuming live dispatch.

Status: **REVIEWED (`[x]` for checklist 609, doc-only)** — data-plane guarantees are in code; the
register itself cannot be automated end-to-end on free-tier RPC without extra calls the free-tier quota budget.
