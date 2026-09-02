# plan-118-A: NIR→machine expansion — honest metrics and attribution instrument

Last updated: 2026-09-01
Overall Effort: huge (>3d — the whole plan-118 family, five letters)
Effort: medium (1h–2h)
Depends on: nothing

plan-118 attacks `planning/speed.md` recommendation 3: the "585:1" NIR→machine
expansion that is the cost ceiling on the entire back end (§5.1). The research
spike behind this family (research worktree, 2026-09-01) attributed every
emitted instruction to the NIR op/value/call that produced it and found the
expansion is **concentrated, not uniform**: five inline-lowering categories are
68 % of all builder-emitted instructions, three generated Unicode functions are
15.0 % of the module, and the "585:1" headline itself is partly a measurement
artifact (the `NIR statements` counter is flat, not recursive — the honest
ratio is **325:1**). The letters:

- **A (this doc)** — land the honest metrics + the per-category instruction
  attribution as a permanent `-vv` instrument, and correct `planning/speed.md`.
- **B** — replace the three generated Unicode IF-chain functions with native
  data tables (−2,556,471 instrs, 15.0 %).
- **C** — out-of-line string concat / `toString` / `io.print` marshalling into
  synthesized `runtime.*` functions (targets 4,764,178 instrs).
- **D** — out-of-line record construction/copy into per-type synthesized
  functions (targets 2,173,050 instrs).
- **E** — shared per-function return epilogue + error-construction blocks
  (targets 2,007,382 + the ~40-instr inline error paths remaining after C/D).

Mechanism M4 from the research — ~50 % of every function's instructions are
stack round-trips (`concat2`: 151 of 300 are `ldr_u64`/`str_u64` through sp
slots; every intermediate value is stored and immediately reloaded) — is
**explicitly out of scope for the whole family**: fixing it means a
values-in-registers builder architecture, a separate investigation this
family's letters do not depend on. Recorded here so it isn't rediscovered.

References:

- `planning/speed.md` §3, §4, §5.1, §5.3, recommendation 3 and 7.
- The spike attribution data reproduced in §2 below (commands inline).
- `src/trace.rs` (`count`, `timed_tally`, `render`), `src/target/shared/lower.rs:30`
  (the flat counter), `src/codegen/engine/function/function_lowering.rs:1115`
  (the machine-instruction counter).
- `src/docs/spec/tooling/07_cli-reference.md:232` — documents the `-vv` counters;
  obligated to stay current.

## Prerequisites

(Stated once here for the family; letters B–E point here plus their own.)

| Must be true | Command | Status |
|---|---|---|
| The `-vv` tracer exists | `grep -n 'fn count' src/trace.rs → src/trace.rs:375` | MET (re-run 2026-09-01: `375:pub(crate) fn count(...)`) |
| `-vv` is print-only (pinned) | `grep -rln artifact_bytes_identical_across_verbosity_levels tests/ → 1 hit` | MET (re-run: `tests/cli_build_verbosity_output.rs`) |

Everything below is written against current `main` (`f4be5ea25` at plan time).

## 1. Goal

- `mfb test -vv` reports a **recursive** NIR op count beside the flat one, a
  per-function machine-instruction leaderboard ("largest lower_function"), and
  a per-category instruction-attribution tally ("costliest expansion") — so
  letters B–E's acceptance criteria are checkable from one build's output, and
  the next person profiling the back end starts from honest numbers.

### Non-goals (explicit constraints)

- **Zero change to emitted artifacts.** This letter is instrumentation only;
  `artifact-gate.sh` byte-identity must hold (the `-vv` machinery is print-only,
  pinned by `artifact_bytes_identical_across_verbosity_levels`).
- No changes to any lowering, and no new CLI flags — everything rides `-vv`.

## 2. Current State (the research findings this letter makes permanent)

