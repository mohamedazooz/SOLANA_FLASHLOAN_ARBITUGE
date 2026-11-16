#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
fail() { echo "FAIL: $*" >&2; exit 1; }

rg -q '^shadow-only = \[\]' crates/arb-bot/Cargo.toml || fail 'shadow-only Cargo feature missing'
rg -q 'args.mode != BotMode::Shadow' crates/arb-bot/src/main.rs || fail 'startup mode lock missing'
rg -q 'signer disabled; no wallet material will be loaded' crates/arb-bot/src/supervisor.rs || fail 'signer disable missing'
rg -q '#\[cfg\(not\(feature = "shadow-only"\)\)\]' crates/arb-bot/src/supervisor/dispatch.rs || fail 'live dispatch cfg gate missing'
rg -q 'cargo build .*--features shadow-only' Dockerfile || fail 'Docker build not shadow-only'
rg -q 'mode = "shadow"' config/config.toml || fail 'runtime config is not shadow'
rg -q '^format = "v1_only"' transaction-profile.toml || fail 'V1-only transaction profile missing'
rg -q 'Self::V1 => Some\(1\)' crates/arb-tx/src/adaptive.rs || fail 'V1 RPC version mapping must be integer 1'
rg -q 'Self::V0 => Some\(0\)' crates/arb-tx/src/adaptive.rs || fail 'V0 RPC compatibility mapping must be integer 0'
rg -q 'pub fn compile_v1_only' crates/arb-tx/src/adaptive.rs || fail 'V1-only compiler entrypoint missing'
rg -q 'build_unsigned_v1_simulation_transaction' crates/arb-tx/src/adaptive.rs || fail 'unsigned V1 simulation builder missing'
grep -Fq 'config.insert("sigVerify".into(), json!(false));' crates/arb-sim/src/lib.rs || fail 'Shadow RPC simulation must disable signature verification explicitly'
rg -q 'Only Solana transaction V1 is permitted' crates/arb-bot/src/supervisor/dispatch.rs || fail 'V1-only dispatch gate missing'
rg -q 'AdaptiveTxCompiler::compile_v1_only' crates/arb-execution/src/lib.rs || fail 'arb-execution V1-only dispatch missing'
if rg -n 'return Ok\(CompiledArbEnvelope::V0JitoAndSwqos' crates/arb-execution/src/lib.rs >/dev/null; then fail 'arb-execution still returns V0'; fi
rg -q 'keypair_path = ""' config/config.toml || fail 'wallet keypair path not cleared'
[[ ! -e run.sh && ! -e deploy && ! -e deploy.tar.gz ]] || fail 'legacy live launcher/deploy artifacts remain'
[[ ! -e files/keypair.ts && ! -e .mcp.json && ! -e .env.bak_20261112_live_flags && ! -e deploy_to_vps.py && ! -e build_and_restart.py ]] || fail 'sensitive artifact remains'
if rg -n -i 'password\s*=|password=|AutoAddPolicy|client\.connect\(' . -g '*.py' -g '*.sh' -g '*.ps1' -g '!verify-shadow-package.sh' >/dev/null; then
  fail 'hard-coded SSH credentials or unsafe host-key handling detected'
fi
if [[ -f .env.shadow.local ]] && grep -Eq '^(SOLANA_PRIVATE_KEY|OPERATOR_PRIVATE_KEY|JITO_AUTH|ARB_LIVE)=.*[^=[:space:]]' .env.shadow.local; then
  fail 'unsafe key/live variable found in .env.shadow.local'
fi
printf 'PASS: static shadow-only packaging invariants\n'
printf 'NOTE: this script does not replace cargo check/test/clippy or the 7-day soak.\n'
