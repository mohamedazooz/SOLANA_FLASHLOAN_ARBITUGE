# Performance Benchmark Report

## Summary

This report documents performance benchmarking for the arb-execution crate.

## Metrics

### Compiler Latency
- V0 compilation: ~1-2ms (estimated)
- V1 compilation: ~2-3ms (estimated)
- Serialization: ~0.5ms (estimated)

### Memory Usage
- V0 transaction: ~1.3KB wire bytes
- V1 transaction: Variable (up to 4KB)
- Account storage: ~64 accounts max

### CPU Utilization
- Single-threaded compilation
- No parallel processing in hot path
- Zero-allocation where possible

### Route Complexity Impact
- 2-hop routes: Lower latency
- 3-hop routes: Higher latency (more accounts)
- DLMM routes: Highest (bin arrays)

## Benchmark Methodology

- Use criterion for reproducible benchmarks
- Measure p50, p95, p99 latencies
- Test with various route complexities
- Report memory allocations

## Recommendations

1. Add criterion benchmarks for all compilation paths
2. Measure actual (not estimated) latencies
3. Profile memory allocations
4. Test with production-like route complexity

## Verdict

**NOTED** — Performance benchmarking infrastructure needs to be implemented. Current estimates are based on code analysis, not actual measurements.