# اعمال HEAD غير المدمجة من dispatch.rs — 2026-11-16

هذا الملف يحفظ **كامل محتوى جانب HEAD** من كتلة التعارض الوحيدة في
`crates/arb-bot/src/supervisor/dispatch.rs` (الأسطر 192–293 من نسخة العمل)
التي كانت تحمل علامات تعارض غير محلولة:

```text
[CONFLICT: HEAD]
   ... محتوى HEAD ...
[CONFLICT: EQUALS]
   ... محتوى origin/main ...
[CONFLICT: origin/main]
```

## لماذا تم اختيار جانب origin/main؟

الجانب HEAD يحوي متغيراً أقدم وغير مكتمل:
1. Safety Floor ثابت 50_000_000 lamports (0.05 SOL) و bloque RPC `get_balance` عند كل dispatch.
   هذا يتعارض مع تصميم mainline الحالي (`MIN_SOL_RESERVE_LAMPORTS=0` افتراضي = fast-path flash loan
   دون حظر RPC) ومع ضبط `min_sol_balance_lamports = 0` في production.toml.
2. Cold-Wallet Auto-Sweep تلقائي — غير مرغوب بدون إذن صريح من المُشغل (الموجّه يمنع أي sweep تلقائي
   للأموال).
3. الجانب origin/main (الإصدار الحالي من main) هو **المسار الثابت المُختبَر في CI**: يفرض سياسة
   V1-only، والتحقق من Transport Capability، وضبط الحد الأدنى للاحتياطي من الإعداد،
   وتسجيل رفض المعاملات بتفاصيل كاملة — وهو متوافق مع الموجه (core).

## محتوى HEAD المحفوظ (حرفي)

```rust
if crate::live_execution_enabled() {
    // Safety Floor Check: Enforce minimum 0.05 SOL reserve (50_000_000 lamports)
    let current_balance = self.rpc_pool.get_balance(&payer).await.unwrap_or(0);
    let min_reserve_lamports = 50_000_000u64;
    if current_balance < min_reserve_lamports {
        warn!(
            "🚨 Safety Floor Active: Wallet balance ({} lamports / {:.4} SOL) is below 0.05 SOL minimum reserve — skipping execution to protect fees",
            current_balance,
            current_balance as f64 / 1e9
        );
        self.telegram
            .alert_opportunity_rejected(
                route_label,
                gross_sol,
                "Safety Floor Reserve",
                &format!(
                    "رصيد المحفظة ({:.4} SOL) أقل من صمام الأمان الأدنى (0.05 SOL)",
                    current_balance as f64 / 1e9
                ),
                current_slot.0,
            )
            .await;
        return;
    }

    // Cold Wallet Auto-Sweep: Transfer accrued profit above reserve
    let target_cold = std::env::var("COLD_WALLET_ADDRESS")
        .or_else(|_| std::env::var("PROFIT_RECEIVER_ADDRESS"))
        .ok()
        .or_else(|| self.config.wallet.cold_wallet_pubkey.clone());

    if let Some(cold_wallet_str) = target_cold
        && let Ok(cold_wallet) = Pubkey::from_str(&cold_wallet_str)
    {
        if let Some(ref signer) = self.keypair {
            if cold_wallet == signer.pubkey() {
                debug!(
                    "Cold wallet destination is identical to signer hot wallet; skipping self-sweep"
                );
                return;
            }
        }
        let sweep_threshold = 250_000_000u64; // 0.25 SOL total (0.05 reserve + 0.20 profit)
        if current_balance > sweep_threshold {
            let sweep_lamports = current_balance
                .saturating_sub(min_reserve_lamports)
                .saturating_sub(10_000);
            let sweep_sol = sweep_lamports as f64 / 1e9;
            info!(
                "💰 Cold Wallet Sweep: Executing on-chain transfer of {:.4} SOL -> {}",
                sweep_sol, cold_wallet
            );
            if let Some(ref signer) = self.keypair {
                let Some(blockhash) = self.fetch_recent_blockhash().await else {
                    warn!("Cold Wallet Sweep skipped: no valid blockhash");
                    return;
                };
                let transfer_ix = arb_tx::build_system_transfer(
                    signer.pubkey(),
                    cold_wallet,
                    sweep_lamports,
                );
                if let Ok(signed_tx) = TxBuilder::build_signed_transaction(
                    signer,
                    &[transfer_ix],
                    blockhash,
                ) {
                    use base64::Engine;
                    let b64 = base64::engine::general_purpose::STANDARD
                        .encode(&signed_tx);
                    match self.rpc_pool.send_raw_transaction(&b64).await {
                    (الجزء الكامل أقصر من نسخة العمل — انظر git HEAD `54d2cbb` للنسخة الكاملة.)
```

---

**التاريخ**: 2026-11-16
**القرار**: دمج = جانب origin/main + توثيق كامل لما أُهمل من HEAD.
**الحالة**: إذا أراد المُشغل الاحتفاظ بـ Cold-Wallet Auto-Sweep، يجب أن يُنفَّذ كأمر يدوي من المُشغل
(مثل سكربت `solana transfer <amount> <cold>`) وليس كمسار تلقائي داخل dispatch.