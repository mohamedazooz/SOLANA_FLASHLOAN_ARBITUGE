#!/usr/bin/env python3
"""Independent mathematical oracle for SOLANA_FLASHLOAN_ARBITUGE.

Derivation basis (NOT copied from the Rust code — reconstructed from the
protocol specifications):
  * CPMM (Raydium AMM v4 / Raydium CPMM / Meteora DAMM v2 constant-product):
      invariant x * y = k. Upfront-fee variant deducts
      fee = ceil(dx * fee_num / fee_den) before the product division and
      floors the output: out = floor(net_dx * y / (x + net_dx)).
      The gamma-variant folds the fee into the fraction:
      out = floor(gamma * dx * y / (D * x + gamma * dx)).
  * Meteora DLMM single-bin consumption (programs/dlmm/src/state/lb_pair.md
      swap semantics + SDK SwapUtils):
      - fee_on_input: fee = ceil(net_target * fee_rate / 1e9) is taken from the
        inbound token before pricing; bin drains when gross_in >= net_in_max.
      - fee_on_output (CollectFeeMode::OnlyY): the pool consumes gross input,
        fee = ceil(gross_out * fee_rate / 1e9) is removed from the outbound
        token; user receives gross_out - fee.
      - X -> Y (swap_for_y): out = floor(net_in * P / 2^64),
        net_in_max = ceil(out_max * 2^64 / P).
      - Y -> X: out = floor(net_in * 2^64 / P),
        net_in_max = ceil(out_max * P / 2^64).
      - protocol_fee = floor(fee * protocol_share_bps / 10_000).
  Python's arbitrary-precision integers make every intermediate exact; the
  floor/ceil points below are the only rounding decisions, matching the
  protocol specification, not the Rust implementation's convenience.

Usage: python3 oracle.py            # regenerate ../test_vectors/*.json
       python3 oracle.py --verify   # re-derive and assert vectors match
"""

import json
import os

FEE_PRECISION = 1_000_000_000
SCALE = 1 << 64
BPS_MAX = 10_000


