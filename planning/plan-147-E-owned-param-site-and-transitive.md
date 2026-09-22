# plan-147-E: The owned-parameter site, transitive hand-over, and `MUT y = p`

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-147-D

After letter D, a direct `acc = helper(acc, …)` is in place. This letter does three
things:

1. It makes that permanent in plan-142's guard. **Site S12, `OwnedParam`**, joins
   `ENABLED_SITES`, so every `Arm` row must fire inside an owned variant, and every
   `cases.tsv` `arm` line must stay flat there.
2. It extends hand-over to the shapes D left out:
   - a **fresh temporary** argument, e.g. `fill(collections::append(xs, n), n - 1)`,
     which covers the recursive case;
   - an **owned parameter passed on** to another owned variant at its last use;
   - **`MUT y = p` / `LET y = p`** at an owned parameter's last use, which moves
     instead of copying. That is the `__json_parseArrayItems` shape
     (`src/codegen/builtins/json/helper_parse_array_items.rs:11`).

References: plan-147-A (goal, prerequisites, §2.3); plan-147-B (the site pattern to
mirror); plan-147-C (§3.2 P3, extended here); plan-147-D (variants, hand-over).

## Prerequisites

See plan-147-A. plan-147-D must be complete: `ls planning/plan-147-D-* 2>/dev/null` →
no matches.

## 1. Goal

- `Site::OwnedParam` is in `ENABLED_SITES`, and `every_arm_row_fires_at_every_enabled_site`
  passes.
- `MFB_SELF_UPDATE_FILTER=OwnedParam cargo test --test rt_inplace_self_update` passes.
- plan-147-A's `recursive-fill` case is un-ignored and passes. No case in
  `rt_owned_argument.rs` is ignored any more, and its guard test asserts an empty
  ignored set.
- plan-147-A's semantics fixture stays green.

### Non-goals

- Record parameters and `RETURN WITH r { f := OP(r.f, …) }` (letter F).
- Every plan-147-A non-goal.

## 2. Current State

(After D; re-read the files named here before Phase 1 and correct anything that
drifted.)

- `ENABLED_SITES` holds the four plan-142 sites plus B's `Return`. `Probe::source` and
  `lowers_in` are per site (`self_update.rs`); `at_site` is in
  `tests/runtime/rt_inplace_self_update.rs`.
- `consumable_params` (C §3.2) counts only direct consuming uses: `RETURN OP(p, …)`
  and `RETURN p`.
- `collect_handover_args` (C §3.1) approves only `NirValue::Local` arguments.
- Pending temporaries: a fresh call result or constructor used as an argument is a
  statement-scope pending temp, freed after the call by the statement's temp
  handling. On an error it is freed by `emit_call_error_exit`
  (`builder_exits.rs:143`). `claim_pending_temp` (`builder_values.rs:600`) is how a
  consumer takes one over.
- `MUT y = p` for a parameter `p`: `lower_value_owned` copies, because a `Local`
  source is aliasing (`value_is_aliasing_source`). `store_is_last_use`
  (`builder_values.rs:1393`) moves instead only for recursive-graph types.

## 3. Design

1. **Site S12.** `Probe::source(Site::OwnedParam)` writes:

   ```
   FUNC step(x AS T, …) AS T
     RETURN OP(x, …)
   END FUNC
   ' run1: MUT x AS T = <setup> ; x = step(x, …)
   ```

   `lowers_in` is the variant symbol `step$own1`. In `rt_inplace_self_update.rs`,
   `at_site(OwnedParam)` is the same loop N and 2N times. The `before` check is not
   applicable: `x` is handed over, so `LET before = x` would make `x` live after the
   op, and C refuses hand-over. The harness therefore compares results against the
   chained-`LET` program only. That skip is the site's definition, not a gap: the
   `before` property at this site is exactly C's H3, pinned by C's unit table and the
   semantics fixture.
2. **Fresh temporaries.** Extend C's H-rules. An argument that is a direct user
   call's or builtin's fresh result, or a constructor, of an H2 type, passed to a
   consumable parameter, is approved; a temporary has no other reader. In codegen,
   `claim_pending_temp` it before the branch, so neither the statement's post-call
   free nor `emit_call_error_exit` frees it. The callee owns it on every exit.
3. **Owned parameter passed on.** Already approved by C once D stops excluding owned
   parameters from roots. The new part is that `consumable_params` becomes a least
   fixpoint over the call graph: `p` is consumable if it has a direct consuming use,
   or if its last use is an argument approved for a consumable parameter of the
   callee. Start from the direct uses and iterate to stability; the sets only grow
   and are finite. For `fill`: `RETURN xs` makes `xs` consumable directly, and
   `fill(append(xs, n), …)` hands over the temporary under point 2.
