//! Canonical TxBuilder: full transaction assembly, signing and serialization.

use crate::*;

/// Builder for constructing atomic arbitrage transactions and private bundles.
#[derive(Debug, Default, Clone)]
pub struct TxBuilder {
    instructions: Vec<CompiledInstruction>,
}

impl TxBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an instruction to the transaction.
    pub fn add_instruction(&mut self, ix: CompiledInstruction) {
        self.instructions.push(ix);
    }

    /// Assemble strictly the atomic 7-instruction (2-hop) or 8-instruction (3-hop) bundle.
    /// Excludes ephemeral ATA creation from the hot path.
    ///
    /// Instruction Layout:
    /// [0] SetComputeUnitLimit (260,000 for 2-hop / 320,000 for 3-hop)
    /// [1] SetComputeUnitPrice (priority fee micro-lamports)
    /// [2] Kamino FlashBorrow (amount: d*)
    /// [3] Swap Leg 1
    /// [4] Swap Leg 2 (and 3 if 3-hop)
    /// [5] (or [6]) Kamino FlashRepay (amount: d* + fee, borrow_instruction_index: 2)
    /// [6] (or [7]) Jito Tip Transfer
    pub fn assemble_atomic_bundle(
        payer: Pubkey,
        tip_account: Pubkey,
        tip_amount: Lamports,
        priority_fee_micro_lamports: u64,
        opportunity: &ExecutionOpportunity,
        kamino_params: &KaminoFlashParams,
        flash_loan_fee_sf: u64,
        min_profit_raw: TokenAmount, // Profit floor in base-mint raw units
    ) -> Result<Vec<CompiledInstruction>, TxError> {
        let hops_count = opportunity.hops.len();
        if !(2..=3).contains(&hops_count) {
            return Err(TxError::InvalidHopCount(hops_count));
        }

        let d_star = opportunity.size_in.as_u64();
        if d_star == 0 {
            return Err(TxError::ZeroBorrowAmount);
        }
        // P0 #4: ensure the route returns to the borrowed mint.
        if let (Some(first), Some(last)) = (opportunity.hops.first(), opportunity.hops.last()) {
            if first.mint_in != last.mint_out {
                return Err(TxError::NonClosingRoute {
                    in_mint: first.mint_in.0.to_string(),
                    out_mint: last.mint_out.0.to_string(),
                });
            }
        }

        let mut ixs = Vec::with_capacity(if hops_count == 2 { 7 } else { 8 });

        // [0] SetComputeUnitLimit: 260_000 (2-hop) or 320_000 (3-hop)
        let cu_limit = get_cu_limit_for_hops(hops_count);
        ixs.push(build_compute_unit_limit(cu_limit));

        // [1] SetComputeUnitPrice
        ixs.push(build_compute_unit_price(priority_fee_micro_lamports));

        // [2] Kamino FlashBorrow (placed at index 2)
        let borrow_idx = ixs.len();
        let accounts = kamino_params.to_flash_accounts(payer);
        ixs.push(kamino::build_kamino_flash_borrow(&accounts, d_star));

        // [3..] Swap Legs
        let slippage_bps: u64 = DEFAULT_HOP_SLIPPAGE_BPS;
        let fee = calculate_kamino_flash_fee(d_star, flash_loan_fee_sf);
        let tip_raw = tip_amount.as_u64();
        let legs = Self::derive_swap_legs(
            opportunity,
            fee,
            tip_raw,
            min_profit_raw.as_u64(),
            slippage_bps,
        )?;
        for (hop, (amount_in, min_out)) in opportunity.hops.iter().zip(legs.iter()) {
            ixs.push(build_swap_hop_instruction(
                hop, payer, *amount_in, *min_out,
            )?);
        }

        // [5] (or [6]) Kamino FlashRepay (borrow_instruction_index = 2).
        // The repay instruction's `amount` argument MUST equal the borrow
        // principal: klend's `flash_repay_checks` enforces
        // `liquidity_amount == borrow amount`, and the origination fee is
        // collected separately, on top, by the handler at repay time.
        let repay_principal = d_star;
        ixs.push(kamino::build_kamino_flash_repay(
            &accounts,
            repay_principal,
            borrow_idx as u8,
        ));

        // [6] (or [7]) Jito Tip Transfer
        if tip_amount.as_u64() > 0 {
            ixs.push(build_marked_jito_tip_instruction(
                payer,
                tip_account,
                tip_amount,
            ));
        }

        Ok(ixs)
    }

    /// Full end-to-end builder producing a signed Solana v0 Versioned Transaction.
    pub fn build_kamino_bundle(
        params: &KaminoBundleParams,
    ) -> Result<VersionedTransaction, TxError> {
        // min_profit_raw from opportunity (base-mint raw units)
        let min_profit_raw = TokenAmount(params.opportunity.net_profit.as_u64());
        let ixs = Self::assemble_atomic_bundle(
            params.payer,
            params.tip_account,
            params.tip_lamports,
            params.priority_fee_micro_lamports,
            params.opportunity,
            params.kamino_params,
            params.flash_loan_fee_sf, // fee_sf now sourced from KaminoBundleParams (ARB-005)
            min_profit_raw,
        )?;

        let message = Self::compile_v0_message(
            params.signer.pubkey(),
            &ixs,
            params.recent_blockhash,
            params.lookup_tables,
        )
        .map_err(|e| TxError::AltCompilationError(e.to_string()))?;

        let signature = params.signer.sign(&message);

        let mut wire_bytes = Vec::with_capacity(1 + 64 + message.len());
        Self::encode_compact_u16(1, &mut wire_bytes);
        wire_bytes.extend_from_slice(&signature);
        wire_bytes.extend_from_slice(&message);

        if wire_bytes.len() > SOLANA_PACKET_MTU_LIMIT {
            return Err(TxError::TransactionTooLarge {
                size: wire_bytes.len(),
                limit: SOLANA_PACKET_MTU_LIMIT,
            });
        }

        Ok(VersionedTransaction {
            signatures: vec![signature],
            message,
            wire_bytes,
        })
    }

    /// Assemble full native arbitrage execution instructions from an Opportunity.
    /// ComputeBudget instructions are strictly injected BEFORE swap instructions.
    pub fn assemble_arbitrage(
        payer: Pubkey,
        tip_account: Pubkey,
        tip_amount: Lamports,
        cu_limit: u32,
        priority_fee_micro_lamports: u64,
        opportunity: &ExecutionOpportunity,
        min_profit_raw: TokenAmount, // Profit floor in base-mint raw units
    ) -> Result<Vec<CompiledInstruction>> {
        let mut ixs = Vec::new();

        if let (Some(first), Some(last)) = (opportunity.hops.first(), opportunity.hops.last()) {
            if first.mint_in != last.mint_out {
                return Err(TxError::NonClosingRoute {
                    in_mint: first.mint_in.0.to_string(),
                    out_mint: last.mint_out.0.to_string(),
                }
                .into());
            }
            if first.mint_in.0 != Pubkey::from_str(WSOL_MINT).unwrap() {
                return Err(TxError::NonWsolFlashLoan(first.mint_in.0.to_string()).into());
            }
        }

        // 1. Compute Budget (Dynamically bounded <= 200,000 CU with 15% buffer, elevated priority fee)
        let effective_cu = calculate_dynamic_cu_limit(cu_limit);
        let dynamic_cu_price = calculate_dynamic_cu_price(
            opportunity.gross_profit.as_u64(),
            priority_fee_micro_lamports,
        );
        ixs.push(build_compute_unit_limit(effective_cu));
        ixs.push(build_compute_unit_price(dynamic_cu_price));

        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(USDC_MINT).unwrap();

        // Identify any intermediate tokens (from 40 trending tokens) needing ephemeral ATAs
        let mut intermediate_atas = Vec::new();
        for hop in &opportunity.hops {
            for mint in [hop.mint_in.0, hop.mint_out.0] {
                if mint != wsol
                    && mint != usdc
                    && !intermediate_atas.iter().any(|(m, _)| *m == mint)
                {
                    let ata = get_associated_token_address(&payer, &mint);
                    intermediate_atas.push((mint, ata));
                }
            }
        }

        // Ephemeral ATA Creation (Idempotent: safe if already exists, creates if missing)
        for (mint, ata) in &intermediate_atas {
            ixs.push(build_create_ata_idempotent(payer, payer, *mint, *ata));
        }

        // 2. Arbitrage Swap Hops (Injected after Compute Budget and ATA creation)
        let slippage_bps: u64 = DEFAULT_HOP_SLIPPAGE_BPS;
        let tip_raw = tip_amount.as_u64();
        let legs = Self::derive_swap_legs(
            opportunity,
            0,
            tip_raw,
            min_profit_raw.as_u64(),
            slippage_bps,
        )?;

        let user_wsol_ata = get_associated_token_address(&payer, &wsol);
        let first_is_pump_buy = opportunity
            .hops
            .first()
            .map(|h| h.protocol == DexProtocol::PumpFun)
            .unwrap_or(false);
        let last_is_pump_sell = opportunity
            .hops
            .last()
            .map(|h| h.protocol == DexProtocol::PumpFun)
            .unwrap_or(false);

        // If Hop 1 is Raydium/Meteora (needs WSOL) but last is Pump.fun (yields native SOL):
        // Wrap amount_in of native SOL to WSOL ATA before Hop 1
        if !first_is_pump_buy && last_is_pump_sell {
            ixs.push(build_create_ata_idempotent(
                payer,
                payer,
                wsol,
                user_wsol_ata,
            ));
            if let Some(first_leg) = legs.first() {
                ixs.push(build_system_transfer(payer, user_wsol_ata, first_leg.0));
                ixs.push(build_sync_native(user_wsol_ata));
            }
        }

        for (hop, (amount_in, min_out)) in opportunity.hops.iter().zip(legs.iter()) {
            ixs.push(build_swap_hop_instruction(
                hop, payer, *amount_in, *min_out,
            )?);
        }

        // If last hop outputted WSOL (e.g. Pump.fun Buy -> Raydium Sell):
        // Unwrap WSOL back to native SOL for the payer
        if first_is_pump_buy && !last_is_pump_sell {
            ixs.push(build_close_token_account(user_wsol_ata, payer, payer));
        }

        // 3. Close Ephemeral ATAs to reclaim 100% of the rent deposit (0.00203928 SOL) directly to payer
        for (_, ata) in &intermediate_atas {
            ixs.push(build_close_token_account(*ata, payer, payer));
        }

        // 4. Jito Tip at the end of the atomic transaction
        if tip_amount.as_u64() > 0 {
            ixs.push(build_marked_jito_tip_instruction(
                payer,
                tip_account,
                tip_amount,
            ));
        }

        Ok(ixs)
    }

    /// Derive `(amount_in, min_amount_out)` per hop.
    ///
    /// B1/B2 fixes:
    /// - `amount_in` for hop `i` is `hop_inputs[i]` — the previous hop's *expected*
    ///   output, never the discounted `min_out` and never the input amount.
    /// - `min_amount_out` discounts `expected_out` by slippage tolerance only,
    ///   applied once per hop (never deducted twice).
    /// - The final hop's `min_out` is raised to the profit floor
    ///   (`borrow + flash_fee + min_profit`).
    ///
    /// `hop_inputs` must be `[borrow, out_1, ..., out_n]` with length
    /// `hops.len() + 1`. There is no legacy fallback: intermediate inputs must
    /// come from the previous hop's actual expected output.
    pub fn derive_swap_legs(
        opportunity: &ExecutionOpportunity,
        flash_fee_raw: u64,
        tip_raw: u64,
        min_profit_raw: u64,
        slippage_bps: u64,
    ) -> Result<Vec<(u64, u64)>, TxError> {
        if opportunity.hops.is_empty() {
            return Ok(Vec::new());
        }

        let inputs = opportunity.hop_inputs.as_slice();
        if inputs.len() != opportunity.hops.len() + 1 {
            return Err(TxError::MissingHopInputs);
        }

        let borrow = inputs[0];
        // Enforce full on-chain solvent floor: borrow + flash_fee + jito_tip + net_retained_profit
        let floor = borrow
            .checked_add(flash_fee_raw)
            .and_then(|v| v.checked_add(tip_raw))
            .and_then(|v| v.checked_add(min_profit_raw))
            .ok_or_else(|| TxError::InvalidInput("profit floor arithmetic overflow".into()))?;

        let mut legs = Vec::with_capacity(opportunity.hops.len());
        for (i, _hop) in opportunity.hops.iter().enumerate() {
            let amount_in = inputs[i];
            let expected_out = inputs[i + 1];
            if amount_in > 0 && expected_out == 0 {
                return Err(TxError::InvalidInput(format!("hop {i} quotes zero output")));
            }

            // Slippage applied once per hop, never deducted twice.
            let min_out_slippage = expected_out
                .checked_sub((expected_out * slippage_bps) / 10_000)
                .ok_or_else(|| {
                    TxError::InvalidInput(format!("hop {i} slippage arithmetic overflow"))
                })?;

            let is_last = i + 1 == opportunity.hops.len();
            let min_out = if is_last {
                if expected_out < floor {
                    return Err(TxError::ProfitFloorNotMet {
                        expected: expected_out,
                        floor,
                    });
                }
                min_out_slippage.max(floor)
            } else {
                min_out_slippage
            };

            legs.push((amount_in, min_out));
        }
        Ok(legs)
    }

    /// Assemble atomic Kamino Flash Loan arbitrage transaction.
    /// Order: ComputeBudget -> Ephemeral ATAs -> FlashBorrow -> Swaps -> FlashRepay -> Close ATAs (Rent Reclaim) -> Jito Tip.
    pub fn assemble_flash_arbitrage(
        payer: Pubkey,
        tip_account: Pubkey,
        tip_amount: Lamports,
        cu_limit: u32,
        priority_fee_micro_lamports: u64,
        opportunity: &ExecutionOpportunity,
        kamino_params: &KaminoFlashParams,
        flash_loan_fee_sf: u64,
        min_profit_raw: TokenAmount, // Profit floor in base-mint raw units
        skip_close_intermediate_mints: &[Pubkey],
    ) -> Result<Vec<CompiledInstruction>> {
        let mut ixs = Vec::new();

        // 1. Compute Budget Limit & Dynamic Priority Fee (Injected BEFORE Swaps & Borrow)
        let effective_cu = calculate_dynamic_cu_limit(cu_limit);
        let dynamic_cu_price = calculate_dynamic_cu_price(
            opportunity.gross_profit.as_u64(),
            priority_fee_micro_lamports,
        );
        ixs.push(build_compute_unit_limit(effective_cu));
        ixs.push(build_compute_unit_price(dynamic_cu_price));

        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(USDC_MINT).unwrap();
        let user_wsol_ata = get_associated_token_address(&payer, &wsol);
        let mut resolved_params = kamino_params.clone();
        if resolved_params.user_destination_liquidity == Pubkey::default()
            || resolved_params.user_destination_liquidity == wsol
        {
            resolved_params.user_destination_liquidity = user_wsol_ata;
        }

        // 2. Ensure user's WSOL ATA exists idempotently for Flash Loan receipt
        ixs.push(build_create_ata_idempotent(
            payer,
            payer,
            wsol,
            user_wsol_ata,
        ));

        // Identify any intermediate tokens needing ephemeral ATAs. Each mint's
        // ATA MUST be derived and created under its own token program: hops
        // carry `token_programs` for `(mint_in, mint_out)`, and deriving a
        // Token-2022 ATA with the legacy SPL program produces a wrong PDA.
        let mut intermediate_atas: Vec<(Pubkey, Pubkey, Pubkey)> = Vec::new();
        for hop in &opportunity.hops {
            for (mint, program) in [
                (hop.mint_in.0, hop.token_programs[0].program_id()),
                (hop.mint_out.0, hop.token_programs[1].program_id()),
            ] {
                if mint != wsol
                    && mint != usdc
                    && !intermediate_atas.iter().any(|(m, _, _)| *m == mint)
                {
                    let ata = get_associated_token_address_with_program_id(&payer, &mint, &program);
                    intermediate_atas.push((mint, ata, program));
                }
            }
        }

        // Ephemeral ATA Creation (Idempotent: zero failure if already exists)
        for (mint, ata, program) in &intermediate_atas {
            ixs.push(build_create_ata_idempotent_for_program(
                payer, payer, *mint, *ata, *program,
            ));
        }

        // 3. Kamino Flash Borrow
        let borrow_idx = ixs.len();
        let borrow_amount = opportunity.size_in.as_u64();
        if let (Some(first), Some(last)) = (opportunity.hops.first(), opportunity.hops.last()) {
            if first.mint_in != last.mint_out {
                return Err(TxError::NonClosingRoute {
                    in_mint: first.mint_in.0.to_string(),
                    out_mint: last.mint_out.0.to_string(),
                }
                .into());
            }
            if first.mint_in.0 != Pubkey::from_str(WSOL_MINT).unwrap() {
                return Err(TxError::NonWsolFlashLoan(first.mint_in.0.to_string()).into());
            }
        }
        ixs.push(build_kamino_flash_borrow(
            &resolved_params,
            payer,
            borrow_amount,
        ));

        // 4. Sequential Genuine Swap Hops (Injected before Repay)
        let slippage_bps: u64 = DEFAULT_HOP_SLIPPAGE_BPS;
        let fee = calculate_kamino_flash_fee(borrow_amount, flash_loan_fee_sf);
        let tip_raw = tip_amount.as_u64();
        let legs = Self::derive_swap_legs(
            opportunity,
            fee,
            tip_raw,
            min_profit_raw.as_u64(),
            slippage_bps,
        )?;

        let first_is_pump_buy = opportunity
            .hops
            .first()
            .map(|h| h.protocol == DexProtocol::PumpFun)
            .unwrap_or(false);
        let last_is_pump_sell = opportunity
            .hops
            .last()
            .map(|h| h.protocol == DexProtocol::PumpFun)
            .unwrap_or(false);

        // If Hop 1 is Pump.fun Buy: unwrap borrowed WSOL into native SOL so Pump.fun can spend it
        if first_is_pump_buy {
            ixs.push(build_close_token_account(user_wsol_ata, payer, payer));
        }

        for (i, (hop, (amount_in, min_out))) in opportunity.hops.iter().zip(legs.iter()).enumerate()
        {
            // If Hop 1 unwrapped user_wsol_ata and Hop 2 outputs WSOL (Raydium/Meteora):
            // Re-create user_wsol_ata so Hop 2 has a valid destination
            if first_is_pump_buy && i == 1 {
                ixs.push(build_create_ata_idempotent(
                    payer,
                    payer,
                    wsol,
                    user_wsol_ata,
                ));
            }
            ixs.push(build_swap_hop_instruction(
                hop, payer, *amount_in, *min_out,
            )?);
        }

        // The repay instruction's `amount` argument MUST be the borrow principal
        // (klend's `flash_repay_checks` enforces `liquidity_amount == borrow
        // amount`). The origination fee is debited separately, on top, from the
        // same source ATA at repay time, so the ATA must hold `principal + fee`.
        let repay_principal = borrow_amount;
        let repay_funding = borrow_amount.saturating_add(fee);

        // If last hop was Pump.fun Sell: user received native SOL, wrap it back into WSOL for Kamino repay
        if last_is_pump_sell {
            ixs.push(build_create_ata_idempotent(
                payer,
                payer,
                wsol,
                user_wsol_ata,
            ));
            ixs.push(build_system_transfer(payer, user_wsol_ata, repay_funding));
            ixs.push(build_sync_native(user_wsol_ata));
        }

        // 5. Kamino Flash Repay (amount == principal; fee collected on top)
        ixs.push(build_kamino_flash_repay_with_index(
            &resolved_params,
            payer,
            repay_principal,
            borrow_idx as u8,
        ));

        // 5. Ephemeral ATA Close & 100% Rent Reclaim directly to payer.
        // Mints listed in `skip_close_intermediate_mints` had a pre-existing ATA
        // before this transaction: closing it would fail (or destroy external
        // balances), so the caller must exclude them from the close list.
        for (mint, ata, program) in &intermediate_atas {
            if skip_close_intermediate_mints.contains(mint) {
                continue;
            }
            ixs.push(build_close_token_account_for_program(
                *ata, payer, payer, *program,
            ));
        }

        // 6. Jito Tip at the end
        if tip_amount.as_u64() > 0 {
            ixs.push(build_marked_jito_tip_instruction(
                payer,
                tip_account,
                tip_amount,
            ));
        }

        Ok(ixs)
    }

    /// Assemble Kamino flash loan with split DEX swap legs.
    /// Order: ComputeBudget -> FlashBorrow -> Split Swaps -> Return Swap -> FlashRepay -> Jito Tip
    #[allow(clippy::too_many_arguments)]
    pub fn assemble_split_flash_arbitrage(
        payer: Pubkey,
        tip_account: Pubkey,
        tip_amount: Lamports,
        cu_limit: u32,
        priority_fee_micro_lamports: u64,
        total_loan: u64,
        legs: &[CompiledInstruction],
        kamino_params: &KaminoFlashParams,
        _flash_loan_fee_sf: u64,
    ) -> Result<Vec<CompiledInstruction>> {
        let mut ixs = Vec::new();
        let effective_cu = calculate_dynamic_cu_limit(cu_limit);
        ixs.push(build_compute_unit_limit(effective_cu));
        ixs.push(build_compute_unit_price(priority_fee_micro_lamports));

        // 1. Kamino Flash Borrow
        let borrow_idx = ixs.len();
        ixs.push(build_kamino_flash_borrow(kamino_params, payer, total_loan));

        // 2. Parallel / Split DEX Legs
        for leg in legs {
            ixs.push(leg.clone());
        }

        // 3. Kamino Flash Repay — `amount` is the principal; the origination fee
        // is debited separately, on top, from the same source ATA by the handler.
        ixs.push(build_kamino_flash_repay_with_index(
            kamino_params,
            payer,
            total_loan,
            borrow_idx as u8,
        ));

        // 4. Standalone Jito Tip
        if tip_amount.as_u64() > 0 {
            ixs.push(build_marked_jito_tip_instruction(
                payer,
                tip_account,
                tip_amount,
            ));
        }

        Ok(ixs)
    }

    /// Compact-u16 encoder according to Solana binary serialization spec.
    pub fn encode_compact_u16(mut val: u16, out: &mut Vec<u8>) {
        loop {
            let mut elem = (val & 0x7f) as u8;
            val >>= 7;
            if val == 0 {
                out.push(elem);
                break;
            } else {
                elem |= 0x80;
                out.push(elem);
            }
        }
    }

    /// Compile a list of instructions into a standard Solana legacy Message byte buffer.
    pub fn compile_message(
        payer: Pubkey,
        instructions: &[CompiledInstruction],
        recent_blockhash: [u8; 32],
    ) -> Result<Vec<u8>, TxError> {
        use std::collections::BTreeMap;

        let mut account_map: BTreeMap<Pubkey, (bool, bool)> = BTreeMap::new();
        // Payer is always account 0: signer and writable
        account_map.insert(payer, (true, true));

        for ix in instructions {
            for acc in &ix.accounts {
                let entry = account_map.entry(acc.pubkey).or_insert((false, false));
                entry.0 |= acc.is_signer;
                entry.1 |= acc.is_writable;
            }
            // Program IDs are non-signer, readonly
            account_map.entry(ix.program_id).or_insert((false, false));
        }

        let mut writable_signers = Vec::new();
        let mut readonly_signers = Vec::new();
        let mut writable_non_signers = Vec::new();
        let mut readonly_non_signers = Vec::new();

        writable_signers.push(payer);

        for (pubkey, (is_signer, is_writable)) in account_map {
            if pubkey == payer {
                continue;
            }
            if is_signer {
                if is_writable {
                    writable_signers.push(pubkey);
                } else {
                    readonly_signers.push(pubkey);
                }
            } else if is_writable {
                writable_non_signers.push(pubkey);
            } else {
                readonly_non_signers.push(pubkey);
            }
        }

        let num_required_signatures = (writable_signers.len() + readonly_signers.len()) as u8;
        let num_readonly_signed_accounts = readonly_signers.len() as u8;
        let num_readonly_unsigned_accounts = readonly_non_signers.len() as u8;

        let mut all_accounts = Vec::new();
        all_accounts.extend(writable_signers);
        all_accounts.extend(readonly_signers);
        all_accounts.extend(writable_non_signers);
        all_accounts.extend(readonly_non_signers);

        let key_to_index: std::collections::HashMap<Pubkey, u8> = all_accounts
            .iter()
            .enumerate()
            .map(|(idx, &pk)| (pk, idx as u8))
            .collect();

        let mut message = Vec::new();
        // Message Header
        message.push(num_required_signatures);
        message.push(num_readonly_signed_accounts);
        message.push(num_readonly_unsigned_accounts);

        // Account keys array
        Self::encode_compact_u16(all_accounts.len() as u16, &mut message);
        for pk in &all_accounts {
            message.extend_from_slice(pk.as_ref());
        }

        // Recent blockhash
        message.extend_from_slice(&recent_blockhash);

        // Compiled instructions
        Self::encode_compact_u16(instructions.len() as u16, &mut message);
        for ix in instructions {
            let prog_idx = *key_to_index.get(&ix.program_id).ok_or_else(|| {
                TxError::InstructionError(format!(
                    "program id not in account map: {}",
                    ix.program_id
                ))
            })?;
            message.push(prog_idx);

            Self::encode_compact_u16(ix.accounts.len() as u16, &mut message);
            for acc in &ix.accounts {
                let acc_idx = key_to_index.get(&acc.pubkey).ok_or_else(|| {
                    TxError::InstructionError(format!("account not in account map: {}", acc.pubkey))
                })?;
                message.push(*acc_idx);
            }

            Self::encode_compact_u16(ix.data.len() as u16, &mut message);
            message.extend_from_slice(&ix.data);
        }

        Ok(message)
    }

    /// Build a standalone Jito tip transaction for 2-transaction MEV bundles.
    pub fn build_jito_tip_transaction(
        signer: &KeypairSigner,
        tip_account: Pubkey,
        tip_amount: Lamports,
        recent_blockhash: [u8; 32],
    ) -> Result<Vec<u8>, TxError> {
        let ix = build_jito_tip_instruction(signer.pubkey(), tip_account, tip_amount);
        Self::build_signed_transaction(signer, &[ix], recent_blockhash)
    }

    /// Build a standalone Jito tip transaction marked with `jitodontfront` sandwich protection.
    pub fn build_marked_jito_tip_transaction(
        signer: &KeypairSigner,
        tip_account: Pubkey,
        tip_amount: Lamports,
        recent_blockhash: [u8; 32],
    ) -> Result<Vec<u8>, TxError> {
        let ix = build_marked_jito_tip_instruction(signer.pubkey(), tip_account, tip_amount);
        Self::build_signed_transaction(signer, &[ix], recent_blockhash)
    }

    /// Construct and sign a fully-formed Solana Wire Format transaction ready for RPC / Jito submission.
    pub fn build_signed_transaction(
        signer: &KeypairSigner,
        instructions: &[CompiledInstruction],
        recent_blockhash: [u8; 32],
    ) -> Result<Vec<u8>, TxError> {
        let message = Self::compile_message(signer.pubkey(), instructions, recent_blockhash)?;
        let signature = signer.sign(&message);

        let mut tx = Vec::with_capacity(1 + 64 + message.len());
        Self::encode_compact_u16(1, &mut tx); // 1 signature
        tx.extend_from_slice(&signature);
        tx.extend_from_slice(&message);
        // NOTE: the 1,232-byte packet ceiling is enforced per *transport*, not
        // here. Legacy/V0 stay at 1,232 bytes, while Solana Transaction V1 is
        // 4,096 bytes on the direct SWQoS/RPC path (SIMD-0296/0385). Callers
        // must validate with `validate_jito_bundle` (Jito ingress) or the
        // supervisor's per-format size gate before submitting.
        Ok(tx)
    }

    /// Compile a list of instructions and Address Lookup Tables into a Solana v0 Message byte buffer.
    pub fn compile_v0_message(
        payer: Pubkey,
        instructions: &[CompiledInstruction],
        recent_blockhash: [u8; 32],
        lookup_tables: &[AddressLookupTable],
    ) -> Result<Vec<u8>> {
        use std::collections::{BTreeMap, HashMap};

        // Track signer/writable requirements for all referenced accounts
        let mut account_map: BTreeMap<Pubkey, (bool, bool)> = BTreeMap::new();
        // Payer is always writable signer
        account_map.insert(payer, (true, true));

        for ix in instructions {
            for acc in &ix.accounts {
                let entry = account_map.entry(acc.pubkey).or_insert((false, false));
                entry.0 |= acc.is_signer;
                entry.1 |= acc.is_writable;
            }
            account_map.entry(ix.program_id).or_insert((false, false));
        }

        struct TableLookupEntry {
            table_key: Pubkey,
            writable_indices: Vec<u8>,
            readonly_indices: Vec<u8>,
        }

        let mut table_entries: Vec<TableLookupEntry> = lookup_tables
            .iter()
            .map(|t| TableLookupEntry {
                table_key: t.key,
                writable_indices: Vec::new(),
                readonly_indices: Vec::new(),
            })
            .collect();

        let mut loaded_writable_keys = Vec::new();
        let mut loaded_readonly_keys = Vec::new();

        let mut static_writable_signers = Vec::new();
        let mut static_readonly_signers = Vec::new();
        let mut static_writable_non_signers = Vec::new();
        let mut static_readonly_non_signers = Vec::new();

        static_writable_signers.push(payer);

        for (pubkey, (is_signer, is_writable)) in account_map {
            if pubkey == payer {
                continue;
            }

            if is_signer {
                if is_writable {
                    static_writable_signers.push(pubkey);
                } else {
                    static_readonly_signers.push(pubkey);
                }
                continue;
            }

            // Check if pubkey exists in any lookup table
            let mut found_in_table = false;
            for (t_idx, table) in lookup_tables.iter().enumerate() {
                if let Some(pos) = table.addresses.iter().position(|&a| a == pubkey) {
                    if pos > 255 {
                        return Err(TxError::InvalidInput(format!(
                            "ALT index overflow: {} > 255 for {}",
                            pos, pubkey
                        ))
                        .into());
                    }
                    found_in_table = true;
                    if is_writable {
                        table_entries[t_idx].writable_indices.push(pos as u8);
                        loaded_writable_keys.push(pubkey);
                    } else {
                        table_entries[t_idx].readonly_indices.push(pos as u8);
                        loaded_readonly_keys.push(pubkey);
                    }
                    break;
                }
            }

            if !found_in_table {
                if is_writable {
                    static_writable_non_signers.push(pubkey);
                } else {
                    static_readonly_non_signers.push(pubkey);
                }
            }
        }

        let num_required_signatures =
            (static_writable_signers.len() + static_readonly_signers.len()) as u8;
        let num_readonly_signed_accounts = static_readonly_signers.len() as u8;
        let num_readonly_unsigned_accounts = static_readonly_non_signers.len() as u8;

        let mut static_account_keys = Vec::new();
        static_account_keys.extend(static_writable_signers);
        static_account_keys.extend(static_readonly_signers);
        static_account_keys.extend(static_writable_non_signers);
        static_account_keys.extend(static_readonly_non_signers);

        let mut key_to_index: HashMap<Pubkey, u8> = HashMap::new();
        for (i, pk) in static_account_keys.iter().enumerate() {
            key_to_index.insert(*pk, i as u8);
        }
        let static_len = static_account_keys.len();
        for (i, pk) in loaded_writable_keys.iter().enumerate() {
            key_to_index.insert(*pk, (static_len + i) as u8);
        }
        let writable_offset = static_len + loaded_writable_keys.len();
        for (i, pk) in loaded_readonly_keys.iter().enumerate() {
            key_to_index.insert(*pk, (writable_offset + i) as u8);
        }

        let mut message = vec![
            0x80, // Versioned Message prefix: 0x80 (version 0)
            num_required_signatures,
            num_readonly_signed_accounts,
            num_readonly_unsigned_accounts,
        ];

        // Static account keys
        Self::encode_compact_u16(static_account_keys.len() as u16, &mut message);
        for pk in &static_account_keys {
            message.extend_from_slice(pk.as_ref());
        }

        // Recent blockhash
        message.extend_from_slice(&recent_blockhash);

        // Instructions
        Self::encode_compact_u16(instructions.len() as u16, &mut message);
        for ix in instructions {
            let prog_idx = *key_to_index
                .get(&ix.program_id)
                .ok_or_else(|| anyhow::anyhow!("Program ID {} not in key map", ix.program_id))?;
            message.push(prog_idx);

            Self::encode_compact_u16(ix.accounts.len() as u16, &mut message);
            for acc in &ix.accounts {
                let acc_idx = *key_to_index
                    .get(&acc.pubkey)
                    .ok_or_else(|| anyhow::anyhow!("Account {} not in key map", acc.pubkey))?;
                message.push(acc_idx);
            }

            Self::encode_compact_u16(ix.data.len() as u16, &mut message);
            message.extend_from_slice(&ix.data);
        }

        // Address table lookups
        let active_table_entries: Vec<&TableLookupEntry> = table_entries
            .iter()
            .filter(|t| !t.writable_indices.is_empty() || !t.readonly_indices.is_empty())
            .collect();

        Self::encode_compact_u16(active_table_entries.len() as u16, &mut message);
        for t in active_table_entries {
            message.extend_from_slice(t.table_key.as_ref());
            Self::encode_compact_u16(t.writable_indices.len() as u16, &mut message);
            message.extend_from_slice(&t.writable_indices);
            Self::encode_compact_u16(t.readonly_indices.len() as u16, &mut message);
            message.extend_from_slice(&t.readonly_indices);
        }

        Ok(message)
    }

    /// Construct and sign a Solana v0 Versioned Transaction with Address Lookup Tables.
    pub fn build_signed_v0_transaction(
        signer: &KeypairSigner,
        instructions: &[CompiledInstruction],
        recent_blockhash: [u8; 32],
        lookup_tables: &[AddressLookupTable],
    ) -> Result<Vec<u8>> {
        let message = Self::compile_v0_message(
            signer.pubkey(),
            instructions,
            recent_blockhash,
            lookup_tables,
        )?;
        let signature = signer.sign(&message);

        let mut tx = Vec::with_capacity(1 + 64 + message.len());
        Self::encode_compact_u16(1, &mut tx); // 1 signature
        tx.extend_from_slice(&signature);
        tx.extend_from_slice(&message);
        Ok(tx)
    }

    /// Create a standard Address Lookup Table containing well-known static program, pool,
    /// vault, and sysvar pubkeys for Raydium, Orca, Kamino, and SPL Token programs on Solana Mainnet.
    pub fn create_default_mainnet_alt() -> AddressLookupTable {
        let alt_key = Pubkey::from_str("ALT1111111111111111111111111111111111111111").unwrap();
        let addresses = vec![
            Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
            Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap(),
            Pubkey::new_from_array(SYSTEM_PROGRAM_ID),
            Pubkey::from_str(SYSVAR_INSTRUCTIONS_ID).unwrap(),
            Pubkey::new_from_array(COMPUTE_BUDGET_PROGRAM_ID),
            Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap(),
            Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap(),
            Pubkey::from_str(KAMINO_WSOL_RESERVE).unwrap(),
            Pubkey::from_str(KAMINO_WSOL_SUPPLY_VAULT).unwrap(),
            Pubkey::from_str(KAMINO_WSOL_FEE_VAULT).unwrap(),
            Pubkey::from_str(RAYDIUM_V4_PROGRAM_ID).unwrap(),
            Pubkey::from_str(RAYDIUM_SOL_USDC_POOL).unwrap(),
            Pubkey::from_str(RAYDIUM_SOL_VAULT).unwrap(),
            Pubkey::from_str(RAYDIUM_USDC_VAULT).unwrap(),
            Pubkey::from_str("5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1").unwrap(), // Raydium Authority
            Pubkey::from_str("srmqPvymJeFKQ4zGQed1GFppgkRHL9kaELCbyksJtPX").unwrap(), // OpenBook / Serum Program
            Pubkey::from_str("8BnEgHoWFysVcuFFX7QBpDmvu3xei5qGE9KitUfqWQH9").unwrap(), // SOL-USDC Serum Market
            Pubkey::from_str(ORCA_WHIRLPOOL_PROGRAM_ID).unwrap(),
            Pubkey::from_str(ORCA_SOL_USDC_POOL).unwrap(),
            Pubkey::from_str(ORCA_SOL_VAULT).unwrap(),
            Pubkey::from_str(ORCA_USDC_VAULT).unwrap(),
            Pubkey::from_str(WSOL_MINT).unwrap(),
            Pubkey::from_str(USDC_MINT).unwrap(),
            Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap(),
        ];
        AddressLookupTable::new(alt_key, addresses)
    }

    /// Sign a pre-compiled versioned (v0) transaction (e.g. from Jupiter v6 API).
    pub fn sign_versioned_transaction(
        signer: &KeypairSigner,
        mut tx_bytes: Vec<u8>,
    ) -> Result<Vec<u8>> {
        if tx_bytes.len() < 66 {
            bail!("Transaction bytes too short for versioned transaction");
        }
        let msg_bytes = &tx_bytes[65..];
        let signature = signer.sign(msg_bytes);
        tx_bytes[1..65].copy_from_slice(&signature);
        Ok(tx_bytes)
    }

    /// Encode instructions into a serialized byte representation for simulation or broadcast.
    pub fn serialize_instructions(instructions: &[CompiledInstruction]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(instructions.len() as u16).to_le_bytes());
        for ix in instructions {
            bytes.extend_from_slice(ix.program_id.as_ref());
            bytes.extend_from_slice(&(ix.accounts.len() as u16).to_le_bytes());
            for acc in &ix.accounts {
                bytes.extend_from_slice(acc.pubkey.as_ref());
                bytes.push(acc.is_signer as u8);
                bytes.push(acc.is_writable as u8);
            }
            bytes.extend_from_slice(&(ix.data.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&ix.data);
        }
        bytes
    }
}