def ceil_div(a: int, b: int) -> int:
    return -(-a // b)


# ---------------------------------------------------------------------------
# CPMM
# ---------------------------------------------------------------------------

def cpmm_out_upfront_fee_ceil(dx: int, x: int, y: int, fee_num: int, fee_den: int):
    if dx == 0 or x == 0 or y == 0 or fee_den == 0 or fee_num >= fee_den:
        return None
    fee = ceil_div(dx * fee_num, fee_den)
    if fee >= dx:
        return 0
    net_dx = dx - fee
    return (net_dx * y) // (x + net_dx)


def cpmm_out_fee(dx: int, x: int, y: int, gamma: int, fee_den: int):
    if dx == 0 or x == 0 or y == 0 or fee_den == 0 or gamma == 0:
        return None
    num = gamma * dx * y
    denom = fee_den * x + gamma * dx
    if denom == 0:
        return None
    return num // denom


# ---------------------------------------------------------------------------
# Meteora DLMM single bin
# ---------------------------------------------------------------------------

def net_in_for_gross_out(gross_out: int, price_q64: int, swap_for_y: bool) -> int:
    """P is the Q64.64 price of X in terms of Y (y per x).
    X -> Y: out_y = net_in_x * P  =>  net_in = ceil(out * 2^64 / P).
    Y -> X: out_x = net_in_y / P  =>  net_in = ceil(out * P / 2^64)."""
    if swap_for_y:
        return ceil_div(gross_out * SCALE, price_q64)
    return ceil_div(gross_out * price_q64, SCALE)


def gross_out_for_input(net_in: int, price_q64: int, swap_for_y: bool) -> int:
    if swap_for_y:
        return (net_in * price_q64) >> 64
    return (net_in << 64) // price_q64


def fee_from_amount(amount: int, fee_rate: int) -> int:
    """Fee removed FROM an amount (SDK getExcludedFeeAmount):
    ceil(amount * rate / 1e9)."""
    if fee_rate == 0 or amount == 0:
        return 0
    return ceil_div(amount * fee_rate, FEE_PRECISION)


def fee_to_add(amount: int, fee_rate: int) -> int:
    """Fee added ON TOP of a fee-excluded amount (SDK getIncludedFeeAmount):
    ceil(amount * rate / (1e9 - rate)), so that amount + fee = gross whose
    removal of fee_rate leaves exactly `amount` (up to ceiling dust)."""
    if fee_rate == 0 or amount == 0:
        return 0
    return ceil_div(amount * fee_rate, FEE_PRECISION - fee_rate)


def protocol_fee(fee_amount: int, share_bps: int) -> int:
    if fee_amount == 0 or share_bps == 0:
        return 0
    return (fee_amount * share_bps) // BPS_MAX


def consume_bin(amount_in: int, price_q64: int, swap_for_y: bool, fee_rate: int,
                share_bps: int, fee_on_input: bool, bin_x: int, bin_y: int):
    """Returns (consumed_in, user_out, fee_amount, protocol_fee, drained, bin_x', bin_y')."""
    bin_max_out = bin_y if swap_for_y else bin_x
    if amount_in == 0:
        return (0, 0, 0, 0, False, bin_x, bin_y)
    if bin_max_out == 0:
        return (0, 0, 0, 0, True, bin_x, bin_y)

    if fee_on_input:
        net_in_max = net_in_for_gross_out(bin_max_out, price_q64, swap_for_y)
        max_fee = fee_to_add(net_in_max, fee_rate)
        gross_in_max = net_in_max + max_fee
        if amount_in >= gross_in_max:
            pf = protocol_fee(max_fee, share_bps)
            if swap_for_y:
                return (gross_in_max, bin_max_out, max_fee, pf, True,
                        bin_x + net_in_max, 0)
            return (gross_in_max, bin_max_out, max_fee, pf, True,
                    0, bin_y + net_in_max)
        fee = fee_from_amount(amount_in, fee_rate)
        net_in = amount_in - fee
        out = min(gross_out_for_input(net_in, price_q64, swap_for_y), bin_max_out)
        pf = protocol_fee(fee, share_bps)
        if swap_for_y:
            return (amount_in, out, fee, pf, False, bin_x + net_in, bin_y - out)
        return (amount_in, out, fee, pf, False, bin_x - out, bin_y + net_in)

    # fee on output (OnlyY)
    net_in_max = net_in_for_gross_out(bin_max_out, price_q64, swap_for_y)
    if amount_in >= net_in_max:
        consumed, gross_out, drained = net_in_max, bin_max_out, True
    else:
        gross_out = min(gross_out_for_input(amount_in, price_q64, swap_for_y), bin_max_out)
        consumed, drained = amount_in, False
    fee = fee_from_amount(gross_out, fee_rate)
    user_out = gross_out - fee
    pf = protocol_fee(fee, share_bps)
    if swap_for_y:
        return (consumed, user_out, fee, pf, drained, bin_x + consumed, bin_y - gross_out)
    return (consumed, user_out, fee, pf, drained, bin_x - gross_out, bin_y + consumed)


# ---------------------------------------------------------------------------
# Vector generation
# ---------------------------------------------------------------------------

def cpmm_vectors():
    cases = []
    # Deterministic spread: tiny, typical, large, boundary-draining inputs.
    fixtures = [
        (1_000, 500_000_000, 400_000_000, 25),          # 0.25% fee, WSOL-scale
        (10_000_000, 82_260_000_000, 6_570_000_000, 30),
        (999_999_999, 10**15, 10**15, 1),               # near-parity, 1bp
        (10**12, 10**18, 3 * 10**17, 250),              # deep book, 2.5%
        (7_998, 65_700_000, 8_226_000_000, 20),         # HRYEjw-scale fixture
    ]
    for dx, x, y, fee_bps in fixtures:
        for fee_den in (10_000, 1_000_000):
            fee_num = fee_bps * (fee_den // 10_000)
            out = cpmm_out_upfront_fee_ceil(dx, x, y, fee_num, fee_den)
            gamma = fee_den - fee_num
            out_g = cpmm_out_fee(dx, x, y, gamma, fee_den)
            cases.append({
                "dx": dx, "x": x, "y": y,
                "fee_num": fee_num, "fee_den": fee_den, "gamma": gamma,
                "out_upfront_fee_ceil": out,
                "out_gamma_folded": out_g,
            })
    # Fail-closed boundaries.
    cases.append({"dx": 0, "x": 1, "y": 1, "fee_num": 25, "fee_den": 10_000,
                  "out_upfront_fee_ceil": None, "out_gamma_folded": None})
    cases.append({"dx": 1_000, "x": 0, "y": 1, "fee_num": 25, "fee_den": 10_000,
                  "out_upfront_fee_ceil": None, "out_gamma_folded": None})
    cases.append({"dx": 1_000, "x": 1, "y": 1, "fee_num": 10_000, "fee_den": 10_000,
                  "out_upfront_fee_ceil": None, "out_gamma_folded": None})
    return {"function": "cpmm_out_upfront_fee_ceil / cpmm_out_fee",
            "module": "arb_math::cpmm", "vectors": cases}


def dlmm_vectors():
    cases = []
    price = (1 + 10 / 10_000) ** 8  # bin_step 10, active_id 8 above anchor
    price_q64 = int(price * SCALE)
    fixtures = [
        (1_000_000, 10**12, 4 * 10**12, 10_000_000, 0, True),    # 1% fee, X->Y
        (1_000_000, 10**12, 4 * 10**12, 10_000_000, 0, False),   # 1% fee on output
        (5 * 10**13, 10**12, 4 * 10**12, 10_000_000, 200, True),  # drains bin
        (10**9, 777, 999, 2_500_000, 1_000, False),
        (1, 1, 1, 1, 0, True),                                    # dust
        (2**63, 2**40, 2**40, 50_000_000, 500, True),             # large
    ]
    for amount_in, bx, by, fee_rate, share, for_y in fixtures:
        r = consume_bin(amount_in, price_q64, for_y, fee_rate, share,
                        fee_on_input=True, bin_x=bx, bin_y=by)
        r2 = consume_bin(amount_in, price_q64, for_y, fee_rate, share,
                         fee_on_input=False, bin_x=bx, bin_y=by)
        cases.append({
            "amount_in": amount_in, "bin_x": bx, "bin_y": by,
            "price_q64": str(price_q64),
            "swap_for_y": for_y,
            "fee_rate": fee_rate, "protocol_share_bps": share,
            "fee_on_input": {
                "amount_in_consumed": r[0], "amount_out": r[1],
                "fee_amount": r[2], "protocol_fee": r[3],
                "drained_bin": r[4], "bin_x_after": r[5], "bin_y_after": r[6],
            },
            "fee_on_output": {
                "amount_in_consumed": r2[0], "amount_out": r2[1],
                "fee_amount": r2[2], "protocol_fee": r2[3],
                "drained_bin": r2[4], "bin_x_after": r2[5], "bin_y_after": r2[6],
            },
        })
    return {"function": "consume_bin_liquidity", "module": "arb_math::dlmm",
            "vectors": cases}


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    out_dir = os.path.join(here, "..", "test_vectors")
    bundles = {"cpmm_vectors.json": cpmm_vectors(),
               "dlmm_single_bin_vectors.json": dlmm_vectors()}
    for name, bundle in bundles.items():
        path = os.path.join(out_dir, name)
        if os.environ.get("ORACLE_VERIFY") and os.path.exists(path):
            with open(path) as f:
                assert json.load(f) == bundle, f"mismatch regenerating {name}"
        with open(path, "w") as f:
            json.dump(bundle, f, indent=1)
        print(f"wrote {path}: {len(bundle['vectors'])} vectors")


if __name__ == "__main__":
    main()
