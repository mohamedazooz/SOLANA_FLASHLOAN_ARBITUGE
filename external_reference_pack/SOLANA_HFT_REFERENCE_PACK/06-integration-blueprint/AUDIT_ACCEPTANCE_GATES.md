# Audit Acceptance Gates

أي finding لا يعتبر Bug إلا إذا احتوى:
- Finding ID.
- Severity + confidence.
- exact file/symbol/line.
- reproducible input/state.
- expected vs actual.
- upstream reference/version.
- proposed minimal fix.
- regression test.
- impact on latency/CU/packet/accounts.
- rollback plan.

## P0 — Must pass before any live attempt
- [ ] Mainnet program IDs and deployment/upgrade authority verified from chain.
- [ ] Atomic borrow → all swaps → repay occurs in one transaction/instruction path as intended.
- [ ] Repayment amount includes exact protocol fee and cannot be bypassed.
- [ ] Post-trade settlement balance >= pre-trade balance + minimum net profit.
- [ ] No stale or mixed-fork pool state can reach signer.
- [ ] Every account owner, mint, token program, PDA, vault and authority is validated.
- [ ] Every protocol adapter matches current official IDL/SDK.
- [ ] Failed simulation / stale quote / incomplete account set is a hard no-send.
- [ ] Submit timeout is reconciled before retry; no duplicate trade risk.
- [ ] Kill switch and hard trade-size ceiling cannot be widened by agent/config.

## P1 — Performance and reliability
- [ ] p50/p95/p99 measured from event receive to signed tx, submit, landed, finalized.
- [ ] CU limit based on measured route shape + bounded safety margin.
- [ ] Jito/SWQoS lane outcomes recorded separately.
- [ ] Queue depth/backpressure/reconnect metrics exist.
- [ ] Slot, write_version, commitment, fork invalidation are recorded.
- [ ] Packet size, account count, loaded account data, ALT coverage checked pre-send.
- [ ] 24h shadow soak with zero unhandled panics, stale-state sends, or silent data gaps.

## P2 — Economic validation
- [ ] quote gross output and net output independently recomputed.
- [ ] DEX fees, creator/protocol fees, Token-2022 transfer fees, flash-loan fee included.
- [ ] base fee, priority fee, Jito tip and failed-attempt cost included.
- [ ] actual landed result reconciled against predicted PnL.
- [ ] profit accounting uses integer base units; no floating point settlement.
- [ ] false-positive opportunity rate and missed opportunity rate measured.

## Hard rules
- No mainnet signing or sending during audit.
- No wallet private keys in prompt, logs, fixtures or ZIP.
- No auto-merge, no direct main modifications.
- No feature addition unless tied to a proven finding.
- Do not label tests "passing" unless command, commit SHA and output are attached.
