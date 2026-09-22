# plan-147-B: `RETURN OP(x, …)` in place at an owned local's last use (site S11)

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-147-A

`RETURN collections::append(out, x)` with `out` an owned local copies `out` today.
`RETURN` is not a self-update site (plan-147-A §2.1), so the copying `append` runs,
and then `out`'s own block is dropped at exit. This letter makes a `RETURN` whose
value is a self-update of an owned local a fifth site, **S11**. The arm updates
`x`'s block in place, and the block is then moved out as the return value, exactly as
`RETURN x` already moves (`builder_exits.rs:plan_returned_move`). It needs no calling-convention
change, and it is useful on its own: 14 such returns in examples, 5 in packages and
2 in tests (plan-147-A §2.2). Letter D reuses it: an owned parameter is just
another owned local.

References: plan-147-A (whole-plan goal, prerequisites, §2.3 semantics audit — rows
S3, S4, S5 are this letter's); `.ai/collections.md` §"In-place mutation: one table,
four sites"; `planning/completed/plan-142-H-globals.md` (the last site added — mirror
its file list).

## Prerequisites

See plan-147-A. plan-147-A must be complete: `ls planning/plan-147-A-* 2>/dev/null` →
no matches (it has moved to `planning/completed/`).

## 1. Goal

- For every `Arm` row of `SELF_UPDATE_TABLE`, the probe written at site `Return` —
  `RETURN OP(x, …)` with `x` an owned local — lowers in place. The matrix test
  `every_arm_row_fires_at_every_enabled_site` passes with `Site::Return` in
  `ENABLED_SITES`.
- `tests/runtime/rt_inplace_self_update.rs` runs every `arm` line of `cases.tsv` at
  site `Return` and every line stays within the N/8 bound.
- plan-147-A's `local-return` case in `tests/runtime/rt_owned_argument.rs` is
  un-ignored and passes.
- plan-147-A's semantics fixture stays green.

### Non-goals

- Parameters. A parameter has no `OwnedValue` cleanup, so this site never fires
  for one. That is letter D.
- `RETURN OP(g, …)` with `g` a global: `Place` covers locals only, and a global
  outlives the return.
- Every plan-147-A non-goal.

## 2. Current State

- `NirOp::Return` (`builder_control.rs:1649`) → `emit_return_exit` →
  `emit_return_exit_inner` → `lower_returned_value` (`builder_exits.rs:228`). That
  path does a plain `lower_value` of the call, then copies, claims the pending temp,
  or elides a move.
- `plan_returned_move` (`builder_exits.rs:418`) moves `RETURN x` for an owned local.
  It removes `x`'s `OwnedValue` cleanup and declines for:
  - a `by_ref` local;
  - a parameter (no cleanup);
  - a live `FOR EACH` iterable;
  - an address-taken local;
  - a `String` with a capacity shadow.
- `try_inplace_self_update(site, value)` (`self_update.rs:234`) runs the arms against
  a `SelfUpdateSite{name, type_, dest, by_ref}`. `InPlaceDest::Direct{slot}` is the
  local destination.
- Scratch: `ops_hold_self_update` (`self_update.rs:372-403`) decides whether a
  function reserves self-update scratch. It matches only `Assign` and `StoreGlobal`.
  An arm that needs scratch at a `Return` would find none.
- Matrix: `Site` enum (`self_update.rs:1287`), `ENABLED_SITES` (`:1303`),
  `Site::lowers_in` (`:1308`), `Probe::source(site)` (`:1321-1367`).
- Runtime harness: `tests/runtime/rt_inplace_self_update.rs` `frame`/`at_site`,
  N/2N `alloc_calls` bound, `LET before = x` check, chained-`LET` result
  check, and `MFB_SELF_UPDATE_FILTER` for narrowing.

### Measured populations

