//! Meteora DLMM (`LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`) Unified Adapter.

use crate::traits::AmmPool;
use arb_constants::{METEORA_BINS_PER_BIN_ARRAY, METEORA_DLMM_PROGRAM_ID};
use arb_math::dlmm::{
    DlmmBin as MathDlmmBin, DlmmParameters, DlmmVariableParameters, get_price_from_id,
    quote_dlmm_multi_bin,
};
use arb_types::{
    ArbError, DexProtocol, DlmmBin, DlmmFeeParams, DlmmState, MintId, PoolExecutionAccounts,
    PoolId, Result, TokenProgramKind,
};
use ruint::aliases::U256;
use solana_pubkey::Pubkey;
use std::str::FromStr;

pub const METEORA_LB_PAIR_MIN_SIZE: usize = 300;
pub const METEORA_BIN_ARRAY_HEADER_SIZE: usize = 56;
pub const METEORA_BIN_SIZE: usize = 144;

#[derive(Debug, Clone)]
pub struct MeteoraDlmmPool {
    pub pool_id: PoolId,
    pub token_x_mint: MintId,
    pub token_y_mint: MintId,
    pub reserve_x: Pubkey,
    pub reserve_y: Pubkey,
    pub oracle: Pubkey,
    pub token_x_program: TokenProgramKind,
    pub token_y_program: TokenProgramKind,
    pub active_id: i32,
    pub bin_step: u16,
    /// Pair status byte (0 = Enabled; non-zero = disabled/tombstoned).
    pub status: u8,
    /// StaticParameters.function_type (ConcreteFunctionType; 0 = Spot).
    pub function_type: u8,
    /// StaticParameters.collect_fee_mode (CollectFeeMode).
    pub collect_fee_mode: u8,
    pub fee_params: Option<DlmmFeeParams>,
    pub volatility_accumulator: u32,
    pub volatility_reference: u32,
    /// On-chain `index_reference` (bin the volatility accumulator is measured against).
    pub index_reference: i32,
    pub last_update_timestamp: i64,
    pub bins: Vec<DlmmBin>,
    pub bin_array_pdas: Vec<Pubkey>,
    pub decimals_x: u8,
    pub decimals_y: u8,
    pub transfer_fee_x_bps: u16,
    pub transfer_fee_y_bps: u16,
    pub has_risky: bool,
}

impl MeteoraDlmmPool {
    pub fn from_account(pool_id: PoolId, data: &[u8]) -> Result<Self> {
        let mut pool = Self {
            pool_id,
            token_x_mint: MintId::default(),
            token_y_mint: MintId::default(),
            reserve_x: Pubkey::default(),
            reserve_y: Pubkey::default(),
            oracle: Pubkey::default(),
            token_x_program: TokenProgramKind::SplToken,
            token_y_program: TokenProgramKind::SplToken,
            active_id: 0,
            bin_step: 1,
            status: 0,
            function_type: 0,
            collect_fee_mode: 0,
            fee_params: None,
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: 0,
            last_update_timestamp: 0,
            bins: Vec::new(),
            bin_array_pdas: Vec::new(),
            decimals_x: 0,
            decimals_y: 0,
            transfer_fee_x_bps: 0,
            transfer_fee_y_bps: 0,
            has_risky: false,
        };
        pool.update_from_account(data)?;
        Ok(pool)
    }

    pub fn bin_id_to_array_index(bin_id: i32) -> i64 {
        let b = i64::from(bin_id);
        let n = METEORA_BINS_PER_BIN_ARRAY;
        if b >= 0 { b / n } else { (b - (n - 1)) / n }
    }

    pub fn derive_bin_array_pda(&self, index: i64) -> Pubkey {
        let prog = Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap_or_default();
        let (pda, _) = Pubkey::find_program_address(
            &[b"bin_array", self.pool_id.0.as_ref(), &index.to_le_bytes()],
            &prog,
        );
        pda
    }

