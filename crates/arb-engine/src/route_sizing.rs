//! Bounded non-monotonic input search shared by graph-generated routes.
//!
//! Unless the complete integer domain is enumerated, this module returns the
//! best candidate observed within a hard evaluation budget. It never labels a
//! sampled result as a proven global optimum.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchProofStatus {
    /// Every integer input in the inclusive domain was evaluated.
    ExhaustiveIntegerDomain,
    /// A deterministic bounded search returned its best observed candidate.
    BestObservedWithinBudget,
}

#[derive(Debug, Clone)]
pub struct SizeSearchResult<T> {
    pub input: u64,
    pub score: i128,
    pub candidate: T,
    pub evaluated_count: usize,
    pub evaluation_budget: usize,
    pub budget_exhausted: bool,
    pub search_complete: bool,
    pub proof_status: SearchProofStatus,
}

/// Compatibility wrapper. Production callers should use the explicit-budget
/// variant so a route cannot silently expand its quote workload.
pub fn optimize_bounded_non_monotonic<T: Clone>(
    max_input: u64,
    coarse_samples: usize,
    max_basins: usize,
    refine_steps: usize,
    boundary_seeds: &[u64],
    evaluate: impl FnMut(u64) -> Option<(T, i128)>,
) -> Option<SizeSearchResult<T>> {
    let default_budget = coarse_samples
        .clamp(2, 48)
        .saturating_mul(3)
        .saturating_add(max_basins.min(8).saturating_mul(refine_steps.clamp(2, 16)))
        .saturating_add(128)
        .clamp(64, 512);
    optimize_bounded_non_monotonic_with_budget(
        max_input,
        coarse_samples,
        max_basins,
        refine_steps,
        boundary_seeds,
        default_budget,
        evaluate,
    )
}

