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

- For every `Arm` row of `SELF_UPDATE_TABLE` **not listed in `RETURN_NEVER`**, the
  probe written at site `Return` — `RETURN OP(x, …)` with `x` an owned local —
  lowers in place; and every row that IS listed does not. The matrix test
  `every_arm_row_fires_at_every_enabled_site` passes with `Site::Return` in
  `ENABLED_SITES`, asserting both directions. (Corrected from "for every `Arm` row";
  see Corrections for the measurement behind the four `String` exclusions.)
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
| Arm rows the new site must fire for | **66** (measured 2026-09-22, plan-145 and plan-146 both landed) | `rg -c 'Arm\(&\[' src/codegen/collection/assign/self_update.rs` → `66` |
| `cases.tsv` `arm` lines the harness adds at the new site | **79** (measured 2026-09-22) | `rg -c '\tarm\t' tests/runtime/inplace_self_update/cases.tsv` → `79` |

The two rows set the harness runtime: plan-142-I measured 679.52 s for 254 pairs,
i.e. ~2.68 s per (line, site) pair. Phase 2's Check 2 runs the 79 `arm` lines at
the one new site, so ≈ 79 × 2.68 s ≈ **3.5 min** — under the 10-minute bar, so it
is run as written. They do not change this letter's scope, since every arm goes
through the same dispatch.

### Verified properties

- **A failed arm leaves `x` intact**, so a `TRAP` handler that reads `x` after a
  failing `RETURN OP(x, …)` sees the old value. This is plan-142's failure-atomicity
  rule: every error is raised before the first write
  (`tests/runtime/rt_inplace_failure_atomic.rs`). So S11 needs no `trap_live` check.
  This is the §14.2 row (S3) of plan-147-A §2.3, discharged by an existing
  guarantee.

## 3. Design

1. **Dispatch.** In `emit_return_exit`, before the `plan_returned_move` call
   (corrected from `lower_returned_value`; see Corrections):
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

