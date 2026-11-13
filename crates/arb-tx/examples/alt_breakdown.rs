//! Throwaway diagnostic: decompose the v0+ALT message size for the 3-pool
//! DLMM route across bin expansions, to explain the measured +99 B/bin growth
//! in the ALT compression matrix test.
//!
//! Scenario A replicates the matrix test exactly: the ALT is built from a
//! FIRST construction of the route, then `params()` is called a SECOND time
//! (fresh random reserves/oracles/bin-array PDAs) for the V0+ALT build. The
//! random keys miss the ALT and fall back to static 32-byte message keys.
//!
//! Scenario B builds the ALT from the SAME construction used for the V0+ALT
//! build, so every non-signer account compresses to a 1-byte ALT index.

use arb_tx::dlmm_3leg_flash_arb::{
    FlashArb3LegParams, FlashArb3LegPoolLeg, build_flash_funded_three_pool_transaction,
};
use arb_tx::{AddressLookupTable, CompiledInstruction, KeypairSigner};
use solana_pubkey::Pubkey;
use std::str::FromStr;

const FLASH_LOAN_FEE_SF: u64 = 11_529_215_046_068;

fn wsol() -> Pubkey {
    Pubkey::from_str(arb_constants::WSOL_MINT).unwrap()
}
fn spl() -> Pubkey {
    Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap()
}

fn leg(
    lb_pair: Pubkey,
    tin: Pubkey,
    tout: Pubkey,
    ain: u64,
    aout: u64,
    bins: Vec<Pubkey>,
) -> FlashArb3LegPoolLeg {
    FlashArb3LegPoolLeg {
        lb_pair,
        reserve_x: Pubkey::new_unique(),
        reserve_y: Pubkey::new_unique(),
        token_x_mint: tin,
        token_y_mint: tout,
        oracle: Pubkey::new_unique(),
        token_x_program: spl(),
        token_y_program: spl(),
        swap_for_y: true,
        amount_in: ain,
        min_amount_out: aout,
        bin_array_pdas: bins,
        bitmap_extension: None,
        host_fee_in: None,
    }
}

fn params(payer: Pubkey, bins_per_leg: usize, v1: bool) -> FlashArb3LegParams {
    let borrow = 1_000_000_000u64;
    let fee = arb_tx::kamino::calculate_kamino_flash_fee(borrow, FLASH_LOAN_FEE_SF);
    let bins = || {
        (0..bins_per_leg)
            .map(|_| Pubkey::new_unique())
            .collect::<Vec<_>>()
    };
    FlashArb3LegParams {
        payer,
        kamino: arb_tx::KaminoFlashParams::default_wsol(),
        borrow_amount: borrow,
        flash_fee: fee,
        leg_1: leg(
            Pubkey::new_from_array([10u8; 32]),
            wsol(),
            Pubkey::new_from_array([0xA0u8; 32]),
            borrow,
            1_100_000_000,
            bins(),
        ),
        leg_2: leg(
            Pubkey::new_from_array([20u8; 32]),
            Pubkey::new_from_array([0xA0u8; 32]),
            Pubkey::new_from_array([0xB0u8; 32]),
            1_100_000_000,
            1_050_000_000,
            bins(),
        ),
        leg_3: leg(
            Pubkey::new_from_array([30u8; 32]),
            Pubkey::new_from_array([0xB0u8; 32]),
            wsol(),
            1_050_000_000,
            fee + borrow + 500_000,
            bins(),
        ),
        cu_limit: 400_000,
        priority_fee_micro_lamports: 50_000,
        jito_tip_account: Pubkey::from_str(arb_constants::JITO_TIP_ACCOUNTS[0]).unwrap(),
        jito_tip_lamports: 1_000,
        recent_blockhash: [0x42u8; 32],
        v1_transport_supported: v1,
    }
}

fn route_alt(signer: &KeypairSigner, ixs: &[CompiledInstruction]) -> AddressLookupTable {
    let mut addresses: Vec<Pubkey> = Vec::new();
    for ix in ixs {
        if ix.program_id != signer.pubkey() && !addresses.contains(&ix.program_id) {
            addresses.push(ix.program_id);
        }
        for acc in &ix.accounts {
            if acc.pubkey != signer.pubkey() && !addresses.contains(&acc.pubkey) {
                addresses.push(acc.pubkey);
            }
        }
    }
    AddressLookupTable::new(Pubkey::new_unique(), addresses)
}

fn static_keys(ixs: &[CompiledInstruction], payer: Pubkey, alt: &AddressLookupTable) -> usize {
    let mut seen: Vec<Pubkey> = vec![payer];
    for ix in ixs {
        if !seen.contains(&ix.program_id) {
            seen.push(ix.program_id);
        }
        for acc in &ix.accounts {
            if !seen.contains(&acc.pubkey) {
                seen.push(acc.pubkey);
            }
        }
    }
    seen.iter().filter(|pk| !alt.addresses.contains(pk)).count()
}

fn unique_count(signer: &Pubkey, ixs: &[CompiledInstruction]) -> usize {
    let mut seen: Vec<Pubkey> = vec![*signer];
    for ix in ixs {
        if !seen.contains(&ix.program_id) {
            seen.push(ix.program_id);
        }
        for acc in &ix.accounts {
            if !seen.contains(&acc.pubkey) {
                seen.push(acc.pubkey);
            }
        }
    }
    seen.len()
}

fn main() {
    let signer = KeypairSigner::new_random();
    for bins_per_leg in [4usize, 5, 6, 8, 10] {
        println!("bins/leg={bins_per_leg}");

        let p1 = params(signer.pubkey(), bins_per_leg, true);
        let v1 = build_flash_funded_three_pool_transaction(&p1, &signer, &[]).unwrap();
        println!(
            "  V1 wire : {} ({} unique accounts)",
            v1.tx_size,
            unique_count(&signer.pubkey(), &v1.instructions)
        );

        let alt_a = route_alt(&signer, &v1.instructions);
        let p2 = params(signer.pubkey(), bins_per_leg, false);
        match build_flash_funded_three_pool_transaction(&p2, &signer, std::slice::from_ref(&alt_a))
        {
            Ok(tx) => println!(
                "  A (test replica, ALT from other construction): wire={} static_keys={}",
                tx.tx_size,
                static_keys(&tx.instructions, signer.pubkey(), &alt_a)
            ),
            Err(e) => println!("  A (test replica, ALT from other construction): FAIL-CLOSED {e}"),
        }

        let mut p3 = params(signer.pubkey(), bins_per_leg, true);
        let v1_b = build_flash_funded_three_pool_transaction(&p3, &signer, &[]).unwrap();
        let alt_b = route_alt(&signer, &v1_b.instructions);
        p3.v1_transport_supported = false;
        match build_flash_funded_three_pool_transaction(&p3, &signer, std::slice::from_ref(&alt_b))
        {
            Ok(tx) => println!(
                "  B (ALT matches construction)                : wire={} static_keys={}",
                tx.tx_size,
                static_keys(&tx.instructions, signer.pubkey(), &alt_b)
            ),
            Err(e) => println!("  B (ALT matches construction)                : FAIL-CLOSED {e}"),
        }
    }
}
