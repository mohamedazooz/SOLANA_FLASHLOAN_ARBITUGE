//! `execute_three_leg_with_tip` instruction implementation.
//!
//! Identical three-leg DLMM route execution and validation to
//! `execute_three_leg`, followed by a conditional on-chain Jito tip:
//!
//! 1. The settlement (WSOL) account must be a `user`-owned WSOL mint account;
//!    WSOL's 1:1 lamport peg is the dimensional premise that allows a
//!    WSOL-measured profit edge to back a lamport-denominated tip.
//! 2. The tip account (the final account in the instruction) must be one of
//!    the 8 canonical Jito block-engine tip accounts.
//! 3. After the canonical three-leg body succeeds, the realized gross edge is
//!    `gross_edge = (wsol_final - wsol_baseline) - leg1_amount_in` and the tip
//!    is `floor(gross_edge * tip_bps / 10_000)`, capped so the retained edge
//!    still covers the flash-loan cost plus the operator-stipulated minimum
//!    profit: `gross_edge - tip >= min_final_wsol_delta - leg1_amount_in`.
//!    Any failure here aborts the whole atomic transaction before the tip
//!    leaves the user.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_account_info::AccountInfo;
use solana_program_error::ProgramResult;

use crate::cpi::invoke_jito_tip;
use crate::validation::{
    BPS_DENOMINATOR, WSOL_MINT, read_token_account_balance, tip_amount_for_bps,
    validate_jito_tip_account, validate_user_token_account,
};

use crate::error::ExecutorError;
use crate::instructions::execute_three_leg::{ExecuteThreeLegArgs, process_execute_three_leg};

/// Parameters for the `execute_three_leg_with_tip` instruction.
///
/// Field order and semantics are identical to [`ExecuteThreeLegArgs`]
/// followed by the tip configuration; this preserves a flat Borsh layout
/// that mirrors the off-chain builder in `arb-tx`.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct ExecuteThreeLegWithTipArgs {
    /// Exact amount in for Leg 1 (borrowed WSOL).
    pub leg1_amount_in: u64,
    /// Minimum acceptable output of Meme A from Leg 1.
    pub leg1_min_amount_out: u64,
    /// Exact amount in for Leg 2 (Meme A).
    pub leg2_amount_in: u64,
    /// Minimum acceptable output of Meme B from Leg 2.
    pub leg2_min_amount_out: u64,
    /// Exact amount in for Leg 3 (Meme B).
    pub leg3_amount_in: u64,
    /// Minimum acceptable output of WSOL from Leg 3.
    pub leg3_min_amount_out: u64,
    /// Minimum final WSOL net increase required across the 3 legs.
    pub min_final_wsol_delta: u64,
    /// Number of bin arrays passed for Leg 1.
    pub leg1_bin_array_count: u8,
    /// Number of bin arrays passed for Leg 2.
    pub leg2_bin_array_count: u8,
    /// Number of bin arrays passed for Leg 3.
    pub leg3_bin_array_count: u8,
    /// Tip share in basis points of the realized gross edge. `tip_bps > 10_000` is rejected.
    pub tip_bps: u64,
}

impl From<&ExecuteThreeLegWithTipArgs> for ExecuteThreeLegArgs {
    fn from(args: &ExecuteThreeLegWithTipArgs) -> Self {
        ExecuteThreeLegArgs {
            leg1_amount_in: args.leg1_amount_in,
            leg1_min_amount_out: args.leg1_min_amount_out,
            leg2_amount_in: args.leg2_amount_in,
            leg2_min_amount_out: args.leg2_min_amount_out,
            leg3_amount_in: args.leg3_amount_in,
            leg3_min_amount_out: args.leg3_min_amount_out,
            min_final_wsol_delta: args.min_final_wsol_delta,
            leg1_bin_array_count: args.leg1_bin_array_count,
            leg2_bin_array_count: args.leg2_bin_array_count,
            leg3_bin_array_count: args.leg3_bin_array_count,
        }
    }
}

/// Account count consumed by the canonical three-leg body (`33 + bin arrays`);
/// the Jito tip account is the next (final) account.
fn three_leg_account_count(c1: usize, c2: usize, c3: usize) -> usize {
    33 + c1 + c2 + c3
}

