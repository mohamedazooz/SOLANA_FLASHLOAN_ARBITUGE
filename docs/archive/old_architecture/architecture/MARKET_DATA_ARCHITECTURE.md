> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# معمارية بيانات السوق (Market Data Architecture)

- **آخر تحديث:** 2026-11-19

## 1. تدفّق البيانات

```
Helius WS / Yellowstone gRPC
        ↓  account updates (slot, write_version)
   decoders (arb-ingest)
        ↓  PoolState
   hydration / dual_ingest  (reject stale writes)
        ↓  consistent snapshot keyed by slot
   arb-dex QuoteEngine
        ↓  fees / reserves / ticks
   arb-engine RouteGraph + sizing
        ↓
   Opportunity (immutable, slot-tagged)
```

## 2. مصادر البيانات

| المصدر | الاستخدام | ملاحظة |
|---|---|---|
| Helius WSS | تحديثات الحسابات | يحتاج `.env` |
| Yellowstone gRPC | تدفّق منخفض الزمن | Linux-only |
| RPC discovery | اكتشاف البولات | poll/backfill |
| DexScreener / GMGN | اكتشاف/وسوم | **ليس** تسعيراً تنفيذياً |

## 3. فكّ الترميز (Decoding)

كل بروتوكول له decoder مستقل في `arb-dex`:

| البروتوكول | الملف | ما يُفكّ |
|---|---|---|
| Raydium v4 | `raydium_v4.rs` | vaults, mints, fee numerator/denominator, need_take_pnl |
| Raydium CLMM | `raydium_clmm.rs` | sqrt_price, ticks, fee |
| Raydium CPMM | `raydium_cpmm.rs` | reserves + fee (1e6 denom) |
| Orca Whirlpool | `orca_whirlpool.rs` | fee_rate, liquidity, sqrt_price, tick_current, mints, vaults |
| Meteora DLMM | `meteora_dlmm.rs` | active bin, bin_step, variable fee params |
| Meteora DAMM v2 | `meteora_damm_v2.rs` | reserves, fees |
| Pump.fun / PumpSwap | `engines/*` | curve/AMM state |

### قواعد إلزامية

- أي حقل مفقود أو إزاحة غير مؤكدة → **رفض** (`None`/`Err`)، لا قيمة افتراضية.
- `fee_rate` الخام هو المصدر؛ لا تُحوَّل إلى bps وتعود.
- `need_take_pnl` تُخصم من الاحتياطي كما يفعل البرنامج.

## 4. الحداثة (Freshness)

| الحاجز | الموقع |
|---|---|
| `max_age_slots` | `arb-engine/src/guarded.rs` |
| `max_quote_age_slots` | `arb-engine/src/meteora_fresh_state.rs` |
| `StalePoolState` | `arb-engine/src/cross_pool_dlmm.rs` |
| رفض الكتابات الأقدم | `arb-ingest/src/dual_ingest.rs` |

**القاعدة:** إذا تعذّر إثبات الحداثة، تُرفض الفرصة. لا يوجد fallback إلى cache.

## 5. حساب التسعير

- المصدر الوحيد: `arb-math`.
  - CPMM: `cpmm_out_upfront_fee_ceil` / `cpmm_out_fee`.
  - CLMM: `quote_clmm` + `compute_swap` + `find_next_tick_boundary`.
  - DLMM: `quote_dlmm_view`.
- `arb-dex` يغلّف هذه الدوال بواجهة `QuoteEngine`.
- `arb-engine` لا يعيد تنفيذ الحساب؛ يستدعي `arb-math`.

### Tick Arrays (Orca)

- المصدر: `orca_tick_array_start_indexes`.
- الـadapter ينشر `[a_to_b ×3][b_to_a ×3]` في `execution_accounts`.
- builder يختار النصف حسب اتجاه الرحلة (`select_tick_arrays`).
- أي غياب لمصفوفة الـtick الحالي → `MissingTickArrayCoverage` (fail closed).

## 6. نقاط الضعف

1. **Token-2022 transfer fees** غير متحقَّقة حياً.
2. **rate-limit** في discovery قد يُسقط بولات دون إنذار واضح (بيئي).
3. **latency_bench** يستخدم عناوين vault مضمّنة.
4. لا يوجد **CI benchmark** للحداثة/التسعير.

## 7. مراجع

- `crates/arb-ingest/src/{helius_ws,decoders,dual_ingest,discovery}.rs`
- `crates/arb-dex/src/*.rs`
- `crates/arb-math/src/{cpmm,clmm}.rs`
- `crates/arb-engine/src/{guarded,meteora_fresh_state,cross_pool_dlmm}.rs`
