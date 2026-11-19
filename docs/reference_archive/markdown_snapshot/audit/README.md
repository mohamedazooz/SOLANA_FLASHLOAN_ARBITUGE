# Independent Audit Artifacts

- `math_oracle/oracle.py` — specification-derived Python oracle (NOT a port of the Rust
  code). Regenerates vectors: `python3 audit/math_oracle/oracle.py`.
  Verify-only mode: `ORACLE_VERIFY=1 python3 audit/math_oracle/oracle.py`.
- `test_vectors/` — committed JSON vectors; consumed by
  `crates/arb-math/tests/python_oracle_differential.rs` (19/19 match required).
- `fixtures/`, `regression/` — reserved for captured mainnet snapshots and failing-input
  regression vectors (empty at this audit pass; population = backlog).

The oracle is deliberately independent from production calculation paths; it shares no
code with the engine and must never be imported by it.
