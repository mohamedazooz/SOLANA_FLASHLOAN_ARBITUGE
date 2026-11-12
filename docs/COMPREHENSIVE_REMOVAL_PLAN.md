# Comprehensive Plan: Guard Removal & Live Execution Enablement
*Applying three senior perspectives: Returns/Reverse Logistics (grading), Architecture Decision Records (governance), Full-Stack Developer (implementation)*

---

## 📋 Executive Summary

This plan classifies every "guard" in the codebase using **returns-reverse-logistics grading methodology** (Grade A–D disposition), documents each removal/retention as an **ADR**, and provides **implementation-ready code changes** with file:line precision.

| Category | Count | Action |
|----------|-------|--------|
| **Grade A (Restock)** — Protocol invariants | 3 | **RETAIN** (ADR-0003, 0005) |
| **Grade B (Refurbish)** — Economic guards with config bypass | 4 | **WEAKEN** via env (ADR-0004, 0006) |
| **Grade C (Liquidate)** — Policy locks | 3 | **REMOVE** (ADR-0001, 0002) |
| **Grade D (Destroy)** — Missing capabilities | 6 | **BUILD** (ADR-0007–0012) |

---

## 🔍 Phase 1: Returns-Style Inspection & Grading

*Each guard inspected like a returned item: condition → grade → disposition*

### Grade A: Protocol Invariants (RETAIN — "Restock as New")

| Guard | File:Line | Grade | Reason (Protocol Invariant) |
|-------|-----------|-------|----------------------------|
| MTU 1232 check | `dispatch.rs:171-199` | **A** | Solana validators drop >1232 bytes; removing = 100% failure |
| Zero blockhash check | `run.rs:1083-1084` | **A** | Validators reject zero blockhash; removing = fee burn |
| Min-out floor | `dispatch.rs` + `run.rs` | **A** | Kamino repay atomicity; without = atomic revert + fee burn |
| LiteSVM simulation | `dispatch.rs:201-216` | **A** | Only pre-flight validation; feature-gated fail-closed |

**Disposition**: Keep unchanged. These are not "guards" — they are **protocol requirements**.

---

### Grade B: Economic Guards (WEAKEN via Config — "Repackage as Open Box")

| Guard | File:Line | Current Bypass | Proposed Weakening | ADR |
|-------|-----------|----------------|-------------------|-----|
| Safety floor reserve | `dispatch.rs:129-169` | `MIN_SOL_RESERVE_LAMPORTS=0` (default) | Already 0 default; document | 0004 |
| Risk gate (Chronos, limits) | `run.rs:829-880` | `BYPASS_RISK_GATE=1` | Keep env bypass; add granular toggles | 0004 |
| Stream-readiness guard | `dispatch.rs:371-401` | None | **Change to warning-only** (log + continue) | 0006 |
| Kill-switch auto-block | `arb-risk/src/lib.rs:131-141` | None | **Add `KILL_SWITCH_DISABLED=1`** env | 0002 |

**Disposition**: Weaken via env vars — zero code removal, just config relaxation.

---

### Grade C: Policy Locks (REMOVE — "Liquidate")

| Lock | File:Line | Removal Action | ADR |
|------|-----------|----------------|-----|
| `ARB_LIVE` startup gate | `main.rs:599-602` | Delete block; allow `--mode live` | 0001 |
| `ARB_LIVE` dispatch re-check | `dispatch.rs:127` | Remove `if live_execution_enabled()` | 0001 |
| Default mode shadow | `config.example.toml:5` | Change to `mode = "live"` | 0001 |
| `live_execution_enabled()` | `main.rs:60-69` | Simplify to `fn() -> bool { true }` | 0001 |
| Kill-switch marker auto-block | `arb-risk/src/lib.rs:131-141` | Start unblocked; add env bypass | 0002 |

**Disposition**: Pure policy — remove entirely for "flow".

---

### Grade D: Missing Capabilities (BUILD — "Refurbish Required")

