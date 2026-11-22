//! Bounded non-monotonic input search shared by graph-generated routes.
//!
//! This returns the best candidate observed within its explicit sample budget;
//! it deliberately does not claim a mathematical global optimum.

use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct SizeSearchResult<T> {
    pub input: u64,
    pub score: i128,
    pub candidate: T,
    pub evaluated_count: usize,
}

pub fn optimize_bounded_non_monotonic<T: Clone>(
    max_input: u64,
    coarse_samples: usize,
    max_basins: usize,
    refine_steps: usize,
    boundary_seeds: &[u64],
    mut evaluate: impl FnMut(u64) -> Option<(T, i128)>,
) -> Option<SizeSearchResult<T>> {
    if max_input == 0 {
        return None;
    }

    let mut points = vec![1, max_input];
    let mut logarithmic = 1u64;
    while logarithmic < max_input && points.len() < 96 {
        let next = ((u128::from(logarithmic) * 3) / 2)
            .max(u128::from(logarithmic).saturating_add(1))
            .min(u128::from(max_input)) as u64;
        if next <= logarithmic {
            break;
        }
        points.push(next);
        logarithmic = next;
    }

    let sample_count = coarse_samples.clamp(2, 48);
    for i in 1..sample_count {
        let point = ((u128::from(max_input) * i as u128) / sample_count as u128) as u64;
        if point > 0 {
            points.push(point);
        }
    }
    points.extend(
        boundary_seeds
            .iter()
            .copied()
            .filter(|&p| p > 0 && p <= max_input),
    );
    points.sort_unstable();
    points.dedup();

    // A present `None` sample is retained as a rejected point so profitable
    // islands between infeasible samples still appear as local peaks.
    let mut evaluations = BTreeMap::<u64, Option<(T, i128)>>::new();
    for &point in &points {
        evaluations.insert(point, evaluate(point));
    }

    let coarse: Vec<u64> = evaluations.keys().copied().collect();
    let mut peak_indices: Vec<usize> = (0..coarse.len())
        .filter(|&index| {
            let score = evaluations[&coarse[index]]
                .as_ref()
                .map(|(_, score)| *score)
                .unwrap_or(i128::MIN);
            let left = index
                .checked_sub(1)
                .map(|i| {
                    evaluations[&coarse[i]]
                        .as_ref()
                        .map(|(_, score)| *score)
                        .unwrap_or(i128::MIN)
                })
                .unwrap_or(i128::MIN);
            let right = coarse
                .get(index + 1)
                .and_then(|size| evaluations[size].as_ref().map(|(_, score)| *score))
                .unwrap_or(i128::MIN);
            score >= left && score >= right && score > i128::MIN
        })
        .collect();

    peak_indices.sort_by_key(|&index| {
        let score = evaluations[&coarse[index]]
            .as_ref()
            .map(|(_, score)| *score)
            .unwrap_or(i128::MIN);
        (std::cmp::Reverse(score), coarse[index])
    });
    peak_indices.truncate(max_basins.min(8));

    let steps = refine_steps.clamp(2, 16) as u128;
    for index in peak_indices {
        let lo = coarse[index.saturating_sub(1)];
        let hi = coarse[(index + 1).min(coarse.len() - 1)];
        if hi <= lo.saturating_add(1) {
            continue;
        }
        for step in 1..steps {
            let point = lo.saturating_add(((u128::from(hi - lo) * step) / steps) as u64);
            if point > lo && point < hi && !evaluations.contains_key(&point) {
                evaluations.insert(point, evaluate(point));
            }
        }
    }

    let evaluated_count = evaluations.len();
    let (input, candidate, score) = evaluations
        .into_iter()
        .filter_map(|(input, result)| result.map(|(candidate, score)| (input, candidate, score)))
        .max_by_key(|(input, _, score)| (*score, std::cmp::Reverse(*input)))?;

    Some(SizeSearchResult {
        input,
        score,
        candidate,
        evaluated_count,
    })
}

#[cfg(test)]
mod tests {
    use super::optimize_bounded_non_monotonic;

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
    fn exhaustive_small_range_matches_the_exact_best_candidate() {
        let score = |input: u64| {
            let first_peak = 200 - i128::from(input.abs_diff(13)) * 7;
            let second_peak = 250 - i128::from(input.abs_diff(37)) * 3;
            first_peak.max(second_peak)
        };
        let exact = (1..=48)
            .map(|input| (input, score(input)))
            .max_by_key(|x| x.1);
        let searched =
            optimize_bounded_non_monotonic(48, 48, 8, 16, &[], |input| Some((input, score(input))))
                .expect("all positive sizes have an evaluation");

        assert_eq!(Some((searched.input, searched.score)), exact);
    }

    #[test]
    fn zero_bound_has_no_candidate() {
        assert!(optimize_bounded_non_monotonic::<()>(0, 16, 4, 8, &[], |_| None).is_none());
    }
}
