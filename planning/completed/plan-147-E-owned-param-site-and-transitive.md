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

- [x] `self_update.rs`: `Site::OwnedParam`, `ENABLED_SITES`, `lowers_in`
      (`name.starts_with("handOver$own")` — it must lower in the VARIANT, never in the
      base) and `Probe::source`. S12 also inherits S11's `RETURN_NEVER` exclusions,
      since it IS S11 inside an owned variant. The helper is `handOver`, not `step`;
      see Corrections.
- [x] `tests/runtime/rt_inplace_self_update.rs`: a dedicated `owned_param_program`
      rather than an `at_site` arm, since the site needs its own `handOver` function
      beside `main`. It keeps **both** checks, not the result check alone — see
      Corrections. Being an ordinary loop in one frame, it needs no idle twin and
      takes the plain N/8 bound.
- [x] Any arm that does not fire: **none**. §3's "Design uncertainty" — whether every
      arm fires at `OwnedParam` without an arm-specific change — resolves YES on the
      first run: the arms see an ordinary `Direct` destination on the variant's
      parameter slot, exactly as they see a local's, and no arm needed touching.

Acceptance: every `Arm` row fires at `OwnedParam`, and every `arm` line is flat there.
  Check 1: `cargo test --bin mfb every_arm_row_fires_at_every_enabled_site` →
  **`ok. 1 passed; 0 failed`** (136.62 s, 2026-09-23), with `Site::OwnedParam` in
  `ENABLED_SITES` and zero `at OwnedParam` failures.
  Check 2: `MFB_SELF_UPDATE_SITES=OwnedParam cargo test --test rt_inplace_self_update`
  → **`ok. 3 passed; 0 failed` (66.67 s)** over 52 case/site pairs. (`SITES` selects the site exactly;
  the plan's `FILTER` matches label substrings.)
Commit: 95d0fe733

### Phase 2 — Transitive hand-over

- [x] `analysis/handover.rs`: fresh-temporary arguments (`is_fresh_temporary`, a
      `temp_mask` on each site), **plus a third rule the plan did not have** — the
      argument-position self-update (`arg_update_mask`); see Corrections.
      `collect_handover_args` now takes the parameters THIS lowering owns, which is
      what separates a base lowering from a variant, and `variant_demand` iterates to
      a fixpoint over `(function, mask)` because a variant approves sites the base
      cannot. New unit rows: `a_fresh_temporary_argument_is_handed_over` (approved,
      and refused to a read-only parameter) and
      `an_owned_parameter_is_handed_on_only_in_a_variant`.
- [x] Codegen: `emit_raw_call_handing_over` claims each handed-over temp off the
      pending list before the branch, and `lower_call_argument` runs the self-update
      seam at an approved argument position (nulling that local's slot instead, since
      the in-place build produces no temporary).
- [x] `rt_owned_argument.rs`: `recursive-fill` un-ignored and flat — **alloc_calls
      1203 → 12 at N = 600**, slope 1200 → 1. Added
      `a_handed_over_temporary_is_freed_once_on_the_failure_route`, the failing-callee
      case with a temporary argument: `double_free_skips == 0`, `live_bytes == 0` at
      exit, exit 0, every call really failed, and `peak_live_bytes` flat in `N`. The
      guard test's ignored set is now **empty**.

Acceptance: `recursive-fill` passes, and so does the failing-temp case.
  Check: `cargo test --bin mfb handover` → **`ok. 6 passed; 0 failed`**;
  `cargo test --test rt_owned_argument` → **`ok. 8 passed; 0 failed; 0 ignored`**
  (2026-09-23), and the failing-temp case passes on top of those.
Commit: d689d8506

### Phase 3 — `MUT y = p`

- [x] `builder_values.rs` `lower_value_owned`: an owned parameter at its last use is
      moved into the new binding rather than copied — `owned_param_bind_moves` asks
      the analysis, and `release_owned_param_to_binding` retires the parameter's
      `OwnedValue` cleanup and nulls its slot, the same transfer
      `plan_returned_move` makes. P3 counts `MUT y = p` as a consuming use, with a
      unit row (`bindsIt` → `{xs}`). The last-use question is answered by a new
      `param_binds` set on `HandOverArgs`, for the same reason the argument rules
      needed the owned-parameter set: `collect_last_use_moves` excludes every
      parameter, so `store_is_last_use` could not answer it.
- [x] `rt_owned_argument.rs`: `bind-param` added and flat in N — `MUT acc = items`
      then an append, called as `xs = extend(xs, i)`.
- [x] Re-run the semantics fixture — **0 diffs**, unchanged.

Acceptance: `bind-param` passes, and the fixture has 0 diffs.
  Check 1: `cargo test --test rt_owned_argument bind_param` → **passed**; the whole
  file → **`ok. 10 passed; 0 failed; 0 ignored`** (2026-09-23).
  Check 2: `bash scripts/test-accept.sh target/release/mfb /tmp/owned-accept 'owned-argument-semantics*'`
  → **`acceptance tests passed (1 test(s) ran)`**, 0 diffs.
  `cargo test --bin mfb handover` → **`ok. 6 passed; 0 failed`**.
Commit: 1c5cf4e7b

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

- **Runtime proof (this letter's Validation Plan).** `/tmp/owned` re-timed on an
  otherwise idle machine, 2026-09-23, against plan-147-A §2.2's baseline:

  | Shape | Baseline | After letter D | After letter E |
  |---|---|---|---|
  | recursive `fill` (N = 5,000) | 1,252 ms | 145,247 µs | **303 µs** |

  That is the transitive hand-over and the argument-position self-update together:
  about **4,100×** faster than the baseline, and **480×** faster than letter D left it.
  The allocation slope is 1200 → 1 at N = 600.

  The other rows are unchanged by this letter. `concat.helper` is worth one note:
  it times between 124 ms and 235 ms across runs, because it is still the O(n²)
  copying path (letter D's `Expect::StillCopies`) and its cost swings with allocator
  state. Its ALLOCATION count is deterministic and unchanged, which is what
  `helper-concat` actually asserts — the timing variance is not a regression.

- **The owned-parameter bind needs its own last-use answer.** §3 point 4 says to
  mirror `plan_returned_move`, which it does — but it cannot ask
  `store_is_last_use`, because that reads `collect_last_use_moves`, which excludes
  every parameter (`excluded_roots`) for the same reason letter C's H1 did. So the
  analysis records the qualifying binds itself, as a `param_binds` set of `op_key`s
  on `HandOverArgs`, asking the same H1-H3 questions of a bind that it asks of a call
  argument. Same root cause as the owned-parameter correction above, one shape over.

- **Handing the temporary over is not enough: the temporary has to be BUILT in
  place.** §3 point 3 says of `fill`: "`RETURN xs` makes `xs` consumable directly, and
  `fill(append(xs, n), …)` hands over the temporary under point 2" — implying points 2
  and 3 are sufficient. They are not. With both implemented, `recursive-fill`
  measured **1203 → 2403** at N = 600/1200, a slope of 1200: unchanged. The temporary was
  indeed handed over, but **building** it still copied `xs`, because
  `collections::append(xs, n)` at an argument position is neither S1 nor S11.

  So this letter adds a third rule, `arg_update_mask`: when an argument is
  `OP(x, …)` and `x` is an owned local at its last use, the argument is built by
  updating `x`'s own block in place, and that block is what is handed over — the same
  reduction letter B made for `RETURN OP(x, …)`, one position over. If no arm fires,
  nothing is emitted and the argument falls back to an ordinary fresh temporary, so
  the callee owns the block either way. With it: **1203 → 12**, slope 1200 → **1**.

- **`collect_handover_args` has to know which parameters the lowering owns.**
  §3 point 3 notes this in passing ("already approved by C once D stops excluding owned
  parameters from roots"), but nothing in D actually made the ANALYSIS aware of it:
  `excluded_roots` excludes every parameter, because in a base lowering the caller
  owns the block, and letter C's H1 consults it. So inside `fill$own1` the analysis
  still refused to touch `xs`. `collect_handover_args` now takes the owned-parameter
  set and lifts the exclusion for exactly those names — which is also what makes the
  base/variant distinction testable, and why `variant_demand` must now iterate to a
  fixpoint rather than sweep the base lowerings once.

- **A scratch arm costs one allocation per CALL at S12, and that is the site's
  shape.** The first run failed 37 of 52 pairs, every one a scratch arm (27 `math::`
  array rows, 10 `collections::` rows — exactly the compiler's `SCRATCH_ARMS`), all at
  a slope of one block per call. The cause is structural: at S12 the statement runs
  inside the callee's owned variant, a FRESH FRAME per call, so each call allocates
  its own self-update scratch. Every other site runs its `N` statements in one frame
  and allocates the scratch once.

  It is the scratch and not a copy, measured on `collections::difference(x, ys)`:

  | | N = 2000 | 2N = 4000 | Slope |
  |---|---|---|---|
  | handed over, arm fires | 2006 | 4006 | 2000 — **one** block per call |
  | lent, copying (`MUT y = difference(x, ys)` / `RETURN y`) | 4006 | 8006 | 4000 — **two** per call |

  So the hand-over halves it, and the bound at S12 for these lines is `N + N/8` — one
  block per call for the scratch, plus the usual slack — which still separates the arm
  (1/call) from the copy (2/call). The harness's `SCRATCH_ARMS` duplicates the
  compiler's, and `scratch_arms_match_the_compiler` parses the compiler's source and
  fails if the two drift. (S11 has the same per-frame cost from its recursion and
  removes it with an idle twin instead; at S12 no twin can isolate it, because a twin
  that does not hand over also does not allocate the scratch.)

- **The probe helper cannot be called `step`.** §3 point 1 writes the S12 probe as
  `FUNC step(x AS T, …) AS T`. `STEP` is a keyword (`FOR i = 1 TO 10 STEP 2`) and
  MFB keywords are case-insensitive, so every probe failed to parse with
  `main.mfb:4 error[1-102-0003 MFB_PARSE_INVALID_IDENTIFIER]: Function name must be an
  identifier` (`src/lexer.rs:1223`). Renamed to `handOver` in both the matrix probe and
  the runtime harness, and `lowers_in` matches `handOver$own` accordingly.

- **S12's harness program keeps the `before` check, which §3 point 1 said to drop.**
  The plan reasoned that `LET before = x` would make `x` live after the op and so make
  plan-147-C refuse the hand-over. It does not: `before` is bound ONCE before the
  loop, and what H3 asks about is whether `x` is live after the *self-update op* —
  which the op's own store target kills. The hand-over is approved and the bound is
  met with the check in place, so the site keeps the stronger pair of checks (the
  value-semantics `before` comparison AND the chained-`LET` result comparison) rather
  than the result comparison alone.

- **S12 gets its own program builder, not an `at_site` arm.** `at_site` rewrites the
  body of `main`; S12 needs a second top-level function (`handOver`) beside it, which
  `frame` cannot express. `owned_param_program` mirrors `return_program`'s shape:
  auxiliary setup `LET`s become the helper's parameters, built once in `main` and lent
  down, and a line's head statements stay in the loop as ordinary S1 assignments so a
  balanced pair stays balanced.

## Summary

E turns D's mechanism into a guarded property, the same way plan-142 did for its
sites. The only new risk is claiming a temporary on the error path, and one
failing-callee case pins it.