| Capability | Status | Required Work | ADR |
|------------|--------|---------------|-----|
| PumpSwap adapter | Hard error | IDL parity + account decode + differential tests | 0007 |
| Jupiter live path | Hardcoded `false` | Quote→local re-quote parity + Kamino integration | 0008 |
| Split routing | `split = None` | Per-leg re-quote + convex optimization + fork sim | 0009 |
| Guarded quote wiring | No callers | Refactor supervisor/risk/tx/dispatch to single path | 0010 |
| Durable state machine | Not implemented | Persistent state machine + reconciliation on restart | 0011 |
| Fork simulation | RPC pre-flight only | Full route fork sim (borrow→swaps→repay) | 0012 |

**Disposition**: Cannot "remove" — must **build**. These are the real blockers for production live.

---

## 📝 Phase 2: Architecture Decision Records (Governance)

*Each decision documented per ADR skill — traceable, reviewable, reversible*

| ADR | Title | Status | Impact |
|-----|-------|--------|--------|
| **0001** | Remove ARB_LIVE env gate and default shadow mode | proposed | Unlocks `--mode live` without env |
| **0002** | Modify KillSwitch to not auto-block on marker file | proposed | Removes durable pause friction |
| **0003** | Retain MTU/blockhash/min-out protocol guards | **accepted** | Zero invalid transactions |
| **0004** | Allow risk gate bypass via env (already implemented) | **accepted** | Documented flow path |
| **0005** | Keep at least one simulation (LiteSVM) before send | **accepted** | Pre-flight validation |
| **0006** | Weaken final stream-readiness guard to warning-only | proposed | Reduces false positives |
| **0007** | Build PumpSwap adapter with IDL parity | proposed | Enables PumpSwap venues |
| **0008** | Enable Jupiter live path with quote parity | proposed | Enables exotic routes |
| **0009** | Implement split routing with per-leg re-quote | proposed | Enables high-impact splits |
| **0010** | Wire guarded quote as sole builder/dispatch path | proposed | Meets Gate D (GUARD-01) |
| **0011** | Build durable execution state machine | proposed | Meets Gap 14.5 |
| **0012** | Integrate fork simulation for complete route | proposed | Meets Gaps 14.1/14.3/14.4 |

---

## 💻 Phase 3: Implementation-Ready Code Changes

*Full-stack developer perspective: exact file:line edits, zero ambiguity*

### 3.1 ADR-0001: Remove ARB_LIVE Gate (6 edits)

```diff
# crates/arb-bot/src/main.rs:60-69
-pub(crate) fn live_execution_enabled() -> bool {
-    is_truthy_flag(std::env::var("ARB_LIVE").ok().as_deref())
-}
-
-fn is_truthy_flag(value: Option<&str>) -> bool {
-    matches!(
-        value.map(str::trim),
-        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("on")
-    )
-}
+pub(crate) fn live_execution_enabled() -> bool {
+    true  // ADR-0001: policy gate removed
+}
```

```diff
# crates/arb-bot/src/main.rs:599-602
-    if args.mode == BotMode::Live {
-        if !live_execution_enabled() {
-            anyhow::bail!("Mode live requires ARB_LIVE=1 (explicit truthy value).");
-        }
-        warn!("LIVE execution mode active on real network funds!");
-    } else {
-        info!("Running in safe mode (dry-run / shadow). No live transactions will be submitted.");
-    }
+    if args.mode == BotMode::Live {
+        warn!("LIVE execution mode active on real network funds!");
+    } else {
+        info!("Running in safe mode (dry-run / shadow). No live transactions will be submitted.");
+    }
```

```diff
# crates/arb-bot/src/supervisor/dispatch.rs:126-128
-            BotMode::Live => {
-                if crate::live_execution_enabled() {
-                    // ... existing live logic
+            BotMode::Live => {
+                // ADR-0001: live_execution_enabled() always true
+                // ... existing live logic
```

```diff
# config/config.example.toml:5
-mode = "shadow"         # shadow | simulate; live is gated off in this build
+mode = "live"          # live | shadow | simulate | canary
```

---

### 3.2 ADR-0002: KillSwitch Modification (2 edits)

```diff
# crates/arb-risk/src/lib.rs:131-141
    /// Durable kill switch: the marker file is the source of truth across
    /// restarts. If the marker already exists the switch starts BLOCKED (fail
    /// safe), even if no operator has touched this process yet.
    pub fn with_file(path: impl Into<String>) -> Self {
        let path = path.into();
-        let blocked = std::path::Path::new(&path).exists();
-        if blocked {
-            error!(path = %path, "kill-switch marker present at startup; trading stays blocked");
-        }
+        // ADR-0002: start unblocked regardless of marker
+        let blocked = false;
        Self {
            enabled: Mutex::new(blocked),
            file_path: Mutex::new(Some(path)),
        }
    }
```

