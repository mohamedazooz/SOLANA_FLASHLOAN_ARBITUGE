# Program Deployment Verification — Phase 6 (Mission §11)

- **Verification date:** 2026-11-23
- **Verdict:** **NOT DEPLOYED / NOT VERIFIED.** The active generic flash route no longer depends on this program; the separate CPI executor remains inactive.

## Read-only Mainnet probe

The configured RPC endpoints in the local `.env` were used for read-only `getAccountInfo` requests. No endpoint URL, API key, wallet secret, or private key is recorded in this report.

| Item | Result |
|---|---|
| Program ID used by Rust client | `ArbExec1111111111111111111111111111111111111` |
| Mainnet account lookup | `null` from SOLANA_RPC_PRIMARY, HELIUS_RPC_URL, SOLAMI_RPC_URL, SHYFT_RPC_URL and NODIT_SOLANA_RPC |
| Alternative ID previously in Anchor.toml | `9xeescXJ8aobZViWxKcMCafJKDdTk85kVL2kQYbABtT9m` — rejected by RPCs as an invalid-size/base58 public key |
| Anchor.toml after local correction | Matches the Rust build-time placeholder above in all cluster sections; this is configuration consistency only, **not deployment evidence** |
| Deployed executable / owner / upgrade authority | Not available because `getAccountInfo` returned null |
| Local SBF build and deployed-binary hash comparison | Not run; no current `cargo-build-sbf`/Anchor deployment evidence was established in this session |
| Deployment or upgrade | **None** |

## Production-path correction

The generic WSOL/Kamino flash assemblers previously appended a standalone `profit_guard` instruction using the placeholder executor ID. That instruction would fail on Mainnet while the program account is absent. The active generic flash builders no longer append that instruction.

Instead, the final hop's `min_amount_out` is constructed from checked integer arithmetic and includes:

`borrow principal + Kamino flash fee + base transaction fee + priority fee + Jito tip + retained minimum net profit`.

The active generic assembler is WSOL-anchored and uses the exact same effective CU limit that it encodes. This makes the final swap itself enforce the return floor without relying on an undeployed program. The standalone guard helper and CPI executor source remain available only for a future, explicitly activated deployment path. The cross-DEX builder delegates to this same generic assembler. The inactive split-routing builder is not eligible for activation: its route-level min-out and aggregate cost proof are still incomplete, and the Supervisor currently leaves split allocation hard-disabled.

## Requirements before activating arb-executor CPI

1. Generate and record a real program keypair/public ID; never ship the vanity placeholder as a deployment identity.
2. Build the SBF program with the pinned toolchain and hash the resulting binary.
3. Deploy only after explicit operator approval, using a controlled deploy key and documented upgrade authority (preferably multisig/cold custody).
4. Verify `getAccountInfo` shows an executable program on Mainnet from at least two independent RPC providers.
5. Verify deployed program data/buffer hash against the locally built artifact and record program owner, loader, upgrade authority, slot, and hashes.
6. Add a live identity regression check before any config can enable the executor route.
7. Add LiteSVM/fork integration tests and run shadow soak before any live enablement.

**Safety status:** live trading remains disabled. RPC checks were read-only. No wallet signing, transaction submission, deployment, or upgrade was performed.
