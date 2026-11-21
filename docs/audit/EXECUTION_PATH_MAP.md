# Execution and Control Path Map

## Dashboard authentication

1. `web/src/components/Login.tsx` collects an operator-supplied password.
2. `web/src/api.ts::api.login` submits it to `POST /api/login`.
3. The returned bearer token is stored in session storage.
4. Authenticated API calls attach the token in the `Authorization` header.

No automatic or hardcoded login credential is permitted.

## Dashboard control path

1. Global dashboard buttons in `web/src/App.tsx` call `api.control(action)`.
2. `web/src/api.ts::api.control` posts to `/api/control/{action}`.
3. `crates/arb-bot/src/web_api.rs` authenticates the request and routes it to the shared control service.
4. `crates/arb-bot/src/control_service.rs::execute_command` maps the action to the same service methods used by Telegram.
5. The dashboard refreshes the authoritative status snapshot after the response; it does not optimistically set execution state.

Emergency resume is intentionally two operations: `clear-emergency-stop` clears the persistent latch, then `start` explicitly unpauses execution. If either request fails, the UI refreshes status and does not claim success.

## WebSocket authentication path

1. `web/src/api.ts::api.wsUrl` requests `POST /api/ws-ticket` with the bearer token.
2. The backend issues a short-lived, single-use ticket.
3. Dashboard and Logs open `/api/ws?ticket=...`.
4. Reconnect obtains a new ticket; the long-lived bearer token is never embedded in the WebSocket URL.

## Execution path

The full candidate-to-transaction-to-dispatch map remains under Phase 0 forensic verification. No Mainnet readiness claim is made by this frontend/control-plane map.