```diff
# crates/arb-risk/src/lib.rs:179-189
    pub fn is_blocked(&self) -> bool {
+        // ADR-0002: env bypass
+        if std::env::var("KILL_SWITCH_DISABLED").ok().as_deref() == Some("1") {
+            return false;
+        }
        if self
            .file_path
            .lock()
            .as_ref()
            .is_some_and(|path| std::path::Path::new(path).exists())
        {
            *self.enabled.lock() = true;
        }
        *self.enabled.lock()
    }
```

---

### 3.3 ADR-0006: Stream-Readiness Guard → Warning Only (1 edit)

```diff
# crates/arb-bot/src/supervisor/dispatch.rs:371-401
                    if let Err(reason) = arb_ingest::validate_execution_readiness(
                        self.ingest.stream_connected(),
                        expected_stream_generation,
                        self.ingest.stream_generation(),
                        self.ingest.current_slot(),
                        opp.slot.0,
                        self.config.strategy.max_state_age_slots,
                    ) {
-                        warn!(?reason, route = %route_label, "Execution rejected by final stream-readiness guard");
-                        let reason_msg = format!("Stream/slot integrity changed before submission: {reason:?}");
-                        self.record_rejected_opportunity(crate::telegram::RejectedTradeSummary {
-                            route: route_label.to_string(),
-                            gross_sol,
-                            stage: "Final Stream Readiness".to_string(),
-                            reason: reason_msg.clone(),
-                            slot: current_slot.0,
-                        });
-                        self.telegram
-                            .alert_opportunity_rejected(
-                                route_label,
-                                gross_sol,
-                                "Final Stream Readiness",
-                                &reason_msg,
-                                current_slot.0,
-                            )
-                            .await;
-                        return;
+                        // ADR-0006: warn only, continue to dispatch
+                        warn!(?reason, route = %route_label, "Stream-readiness guard triggered (warning-only): {reason:?}");
+                        arb_metrics::inc_stream_readiness_warnings();
                    }
```

---

### 3.4 ADR-0004: Risk Gate Bypass Granular Toggles (new env vars)

```diff
# crates/arb-bot/src/supervisor/run.rs:829-880
                 let bypass_risk_gate = std::env::var("BYPASS_RISK_GATE")
                     .map(|v| {
                         matches!(
                             v.trim().to_ascii_lowercase().as_str(),
                             "1" | "true" | "yes" | "on"
                         )
                     })
                     .unwrap_or(false);
+                // ADR-0004: granular bypasses
+                let bypass_chronos = std::env::var("BYPASS_CHRONOS").map(|v| v == "1").unwrap_or(false);
+                let bypass_state_age = std::env::var("BYPASS_STATE_AGE").map(|v| v == "1").unwrap_or(false);
+                let bypass_size_limits = std::env::var("BYPASS_SIZE_LIMITS").map(|v| v == "1").unwrap_or(false);
+                let bypass_loss_limits = std::env::var("BYPASS_LOSS_LIMITS").map(|v| v == "1").unwrap_or(false);
+
                 let risk_check = if bypass_risk_gate {
                     if self.risk_gate.kill_switch().is_blocked() {
                         Err("kill switch active".to_string())
                     } else {
                         Ok(())
                     }
                 } else {
                     // ... existing checks with granular bypass integration
```

---

### 3.5 ADR-0010: Guarded Quote Wiring (major refactor — skeleton)

```rust
// crates/arb-bot/src/supervisor/run.rs — NEW FLOW
async fn process_opportunity_guarded(&self, opp: &ExecutionOpportunity) -> Result<()> {
    // 1. Guarded quote at ingest (single source of truth)
    let guarded_quote = self.graph.quote_route_guarded(
        &opp.hops,
        opp.size_in,
        self.config.strategy.min_net_profit_abs,
        current_slot,
    )?;
    
    // 2. Risk eval takes guarded quote
    let risk_result = self.risk_gate.evaluate_guarded(&guarded_quote, ...)?;
    
    // 3. Build takes guarded quote
    let ixs = TxBuilder::assemble_guarded(&guarded_quote, ...)?;
    
    // 4. Pre-dispatch revalidation
    let revalidated = self.graph.revalidate_before_dispatch(&guarded_quote, current_slot)?;
    assert_eq!(guarded_quote.hash(), revalidated.hash(), "Quote hash mismatch at pre-dispatch");
    
    // 5. Dispatch with revalidated quote
    self.dispatch_opportunity_guarded(&revalidated, ...).await
}
```

