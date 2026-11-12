# Solana Arbitrage — Shadow-only package

This ZIP is packaged for **read-only market monitoring and shadow opportunity accounting only**.

- Build artifact: `arb-bot` with Cargo feature `shadow-only`.
- Runtime: `docker compose up -d --build` (the only included compose file is locked to shadow-only).
- Setup: copy `.env.shadow.example` to `.env.shadow.local` and provide read-only provider/RPC credentials only.
- Start: `./run-shadow.sh`; stop: `./stop-shadow.sh`.
- No wallet keypair is required or loaded. Do not add private keys to the environment.

Read `SHADOW_RELEASE_STATUS.md` before deployment. This is not a live-trading release and does not claim all historical audit findings have been closed.


## Transaction format

This build is V1-only. See [`V1_ONLY_RELEASE_POLICY.md`](V1_ONLY_RELEASE_POLICY.md) and [`transaction-profile.toml`](transaction-profile.toml). Legacy/V0/ALT compilation is not permitted. Meteora DLMM BinArray PDAs are inline V1 accounts; exact traversal-derived arrays must fit the current 64-account budget. Shadow builds an unsigned V1 envelope with zeroed payer signature slot and simulates it with `sigVerify=false`; it never broadcasts.
