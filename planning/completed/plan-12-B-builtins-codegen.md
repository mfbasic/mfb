# plan-12-B: Coverage — builtins + Tier-B codegen

Last updated: 2026-07-01
Effort: large

Part **B** of plan-12 (Built-in Unit Tests + Coverage). Covers the builtins/audit/man tables and the
Tier-B codegen (per-builder instruction/plan assertions). Shared design lives in the overview:
[plan-12-unit-tests.md](plan-12-unit-tests.md).

- **Depends on:** plan-12-A (tooling must exist).
- **Design:** overview "Testing strategy by tier" (Tier B), "Per-file inventory" (builtins, arch/codegen).

## Phases

### Phase B1 — Builtins + audit + man

Mostly signature/validation tables; fast wins.

- [ ] Cover the builtins + audit + man signature/validation tables to ~98%.

Acceptance: the Tier-A builtins/audit/man files ≥95% (target 98%) in both reports; suite green.
Commit: —

### Phase B2 — Tier B codegen

Per-builder instruction/plan assertions.

- [ ] Start with the data-shaped `validate.rs`, `plan.rs`, `runtime.rs`, `nir.rs`.
- [ ] Then the `builder_*` family, then `code/mod.rs`.
- [ ] Coordinate with `plan-11-large-files.md`: split a file *then* cover it when the split exposes seams.

Acceptance: the Tier-B codegen files ≥95% via emitted-artifact assertions in both reports; suite green.
Commit: —