/// Bounded deterministic search over an integer input domain.
///
/// Search stages:
/// 1. Exhaustive enumeration when the entire raw-unit domain fits the budget.
/// 2. Deterministic geometric and linear seeds plus quote-derived breakpoints.
/// 3. Local refinement around the strongest sampled basins.
/// 4. Adaptive midpoint subdivision, prioritizing the widest remaining
///    intervals (ties prefer intervals adjacent to stronger observed scores).
///
/// The sampled stages are a heuristic, not a mathematical proof: without valid
/// interval bounds the optimizer reports BestObservedWithinBudget.
pub fn optimize_bounded_non_monotonic_with_budget<T: Clone>(
    max_input: u64,
    coarse_samples: usize,
    max_basins: usize,
    refine_steps: usize,
    boundary_seeds: &[u64],
    evaluation_budget: usize,
    mut evaluate: impl FnMut(u64) -> Option<(T, i128)>,
) -> Option<SizeSearchResult<T>> {
    if max_input == 0 || evaluation_budget == 0 {
        return None;
    }

    let budget = evaluation_budget.min(1_000_000);
    let mut evaluations = BTreeMap::<u64, Option<(T, i128)>>::new();

    // True proof is only possible if every integer raw-unit amount is tested.
    if max_input <= budget as u64 {
        for point in 1..=max_input {
            evaluations.insert(point, evaluate(point));
        }
        return build_result(max_input, budget, evaluations, true);
    }

    let mut points = vec![1, max_input];

    // Geometric grid for order-of-magnitude coverage.
    let logarithmic_limit = budget.clamp(2, 96);
    let mut logarithmic = 1u64;
    while logarithmic < max_input && points.len() < logarithmic_limit {
        let next = ((u128::from(logarithmic) * 3) / 2)
            .max(u128::from(logarithmic).saturating_add(1))
            .min(u128::from(max_input)) as u64;
        if next <= logarithmic {
            break;
        }
        points.push(next);
        logarithmic = next;
    }

    // Uniform grid protects against long flat regions and poorly placed log seeds.
    let sample_count = coarse_samples.clamp(2, 48);
    for i in 1..sample_count {
        let point = ((u128::from(max_input) * i as u128) / sample_count as u128) as u64;
        if point > 0 {
            points.push(point);
        }
    }

    // Preserve all boundary seeds when they fit. If not, keep endpoints and
    // choose a deterministic, evenly spaced subset to respect the hard budget.
    let mut boundaries: Vec<u64> = boundary_seeds
        .iter()
        .copied()
        .filter(|&point| point > 0 && point <= max_input)
        .collect();
    boundaries.sort_unstable();
    boundaries.dedup();

    let remaining_seed_slots = budget.saturating_sub(points.len().min(budget));
    if boundaries.len() > remaining_seed_slots && remaining_seed_slots > 0 {
        let last = boundaries.len() - 1;
        for index in 0..remaining_seed_slots {
            let selected = if remaining_seed_slots == 1 {
                last / 2
            } else {
                index * last / (remaining_seed_slots - 1)
            };
            points.push(boundaries[selected]);
        }
    } else if remaining_seed_slots > 0 {
        points.extend(boundaries);
    }

    points.sort_unstable();
    points.dedup();
    if points.len() > budget {
        points.truncate(budget);
        if points.last().copied() != Some(max_input) {
            if let Some(last) = points.last_mut() {
                *last = max_input;
            }
        }
        points.sort_unstable();
        points.dedup();
    }

    for point in points {
        if evaluations.len() >= budget {
            break;
        }
        evaluations.insert(point, evaluate(point));
    }

    // Refine each promising sampled local maximum before spending the remaining
    // budget on broad adaptive coverage.
    let coarse: Vec<u64> = evaluations.keys().copied().collect();
    let mut peak_indices: Vec<usize> = (0..coarse.len())
        .filter(|&index| {
            let score = score_at(&evaluations, coarse[index]);
            let left = index
                .checked_sub(1)
                .map(|i| score_at(&evaluations, coarse[i]))
                .unwrap_or(i128::MIN);
            let right = coarse
                .get(index + 1)
                .map(|size| score_at(&evaluations, *size))
                .unwrap_or(i128::MIN);
            score >= left && score >= right && score > i128::MIN
        })
        .collect();

    peak_indices.sort_by_key(|&index| {
        (
            std::cmp::Reverse(score_at(&evaluations, coarse[index])),
            coarse[index],
        )
    });
    peak_indices.truncate(max_basins.min(8));

    let steps = refine_steps.clamp(2, 16) as u128;
    'basins: for index in peak_indices {
        let lo = coarse[index.saturating_sub(1)];
        let hi = coarse[(index + 1).min(coarse.len() - 1)];
        if hi <= lo.saturating_add(1) {
            continue;
        }
        for step in 1..steps {
            if evaluations.len() >= budget {
                break 'basins;
            }
            let point = lo.saturating_add(((u128::from(hi - lo) * step) / steps) as u64);
            if point > lo && point < hi && !evaluations.contains_key(&point) {
                evaluations.insert(point, evaluate(point));
            }
        }
    }

    // Adaptive subdivision: split the widest unvisited interval. Ties prefer
    // intervals with stronger adjacent observed scores, then smaller input.
    // This is a deterministic coverage heuristic, not a valid profit upper bound.
    while evaluations.len() < budget {
        let samples: Vec<(u64, i128)> = evaluations
            .iter()
            .map(|(point, result)| {
                (
                    *point,
                    result
                        .as_ref()
                        .map(|(_, score)| *score)
                        .unwrap_or(i128::MIN),
                )
            })
            .collect();
        let best_interval = samples
            .windows(2)
            .filter_map(|pair| {
                let (lo, lo_score) = pair[0];
                let (hi, hi_score) = pair[1];
                (hi > lo.saturating_add(1)).then_some((
                    hi - lo,
                    lo_score.max(hi_score),
                    lo,
                    lo + (hi - lo) / 2,
                ))
            })
            .max_by_key(|(width, score, lo, _)| (*width, *score, std::cmp::Reverse(*lo)));

        let Some((_, _, _, midpoint)) = best_interval else {
            break;
        };
        if midpoint == 0 || evaluations.contains_key(&midpoint) {
            break;
        }
        evaluations.insert(midpoint, evaluate(midpoint));
    }

    build_result(max_input, budget, evaluations, false)
}

fn score_at<T>(evaluations: &BTreeMap<u64, Option<(T, i128)>>, input: u64) -> i128 {
    evaluations
        .get(&input)
        .and_then(Option::as_ref)
        .map(|(_, score)| *score)
        .unwrap_or(i128::MIN)
}

fn build_result<T: Clone>(
    max_input: u64,
    budget: usize,
    evaluations: BTreeMap<u64, Option<(T, i128)>>,
    exhaustive: bool,
) -> Option<SizeSearchResult<T>> {
    let evaluated_count = evaluations.len();
    let (input, candidate, score) = evaluations
        .into_iter()
        .filter_map(|(input, result)| result.map(|(candidate, score)| (input, candidate, score)))
        .max_by_key(|(input, _, score)| (*score, std::cmp::Reverse(*input)))?;

    let search_complete = exhaustive || evaluated_count as u64 == max_input;
    Some(SizeSearchResult {
        input,
        score,
        candidate,
        evaluated_count,
        evaluation_budget: budget,
        budget_exhausted: evaluated_count >= budget && !search_complete,
        search_complete,
        proof_status: if search_complete {
            SearchProofStatus::ExhaustiveIntegerDomain
        } else {
            SearchProofStatus::BestObservedWithinBudget
        },
    })
}

