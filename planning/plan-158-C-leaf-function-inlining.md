# plan-158-C: Inline one-expression scalar helpers at the default level

Last updated: 2026-09-26
Effort: large (3h–1d)
Depends on: plan-158-B

bug-740 mechanism (c). brogue's hot loop (`cellularAutomataRound`) calls four
one-line helpers per neighbour — `gi(x, y) = x * DROWS + y`, `nbDx`, `nbDy`,
`coordinatesAreInMap` — and each call pays a frame, a `bl`, the result protocol
and the caller's error check. This plan adds a Level-1 Opt1 row that replaces a
call to such a helper with the helper's expression when every argument is a
literal or a local, and extends `NirSourceLoc` so an error raised inside the
inlined expression still names the helper's file, line and column. The single
behavioral outcome: at `-O1` those calls are gone, and every program computes,
raises and reports exactly what it did.

References:

- `bugs/bug-740-brogue-dig-leaf-call-and-spill-overhead.md` — mechanism (c).
- `mfb spec language error-model` §8.5a (`ErrorLoc`), §8.6 (TRAP routes),
  rule 11 (inline `TRAP` covers calls and operators).
- `.ai/codegen-invariants.md` — "The inline-TRAP region is built in `ir::lower`"
  (every call in a trapped expression is lifted into a `CallResult`).
- `planning/optimizations.md` — "Function inlining" is a Level-4 row.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-158-B complete | `ls planning/plan-158-B-*` → no match (archived) | NOT MET (2026-09-26, plan-158-B awaiting its heavy-lock gate) |

## 1. Goal

- `mfb build -v examples/brogue` reports "Leaf function inlining" firing, and
  the `.ncode` of a caller no longer `bl`s an inlined helper.
- Every program's output, exit code, runtime errors (code, message, `ErrorLoc`
  file/line/char), `TRAP` routing and `--debug` report are unchanged at every
  level.
- `tools/brogue-perf/bench.sh` ratio lower than plan-158-B's floor.

### Non-goals (explicit constraints)

- **Evaluation order and side effects unchanged.** Only literal and local
  arguments are substituted: they have no effect, cannot fail, and cannot change
  while the helper's expression runs (it calls no user code). A non-atomic
  argument keeps the call.
- **No user call, no function value, no storage** inside an inlined expression:
  the callee's body is `RETURN <expr>` over literals, parameters, globals,
  operators and builtin calls; parameters and result are one-word scalars. No
  copy, ownership, resource or plan-147 `$own` variant is involved (variants
  exist only for owned collection parameters).
- **Error attribution unchanged**: same code, message, file, line and column.
- **Inline `TRAP` untouched**: a `CallResult`, and any call under a
  `CallResult`/`Checked` capture, stays a call.
- No `ISOLATED` callee, no `SUB`. The callee is still emitted for other callers.
- `-O0` codegen unchanged; `.ir`/`.ast` unchanged (the row is on NIR).

## 2. Current State

- There is no inliner (`src/optimizer/opt1/` has none; the catalog lists
  inlining as Level 4, unlanded).
- `NirSourceLoc` carries line/column; `ErrorLoc.filename` comes from the
  function being compiled (`CodeBuilder::current_file`, read in exactly two
  places in `builder_error_emission.rs`).
- The front end enforces exact argument and return types (`TYPE_CALL_ARGUMENT_MISMATCH`,
  `TYPE_RETURN_MISMATCH`, verified by building a probe that passes a `Byte` to an
  `Integer` parameter and returns `1` from a `Float` function: both rejected), so
  substituting an argument never skips a conversion.
- An operator failure in a function body reaches the function-level `TRAP`
  exactly as a call's failure does (probe: `direct(max)` with `x + 1` in the body
  and `viaCall(max)` through a helper both print "trapped", with the line of the
  failing `+`).

### Measured populations

| What | Count | Command |
|---|---|---|
| brogue functions that are scalar one-expression leaves | 21 | NIR scan of `mfb build -nir examples/brogue` (session probe) |
| brogue call sites to them | 226: 123 cross-file atomic, 33 same-file atomic, 57 cross-file non-atomic, 13 same-file non-atomic | same probe |
| inlined, same-file only (prototype) | 25 | `mfb build -v` → `Leaf function inlining: 25` |
| inlined, with the cross-file location tag | 126 | `mfb build -v` → `Leaf function inlining: 126` |
| ratio (lock NOT held, median of 5): plan-158-B 7.5x → same-file 6.3x → cross-file 5.9x | | `/tmp/b740-measure.py 5 oracle dev1 dev_c dev_c2` |

## 3. Design Overview

1. **`NirSourceLoc::file`** (`u32`, `0` = the owning function's file, else an
   interned path via `nir::source_file_id`/`source_file_name`). The error
   emission reads it (`CodeBuilder::error_filename`) in its two filename sites.
2. **`walk_ops_values_mut` / `walk_value_mut`** in `nir/visit.rs`: the mutable
   twin of the traversal seam (children before parents, exhaustive), so the row
   does not hand-write a recursion.
3. **The row** (`optimizer::opt1::inline::inline_leaf_calls`, Level 1), run after
   literal-`LET` folding and before the local rewrites: collect leaves; per
   caller (skipping any with a call under a capture wrapper), compute local
   types; replace each eligible `Call` by the callee's expression with
   parameters substituted and — across files — every located node tagged with
   the callee's file.