    pub fn bin_array_pdas_around_active(&self) -> Vec<Pubkey> {
        let idx = Self::bin_id_to_array_index(self.active_id);
        let step = u32::from(self.bin_step.max(1));
        let bins_traversed = 500u32 / step + 1;
        let arrays_needed = (bins_traversed as i64).div_euclid(METEORA_BINS_PER_BIN_ARRAY) + 1;
        let mut indices = Vec::with_capacity(arrays_needed as usize * 2);
        indices.push(idx);
        for i in 1..arrays_needed {
            indices.push(idx - i);
            indices.push(idx + i);
        }
        indices
            .into_iter()
            .map(|i| self.derive_bin_array_pda(i))
            .collect()
    }

    /// Derive the Meteora DLMM `BinArrayBitmapExtension` PDA for this pool.
    /// Official seeds: `[b"bitmap", lb_pair]`.
    pub fn derive_bin_array_bitmap_extension_pda(&self) -> Pubkey {
        let program_id = Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap_or_default();
        let seeds: &[&[u8]] = &[b"bitmap", self.pool_id.0.as_ref()];
        Pubkey::find_program_address(seeds, &program_id).0
    }

    /// Parse Meteora `BinArray` account bytes into 70 `DlmmBin` records.
    ///
    /// The on-chain `Bin` struct (144 bytes, DLMM v0.12.0) keeps limit-order
    /// liquidity separate from the curve liquidity:
    ///
    /// ```text
    ///    0..8    amount_x
    ///    8..16   amount_y
    ///   16..32   price (u128, Q64.64)
    ///   32..40   liquidity_supply
    ///   40..48   reward_per_token_stored
    ///   48..64   fee_amount_x_per_token_stored
    ///   64..80   fee_amount_y_per_token_stored
    ///   80..96   fee_infos[0]
    ///   96..112  fee_infos[1]
    ///  112..120  open_order_amount
    ///  120..128  _padding
    ///  128..136  processed_order_remaining_amount
    ///  136..140  _padding
    ///  140       limit_order_ask_side
    ///  141..144  _padding
    /// ```
    ///
    /// Limit-order liquidity is executable by the swap, so it is folded into
    /// `amount_x`/`amount_y` here (ask side => X, bid side => Y). The protocol
    /// fee attribution for that liquidity is therefore approximated as curve
    /// liquidity; that approximation is documented in the ADR.
    pub fn parse_bin_array_account(data: &[u8]) -> Result<Vec<DlmmBin>> {
        let n = METEORA_BINS_PER_BIN_ARRAY as usize;
        let expected_min = METEORA_BIN_ARRAY_HEADER_SIZE + n * METEORA_BIN_SIZE;
        if data.len() < expected_min {
            return Err(ArbError::DeserializationError(format!(
                "Meteora BinArray len {} < {}",
                data.len(),
                expected_min
            )));
        }
        let array_index = i64::from_le_bytes(data[8..16].try_into().unwrap());
        let start_bin_id = i32::try_from(array_index.saturating_mul(METEORA_BINS_PER_BIN_ARRAY))
            .map_err(|_| ArbError::Overflow)?;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let base = METEORA_BIN_ARRAY_HEADER_SIZE + i * METEORA_BIN_SIZE;
            let curve_x = u64::from_le_bytes(data[base..base + 8].try_into().unwrap());
            let curve_y = u64::from_le_bytes(data[base + 8..base + 16].try_into().unwrap());
            let open_order_amount =
                u64::from_le_bytes(data[base + 112..base + 120].try_into().unwrap());
            let processed_order_remaining_amount =
                u64::from_le_bytes(data[base + 128..base + 136].try_into().unwrap());
            let ask_side = data[base + 140] != 0;
            let limit_order_amount =
                open_order_amount.saturating_add(processed_order_remaining_amount);
            let (amount_x, amount_y) = if ask_side {
                (curve_x.saturating_add(limit_order_amount), curve_y)
            } else {
                (curve_x, curve_y.saturating_add(limit_order_amount))
            };
            if amount_x > 0 || amount_y > 0 {
                out.push(DlmmBin {
                    bin_id: start_bin_id.saturating_add(i as i32),
                    amount_x: u128::from(amount_x),
                    amount_y: u128::from(amount_y),
                });
            }
        }
        Ok(out)
    }
}