/// Geometric candidate-borrow ladder shared by the production supervisor and
/// rehearsal harness. Integer-only and overflow-safe.
#[must_use]
pub fn build_size_ladder(from_size: u64, ceiling: u64, capacity: usize) -> Vec<u64> {
    let ceiling = ceiling.max(1);
    let capacity = capacity.clamp(1, 8);
    let mut sizes = Vec::with_capacity(capacity);
    let mut s = from_size.min(ceiling).max(1);
    while sizes.len() < capacity.saturating_sub(1) && s < ceiling {
        sizes.push(s);
        let next = s.saturating_mul(3).max(s.saturating_add(1));
        if next <= s {
            break;
        }
        s = next.min(ceiling);
    }
    if sizes.last().copied() != Some(ceiling) && capacity > 1 {
        sizes.push(ceiling);
    }
    if sizes.is_empty() {
        sizes.push(if capacity == 1 { s } else { ceiling });
    }
    sizes
}

#[cfg(test)]
mod tests {
    use super::{
        SearchProofStatus, build_size_ladder, optimize_bounded_non_monotonic,
        optimize_bounded_non_monotonic_with_budget,
    };

    #[test]
    fn size_ladder_is_deterministic_and_includes_the_ceiling() {
        assert_eq!(build_size_ladder(5, 100, 4), vec![5, 15, 45, 100]);
        assert_eq!(build_size_ladder(100, 100, 1), vec![100]);
    }

    #[test]
    fn selects_best_late_basin_and_refines_around_it() {
        let result = optimize_bounded_non_monotonic(100, 12, 2, 8, &[], |input| {
            let score = if (70..=85).contains(&input) {
                1_000 - i128::from(input.abs_diff(78))
            } else if (20..=35).contains(&input) {
                100 - i128::from(input.abs_diff(27))
            } else {
                -10
            };
            Some((input, score))
        })
        .expect("at least one candidate");

        assert_eq!(result.input, 78);
        assert_eq!(result.score, 1_000);
        assert!(result.evaluated_count > 12);
        assert!(result.evaluated_count <= result.evaluation_budget);
    }

    #[test]
    fn capacity_boundary_seed_can_reveal_a_narrow_profitable_interval() {
        let result = optimize_bounded_non_monotonic(1_000_000, 8, 4, 8, &[500_000], |input| {
            (499_990..=500_010)
                .contains(&input)
                .then_some((input, 7_000 - i128::from(input.abs_diff(500_000))))
        })
        .expect("capacity seed falls inside the narrow island");

        assert_eq!(result.input, 500_000);
        assert_eq!(result.score, 7_000);
    }

    #[test]
    fn exhaustive_small_domain_is_marked_as_proven() {
        let result = optimize_bounded_non_monotonic_with_budget(64, 12, 4, 8, &[], 64, |input| {
            Some((input, -(i128::from(input.abs_diff(37)))))
        })
        .unwrap();
        assert_eq!(result.input, 37);
        assert_eq!(result.evaluated_count, 64);
        assert!(result.search_complete);
        assert_eq!(
            result.proof_status,
            SearchProofStatus::ExhaustiveIntegerDomain
        );
        assert!(!result.budget_exhausted);
    }

    #[test]
    fn non_exhaustive_search_reports_best_observed_and_respects_hard_budget() {
        let seeds: Vec<u64> = (1..=10_000).collect();
        let result = optimize_bounded_non_monotonic_with_budget(
            10_000_000,
            40,
            8,
            16,
            &seeds,
            96,
            |input| Some((input, i128::from(input % 97))),
        )
        .unwrap();
        assert_eq!(result.evaluation_budget, 96);
        assert_eq!(result.evaluated_count, 96);
        assert!(result.budget_exhausted);
        assert!(!result.search_complete);
        assert_eq!(
            result.proof_status,
            SearchProofStatus::BestObservedWithinBudget
        );
    }

    #[test]
    fn adaptive_search_can_find_a_peak_between_coarse_seeds() {
        let result =
            optimize_bounded_non_monotonic_with_budget(1_000, 4, 2, 4, &[], 256, |input| {
                let score = 10_000 - i128::from(input.abs_diff(613));
                Some((input, score))
            })
            .unwrap();
        assert!(result.input.abs_diff(613) <= 2);
        assert!(!result.search_complete);
    }

    #[test]
    fn all_none_is_rejected_even_when_budget_is_available() {
        assert!(
            optimize_bounded_non_monotonic_with_budget::<()>(100, 8, 4, 8, &[], 48, |_| None)
                .is_none()
        );
    }
}