Correctness risk concentrates in the eligibility rules; each one is a
module-doc bullet with a unit test that pins the refusal. The runtime test
compares every observable at `-O0` and `-O1` on a two-file program and proves
(by mutation, in the session) that it catches a wrong `ErrorLoc` file.

**Gate class: not byte-identical.** `.ncode`/`.ncodesum` move for every fixture
with an eligible call; `.nir` moves where a fixture carries one. `.ir`/`.ast` do
not move.

Rejected alternatives:

- *Same-file only.* Inlines 25 of 156 atomic sites in brogue; the file tag costs
  one field and two read sites.
- *Hoisting non-atomic arguments into temporaries.* Changes where an argument's
  trap fires relative to earlier operands of the enclosing expression unless the
  call is the first thing evaluated; not needed for the goal.
- *General inlining at Level 4.* Out of scope; this row is the boundary-free
  subset.

## Phases

### Phase 1 — the location tag, the mutable walker, and the row

(One phase: the tag and the walker have no caller but the row, so landing them
alone would leave dead code.)

- [x] `nir/mod.rs`: `NirSourceLoc::file`, `source_file_id`, `source_file_name`;
      `nir/lower.rs`: `file: 0`.
- [x] `builder_error_emission.rs`: `error_filename()` in both filename sites.
- [x] `nir/visit.rs`: `walk_ops_values_mut`, `walk_value_mut`.
- [x] `src/optimizer/opt1/inline.rs` + `mod.rs` wiring, `stats.rs` counter,
      `catalog.rs` row ("Leaf function inlining", Level 1, NIR).
- [x] Unit tests: substitution with swapped arguments; cross-file tag; refusals
      (non-atomic, mistyped literal, unknown local, arity, user call in the body,
      two statements, `String` result, `ISOLATED`, `SUB`, `CallResult`, call
      under `Checked`); off at `-O0`. `cargo test --bin mfb opt1::inline` → 6
      passed.
- [x] Runtime test `tests/runtime/rt_leaf_inlining.rs` (two-file program):
      same output at `-O0`/`-O1` incl. overflow and index errors naming
      `src/helpers.mfb` and the helper's line, a function-level `TRAP`
      catching an inlined failure, and an `AND`-chain helper as a condition;
      control: `main` has no `bl` to `cell`/`inMap` at `-O1`, has them at `-O0`,
      and keeps the inline-`TRAP`ped `delta` call.
      `MFB_TEST_EXE=target/debug/mfb cargo test --test rt_leaf_inlining` → 2
      passed; the control FAILS with the main-tip compiler (RED at base); with
      `error_filename` forced to the function's file the equivalence test FAILS
      (`file=src/main.mfb` at `-O1`), so the test does catch a wrong file.
- [x] brogue `dig` vs oracle, 8 seeds × 40 depths (debug-built compiler): 320
      pairs, 0 mismatches.

Acceptance: those tests pass; brogue 320/320.
Commit: —

### Phase 2 — goldens, docs, budget

- [ ] Regenerate and classify the native goldens; inspect one fixture.
- [x] Spec: `architecture native-ir` (the `file` field, the mutable walker),
      `tooling cli-reference` (`-O1` list); `planning/optimizations.md`
      ("Function inlining" row: the landed Level-1 subset).
- [ ] `tools/brogue-perf/bench.sh`: tighten `BUDGET_RATIO` to the measured
      floor.

Acceptance: `scripts/artifact-gate.sh target/release/mfb all` → `0 diff(s)`.
Commit: —

## Validation Plan

- Tests: above; the existing runtime suite (every `rt_*`, `rt-behavior`,
  `rt-error` fixture now runs with inlined helpers wherever eligible).
- Runtime proof: `examples/brogue/check/check-terrain.sh 8 1`; the bench.
- Self-update arms: `rt_inplace_self_update --ignored`, narrowed
  (`MFB_SELF_UPDATE_FILTER`) — inlining changes the shape of index arguments
  those arms see (`set(grid, gi(i, j), v)` → `set(grid, i * 29 + j, v)`).
- Cross-target: as plan-158-B (artifact gate; linux-runtime-proof on
  2223/2229/2227; brogue on every target incl. the Windows box).
- Final gate (once, heavy lock): `cargo test --release --no-fail-fast`,
  `scripts/artifact-gate.sh target/release/mfb all`.

## Open Decisions

- **Level.** The catalog classifies inlining as Level 4 because in general it
  makes stack traces and breakpoints lie. MFBASIC has neither a stack trace nor a
  debugger mapping, and this subset keeps every error's location, so the
  boundary is not observable. **Recommended and taken: Level 1**, per the owner's
  bug-740 decision that the goal is met at the default level without changing
  semantics. Recorded here because it departs from the catalog's classification.
- Inline non-atomic arguments? — **No** (see rejected alternatives).

## Corrections

1. The draft split the location tag and walker (Phase 1) from the row (Phase 2);
   neither has another caller, so they land together as Phase 1.
2. Developed in the scratch worktree `/tmp/b740-dev` while plan-158-A/B waited
   on the heavy lock, and cherry-picked onto `worktree-B-740` after plan-158-B
   completed.

## Summary

The row is small; the care is in the refusals and in the one piece of
infrastructure it needs — a per-node file so an inlined error still points at
the helper. What stays uninlined: anything with a statement, a local, a
non-scalar value, a user call, or a computed argument.