impl AmmPool for MeteoraDlmmPool {
    fn pool_id(&self) -> PoolId {
        self.pool_id
    }

    fn protocol(&self) -> DexProtocol {
        DexProtocol::MeteoraDlmm
    }

    fn base_mint(&self) -> MintId {
        self.token_x_mint
    }

    fn quote_mint(&self) -> MintId {
        self.token_y_mint
    }

    fn reserves(&self) -> (u128, u128) {
        let Ok(price) = get_price_from_id(self.active_id, self.bin_step) else {
            return (0, 0);
        };
        if price == 0 {
            return (0, 0);
        }
        let sum_x: u128 = self
            .bins
            .iter()
            .find(|b| b.bin_id == self.active_id)
            .map(|b| b.amount_x)
            .filter(|v| *v > 0)
            .unwrap_or_else(|| self.bins.iter().map(|b| b.amount_x).sum());
        let ry = (U256::from(sum_x) * U256::from(price)) >> 64usize;
        (sum_x, u128::try_from(ry).unwrap_or(u128::MAX))
    }

    fn fee_bps(&self) -> Option<u16> {
        let p = self.fee_params.as_ref()?;
        let base = u64::from(p.base_factor) * u64::from(self.bin_step) * 10;
        let scaled = if p.base_fee_power_factor == 0 {
            base
        } else {
            base.checked_mul(10u64.checked_pow(u32::from(p.base_fee_power_factor))?)?
        };
        u16::try_from(scaled.saturating_mul(10_000) / 1_000_000_000).ok()
    }

    fn dlmm_state(&self) -> Option<DlmmState> {
        Some(DlmmState {
            active_bin_id: self.active_id,
            status: self.status,
            bin_step_bps: self.bin_step,
            bins: self.bins.clone(),
            params: self.fee_params.clone(),
            volatility_accumulator: self.volatility_accumulator,
            volatility_reference: self.volatility_reference,
            index_reference: self.index_reference,
            last_update_timestamp: self.last_update_timestamp,
        })
    }

    fn decimals(&self) -> (u8, u8) {
        (self.decimals_x, self.decimals_y)
    }

    fn token_programs(&self) -> [TokenProgramKind; 2] {
        [self.token_x_program, self.token_y_program]
    }

    fn transfer_fees_bps(&self) -> (u16, u16) {
        (self.transfer_fee_x_bps, self.transfer_fee_y_bps)
    }

    fn has_risky_token(&self) -> bool {
        self.has_risky
    }

    fn execution_accounts(&self) -> Option<PoolExecutionAccounts> {
        if self.reserve_x == Pubkey::default() || self.reserve_y == Pubkey::default() {
            return None;
        }
        let pdas = if self.bin_array_pdas.is_empty() {
            self.bin_array_pdas_around_active()
        } else {
            self.bin_array_pdas.clone()
        };
        Some(PoolExecutionAccounts::MeteoraDlmm {
            token_x_mint: self.token_x_mint.0,
            token_y_mint: self.token_y_mint.0,
            reserve_x: self.reserve_x,
            reserve_y: self.reserve_y,
            oracle: self.oracle,
            bin_array_pdas: pdas,
            bitmap_extension: Some(self.derive_bin_array_bitmap_extension_pda()),
        })
    }

    fn required_dependency_pubkeys(&self) -> Vec<Pubkey> {
        let mut keys = vec![
            self.reserve_x,
            self.reserve_y,
            self.oracle,
            self.token_x_mint.0,
            self.token_y_mint.0,
        ];
        keys.extend(self.bin_array_pdas_around_active());
        keys
    }