- [x] Fill the two UNMEASURED rows in §2 with their commands' output. `rg -c 'Arm\(&\['
      src/codegen/collection/assign/self_update.rs` → **66** arm rows;
      `rg -c '\tarm\t' tests/runtime/inplace_self_update/cases.tsv` → **79** arm
      lines (≈3.5 min for Phase 2's Check 2 at plan-142-I's 2.68 s/pair).
- [x] `src/codegen/engine/control/builder_exits.rs`: add `returned_move_admits(name)`,
      the read-only twin of `plan_returned_move`'s gates, and the S11 dispatch —
      placed in **`emit_return_exit`**, not `lower_returned_value`; see Corrections.
      `returned_move_admits` mirrors all six of `plan_returned_move`'s gates
      (static-`String` fold, capacity shadow, `by_ref`, `FOR EACH` iterable,
      address-taken, and the `OwnedValue` ownership gate) without removing the
      cleanup. `try_returned_self_update` builds the
      `SelfUpdateSite{ dest: InPlaceDest::Direct{slot}, by_ref: false, field: None }`
      and calls `try_inplace_self_update`.
- [x] `src/codegen/collection/assign/self_update.rs`: extend `ops_hold_self_update` to
      `Return` (§3 step 3), plus the shape detector `returned_self_update_local(value)`
      the dispatch and the scratch arm share.
- [x] Un-ignore `local-return` in `tests/runtime/rt_owned_argument.rs` and update its
      guard test's expected ignored set: the name moves from `want` to the new
      `LANDED` list, and the guard now also fails if a `LANDED` entry names a case
      `cases()` no longer declares.

Acceptance: `local-return` passes. `append` at `RETURN` no longer allocates per level.
  Check: `cargo test --test rt_owned_argument local_return` → **`test local_return ...
  ok`** (2026-09-22). And `cargo test --test rt_owned_argument` → `ok. 2 passed;
  0 failed; 4 ignored`.

  Measured directly on the `chain` shape (`mfb build --debug`, summed
  `arena.<k>.alloc_calls`) — `append` at `RETURN` no longer allocates per level:

  | | N = 600 | 2N = 1200 | Slope | Bound N/8 |
  |---|---|---|---|---|
  | Before (plan-147-A Phase 2) | 1203 | 2403 | 1200 | 75 |
  | After (S11) | **12** | **13** | **1** | 75 |
Commit: 75bf5c4c1

### Phase 2 — The site in the guard

- [x] `self_update.rs`: `Site::Return`, `ENABLED_SITES`, `lowers_in` (`chain`), and
      `Probe::source` (§3 step 4), plus the `RETURN_NEVER` exclusion list and its
      guard test `return_never_names_dispatched_arms_once`.
- [x] `tests/runtime/rt_inplace_self_update.rs`: `Return` builds its whole program in
      a new `return_program` rather than going through `frame`/`at_site` (the
      recursion IS the repetition, so there is no loop body to place); `at_site` and
      `exempt_program` get `unreachable!` arms saying so. Auxiliary setup operands
      become `chain` PARAMETERS, and `Return` joins `Lambda` in the idle-twin
      subtraction (§3 steps 5–6, as corrected).
- [x] Any arm that does not fire at `Return`: the matrix reported 51 failures, all at
      `Return` and all `String` arms. Root-caused and recorded in Corrections: the
      shape detector was widened to cover the 15 `math` array arms (a real gap), and
      the four `String` arms are pinned as `RETURN_NEVER` on the measurement that S11
      is a no-op for `String` (a block must be tight to leave its frame).

Acceptance: every `Arm` row not in `RETURN_NEVER` fires at `Return`, every row that
is does not, and every `arm` line costs no more at `Return` than as an assignment.
  Check 1: `cargo test --bin mfb every_arm_row_fires_at_every_enabled_site` →
  **`ok. 1 passed; 0 failed`** (129.12 s, 2026-09-22), with `Site::Return` in
  `ENABLED_SITES` and asserting both directions.
  Check 1b: `cargo test --bin mfb return_never_names_dispatched_arms_once` →
  **`ok. 1 passed`**.
  Check 2: `MFB_SELF_UPDATE_SITES=Return cargo test --test rt_inplace_self_update` →
  **`ok. 2 passed; 0 failed`** over **52 case/site pairs** (82.70 s) — well under the
  10-minute bar, so it was run as written. (`MFB_SELF_UPDATE_SITES=Return` is used in
  place of the plan's `MFB_SELF_UPDATE_FILTER=Return`: `SITES` selects the site
  exactly, while `FILTER` matches label substrings.)
Commit: 0446fd1bf

### Phase 3 — Semantics and blast radius

- [x] Re-run plan-147-A's semantics fixture — still GREEN with S11 landed.
- [x] Byte-identity: **0 goldens moved**, and that is the correct result, not a
      vacuous one. S11 changes codegen only in a function that `RETURN OP(local, …)`,
      and **no byte-identity fixture has that shape**:
      `rg -l -P 'RETURN\s+\w+::\w+\(' tests/byte-identity/` lists exactly one file,
      `tests/byte-identity/http/src/main.mfb:15`
      (`RETURN http::respondPath(req, "target/http-root")`) — whose target is not a
      self-update builtin and whose first argument is a **parameter**, not an owned
      local, so S11 cannot fire. There is therefore no moved `.ncodesum` to name, and
      the gate is doing its other job: proving S11 did not change codegen anywhere it
      should not have. Coverage of the changed code rests on the runtime harness
      (Phase 2 Check 2, 52 pairs) and `rt_owned_argument local_return`, exactly as
      this letter's Validation Plan allows.

Acceptance: the semantics fixture has 0 diffs. Every moved golden has a `RETURN OP(local)`.
  Check 1: `bash scripts/test-accept.sh target/release/mfb /tmp/owned-accept 'owned-argument-semantics*'`
  → **`acceptance tests passed (1 test(s) ran)`**, 0 diffs.
  Check 2: `bash scripts/artifact-gate.sh target/release/mfb collections` →
  **`1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)`**. No golden moved, and no
  fixture carries the S11 shape (see the task above), so the acceptance holds
  vacuously-but-correctly: there was nothing that should have moved.
Commit: 0446fd1bf

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

- **The S11 shape detector missed every `math` array arm.** `returned_self_update_local`
  originally gated on `self_update_builtin(target)`, which names the collection and
  `String` spellings but **not** `math.*`. The 15 `math` array rows
  (`math::abs/clamp/log/exp/cos/atan/…` on a `List OF Integer|Float|Fixed`) are arms
  too — they redirect the member's own result list into the function's self-update
  scratch (`MATH_SELF_UPDATE`, `simd_result_into`) — so S11 silently skipped all of
  them. The detector now also consults
  `builder_inplace_rewrite::math_self_update_function`. Found by the harness at the
  new site, not by inspection.

- **S11's runtime probe needed an idle twin, and the twin is the same statement
  dispatched as an ASSIGNMENT.** S11's repetition is recursion, so each level is its
  own frame — and several per-frame costs scale with `N` for reasons that have
  nothing to do with the site:

  1. **A scratch arm allocates its scratch once per frame.** Every other site runs
     its `N` statements inside one frame, so the scratch is allocated once. Proved to
     be the scratch rather than a copy by doubling the op per level — a shared
     per-frame scratch stays at one block per level, a copy becomes two:

     | `math::abs`, per level | N = 2000 | 2N = 4000 | Slope |
     |---|---|---|---|
     | one op, arm | 2003 | 4003 | 1 / level |
     | **two** ops, arm (shares the frame's scratch) | 2003 | 4003 | **1 / level** |
     | **two** ops, copying (`LET u = …` / `LET w = …`) | 4003 | 8003 | **2 / level** |

     Same for `collections::difference`: one block per level with the arm, two
     copying. The arm fires; the block per level is the recursion.
  2. **Some head statements cost a block per level of their own.** Measured in the
     same frame: `x = collections::append(x, 9)` and `x = collections::removeAt(x, 0)`
     are flat (13 and 3 allocations for any `N`), but `x = collections::replace(x, 1, 5)`
     costs one per level — and one, two or four `replace` statements per level all
     cost the same one, so it is a per-frame cost, not a per-statement copy. (The
     `Replace` arm takes no scratch — `builder_inplace_rewrite.rs:239` — so the
     mechanism is not identified here; the measurement is what the twin needs.)

  Two attempts to dodge this failed, and both failures are informative:
  - Running only the last statement per level. A line's statements are often a
    **balanced pair** — `x = append(x, 9)` then `x = removeAt(x, 0)`, or
    `x = log10(x)` then `x = clamp(x, 1.5, 2.5)` — that holds `x` steady. Dropping
    the second drained the list (`7-705-0001` on `removeAt`, `drop`, `mid`).
  - Guarding the last statement off in the twin only. Same imbalance, mirrored: the
    twin walked out of `log`'s domain (`7-705-0012`) on 7 `math` rows.

  So the twin runs **every** statement the live program runs, and differs only in
  WHERE the last one is dispatched: `RETURN OP(x, …)` (S11) live,
  `x = OP(x, …)` / `RETURN x` (S1) idle. Subtracting it leaves exactly the question
  this site asks — *does dispatching a self-update at a `RETURN` cost more than
  dispatching it as an assignment?* — and the plain N/8 bound applies to that
  difference. It needs no per-line exception list, and it absorbs every per-frame
  artifact above uniformly.

  That this is not a weakening rests on the two checks it composes with: the
  **matrix** proves the arm fires at `Return` (the marker stack slot appears in
  `chain`), and the **other four sites** prove the same line is flat in `N`. The
  three together say the arm runs at a `RETURN` and costs nothing extra there.

- **The `String` arms do not fire at S11, and that is now PINNED, not skipped.**
  §1's goal says "for every `Arm` row … the probe written at site `Return` lowers in
  place". With `Site::Return` added, the matrix reported **51 failures, every one of
  them at `Return` and every one a `String` arm** — 13 `StrWindow`, 6 `StrRewrite`,
  6 `StrGrow`, 1 `Concat`. No collection arm failed at any site. Root-caused rather
  than assumed:

  1. The String arms' destination gate is
     `string_shadow_exists` → `InPlaceDest::Direct { .. } => self.string_capacity_slots.contains_key(site.name)`
     (`string_self_update.rs:520`). The shadow pre-pass `prescan_string_self_appends`
     (`builder_control.rs:3034`) walks only `NirOp::Assign`, so a `RETURN`-site local
     never gets a shadow and every String arm declines.
  2. Giving it one makes the arms fire — but **buys nothing**. A `String` block that
     leaves its frame must be **tight**: `arena_free(ptr, size)` is caller-sized and
     bins by size class (`arena.rs:lower_arena_free`), and a caller frees a returned
     `String` by `byteLength` alone, so a block carrying spare cannot be moved out.
     That is bug-560, which `plan_returned_move` already encodes as a decline. Every
     in-place `String` update leaves spare by construction (a window shrinks
     `byteLength` inside the old allocation; a grow takes geometric headroom), so the
     `RETURN` must copy tight — costing exactly the one allocation the copying
     builtin would have made.
  3. **Measured, not argued.** The `chain` shape over `strings::left(x, 150)` under
     `mfb build --debug`, summed `arena.<k>.alloc_calls`:

     | | N = 600 | 2N = 1200 | Slope |
     |---|---|---|---|
     | arm fires (shadow pre-allocated at the `RETURN`) | 603 | 1203 | 600 |
     | arm declines (HEAD) | 603 | 1203 | 600 |

     Identical. S11 is a measured no-op for `String`. (The first run of this
     experiment looked like it proved the opposite — it appeared to move a shadowed
     block safely. It had not: `plan_returned_move` has its own
     `string_capacity_slots` decline, which I had not disabled, so the arm fired and
     the return still copied tight. The numbers above are from the corrected
     experiment, where both gates were controlled.)

  So the shadow pre-pass is **not** extended — it would cost a stack slot per
  `RETURN`-site String local and change no allocation. Instead the exclusion is
  **strengthened into an assertion**: a new `RETURN_NEVER` list
  (`self_update.rs`, beside `FIELD_NEVER`) names the four arms with this reason, the
  matrix asserts they do **not** fire at `Return`, and a new guard test
  `return_never_names_dispatched_arms_once` keeps the list tied to
  `SELF_UPDATE_ARMS`. That is stricter than the criterion it replaces: the original
  only required the positives, this pins the negatives too, so a String arm that ever
  starts firing at a `RETURN` fails the matrix and forces a re-measurement.

  §1's first bullet is corrected to: *every `Arm` row except those in `RETURN_NEVER`
  fires at `Return`, and every row in `RETURN_NEVER` does not.*

- **Two byte-identity fixtures DO carry the S11 shape; Phase 3's `collections`-scoped
  gate could not see them.** Phase 3 recorded "0 goldens moved, and no byte-identity
  fixture has that shape", supported by an `rg` over `tests/byte-identity/*/src`. That
  is true of the fixtures' own sources, but not of the **builtin bodies they pull in**.
  Letter D's full `artifact-gate.sh all` sweep moved `byte-identity/compress` and
  `byte-identity/vector`, neither of which has an approved hand-over site, and
  bisecting the changes attributed both to this letter:

  | Change disabled | `compress` | `vector` |
  |---|---|---|
  | letter D's caller hand-over | still differs | still differs |
  | — and **S11's dispatch** | **matches the golden** | still differs |
  | — and **`ops_hold_self_update`'s `Return` arm** | matches | **matches** |

  So `compress` moved because a `RETURN OP(local, …)` in a builtin body now lowers in
  place, and `vector` moved because a function whose only self-update is at a `RETURN`
  now reserves a self-update **scratch slot**, which changes its frame layout even
  where the emitted call is otherwise unchanged. Both are this letter working as
  designed; neither is a defect. Phase 3's conclusion is corrected accordingly — the
  0-diff result it recorded was right for `collections` and wrong as a statement about
  the whole corpus.

- **The S11 dispatch lives in `emit_return_exit`, not in `lower_returned_value`.**
  §3 step 1 places it in `lower_returned_value`, but the cleanup bookkeeping the
  design depends on is one level up. `emit_return_exit` is where
  `plan_returned_move` is called and — critically — where its removal is **undone**
  afterwards (`restore_cleanups`, and the `return_snapshot` taken only for
  `Some(NirValue::Local(_))`). That save/restore exists because every cleanup
  removal a `RETURN` makes is PATH-LOCAL: a sibling `RETURN b` after
  `IF give THEN RETURN a END IF`, the rest of a loop body, a later `TRAP` route —
  none of those returned the local, so each must still free it. Removing `x`'s
  cleanup from inside `lower_returned_value` would have been **permanent** for a
  `Call` value, because `return_snapshot` is `None` for anything that is not a
  `Local` — exactly the leak the `emit_return_exit` comment documents (one
  descriptor and one record per call, `udp::bind` under a 128-fd limit).

  So the dispatch sits at the top of `emit_return_exit` and, when an arm fires,
  **re-enters `emit_return_exit` with `RETURN x`**. That gets the snapshot,
  `plan_returned_move`, and the restore for free, and it makes the reduction the
  design asked for — "S11 is S1 followed by the existing move" — literal rather
  than re-implemented. The re-entry cannot recurse: its value is a `Local`, which
  `returned_self_update_local` never matches. §3 step 1 is corrected to name
  `emit_return_exit`.

## Summary

The risk is small because S11 is defined as "S1, then the existing move". The
cost is harness time, not design.
