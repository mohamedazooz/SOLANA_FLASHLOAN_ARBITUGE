# FRONTEND AUDIT — Phase 10 (Mission §15)

**Status:** ACTIVE — source-verified 2026-11-22. Target: `web/` (React 18 + Vite 6 +
TypeScript 5.6, Tailwind 3), 2,528 LOC across `src/` + components.

---

## 1. Stack & build (verified from `web/package.json`)

- React ^18.3.1, Vite ^6.0.3, TypeScript ^5.6.3 (`tsc -b && vite build` = strict
  type-check gate before every build — **implemented**).
- No router; single-page dashboard. Pinned ranges in package-lock.

## 2. Auth handling (verified — `web/src/api.ts:1–30`)

- Session token in `sessionStorage` (`arb_session_token`), cleared on tab close; sent as
  `Authorization: Bearer` header.
- No token in URLs for data calls; the WS path uses a **server-issued single-use
  ticket** (`POST /api/ws-ticket`), keeping the session token out of `?ticket=` reuse
  (api.ts:166 comment mirrors server contract).
- Login expiry: `expires_at_unix` surfaced; UI re-login on 401 (status handler).

## 3. Components (verified on disk)

| Component | Role | Risk class |
|---|---|---|
| `Login.tsx` | password → token | auth |
| `Overview.tsx` | status snapshot | read |
| `Opportunities/Executions/Rejections/Logs/Pools/Providers/RouteMonitor` | data projections | read |
| `RiskControl.tsx` | kill switch / emergency buttons | **CONTROL** — posts to `/api/control/*` with confirm |
| `StrategyConfig.tsx` | threshold overrides | **CONTROL** — posts `/api/config` |
| `TelegramPanel.tsx` | telegram status/actions | CONTROL |
| `Sidebar.tsx`, `styles.css` | chrome | read |

Frontend never computes trading decisions — it is a projection + control surface only;
**no frontend state is a source of truth for execution** (all state comes from the
backend's status/executions endpoints — verified: api.ts has no local arbitration logic).

## 4. Security-contract test (verified)

`web/security-contract.test.mjs` (`npm run test:security`, node --test):
- **3 passed / 0 failed** locally this session (and "Vercel Preview Comments" + web-build
  CI checks on PR #130/131/132 runs).
- Contract enforced: token stays in sessionStorage, never in localStorage; no token in
  fetch URLs; control POSTs require confirm.

## 5. Verification commands (this session)

- `node --test web/security-contract.test.mjs` → **pass 3 / fail 0** (duration ~91 ms).
- Type-check + prod build: run by CI as `tsc -b && vite build` — **passing** on the
  latest runs of the open PRs (checked rollup on #130: "Web app build (React + Vite)
  SUCCESS").

## 6. Findings

| Finding | Severity | Summary |
|---|---|---|
| FRT-F-2026-11-22-N1 | P3 | Error surfaces on failed POSTs are minimal (alert-level messaging in two components — `Control` panels) — cosmetic UX gap, no security impact |
| FRT-F-2026-11-22-N2 | P3 | Session token survives as long as the tab is open (sessionStorage TTL is tab-bound, matching the server TTL — no refresh token flow exists by design) |

**Non-issues verified deliberately:** no `dangerouslySetInnerHTML` (grep-clean); no
secrets in bundle (only placeholder env example keys); no predetermined wallet
interaction from the browser (no signing library in web deps — signing is server-side
only by design).
