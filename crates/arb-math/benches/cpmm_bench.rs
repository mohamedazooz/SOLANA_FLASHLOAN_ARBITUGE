//! Criterion benchmarks for the CPMM sizing engine.
//!
//! Run: `cargo bench -p arb-math`

use criterion::{Bencher, Criterion, criterion_group, criterion_main};

fn bench_cpmm_out_basic(b: &mut Bencher) {
    b.iter(|| arb_math::cpmm::cpmm_out(1_000_000, 10_000_000, 10_000_000, 9_975).unwrap());
}

fn bench_optimal_two_pool(b: &mut Bencher) {
    let l1 = arb_math::cpmm::Leg {
        r_in: 1_000_000_000,
        r_out: 1_000_000_000,
        gamma: 9_975,
    };
    let l2 = arb_math::cpmm::Leg {
        r_in: 1_000_000_000,
        r_out: 1_050_000_000,
        gamma: 9_975,
    };
    b.iter(|| arb_math::cpmm::optimal_two_pool(&l1, &l2));
}

fn bench_cpmm(c: &mut Criterion) {
    let mut group = c.benchmark_group("cpmm");
    group.bench_function("cpmm_out", bench_cpmm_out_basic);
    group.bench_function("optimal_two_pool", bench_optimal_two_pool);
    group.finish();
}

criterion_group!(benches, bench_cpmm);
criterion_main!(benches);
