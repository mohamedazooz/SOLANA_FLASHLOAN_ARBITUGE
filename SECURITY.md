# Security Policy

## ⚠️ Public Repository — Important Notice

This repository contains **source code only**. No secrets, private keys, API keys,
or wallet credentials are stored in this repository.

- All sensitive values live exclusively in the VPS `.env` file (gitignored)
- The `.env` file is listed in `.gitignore` and must **never** be committed
- SSH deploy keys are stored only in GitHub Actions Secrets

## Supported Versions

| Version | Supported |
| ------- | --------- |
| `main`  | ✅ Yes    |

## Reporting a Vulnerability

If you discover a security vulnerability in this project:

1. **Do NOT open a public GitHub issue**
2. Email the maintainer privately
3. Include a description of the vulnerability and steps to reproduce
4. Allow up to 48 hours for an initial response

## Security Practices

- 🔒 `.env` and all secret files are gitignored
- 🔒 Wallet private keys never appear in code or commits
- 🔒 SSH deploy key is ed25519, stored only in GitHub Secrets
- 🔒 Bot runs as non-root user on VPS
- 🔒 All transactions go through Jito private bundles (no public mempool)
- 🔒 Admin API bound to loopback (127.0.0.1) only
- 🔒 Kill-switch file mechanism for emergency stop

## Branch Protection

The `main` branch is protected:
- GitHub Actions must pass before any merge
- Direct pushes by owner only
- CODEOWNERS review required for critical files
