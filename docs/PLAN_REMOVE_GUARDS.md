# خطة: إزالة الأقفال والحراسة المشددة وتمكين execution live (انسيابية الكود)

> **تحذير مالي (اقرأ أولاً):** هذا المستودع يتعامل بأموال حقيقية على Solana. إزالة الحراسة لا "تجعل الكود يتدفق" — بعض الحراسة هي *متطلبات بروتوكول* (MTU 1232 بايت، blockhash غير صفري، atomicity)، وليست "أماناً اختيارياً". إزالتها لا تمكّن execution، بل تُرسل معاملات معطوبة تُرفض on-chain (أو أسوأ: معاملات تُفقد أموالاً). الكود himself يعترف بأن `ARB_LIVE` gate و kill-switch و fail-closed هي حماية متعمدة، وليس bugs.

> **توصية:** لا تقم بـ big-bang removal. اتبع الخطة على مراحل، مع shadow soak 24–72h بين كل مرحلة، وابدأ بـ "القفل (lock)" فقط، وليس بالحراسة (guard).

---

## A. تحليل: "قفل" vs "حارس" vs "فجوة معمارية"

| 类型 | مثال | إزالة آمنة؟ | السبب |
|---|---|---|---|
| **قفل (Lock)** | `ARB_LIVE` gate، mode default shadow، kill-switch marker | نعم (إذا قبلت المخاطرة) | بوابة policy خالصة — لا تؤثر على صحة المعاملة |
| **حارس بروتوكول (Protocol guard)** | MTU 1232، zero blockhash، min-out floor، atomic repay | **لا** — إزالتها = معاملات معطوبة on-chain | حدود شبكة Solana / invariant Kamino |
| **حارس اقتصادي (Economic guard)** | safety floor reserve، risk gate، Chronos EV، state staleness | "آمن" للإزالة من حيث الإرسال، لكن خطر مالي | تمنع خسائر — إزالتها = no safety net |
| **حارس simulation** | LiteSVM، RPC pre-flight sim | إزالتها = إرسال بدون verify | simulation هي *الدليل الوحيد* قبل الإرسال |
| **فجوة معمارية (Architectural gap)** | GUARD-01 (quote gardé بدون callers)، durable state machine، fork sim | **لا يمكن "إزالة" — يجب بناؤها** | إزالة = لا يوجد بديل |

---

## B. الخطة (4 مراحل)

### المرحلة 1 — إزالة الأقفال (Locks only) — risk: منخفض (policy)

**الهدف:** السماح بـ `--mode live` بدون `ARB_LIVE=1`، وبدون kill-switch marker.

| # | الملف:السطر | التغيير | ملاحظة |
|---|---|---|---|
| 1.1 | `crates/arb-bot/src/main.rs:599-602` | احذف كتلة `if args.mode == BotMode::Live { if !live_execution_enabled() { bail!(…) } }` (أو اجعلها warning only) | يسمح بـ live بدون env gate |
| 1.2 | `crates/arb-bot/src/main.rs:60-69` | اجعل `live_execution_enabled()` تُرجع `true` دائماً (أو احذف استدعاءات 127/128 في dispatch) | dispatch.rs:127 يعيد التحقق |
| 1.3 | `crates/arb-bot/src/supervisor/dispatch.rs:127` | احذف `if crate::live_execution_enabled() { … }` wrapper (أو اجعله no-op) | |
| 1.4 | `config/config.example.toml:5` | `mode = "live"` (أو "shadow" مع `--mode live`) | |
| 1.5 | `crates/arb-risk/src/lib.rs:131-141` | `with_file` → start unblocked (أو استخدم `KillSwitch::new()` بدلاً من `with_file`) | **خطر:** pause durables يفقد معنى |
| 1.6 | `run.sh:13` | `export ARB_LIVE=1` يمكن إزالته | |

**اختبار:** `cargo test --workspace` + تشغيل `--mode live` على devnet/testnet (لا mainnet).

---

### المرحلة 2 — تمكين adapters معطلة (Adapters) — risk: متوسط (correctness)

**الهدف:** PumpSwap، Jupiter، Split، Route worker.

