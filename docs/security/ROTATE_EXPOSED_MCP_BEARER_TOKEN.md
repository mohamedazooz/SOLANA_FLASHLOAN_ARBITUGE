# Exposed MCP Bearer Token — Rotation Required

## Finding

A full-history Gitleaks scan on 2026-11-27 found the same JWT-like bearer token embedded in multiple tracked scripts at commit 06f53505ae68510f20e3ea2affc1aa02a65a311f:

- scripts/test_pro_endpoints.py
- scripts/test_mcp.py
- scripts/test_public_tools.py
- scripts/test_mcp_call.py
- scripts/solscan_tool.py (fallback returned by load_api_key())

The token payload contains a non-placeholder email claim and a 2026 issued-at value, with no exp claim. Treat it as potentially active and compromised. The token value and email are intentionally not reproduced here.

## Code remediation

- MCP test scripts now require MCP_TEST_TOKEN from the process environment; no bearer token is embedded in source.
- scripts/solscan_tool.py no longer returns a hard-coded fallback. It uses SOLSCAN_API_KEY or the existing local key-file lookup and fails explicitly when no key is configured.
- Keep MCP_TEST_TOKEN and SOLSCAN_API_KEY separate and scoped to their respective services.

## Required operator action

1. Revoke the exposed bearer token at its issuer. If it is minted from a shared signing secret, rotate that secret and invalidate all tokens signed with the old key.
2. Issue a new, least-privilege test token and store it only in the appropriate GitHub Actions secret or local environment; never commit it.
3. Verify that the old token is rejected by the issuing service after rotation.
4. Decide separately whether to rewrite Git history to purge the old blob. This has force-push and collaborator coordination consequences, so no history rewrite was performed in this change.

## Scanner notes

The manual full-history scan reported 33 findings. Besides the repeated JWT, results include base58-looking Solana addresses and test/documentation fixtures that require individual classification. Do not add broad path or rule allowlists. Only add exact, reviewed false-positive fingerprints after the exposed bearer token has been revoked.