| What | Count | Command |
|---|---|---|
| `RETURN collections::<mutating op>(local, …)` sources | examples 14, packages 5, tests 2 | plan-147-A §2.2, row 2 |
| Arm rows the new site must fire for | UNMEASURED until plan-145/146 land: they add rows | Phase 1 task 1: `cargo test --bin mfb self_update -- --nocapture 2>&1 \| rg -c 'Arm'`, or count `Arm(` in `SELF_UPDATE_TABLE`: `rg -c 'Arm\(&\[' src/codegen/collection/assign/self_update.rs` |
| `cases.tsv` `arm` lines the harness adds at the new site | UNMEASURED until plan-145/146 land | `rg -c '\tarm\t' tests/runtime/inplace_self_update/cases.tsv` |

The two UNMEASURED rows set the harness runtime: plan-142-I measured 679.52 s for
254 pairs. They do not change this letter's scope, since every arm goes through
the same dispatch.

### Verified properties

- **A failed arm leaves `x` intact**, so a `TRAP` handler that reads `x` after a
  failing `RETURN OP(x, …)` sees the old value. This is plan-142's failure-atomicity
  rule: every error is raised before the first write
  (`tests/runtime/rt_inplace_failure_atomic.rs`). So S11 needs no `trap_live` check.
  This is the §14.2 row (S3) of plan-147-A §2.3, discharged by an existing
  guarantee.

## 3. Design

1. **Dispatch.** In `lower_returned_value`, before the plain `lower_value`:
   - when `value` is `is_self_update_call` on `Local(x)` and `plan_returned_move`
     would accept `x` (checked with a read-only twin, `returned_move_admits(x)`,
     that does not remove the cleanup);
   - build `SelfUpdateSite{ name: x, type_, dest: InPlaceDest::Direct{slot}, by_ref: false }`
     and call `try_inplace_self_update`;
   - if an arm fires, lower the return as `RETURN x`: call `plan_returned_move(x)`,
     which now succeeds, and continue down the existing move path;
   - if no arm fires, emit nothing and fall through to today's path.

   `plan_returned_move`'s gates are exactly the ones S11 needs. A local it may move
   is one nothing else reads after the return.
2. **Operands that read `x`.** `site.read_by` already makes arms decline
   `append(x, x)`-style self-aliases. They behave the same here.
3. **Scratch.** Extend `ops_hold_self_update` to also match
   `Return { value }` when the value is a self-update-shaped call on a local.
4. **Matrix site.** Add `Site::Return` to `Site` and `ENABLED_SITES`.
   `Probe::source(Site::Return)` writes a recursive chain:

   ```
   FUNC chain(k AS Integer) AS T
     IF k = 0 THEN RETURN <setup value>
     MUT x AS T = chain(k - 1)
     RETURN OP(x, …)
   END FUNC
   ```

   `lowers_in` is `chain`. The marker slot must appear in `chain`.
5. **Runtime harness.** Add `Return` to `at_site`, using the same `chain` shape
   driven to depth N and 2N. A copying lowering allocates at least once per level,
   so it fails the N/8 bound; the in-place one allocates only the arm's amortized
   growth. `String` rows use the same shape.
6. **The `before` check** at `Return` compares `chain(k)`'s result with a chained-`LET`
   program. There is no caller `x` to compare, since the local is gone at return.

**Correctness risk:** concentrated in step 1's interaction with pending temps and
cleanups on the error path. If an arm fires and a later operand fails, the arm
itself has raised first (failure atomicity). So the only new state is "arm fired,
then the move". That ordering is identical to `x = OP(x, …); RETURN x`, which is
S1 followed by a move and is already correct. The design deliberately reduces S11 to
that pair.

**Rejected:** desugaring `RETURN OP(x, …)` into `x = OP(x, …); RETURN x` in NIR.
That is simpler, but it changes NIR for every such function, which churns
`.nir` goldens. It also requires `x` to be `MUT`, and a `LET x` can be updated
in place just as legally here (plan-147-A §2.3 S5).

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work; `- [~]` for partial with one line on what remains; moot tasks are
> struck through with evidence, never deleted; fill `Commit:` when a phase lands.
> **An unticked box means NOT DONE.**