| # | الملف:السطر | التغيير | متطلب مسبق (لا تتجاوز) |
|---|---|---|---|
| 2.1 | `crates/arb-tx/src/lib.rs:1443` | استبدل hard error بـ adapter فعلي (decode pool accounts من on-chain) | IDL parity for PumpSwap (حالياً "not enabled; shadow-only") |
| 2.2 | `crates/arb-bot/src/supervisor/run.rs:912` | `route_via_jupiter = true` | Jupiter quote parity (fees, account metas, min-out, flash-loan repay) — حالياً "not proof" |
| 2.3 | `crates/arb-bot/src/supervisor/run.rs:814` | `split = Some(...)` (أو حساب split فعلي) | Per-leg re-quoting + aggregate min-out (SPLIT-01) — حالياً "not implemented" |
| 2.4 | `crates/arb-engine/src/route_worker.rs:310-318` | إعادة تطبيق `start_worker_loop` (بدون fabricated economics: Kamino fee from on-chain، tip from adaptive ledger) | FIN-01/FIN-03 compliance |
| 2.5 | `crates/arb-bot/src/supervisor/run.rs:375` | تمكين HTTP catalog discovery | |

**تحذير:** 2.1–2.3 حالياً معطلة *عمداً* لأن parity غير مثبت. تمكينها بدون parity = معاملات معطوبة. **لا تتجاوز المتطلبات المسبقة.**

---

### المرحلة 3 — إزالة/تخفيف الحراس (Guards) — risk: عالٍ (مالي)

**توصية:** *لا* تزل كل الحراس. إزالة "harsh guard" ≠ "flow". بعضها بروتوكول، بعضها اقتصادي.

#### 3a. حراس بروتوكول — **أبقِها** (لا تزل)

| # | الملف:السطر | السبب للإبقاء |
|---|---|---|
| — | `dispatch.rs:171-199` (MTU 1232) | حد сеть Solana — معاملات >1232 dropped by validators |
| — | `run.rs:1083-1084` (zero blockhash) | validators يرفضون blockhash صفر |
| — | `dispatch.rs` min-out floor | Kamino repay atomicity — بدون min-out، swap يمكن أن يفشل repay → tx revert (an atomic، لكن gas/fee lost) |

#### 3b. حراس اقتصادي/simulation — **تخفيف، وليس إزالة**

| # | الملف:السطر | التغيير المقترح (تخفيف) | risk |
|---|---|---|---|
| 3b.1 | `dispatch.rs:129-169` (safety floor reserve) | اجعل `min_reserve_lamports` default 0 (fast-path flash loan) — **بالفعل default 0** (`config.example.toml:46`: `min_sol_balance_lamports = 0`). فقط تأكد من env `MIN_SOL_RESERVE_LAMPORTS=0`. | منخفض |
| 3b.2 | `dispatch.rs:201-216` (LiteSVM sim) | اجعل `local_sim` warning-only (لا `return` on failure) — **خطر:** LiteSVM feature-off يُرجع `success=false` (fail-closed by design) | متوسط |
| 3b.3 | `dispatch.rs:218-342` (RPC pre-flight sim) | `ENABLE_PREFLIGHT_SIMULATION=false` (أو `FAST_PATH_SKIP_RPC_SIM=true`) → skip sim | **عالٍ** — sim هو الدليل الوحيد قبل الإرسال |
| 3b.4 | `dispatch.rs:371-401` (final stream-readiness) | تخفيف: اجعل guard يُسجل warning بدلاً من `return` | **عالٍ** — stream generation/slot integrity |
| 3b.5 | `run.rs:882-904` (risk gate) | `BYPASS_RISK_GATE=1` → bypass (موجود بالفعل) | **عالٍ** — no safety net |
| 3b.6 | `run.rs:809-810` (state staleness) | زيادة `max_state_age_slots` (مثلاً 4→15) أو `graph_max_age_slots` | متوسط |
| 3b.7 | `run.rs:1083-1084` (zero blockhash) | **أبقِ** — بروتوكول | — |

#### 3c. kill-switch — **تخفيف**

| # | الملف:السطر | التغيير |
|---|---|---|
| 3c.1 | `crates/arb-risk/src/lib.rs:179-189` | اجعل `is_blocked()` يقرأ env `KILL_SWITCH_DISABLED=1` → return false | **عالٍ** — يفقد durable pause |

---

### المرحلة 4 — الفجوات المعمارية (لا "إزالة"، بل "بناء")

**GUARD-01 (OPEN)** — `quote_route_guarded` / `revalidate_before_dispatch` بدون callers:
- **الخيار A (صحيح):** wire guarded quote path as the *only* builder/dispatch entry؛ assert same quote hash at risk/build/pre-dispatch. (это то, что требует release readiness.)
- **الخيار B (تخفيف، محفوف بالمخاطر):** إزالة guarded quote полностью; استمرار direct path. **هذا هو "الانسيابية" التي طلبتها، لكنه يلغي defense-in-depth.** Gate C/D останется OPEN.

**Durable state machine (14.5 OPEN):** لا يمكن "إزالة" — يجب بناؤها (accepted→landed reconciliation). بدونها، crash بعد send = bundle غير متعقب.

