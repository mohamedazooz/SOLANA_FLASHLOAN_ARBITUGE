use arb_math::pump_boost_arb::*;

#[test]
fn test_completion_cost() {
    assert_eq!(completion_cost(85_000_000_000), 0);
    assert_eq!(completion_cost(66_000_000_000), 19_000_000_000);
    assert_eq!(completion_cost(50_000_000_000), 35_000_000_000);
}

#[test]
fn test_tokens_acquired_on_curve() {
    // Typical curve starting values: vsol = 30 SOL, vtok = 1073M tokens (atomic 1e6 = 1_073_000_000_000_000)
    // Suppose S = 66 SOL, so current vsol = 30 + 66 = 96 SOL = 96_000_000_000 lamports.
    // Cost to finish to 85 SOL (vsol = 115 SOL) is 19 SOL = 19_000_000_000 lamports.
    let vsol = 96_000_000_000u64;
    let k = 30u128 * 1_000_000_000 * 1_073_000_000_000_000u128;
    let vtok = (k / (vsol as u128)) as u64;

    let cost = 19_000_000_000u64;
    let acquired = tokens_acquired_on_curve(vsol, vtok, cost).expect("tokens acquired");
    assert!(acquired > 0);

    // After adding 19 SOL, vsol_end is 115 SOL
    let vsol_end = 115_000_000_000u128;
    let vtok_end = (k / vsol_end) as u64;
    assert_eq!(acquired, vtok - vtok_end);
}

#[test]
fn test_evaluate_boost_sweet_spot() {
    // S = 66 SOL, cost = 19 SOL
    let vsol = 96_000_000_000u64;
    let k = 30u128 * 1_000_000_000 * 1_073_000_000_000_000u128;
    let vtok = (k / (vsol as u128)) as u64;

    let eval = evaluate_pump_boost_arb(
        66_000_000_000, // 66 SOL real reserves
        vsol,
        vtok,
        30,          // 0.30% flash loan fee
        100_000_000, // 0.10 SOL Jito tip
        10_000,      // 100% theoretical efficiency
        0,           // 0 min profit
    )
    .expect("valid evaluation");

    assert_eq!(eval.completion_cost_lamports, 19_000_000_000);
    assert!(eval.acquired_tokens > 0);
    assert!(eval.boost_absorbed_lamports > 0);
}