    fn apply_dependency_account(&mut self, pubkey: &Pubkey, data: &[u8]) -> Result<bool> {
        if let Ok(parsed_bins) = Self::parse_bin_array_account(data) {
            for bin in parsed_bins {
                if let Some(existing) = self.bins.iter_mut().find(|b| b.bin_id == bin.bin_id) {
                    *existing = bin;
                } else {
                    self.bins.push(bin);
                }
            }
            if !self.bin_array_pdas.contains(pubkey) {
                self.bin_array_pdas.push(*pubkey);
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn update_from_account(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < METEORA_LB_PAIR_MIN_SIZE {
            return Err(ArbError::DeserializationError(format!(
                "Meteora LbPair len {} < {}",
                data.len(),
                METEORA_LB_PAIR_MIN_SIZE
            )));
        }
        let read_pk = |off: usize| -> Result<Pubkey> {
            let b: [u8; 32] = data[off..off + 32]
                .try_into()
                .map_err(|_| ArbError::DeserializationError("bad pubkey".into()))?;
            Ok(Pubkey::new_from_array(b))
        };
        // LbPair layout, verified against the official DLMM v0.12.0 IDL
        // (`idls/dlmm.json`, metadata `lb_clmm 0.12.0`, program
        // LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo) with `repr(C)` +
        // bytemuck layout. Account offsets include the 8-byte discriminator.
        //
        //   8..40  StaticParameters
        //           8..10  base_factor            (u16)
        //          10..12  filter_period          (u16)
        //          12..14  decay_period           (u16)
        //          14..16  reduction_factor       (u16)
        //          16..20  variable_fee_control   (u32)
        //          20..24  max_volatility_accumulator (u32)
        //          24..28  min_bin_id             (i32)
        //          28..32  max_bin_id             (i32)
        //          32..34  protocol_share         (u16)
        //             34    base_fee_power_factor  (u8)
        //             35    function_type          (u8)
        //             36    collect_fee_mode       (u8)
        //          37..40  padding
        //  40..72  VariableParameters
        //          40..44  volatility_accumulator
        //          44..48  volatility_reference
        //          48..52  index_reference
        //          56..64  last_update_timestamp
        //     72    bump_seed
        //  73..75  bin_step_seed
        //     75    pair_type
        //  76..80  active_id (i32)
        //  80..82  bin_step  (u16)
        //     82    status
        //  88..120  token_x_mint
        // 120..152  token_y_mint
        // 152..184  reserve_x
        // 184..216  reserve_y
        // 216..232  protocol_fee (ProtocolFee: amount_x u64, amount_y u64)
        // 232..264  _padding_1
        // 264..552  reward_infos [RewardInfo; 2] (144 bytes each)
        // 552..584  oracle
        // 584..712  bin_array_bitmap [u64; 16]
        // 712..720  last_updated_at (i64)
        let base_factor = u16::from_le_bytes(data[8..10].try_into().unwrap());
        let filter_period = u16::from_le_bytes(data[10..12].try_into().unwrap());
        let decay_period = u16::from_le_bytes(data[12..14].try_into().unwrap());
        let reduction_factor = u16::from_le_bytes(data[14..16].try_into().unwrap());
        let variable_fee_control = u32::from_le_bytes(data[16..20].try_into().unwrap());
        let max_volatility_accumulator = u32::from_le_bytes(data[20..24].try_into().unwrap());
        let protocol_share = u16::from_le_bytes(data[32..34].try_into().unwrap());
        let base_fee_power_factor = data[34];
        let function_type = data[35];
        let collect_fee_mode = data[36];

        self.fee_params = Some(DlmmFeeParams {
            base_factor,
            filter_period,
            decay_period,
            reduction_factor,
            variable_fee_control,
            max_volatility_accumulator,
            protocol_share,
            base_fee_power_factor,
            function_type,
            collect_fee_mode,
        });
        self.volatility_accumulator = u32::from_le_bytes(data[40..44].try_into().unwrap());
        self.volatility_reference = u32::from_le_bytes(data[44..48].try_into().unwrap());
        self.index_reference = i32::from_le_bytes(data[48..52].try_into().unwrap());
        self.last_update_timestamp = i64::from_le_bytes(data[56..64].try_into().unwrap());

        self.active_id = i32::from_le_bytes(data[76..80].try_into().unwrap());
        self.bin_step = u16::from_le_bytes(data[80..82].try_into().unwrap()).max(1);
        self.status = data[82];
        self.token_x_mint = MintId(read_pk(88)?);
        self.token_y_mint = MintId(read_pk(120)?);
        self.reserve_x = read_pk(152)?;
        self.reserve_y = read_pk(184)?;
        self.oracle = read_pk(552)?;
        self.bin_array_pdas = self.bin_array_pdas_around_active();
        Ok(())
    }

    fn quote(&self, amount_in: u64, a_to_b: bool) -> Result<u64> {
        if self.has_risky {
            return Err(ArbError::InvalidInput("risky Token-2022 extension".into()));
        }
        if self.status != 0 {
            return Err(ArbError::InvalidInput(
                "Meteora DLMM swap is disabled by LbPair status".into(),
            ));
        }
        let params = self
            .fee_params
            .as_ref()
            .ok_or_else(|| ArbError::InvalidInput("Meteora DLMM missing fee params".into()))?;
        if params.function_type != 0 || params.collect_fee_mode > 1 {
            return Err(ArbError::InvalidInput(
                "Meteora DLMM function/fee mode is not supported by the accepted quote adapter"
                    .into(),
            ));
        }
        if self.bins.is_empty() {
            return Err(ArbError::InvalidInput(
                "Meteora DLMM missing bin arrays".into(),
            ));
        }
        if self.transfer_fee_x_bps != 0 || self.transfer_fee_y_bps != 0 {
            return Err(ArbError::InvalidInput(
                "Meteora DLMM Token-2022 transfer fees require epoch/max-fee-aware mint context"
                    .into(),
            ));
        }
        let net_in = amount_in;
        let mut math_bins: Vec<MathDlmmBin> = self
            .bins
            .iter()
            .map(|b| MathDlmmBin {
                id: b.bin_id,
                amount_x: u64::try_from(b.amount_x).unwrap_or(u64::MAX),
                amount_y: u64::try_from(b.amount_y).unwrap_or(u64::MAX),
                price_q64: get_price_from_id(b.bin_id, self.bin_step).unwrap_or(0),
            })
            .collect();
        let pool_state = arb_math::dlmm::DlmmPoolState {
            active_id: self.active_id,
            bin_step: self.bin_step,
            status: self.status,
            parameters: DlmmParameters {
                bin_step: self.bin_step,
                base_factor: params.base_factor,
                filter_period: params.filter_period,
                decay_period: params.decay_period,
                reduction_factor: params.reduction_factor,
                variable_fee_control: params.variable_fee_control,
                max_volatility_accumulator: params.max_volatility_accumulator,
                protocol_share: params.protocol_share,
                base_fee_power_factor: params.base_fee_power_factor,
                collect_fee_mode: params.collect_fee_mode,
            },
            v_parameters: DlmmVariableParameters {
                volatility_accumulator: self.volatility_accumulator,
                volatility_reference: self.volatility_reference,
                index_reference: self.index_reference,
                last_update_timestamp: self.last_update_timestamp,
            },
        };
        let q = quote_dlmm_multi_bin(
            &pool_state,
            &mut math_bins,
            net_in,
            a_to_b,
            if self.last_update_timestamp > 0 {
                Some(self.last_update_timestamp)
            } else {
                None
            },
        )
        .map_err(|e| ArbError::InvalidInput(format!("DLMM quote error: {e:?}")))?;
        Ok(q.amount_out)
    }
}