/// Execute the 3-leg DLMM arbitrage route, then conditionally tip Jito from
/// the realized profit edge. Everything is atomic: any leg failure or any
/// tip-constraint violation reverts the entire transaction.
pub fn process_execute_three_leg_with_tip<'a>(
    accounts: &'a [AccountInfo<'a>],
    args: ExecuteThreeLegWithTipArgs,
) -> ProgramResult {
    if args.tip_bps > BPS_DENOMINATOR {
        return Err(ExecutorError::TipDenominatorInvalid.into());
    }

    let c1 = args.leg1_bin_array_count as usize;
    let c2 = args.leg2_bin_array_count as usize;
    let c3 = args.leg3_bin_array_count as usize;

    let body_accounts = three_leg_account_count(c1, c2, c3);
    if accounts.len() < body_accounts + 1 {
        return Err(ExecutorError::NotEnoughAccountInfos.into());
    }
    if accounts.len() > body_accounts + 1 {
        return Err(ExecutorError::InvalidInstructionData.into());
    }

    let user = &accounts[0];
    let user_wsol_account = &accounts[1];
    let jito_tip_account = &accounts[body_accounts];

    validate_jito_tip_account(jito_tip_account)?;

    // Dimensional guard: the settlement account must be a `user`-owned WSOL
    // token account so that the realized WSOL edge below is 1:1 with lamports.
    validate_user_token_account(user_wsol_account, user.key, Some(&WSOL_MINT))
        .map_err(|_| ExecutorError::TipRequiresWsolMint)?;

    // Pre-execution WSOL balance establishes the pre-borrow baseline that the
    // tip sizing below is measured against (mirrors the canonical processor).
    let wsol_pre = read_token_account_balance(user_wsol_account)?;
    let wsol_baseline = wsol_pre
        .checked_sub(args.leg1_amount_in)
        .ok_or(ExecutorError::CalculationOverflow)?;

    // Delegate the entire routed execution and its sequential-balance
    // validation to the canonical, single-source processor.
    process_execute_three_leg(&accounts[..body_accounts], ExecuteThreeLegArgs::from(&args))?;

    // Realized WSOL edge above the pre-borrow baseline, net of the borrowed
    // principal. WSOL units == lamports (peg verified above).
    let wsol_final = read_token_account_balance(user_wsol_account)?;
    let gross_edge = wsol_final
        .checked_sub(wsol_baseline)
        .and_then(|delta| delta.checked_sub(args.leg1_amount_in))
        .ok_or(ExecutorError::TipExceedsNetProfit)?;

    let tip_amount = tip_amount_for_bps_bounded(
        gross_edge,
        args.tip_bps,
        args.leg1_amount_in,
        args.min_final_wsol_delta,
    )?;

    invoke_jito_tip(user, jito_tip_account, tip_amount)
}

/// Size the tip and enforce the retained-edge floor:
/// `gross_edge - tip >= min_final_wsol_delta - leg1_amount_in` (floored at 0
/// when the operator's `min_final_wsol_delta` does not exceed the principal).
///
/// The canonical body already guarantees `wsol_final - wsol_baseline >=
/// min_final_wsol_delta`, hence `gross_edge >= min_retained`, so the floor is
/// only violated when the configured share would consume it.
fn tip_amount_for_bps_bounded(
    gross_edge: u64,
    tip_bps: u64,
    leg1_amount_in: u64,
    min_final_wsol_delta: u64,
) -> Result<u64, ExecutorError> {
    let min_retained = min_final_wsol_delta.saturating_sub(leg1_amount_in);
    let tip = tip_amount_for_bps(gross_edge, tip_bps)?;
    let retained = gross_edge
        .checked_sub(tip)
        .ok_or(ExecutorError::CalculationOverflow)?;
    if retained < min_retained {
        return Err(ExecutorError::TipExceedsNetProfit);
    }
    Ok(tip)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_args() -> ExecuteThreeLegWithTipArgs {
        ExecuteThreeLegWithTipArgs {
            leg1_amount_in: 1_000_000_000,
            leg1_min_amount_out: 25_000_000,
            leg2_amount_in: 25_000_000,
            leg2_min_amount_out: 50_000_000_000,
            leg3_amount_in: 50_000_000_000,
            leg3_min_amount_out: 1_050_000_000,
            min_final_wsol_delta: 1_010_000_000,
            leg1_bin_array_count: 2,
            leg2_bin_array_count: 3,
            leg3_bin_array_count: 2,
            tip_bps: 7_000,
        }
    }

    #[test]
    fn test_with_tip_args_serialization_roundtrip() {
        let args = base_args();
        let mut buf = Vec::new();
        args.serialize(&mut buf).expect("serialization succeeds");
        let deserialized =
            ExecuteThreeLegWithTipArgs::try_from_slice(&buf).expect("deserialization succeeds");
        assert_eq!(args, deserialized);
        assert_eq!(buf.len(), 7 * 8 + 3 + 8);
    }

    #[test]
    fn test_tip_sizing_honors_bps_and_floor() {
        // gross_edge = 40_000_000; min_final - leg1 = 10_000_000
        let tip = tip_amount_for_bps_bounded(40_000_000, 7_000, 1_000_000_000, 1_010_000_000)
            .expect("tip sizing succeeds");
        assert_eq!(tip, 28_000_000);
        assert!(tip_amount_for_bps_bounded(40_000_000, 0, 1_000_000_000, 1_010_000_000).is_ok());
        assert_eq!(
            tip_amount_for_bps_bounded(40_000_000, BPS_DENOMINATOR, 1_000_000_000, 1_010_000_000),
            Err(ExecutorError::TipExceedsNetProfit),
            "a 100% share would consume the operator's stipulated minimum profit"
        );
        assert_eq!(
            tip_amount_for_bps_bounded(40_000_000, BPS_DENOMINATOR + 1, 1_000, 1_000),
            Err(ExecutorError::TipDenominatorInvalid)
        );
    }

    #[test]
    fn test_tip_floor_floored_at_zero_when_no_profit_stipulation() {
        assert_eq!(
            tip_amount_for_bps_bounded(40_000_000, BPS_DENOMINATOR, 1_000_000_000, 1_000_000_000)
                .unwrap(),
            40_000_000
        );
    }

    #[test]
    fn test_three_leg_account_count() {
        assert_eq!(three_leg_account_count(0, 0, 0), 33);
        assert_eq!(three_leg_account_count(1, 2, 3), 39);
    }
}