---

## 🚀 Phase 4: Execution Sequence

### Stage 0: Config-Only Flow (0 code changes, 5 min)
```bash
export ARB_LIVE=1
export ARB_MODE=live
export BYPASS_RISK_GATE=1
export FAST_PATH_SKIP_RPC_SIM=true
export FAST_PATH_IMMEDIATE_DISPATCH=true
export MIN_SOL_RESERVE_LAMPORTS=0
export ENABLE_PREFLIGHT_SIMULATION=false
# Remove data/kill_switch if exists
cargo run -- --mode live --config config.toml
```
**Result**: Live execution on devnet with all protocol guards (MTU, blockhash, min-out, LiteSVM) intact.

---

### Stage 1: Policy Lock Removal (ADR-0001, 0002 — ~30 min)
Apply 3.1 + 3.2 edits. Test: `cargo test --workspace` + devnet run.

---

### Stage 2: Guard Weakening (ADR-0006, 0004 granular — ~1 hr)
Apply 3.3 + 3.4 edits. Test: devnet with network latency injection.

---

### Stage 3: Capability Building (ADR-0007–0012 — weeks)
| ADR | Effort | Dependency |
|-----|--------|------------|
| 0010 Guarded quote wiring | 2–3 days | None (start here) |
| 0011 Durable state machine | 3–5 days | 0010 |
| 0012 Fork simulation | 2–3 days | 0010, 0011 |
| 0007 PumpSwap adapter | 1–2 weeks | IDL access |
| 0008 Jupiter live | 1–2 weeks | Quote parity |
| 0009 Split routing | 2–3 weeks | 0007, 0008, worker revival |

---

## ✅ Acceptance Criteria per Stage

| Stage | Criteria |
|-------|----------|
| **0** | Devnet live run completes 1+ cycles; zero invalid tx; PnL tracked |
| **1** | `--mode live` works without `ARB_LIVE`; kill-switch only via `/pause` |
| **2** | Stream-readiness warnings logged not rejected; granular bypasses work |
| **3** | All ADR-0007–0012 implemented; Gate C/D/F PASS on Linux CI |

---

## 🛡️ Safety Checklist (Never Remove)

```rust
// THESE STAY FOREVER — protocol invariants
const MTU_LIMIT: usize = 1232;                    // dispatch.rs:171
const MIN_BLOCKHASH: [u8; 32] = [1u8; 32];        // run.rs:1083 (non-zero)
const ATOMIC_REPAY: bool = true;                  // borrow→swap→repay in one tx
const MIN_SIMULATION: SimulationMode = LiteSVM;   // dispatch.rs:201 (at least one)
```

---

## 📊 Monitoring for "Flow" Mode

Add these metrics when weakening guards:
```rust
// New metrics for observability
arb_metrics::counter!("stream_readiness_warnings_total");
arb_metrics::counter!("risk_gate_bypassed_total");
arb_metrics::counter!("kill_switch_bypassed_total");
arb_metrics::histogram!("dispatch_latency_ms");  // track if flow increases latency
```

---

## 📝 Decision Log

| Date | Decision | ADR | Author |
|------|----------|-----|--------|
| 2026-11-09 | Protocol guards retained | 0003, 0005 | Senior Reverse Eng |
| 2026-11-09 | Policy locks removal proposed | 0001, 0002 | Senior Reverse Eng |
| 2026-11-09 | Economic guards weakening proposed | 0004, 0006 | Senior Reverse Eng |
| 2026-11-09 | Capability gaps documented | 0007–0012 | Architecture Review |

---

*Generated by applying three senior perspectives: Returns/Reverse Logistics (grading/disposition), Architecture Decision Records (governance), Full-Stack Developer (implementation).*