4. **`MUT y = p` / `LET y = p` at an owned parameter's last use.** In
   `lower_value_owned`, when the source is an owned parameter and the store is its
   last use, move instead of copying. Mirror `plan_returned_move`: transfer the
   parameter's `OwnedValue` cleanup to `y`'s slot and null the parameter's slot. Add
   this shape to P3 as a consuming use.

**Correctness risk:** point 2, on the error path. A temporary claimed for hand-over
and then also freed by `emit_call_error_exit` is a double free. The claim must remove
the temp from the pending list **before** the call instruction, exactly as the
return path's claim does. Phase 2's failing-callee case with a temporary argument
pins it.

**Design uncertainty:** whether every arm fires at `OwnedParam` without an
arm-specific change. The arms see an ordinary `Direct` destination, so they should,
but Phase 1 runs the matrix first to find out cheaply.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work; `- [~]` for partial with one line on what remains; moot tasks are
> struck through with evidence, never deleted; fill `Commit:` when a phase lands.
> **An unticked box means NOT DONE.**

### Phase 1 — Site S12 in the guard

- [ ] `self_update.rs`: `Site::OwnedParam`, `ENABLED_SITES`, `lowers_in`, and
      `Probe::source` (§3 point 1).
- [ ] `tests/runtime/rt_inplace_self_update.rs`: `at_site(OwnedParam)`, with the result
      check only (§3 point 1).
- [ ] Any arm that does not fire: find the cause and fix it at the variant's
      parameter registration, not in the arm. Record each in Corrections.

Acceptance: every `Arm` row fires at `OwnedParam`, and every `arm` line is flat there.
  Check 1: `cargo test --bin mfb every_arm_row_fires_at_every_enabled_site` → passed
  (est. 3 min).
  Check 2: `MFB_SELF_UPDATE_FILTER=OwnedParam cargo test --test rt_inplace_self_update`
  → passed. Estimate it from plan-147-B Phase 2's measured time for one site; over 10
  min because this is the only check that runs every arm at the site.
Commit: —

### Phase 2 — Transitive hand-over

- [ ] `analysis/handover.rs`: fresh-temporary arguments (§3 point 2) and the
      `consumable_params` fixpoint (§3 point 3). Add rows to C's unit table: temp
      approved; temp to a non-consumable parameter refused; mutual recursion reaches a
      fixpoint.
- [ ] Codegen: claim the handed-over temp before the branch (§3 point 2).
- [ ] `rt_owned_argument.rs`: un-ignore `recursive-fill`. Add a failing-callee case with
      a temporary argument (2N runs, no double free, flat `peak_live_bytes`), and set
      the guard test's ignored set to empty.

Acceptance: `recursive-fill` passes, and so does the failing-temp case.
  Check: `cargo test --bin mfb handover && cargo test --test rt_owned_argument` → all
  passed (est. 6 min).
Commit: —

### Phase 3 — `MUT y = p`

- [ ] `builder_values.rs` `lower_value_owned`: move an owned parameter at its last use
      (§3 point 4). Add P3's new consuming use and its unit rows.
- [ ] `rt_owned_argument.rs`: add `bind-param`. The helper is
      `MUT acc = items; FOR … acc = collections::append(acc, …) NEXT; RETURN acc`, called
      as `x = helper(x, …)`, and it must be flat in N.
- [ ] Re-run the semantics fixture.

Acceptance: `bind-param` passes, and the fixture has 0 diffs.
  Check 1: `cargo test --test rt_owned_argument bind_param` → passed (est. 2 min).
  Check 2: `bash scripts/test-accept.sh target/release/mfb /tmp/owned-accept 'owned-argument-semantics*'`
  → 0 diffs (est. 2 min).
Commit: —

## Validation Plan

- Tests: the S12 matrix and harness site, the fixpoint unit rows, `recursive-fill`,
  the failing-temp case, `bind-param`.
- Coverage check: `rt_owned_argument.rs`'s guard asserts nothing is ignored.
- Runtime proof: `/tmp/owned`'s recursive `fill` time, recorded in Corrections next
  to plan-147-A §2.2's 1,252 ms.
- Doc sync: letter F.
- Final gate: letter F.

## Open Decisions

- Should `MUT y = x` for an ordinary owned local at its last use also move, not only
  for owned parameters? **Recommended: no, not in plan-147.** It is a general change
  to copy-insertion for every flat type, with its own golden churn across the tree.
  It belongs in its own plan, next to plan-134's graph-type moves.

## Corrections

## Summary

E turns D's mechanism into a guarded property, the same way plan-142 did for its
sites. The only new risk is claiming a temporary on the error path, and one
failing-callee case pins it.
