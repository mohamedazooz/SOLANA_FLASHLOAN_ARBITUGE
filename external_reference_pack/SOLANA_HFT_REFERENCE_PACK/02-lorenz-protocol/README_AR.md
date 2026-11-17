# 02 — Lorenz Protocol

**Upstream:** https://github.com/lorenz-protocol/lorenz  
**Default branch:** `main`  
**License:** MIT  
**Nature:** Experimental research framework؛ README يصرح بأنه لا يتداول live ولم يخضع لتدقيق أمني خارجي.

## القيمة الأساسية
Lorenz مفيد كمرجع معماري واختباري، وليس كـexecution engine جاهز.

### A. Integer-only settlement math
`crates/lorenz-amm/src/lib.rs` ينفذ CPMM math باستخدام integer arithmetic وchecked operations:
- fee applied to input.
- checked multiplication/addition.
- exact-output inverse with ceiling rounding.
- property tests مثل monotonicity وbounded output.

المعادلات الأساسية:
```text
in_after_fee = floor(amount_in * (10_000 - fee_bps) / 10_000)
amount_out   = floor(reserve_out * in_after_fee /
                     (reserve_in + in_after_fee))
```
لا تستخدم floating point في amounts التي تدخل في settlement.

### B. Graph arbitrage
`crates/lorenz-graph/src/lib.rs` هو المرجع لدراسة negative-cycle detection / Bellman-Ford على token graph.
استخدمه لمقارنة اكتشاف cycle topology فقط. لا تخلط edge weights التحليلية مع quote executable exact-in.

### C. Explicit invariants
`docs/INVARIANTS.md` يربط كل ضمان بمكان تنفيذه واختباره:
- atomic-or-revert.
- minimum profit floor.
- bounded spend.
- scoped authority.
- integer settlement math.
- strict config.
- agent can only tighten limits.
- shared cost model.
- append-only decision ledger.

هذه أفضل ممارسة تستحق النقل إلى وثائق مشروعك: كل invariant يجب أن يذكر enforcement point واختباراً يثبت ذلك.

### D. Risk manager
`crates/lorenz-agent/src/risk.rs` يقدم kill switch وconsecutive-loss guard وdynamic cap لا يستطيع الوكيل رفعه فوق hard ceiling.
انقل المبدأ فقط: الـLLM أو agent لا يملك صلاحية توسيع حدود المخاطر.

### E. Yellowstone seam — نقطة تحذير مهمة
`crates/lorenz-stream/src/geyser.rs` يعلن صراحة أن `connect()` يرجع `NotImplemented`؛ أي أن هذا المشروع لا يقدم transport live جاهزاً في هذه الطبقة.
لا تعتبر وجود `GeyserConfig` أو module باسم geyser دليلاً على اتصال Yellowstone فعلي.

## الملفات المرجعية
- `crates/lorenz-amm/src/lib.rs`
- `crates/lorenz-graph/src/lib.rs`
- `crates/lorenz-dex/src/decoder.rs`
- `crates/lorenz-stream/src/geyser.rs`
- `crates/lorenz-agent/src/risk.rs`
- `crates/lorenz-backtest/src/lib.rs`
- `docs/INVARIANTS.md`
- `docs/ARCHITECTURE.md`
- `docs/SECURITY.md`

## ما يجب اختباره في مشروعك
- arithmetic overflow boundaries.
- exact-input/exact-output rounding.
- monotonicity.
- output cannot exceed reserve/depth.
- cycles reconstructed from executable directed edges.
- risk cap cannot be widened by config/agent.
- cost model parity between backtest and live quote.
