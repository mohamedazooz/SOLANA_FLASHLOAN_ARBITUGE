# Deployment Architecture

**Scope:** how `arb-bot` reaches a VPS, what is deployed, and what is prohibited from deploying automatically.
**Status of this document:** `REPOSITORY_VERIFIED` — every claim below is checked against the tracked workflow/Docker/scripts on the branch that introduced it.

## Topology

```
GitHub repo (main)
   │  workflow_dispatch (manual only)
   ▼
deploy-vps.yml ── SSH (secrets VPS_HOST/VPS_USER/VPS_PORT/VPS_SSH_KEY)
   │              deploy pinned github.sha, never origin/main head
   ▼
VPS: ~/SOLANA_FLASHLOAN_ARBITUGE
   ├─ cargo build --release --locked -p arb-bot --bin arb-bot
   ├─ restart (systemd arb-bot.service, or nohup fallback)
   ├─ health check (is-active / PID)
   └─ auto-rollback to .deploy_previous_commit on any failure
```

## Deployment paths (in order of preference)

| Path | Entry point | Behavior | Approval |
|---|---|---|---|
| Pinned-commit workflow deploy | `.github/workflows/deploy-vps.yml` (`workflow_dispatch`) | Fetches main, verifies `github.sha` is an ancestor of origin/main, detaches onto exactly that commit, builds `--locked`, restarts, health-checks, auto-rolls-back on failure | GitHub Environment `production` (operator must enable required reviewers) |
| Release-directory deploy | `deploy/production/deploy.sh` (run manually on VPS) | Copies a pre-built binary pair (`arb-bot`, `validate`) into `releases/<timestamp>/`, validates `production.toml`, flips `current` symlink, restarts; rollback via `deploy/production/rollback.sh` | Operator-only (SSH) |
| First-time provisioning | `scripts/deploy_vps.sh` (run on VPS) | Installs deps, prompts secrets (never echoes), **forces `ARB_LIVE=0` / `DRY_RUN=true`**, installs systemd unit + watchdog + logrotate, builds, starts shadow | Operator-only (SSH) |

## Docker

- `Dockerfile`: multi-stage, pinned `rust:1.99.1-slim-bookworm` (matches `rust-toolchain.toml`) and `node:22-slim`; `--locked` build; non-root user `arb`; read-only FS compatible; HEALTHCHECK hits the local `127.0.0.1:9100/metrics` endpoint; default CMD is `--mode shadow`.
- `docker-compose.yml`: shadow-only runtime (`--mode shadow`), binds metrics to loopback only, `read_only: true`, `cap_drop: ALL`, `no-new-privileges`, tmpfs `/tmp` noexec.
  *The former `docker-compose.shadow.yml` was a byte-identical duplicate and was removed in the same phase (dead code).*

## Invariants

1. **No auto-deploy on merge.** The deploy workflow has only `workflow_dispatch`.
2. **Pinned commit.** The remote refuses to deploy a SHA that is not an ancestor of origin/main; concurrent merges after dispatch cannot change what is built.
3. **`--locked` builds everywhere** (Docker, workflow, deploy scripts) — a stale `Cargo.lock` fails instead of silently drifting.
4. **Fail-closed modes.** Provisioning forces shadow/DRY_RUN; containers default to `--mode shadow`; live mode requires an explicit operator change on the VPS itself.
5. **Rollback capability.** `.deploy_previous_commit` on the VPS records the prior commit; the workflow rolls back automatically when build/restart/health fails. Manual rollback: `deploy/production/rollback.sh` (symlink path) or `git checkout --detach $(cat .deploy_previous_commit)` (checkout path).

## Build provenance

- Toolchain pinned via `rust-toolchain.toml` (1.99.1); Docker builder image pins the same version.
- CI runs gitleaks (full history) and cargo-deny (advisories/bans/sources/licenses) on every PR/push, plus a weekly scheduled advisory re-scan (RustSec data moves without repo pushes).
- Third-party GitHub Actions are pinned to full commit SHAs, not tags.

## Known gaps (explicit)

- No SBOM artifact is produced or stored yet (P3; candidate: `cargo cyclonedx` in a workflow_dispatch release job).
- Postgres/Redis are not part of the single-node deployment topology; SQLite is the current durable store (see `DATA_MODEL.md`).
- The `production` GitHub Environment must be created by the operator before the approval gate is live; until then the workflow is dispatch-only (manual trigger is the only gate).