**Fork simulation (14.1/14.3/14.4):** لا يمكن "إزالة" — يجب بناؤها.

---

## C. خلاصة "ما à شيل / ما à بقي"

### شيل (can be removed safely-ish):
- `ARB_LIVE` gate (1.1–1.3)
- mode default shadow → live (1.4)
- kill-switch marker auto-block (1.5) — with warning
- `BYPASS_RISK_GATE=1` already supported (3b.5)
- `FAST_PATH_SKIP_RPC_SIM=true` already supported (3b.3)
- `MIN_SOL_RESERVE_LAMPORTS=0` already default (3b.1)
- `max_state_age_slots` tuning (3b.6)

### أبقِ (do not remove — protocol/economic safety):
- MTU 1232 check (dispatch.rs:171-199)
- Zero blockhash check (run.rs:1083-1084)
- Min-out floor (economic invariant for Kamino repay)
- Atomicity (borrow→swap→repay in one tx) — это сама суть
- At least *one* simulation (LiteSVM or RPC) before send
- Stream-readiness guard (or weaken, don't delete)

### لا "شيل" — "ابنِ":
- PumpSwap adapter (2.1) — needs IDL parity
- Jupiter live path (2.2) — needs quote parity
- Split routing (2.3) — needs per-leg re-quote
- Route worker (2.4) — needs non-fabricated economics
- GUARD-01 wiring (4) — needs guarded quote as sole path
- Durable state machine (4) — needs implementation
- Fork simulation (4) — needs implementation

---

## D. "الانسيابية" (smooth flow) — ما هي *فعلاً*؟

إذا كنت تريد "انسيابية" (no friction، no false positives، fast path)، then:

1. **الأسهل (config only, 0 code changes):**
   ```
   ARB_LIVE=1
   ARB_MODE=live
   BYPASS_RISK_GATE=1          # or tune risk limits
   FAST_PATH_SKIP_RPC_SIM=true  # skip RPC sim (keep LiteSVM)
   FAST_PATH_IMMEDIATE_DISPATCH=true
   MIN_SOL_RESERVE_LAMPORTS=0
   ENABLE_PREFLIGHT_SIMULATION=false  # or keep true for safety
   ```
   + إزالة kill-switch marker.
   → يمنح "انسيابية" مع إبقاء MTU/blockhash/min-out/sim (LiteSVM).

2. **المرحلة 1 (locks only):** إزالة ARB_LIVE gate + mode default → live بدون env var.

3. **المرحلة 3b (guards تخفيف):** إزالة safety floor (already 0 default)، تخفيف risk gate، تخفيف stream-readiness guard. **هنا يبدأ "الانسيابية" الحقيقية — و danger.**

4. **لا تفعل (إلا إذا قبلت خسارة أموال):** إزالة MTU، zero blockhash، min-out floor، جميع simulation، kill-switch، durable pause.

---

## E. توصية (opinionated)

**لا أوصي بإزالة كل الحراسة.** "الحراسة المشددة" ليست obstacle — هي *السبب* الذي يجعل الكود safe للتشغيل. إزالتها بالكامل (big-bang) = arbitrage bot يرسل معاملات on-chain بدون verify، بدون pause، بدون safety net. في أسوأ حالة: Kamino flash loan not repaid → tx revert (atomic، لكن fees lost)；في حالة أسوأ: slippage beyond min-out → swap executes at bad price → realized loss.

**البديل (recommended):** 
- اتبع **المرحلة 1 (locks)** + **المرحلة D.1 (config-only smoothness)** أولاً.
- شغّل على **devnet** (أو mainnet بـ capital صغير) مع shadow soak.
- إذا أردت "انسيابية" más، ففقط **المرحلة 3b** (تخفيف، not حذف) with monitoring.
- **لا تفعل** إزالة MTU/blockhash/min-out/sim/kill-switch/durable pause.

---

## F. إذا أردت أن *أقوم* بالتغيير (execution)

قل "نفّذ المرحلة X" وسأقوم بـ:
- Phase 1 only (locks) — ~6 edits، low risk.
- Phase 1 + 3b (locks + guard relaxation) — ~12 edits، medium risk.
- Full big-bang (all locks + guards + adapters + gaps) — **لا أوصي**؛ requires ~40+ edits + new implementations (PumpSwap adapter، Jupiter parity، split re-quote، durable state machine، fork sim) which are not "removal" but "construction".

> **قرارك:** أي مرحلة تختار؟ (1 / 1+3b / big-bang) — و على أي сеть (devnet / mainnet)؟