All numbers measured 2026-09-01 on the research-worktree spike build
(instruction-delta attribution hooks around `lower_ops_inner` per op and
`lower_value` per value, exclusive self-minus-children accounting), corpus
`cd tests/acceptance && MFB_SPIKE_EXPANSION=1 ../../target/release/mfb test -vv`:

### Measured populations

| What | Count | Source |
|---|---|---|
| Machine instructions (module) | 17,079,160 | `-vv` counter, spike run (`speed.md` recorded 16,957,344 at HEAD) |
| Flat `NIR statements` counter | 29,088 | `-vv` counter (`sum(body.len())` — counts a whole loop as 1) |
| **Recursive** NIR ops | 52,548 | spike `recursive_op_count` visitor summed over 1,616 `SPIKEFN` lines |
| Honest expansion ratio | **325:1** | 17,079,160 / 52,548 |
| `binop:&` (string concat) | 2,907,604 instrs / 17,221 sites = 169/site | spike attribution dump |
| `val:Constructor` | 2,173,050 / 7,876 = 276/site | ditto |
| `op:Return` | 2,007,382 / 11,432 = 176/site | ditto |
| `call:toString` | 1,030,128 / 5,826 = 177/site | ditto |
| `rtcall:io.print` | 826,446 / 3,193 = 259/site | ditto |
| Top-5 categories combined | 8,944,610 = 67.9 % of 13,175,351 attributed | ditto |
| `#strings_genCat` + `#regex_genCat` + `#regex_scriptOf` | 1,057,783 + 1,057,783 + 440,905 = 2,556,471 (15.0 %) | `SPIKEFN` lines |
| Generated test-harness functions (`__mfb_test_*`) | 825 fns, 12,554,189 instrs = **73.5 % of the module** | `SPIKEFN` lines |
| Micro-fixture: `RETURN a & b` | 300 instrs (83 `ldr_u64` + 68 `str_u64`; two inline byte-at-a-time copy loops; ~45-instr inline alloc-failure error path) | `mfb build --ncode` on a 2-function project |
| Micro-fixture: `RETURN toString(n)` | 315 instrs, 2 allocs, 2 inline error paths | ditto |
| Micro-fixture: `RETURN p.x + p.y` | 216 instrs | ditto |

### Verified properties

- **The flat counter undercounts by 1.8×** — read `src/target/shared/lower.rs:30`:
  it sums `function.body.len()`, so nested `If`/loop/`Match` bodies count as one.
- **The attribution totals reconcile**: 13,175,351 attributed (builder-emitted,
  exclusive) vs 17,079,160 final — the gap is prologue/frame code, regalloc
  spills, and zeroing outside op frames; peepholes shrink the builder count.
- **`-vv` output is byte-neutral to artifacts** — pinned by
  `artifact_bytes_identical_across_verbosity_levels` (grep above).

## 3. Design Overview

Three additions, all inside the existing `trace` idioms:

1. **Recursive op counter** — beside the flat `NIR statements` count in
   `src/target/shared/lower.rs`, add `trace::count("NIR ops (recursive)", …)`
   using an exhaustive `nir::visit::NirVisitor` walk (the spike's
   `recursive_op_count`; the visitor seam is the one bug-328 made exhaustive).
   Keep the flat counter (renaming it would churn the spec doc for no gain;
   the new row's name makes the difference visible).
2. **Per-function instruction leaderboard** — at
   `function_lowering.rs:1115`, record `trace::item("largest lower_function",
   || function.name.clone(), Duration-from-count)` — or, cleaner, extend
   `trace` with a size-tally twin of `timed_tally` if `item`'s Duration typing
   fights it. Mirrors the existing "slowest lower_function" leaderboard so the
   §5.3-style question ("is it one pathological function?") is answerable for
   *size*, not just time.
3. **Category attribution tally** — the spike's exclusive-attribution hooks
   (a frame stack keyed by op tag / value tag / `call:{target}`), landed
   behind `trace::enabled()` instead of an env var, feeding a
   "costliest expansion" render section like the existing "costliest
   abi_inline builtin" one. Hook points: around the per-op closure in
   `builder_control.rs:373` (`lower_ops_inner`) and around
   `lower_value_inner` in `builder_values.rs` (`lower_value`, which already
   has the wrap-the-inner-call shape). Cost when `-vv` is off: one branch per
   op/value.

Byte-identity IS this letter's gate (print-only change).

Rejected: replacing the flat counter (spec-doc churn, and its history in
`speed.md` §1 keeps meaning); a separate `--expansion-report` flag (the `-vv`
report is where every other profiling row lives).

## Phases

### Phase 1 — counters and leaderboard

- [x] `src/target/shared/lower.rs`: add the recursive `NIR ops (recursive)`
      counter (exhaustive `nir::visit` walk).
- [x] `src/codegen/engine/function/function_lowering.rs`: record per-function
      instruction counts into a "largest lower_function" leaderboard.
- [x] `src/trace.rs`: size-tally support if `timed_tally`/`item` cannot carry a
      count cleanly (a `count_tally(bucket, label, amount)` twin). — `item`
      cannot: its leaderboard is `Duration`-keyed end to end (insertion sort,
      `millis()` render), so a count would have to be smuggled through a
      `Duration` and rendered as milliseconds. Landed `size_item` (the
      leaderboard twin) here; `count_tally` (the tally twin) lands with its
      only consumer in Phase 2, so no phase commits an unused function.

Acceptance: `cd tests/acceptance && ../../target/release/mfb test -vv` prints
the recursive counter (~52.5k) and a size leaderboard whose top three rows are
`#strings_genCat`, `#regex_genCat` (equal counts), `#regex_scriptOf`;
`rustup run 1.96.0 cargo test --no-fail-fast` green;
`scripts/artifact-gate.sh all` 0 diffs.

MET, measured 2026-09-01 in this worktree (`cd tests/acceptance &&
../../target/release/mfb test -vv`, log `/tmp/p118_vv_phase1.log`):

```
--- trace: largest lower_function (20 of 1616 items, 17079160 total) ---
     1057783  #regex_genCat
     1057783  #strings_genCat
      440905  #regex_scriptOf
       71647  __mfb_test_case_266
--- trace: counters ---
NIR functions                    1616
NIR statements                  29088
NIR ops (recursive)             52548
machine instructions         17079160
```

Every §2 number reproduces exactly: 52,548 recursive ops, 29,088 flat, and
17,079,160 machine instructions → **325:1**, not 585:1.
`cargo test --no-fail-fast`: 90 suites, 0 failures.
`scripts/artifact-gate.sh all`: 1325 tests, 1823 goldens, **0 diffs**.
Commit: 502fb835f

### Phase 2 — attribution tally

- [x] `src/codegen/engine/` new module (production version of the spike's
      `spike.rs`): frame stack + exclusive tally, gated on `trace::enabled()`.
      — `src/codegen/engine/expansion.rs`, plus `trace::count_tally` (the tally
      twin of `timed_tally`) landing here with its only consumer.
- [x] Hook `lower_ops_inner` (per-op) and `lower_value` (per-value/target).
- [x] Render a "costliest expansion" section in `trace::render()` (top ~40 rows).
- [x] Doc sync: `src/docs/spec/tooling/07_cli-reference.md` `-vv` section
      (new counter + leaderboard + tally); `planning/speed.md` — append a dated
      correction to §5.1/§5.2-style prose recording the honest 325:1 ratio and
      the attribution table (recommendation 3 is now root-caused).

Acceptance: the `-vv` report over `tests/acceptance` reproduces §2's table
within noise (top row `binop:&` ≈ 2.9M exclusive instrs); `cargo test
--no-fail-fast` green; `scripts/artifact-gate.sh all` 0 diffs;
`cargo check --all-targets` clean; both-root `cargo fmt` run.

MET, measured 2026-09-01 (`/tmp/p118_vv_phase2.log`). Not "within noise" —
**exact**, every row:

```
--- trace: costliest expansion (40 of 1821 keys, 13175351 total, exclusive) ---
     2907604     17221x  binop:Concat
     2173050      7876x  val:Constructor
     2007382     11432x  op:Return
     1030128      5826x  call:toString
      826446      3193x  rtcall:io.print
      551939     12407x  op:Bind
      319358      4540x  op:Assign
      255872      3609x  op:Fail
      245967      1828x  binop:Add
```

Top five = 8,944,610 = 67.9 % of 13,175,351 attributed, as §2 states.
`cargo test --no-fail-fast`: 89 suites ok; the 90th (`tests/golden.rs`) hit a
peer session's artifact-gate lock (exit 98 — "could not START ... nothing was
checked", not a golden result) and passes uncontended:
`cargo test --test golden` → ok, 1823 goldens, 0 diffs in 266.68s.
`scripts/artifact-gate.sh all` standalone: 1325 tests, 1823 goldens, **0 diffs**.
`cargo check --all-targets`: clean. Both-root `cargo fmt` run.
Commit: f86af39a7

## Validation Plan

- Tests: existing `artifact_bytes_identical_across_verbosity_levels` pins
  byte-neutrality; add a unit test for the recursive counter on a nested-loop
  NIR fixture (flat=1, recursive=N) beside the visit tests in
  `src/target/shared/nir/visit.rs`.
- Runtime proof: the `-vv` report itself over `tests/acceptance` (numbers above).
- Acceptance: full `cargo test --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all` (0 diffs — this letter only).

## Open Decisions

- ~~Whether the attribution tally lands permanently or stays a diff kept in this
  plan directory~~ — **DECIDED: landed permanently** (`src/codegen/engine/expansion.rs`,
  commit `f86af39a7`). Cost when `-vv` is off is one relaxed atomic load per op
  and per value; `artifact-gate.sh all` is 0 diffs over 1823 goldens with it in
  the tree, and it is the instrument B–E's acceptance criteria are stated in.

## Corrections

1. **The attribution key for concat renders `binop:Concat`, not `binop:&`.**
   §2 and Phase 2's acceptance spell the row `binop:&`; the key is built from
   the `NirValue::Binary { op }` debug name (`format!("binop:{op:?}")`), so it
   prints `binop:Concat`. Same row, same 2,907,604 / 17,221x. Spelled correctly
   here and in `planning/speed.md`; plan-118-C's acceptance means this row.

2. **`trace::item` cannot carry a size, so the leaderboard needed a twin, not
   a parameter.** Phase 1 hedged ("size-tally support *if* `timed_tally`/`item`
   cannot carry a count cleanly"). It cannot: `Bucket` is `Duration` end to end
   — the insertion-sort comparison, the `total`, and the `millis()` render —
   so a count would have to be smuggled through a `Duration` and printed as
   milliseconds. Landed `size_item` + `SizeBucket` (leaderboard twin) and
   `count_tally` + `CountTally` (tally twin) instead.

3. **`count_tally` landed in Phase 2, not Phase 1.** Phase 1's task list puts
   the whole of the `trace.rs` work in Phase 1, but `count_tally`'s only caller
   is Phase 2's attribution module: committing it in Phase 1 would have left a
   `dead_code` warning in a committed state, which `cargo check --all-targets`
   is supposed to keep clean. Split so each phase commits only what it uses.

4. **A found bug, fixed alongside (separate commit `ebf13af74`).** The first
   line of `resolve_link_libraries`' doc comment in
   `src/codegen/engine/builder/mod.rs` had been stranded on top of
   `resolve_closer_symbol` by `efba4fa8b` (the src/codegen tier relocation), so
   one item's doc claimed to resolve `LINK` libraries and the other's opened
   mid-sentence. Doc comments only; no emitted bytes.

## Summary

Zero-risk instrumentation letter; its value is that every later letter's
"expected −N instructions" becomes a checkable number in one build's output.
