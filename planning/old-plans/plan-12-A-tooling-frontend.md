# plan-12-A: Coverage — tooling + Tier-A front-end

Last updated: 2026-07-01
Effort: large

Part **A** of plan-12 (Built-in Unit Tests + Coverage). Stands up the coverage tooling and covers
the Tier-A front-end (the biggest coverage gain per test). Shared design (why cargo-llvm-cov,
targets/exclusions, testing strategy by tier, per-file inventory, conventions) lives in the
overview: [plan-12-unit-tests.md](plan-12-unit-tests.md).

- **Depends on:** nothing — land first (every later sub-plan's PRs show deltas against this).
- **Blocks:** plan-12-B, plan-12-C.
- **Design:** overview "Tooling setup", "Testing strategy by tier" (Tier A), "Per-file inventory".

## Phases

### Phase A1 — Tooling & baseline

- [ ] Add `rust-toolchain.toml`, `scripts/coverage.sh`, `scripts/coverage-check.sh`, `.github/workflows/coverage.yml`, `src/testutil.rs`.
- [ ] Record the **baseline per-file report** as the starting line.
- [ ] Finalize the `mod.rs` exclusion list.

Acceptance: `sh scripts/coverage.sh` then `sh scripts/coverage-check.sh` run clean locally and in CI and emit a per-file baseline report; the exclusion list is committed.
Commit: —

### Phase A2 — Tier A front-end

Biggest coverage gain per test.

- [ ] Unit-test `numeric`, `escape`, `lexer`, `rules`, `target` (95–100%).
- [ ] Then the big three `ast` / `ir` / `binary_repr`.
- [ ] Then `typecheck`, `resolver`, `monomorph`.

Acceptance: every listed front-end file ≥95% line coverage in both reports; `cargo test --workspace` green.
Commit: —
