# Mandatory Agent Task Handoff Protocol

> **CRITICAL DIRECTIVE**: No agent may rely on undocumented conversation context as the authoritative project state. Every task transition, feature completion, or engineering handoff MUST provide this structured manifest.

---

## 1. Mandatory Handoff Template

Whenever completing a task or handing off to another agent (or the human operator), paste and complete the following block in full:

```markdown
TASK_ID: [e.g. TASK-P2-001]
PHASE: [e.g. Phase 2 — Critical Correctness Defects]
BASE_COMMIT: [e.g. d090123]
FILES_CHANGED:
  - [path/to/file1.rs]
  - [path/to/file2.rs]
FUNCTIONS_CHANGED:
  - [TypeName::function_name]
OBSERVED_BEHAVIOR: [Precise description of bug or baseline behavior before modification]
ROOT_CAUSE: [Root cause analysis detailing why the flaw existed]
IMPLEMENTATION: [Exact architectural and code changes introduced]
TESTS_EXECUTED:
  - [cargo test command 1]
  - [cargo test command 2]
TEST_RESULTS: [Exact terminal output and test counts]
CI_STATUS: [PASS / PENDING / FAILED (with run ID)]
KNOWN_LIMITATIONS: [Any known edge cases or deferred edge handling]
OPEN_QUESTIONS: [Unresolved technical questions requiring research or operator input]
REMAINING_RISKS: [Residual operational, quantitative, or latency risks]
NEXT_ACTION: [Immediate next task ID and assigned agent role]
```

---

## 2. Handoff Verification Checklist

Before accepting a handoff, the receiving agent or Supervisor MUST verify:
1. `TASK_ID` exists in [`docs/active/MASTER_TASK_REGISTER.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_TASK_REGISTER.md).
2. `BASE_COMMIT` matches current git branch head.
3. Every file listed under `FILES_CHANGED` has been committed to a feature branch.
4. `TEST_RESULTS` contains copy-pasted reproducible command outputs, not verbal assertions.
5. `NEXT_ACTION` names the specific next task from the roadmap.