### Phase 1 — Measure, then the dispatch

- [ ] Fill the two UNMEASURED rows in §2 with their commands' output.
- [ ] `src/codegen/engine/control/builder_exits.rs`: add `returned_move_admits(name)`,
      the read-only twin of `plan_returned_move`'s gates, and the S11 dispatch in
      `lower_returned_value` (§3 step 1).
- [ ] `src/codegen/collection/assign/self_update.rs`: extend `ops_hold_self_update` to
      `Return` (§3 step 3).
- [ ] Un-ignore `local-return` in `tests/runtime/rt_owned_argument.rs` and update its
      guard test's expected ignored set.

Acceptance: `local-return` passes. `append` at `RETURN` no longer allocates per level.
  Check: `cargo test --test rt_owned_argument local_return` → 1 passed (est. 2 min).
Commit: —

### Phase 2 — The site in the guard

- [ ] `self_update.rs`: `Site::Return`, `ENABLED_SITES`, `lowers_in`, and
      `Probe::source` (§3 step 4).
- [ ] `tests/runtime/rt_inplace_self_update.rs`: `Return` in `frame`/`at_site`
      (§3 steps 5–6).
- [ ] Any arm that does not fire at `Return`: fix the arm's destination handling (it
      should not care which op dispatched it). Record each one in Corrections with the
      reason.

Acceptance: every `Arm` row fires at `Return`, and every `arm` line is flat in N
there.
  Check 1: `cargo test --bin mfb every_arm_row_fires_at_every_enabled_site` → passed
  (est. 3 min).
  Check 2: `MFB_SELF_UPDATE_FILTER=Return cargo test --test rt_inplace_self_update` →
  passed. Est. from plan-142-I's 679.52 s / 254 pairs × the arm-line count measured in
  Phase 1. This one runs >10 min because it is the only check that runs every arm's
  runtime program at the new site; the matrix test compiles but does not run.
Commit: —

### Phase 3 — Semantics and blast radius

- [ ] Re-run plan-147-A's semantics fixture.
- [ ] Byte-identity: S11 changes codegen only in functions that `RETURN OP(local, …)`.
      Run `scripts/artifact-gate.sh target/release/mfb collections` and name every
      moved `.ncodesum` in Corrections, confirming each fixture has such a `RETURN`
      (`rg -n 'RETURN collections::' tests/byte-identity/<pkg>/src`). A diff in a
      fixture with no such `RETURN` is a bug; objdump that one fixture.

Acceptance: the semantics fixture has 0 diffs. Every moved golden has a `RETURN OP(local)`.
  Check 1: `bash scripts/test-accept.sh target/release/mfb /tmp/owned-accept 'owned-argument-semantics*'`
  → 0 diffs (est. 2 min).
  Check 2: `bash scripts/artifact-gate.sh target/release/mfb collections` → diffs only in
  fixtures listed in Corrections (est. 1 min).
Commit: —

## Validation Plan

- Tests: the matrix site, the harness site, `local-return`.
- Coverage check: Phase 3 Check 2 confirms the changed code is reached by a
  byte-identity fixture; if none moves, say so and rely on the harness.
- Runtime proof: `tests/runtime/rt_owned_argument.rs local_return`.
- Doc sync: none here. `.ai/collections.md` and spec §14.6 change once, in F, when
  all sites exist.
- Final gate: plan-147-F.

## Open Decisions

- Should S11 also cover `RETURN WITH r { f := OP(r.f, …) }` (a field of an owned local
  record)? **Recommended: yes, in letter F**, through plan-145's field seam, once the
  parameter case needs it too. Not here.

## Corrections

## Summary

The risk is small because S11 is defined as "S1, then the existing move". The
cost is harness time, not design.
