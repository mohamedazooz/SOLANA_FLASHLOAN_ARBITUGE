# Security Operations and Secret Rotation

This document is the operator procedure for handling the bot's secrets. It
contains **no** secret values. Never paste a key, token, or private key into an
issue, chat, log, or commit.

## 1. Secret inventory

| Secret | Used by | Storage | Rotation impact |
|---|---|---|---|
| `SOLANA_PRIVATE_KEY` / keypair file | `arb-bot` signer, sweep | `.env` or `secrets/solana_keypair.json` (gitignored) | New hot wallet address; fund it with fee SOL before restart. |
| `OPERATOR_PRIVATE_KEY`, `OPERATOR_PUBLIC_KEY` | Operator tooling | `.env` | Re-authorize operator scripts. |
| `PROFIT_RECEIVER_ADDRESS`, `COLD_WALLET_ADDRESS` | Profit sweep | `.env` / config | Verify the new address, then sweep. |
| `SENTINEL_ADMIN_KEY` / `*_ADMIN_KEY(S)` | Admin API auth | `.env` | Restart the bot; old token stops working immediately. |
| `JWT_SECRET`, `SENTINEL_WS_SECRET` | Webhook/dashboard auth | `.env` | Invalidates existing sessions. |
| `JUPITER_API_KEY`, `HELIUS_API_KEY`, `SHYFT_API_KEY`, `SOLAMI_API_KEY` | Data/RPC providers | `.env` | Update provider dashboards and `.env`; restart. |
| `TELEGRAM_BOT_TOKEN` | Alerts | `.env` | Revoke via BotFather, issue a new token, update `.env`. |
| `ALCHEMY_WEBHOOK_SIGNING_KEY` | Webhook verification | `.env` | Update both the Alchemy webhook and the service. |
| `HF_TOKEN`, S3 keys | Optional research tooling | `.env` | Not required for trading; rotate/remove if unused. |

If a secret is not needed by the running strategy, delete it from `.env` rather
than leaving a live credential unused.

## 2. Rotation procedure (no downtime panic)

1. **Stop trading first.** `POST /pause` with the current admin token, or stop
   the process. Confirm the kill-switch marker exists (`data/kill_switch`).
2. **Create the replacement** at the provider/wallet. Do not delete the old one
   yet.
3. **Stage the new value** in the deployment secret store (not in git, not in a
   shell history). For a Solana keypair, fund the new address with the fee
   reserve before switching.
4. **Restart with the new value** (or reload, if the process supports it — this
   build reads env at startup). Confirm startup logs show the expected mode and
   no configuration error.
5. **Verify** with read-only checks: `/health`, `/status`, metrics endpoint,
   `validate` binary, and a shadow run. Do not send a live transaction as a test.
6. **Revoke the old credential** at the provider. For a keypair, move remaining
   funds and stop using the old address.
7. **Record** the rotation date, owner, and reason in the operations log. Never
   record the value.

## 3. If a secret is suspected leaked

1. Treat the wallet as compromised: pause the bot immediately and move funds to
   a pre-created safe address.
2. Revoke the credential at the provider.
3. Rotate the keypair and all provider/admin tokens.
4. Scan git history and artifacts:
   ```bash
   gitleaks git --log-opts=--all --redact
   gitleaks dir . --redact
   ```
5. If the leak was committed, rewriting history is not sufficient by itself:
   assume the value is public. Rotation is mandatory.
6. File an incident note with timeline, scope, and remediation. Do not include
   the secret.

## 4. Operational hardening checklist

- [ ] Bot runs as non-root (`arb`) in the container.
- [ ] `.env` is `0600`, owned by the service user, outside the repo if possible.
- [ ] Keypair file is `0600`, mounted read-only, never baked into an image.
- [ ] `.env` is in `.gitignore` and `.dockerignore`.
- [ ] Admin API binds to loopback; remote access uses an SSH tunnel/VPN.
- [ ] `SENTINEL_ADMIN_KEY` is not a placeholder; the admin listener is disabled
      if it is.
- [ ] Telegram chat is private and membership is reviewed.
- [ ] `KILL_SWITCH_FILE` points at persistent storage, not `/tmp`.
- [ ] Host time is NTP-synced (slot timing and Jito windows depend on it).
- [ ] Logs do not contain secrets; redact provider URLs/tokens before sharing.
- [ ] Backup of `data/` is encrypted and access-controlled.

## 5. Admin API authentication

The admin listener (`admin.listen`, default `127.0.0.1:9101`) requires one of:

```
Authorization: Bearer <SENTINEL_ADMIN_KEY>
X-Admin-Token: <SENTINEL_ADMIN_KEY>
```

Rules:

- A missing, empty, or placeholder value disables the listener entirely.
- The comparison is constant-time.
- `/health` and `/status` are read-only and unauthenticated; `/pause` and
  `/resume` are mutating and authenticated.
- The kill-switch marker is the durable source of truth: if it exists at
  startup, the bot starts paused even if the admin API is down.

## 6. Reporting

Report a suspected vulnerability privately to the repository owner. Do not open
a public issue with reproduction details or any credential. Include the affected
commit, a minimal reproducer, and whether funds are at risk.
