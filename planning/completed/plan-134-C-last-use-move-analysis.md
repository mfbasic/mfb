# plan-134-C: A last-use move analysis for owning stores

Last updated: 2026-09-13
Effort: large (3h–1d)
Depends on: plan-134-B

A per-function analysis that answers, for every owning store whose source is a local: **is
this the last read of that local on every path?** Letters D and E use the answer to move the
value instead of deep-copying it (`mfb spec language memory-semantics` §14.2: "If the value's
type is copyable and the binding is not used again, the compiler may move it"). Without it,
D's copy-insertion would deep-copy every `item` the decoders append — `json::parse` array
building (`acc = collections::append(acc, item)`, `json/helper_parse_array_items.rs`) and the
regex parser (`opts`, `parts`) are exactly that shape (5 sites, plan-134-A §2.1).

This letter adds the analysis and its tests **with no callers**: it changes no emitted code.

Behavioural outcome: none observable; the analysis's answers on a fixed set of NIR functions
equal a hand-derived table.

References: plan-134-A; `mfb spec language memory-semantics` §14.1–§14.2, §14.9 (move
tracking); `src/codegen/engine/function/function_lowering.rs` (the `collect_*` prescans this
mirrors); `src/codegen/engine/control/builder_exits.rs::plan_returned_move` (the one existing
move).

Prerequisites: see plan-134-A; plan-134-B complete (`ls planning/completed/plan-134-B-*`) — MET
2026-09-13.

## 1. Goal

- `function_lowering.rs` exposes `collect_last_use_moves(function) -> MoveSites`, a set of
  `(op index, local name)` that are safe to move.
- A unit table covers every shape in §3 and passes.
- Emitted code is byte-identical on every target (no caller reads the set yet).

### Non-goals

- No change to diagnostics: `TYPE_USE_AFTER_MOVE` and `ir::verify` move tracking are
  source-level and untouched; this is a codegen optimization only (§14.1 last paragraph).
- No move for a value with a resource inside it, a `by_ref` local, or a global.

## 2. Current State

- The only copy elision is `plan_returned_move` / `move_elided` for `RETURN <owned local>`,
  which needs the local's `ActiveCleanup::OwnedValue` to exist.
- No liveness exists outside the register allocator (`grep -rni "last.use\|liveness"
  src/codegen/engine/analysis` → empty; `engine/regalloc/linear_scan.rs` is register-level).
- Name-set prescans in `function_lowering.rs`: `collect_borrow_get_locals`,
  `collect_value_used_locals`, `collect_reassigned_locals`, `address_taken_locals`.
- `for_each_iterable_locals` (`builder_control.rs`) blocks moves/frees of a live `FOR EACH`
  iterable; `by_ref` is on `LocalValue` (`engine/builder/mod.rs`).

## 3. Design

A backward liveness pass over the function's NIR ops, computed once per function before
lowering (like `collect_borrow_get_locals`):

- **Live-after sets** per op for locals only. A `NirValue::Local(x)` read makes `x` live; a
  `Bind`/`Assign` to `x` kills it. Loops: iterate to a fixed point over the loop body so a
  local read on the next iteration is live at the back edge. `TRAP` handlers and every exit
  path (`RETURN`, `EXIT`, `CONTINUE`, `FAIL`) are successors.
- **A store is a move site** when its source operand is exactly `NirValue::Local(x)` and `x`
  is not live after the op, and none of these hold (each fails closed to "copy"):
  `x` is `by_ref`, address-taken (`address_taken_locals`), captured by any closure, a live
  `FOR EACH` iterable, a borrow-`get` local, a parameter (the callee borrows it — §2 of
  plan-134-G's audit), or its type contains a resource.
- **Owning stores covered:** `Bind`, `Assign`, `StoreGlobal`, `Return`, a record/union
  constructor operand, a collection literal element, the `item` of an
  `append`/`insert`/`set`/`prepend`, a closure capture.
- **Fail closed.** Anything the pass does not model (an op kind it does not know) makes every
  local it touches live — a missed move is a copy, never a use-after-free. The match over op
  kinds is exhaustive with no wildcard, so a new `NirOp` is a build error (bug-567 precedent).

Rejected: a flow-insensitive "used exactly once" count (wrong inside loops: `append(acc,
item)` in a loop reads `item` once per iteration, but `item` is rebound each iteration — only
real liveness sees that); reusing regalloc liveness (it is per vreg after lowering, too late
for the store decision).

## Phases

### Phase 1 — the pass and its table

- [x] `src/codegen/engine/function/function_lowering.rs`: `MoveSites` type and
      `collect_last_use_moves`, exhaustive over `NirOp`. — in
      `src/codegen/engine/analysis/last_use.rs` (see Corrections: home, field-sensitive places,
      address-keyed sites); both the liveness transfer and the site/handler walks match every
      `NirOp` variant with no wildcard.
- [x] Unit tests (same file's test module), each a small NIR function with the expected set:
      a bind whose source is never read again (move); read again after (copy); read on the next
      loop iteration (copy); rebound each iteration then appended (move — the json shape);
      read in a `TRAP` handler after the store (copy); captured by a later lambda (copy);
      `FOR EACH` over it live (copy); parameter source (copy); `by_ref` (copy); resource-bearing
      type (copy); `RETURN` of a local (move, agreeing with `plan_returned_move`). —
      `collect_last_use_moves_follows_the_hand_derived_table` (the ten source-lowered shapes plus
      a field row: `LET k = h.kids` then only `h.tag` read → move),
      `collect_last_use_moves_never_moves_a_by_ref_capture` and
      `…_never_moves_a_resource_bearing_value` (hand-built, each with a positive control).
- [x] A table test over the real decoder bodies: the NIR of `json`'s array-items helper and
      `regex`'s concat/alt helpers yields a move at each of the 5 append sites
      (plan-134-A §2.1). — `collect_last_use_moves_finds_the_five_decoder_append_sites`:
      `#json_parseArrayItems` (`item`, and `parsed.value` at its bind), `#json_revive`
      (`revivedItem`), `#regex_parseAlt` (`nextc.node`), `#regex_parseConcat` (`q.node` ×2).

Acceptance: the analysis gives the hand-derived answer for every shape, including the 5
decoder sites.
  Check: `cargo test --release --bin mfb -- collect_last_use_moves` → all passed (est. 3 min).
  Result: met — `4 passed; 0 failed` (first run: 2 failed on test setup only — helper names are
  `#json_…` once lowered, and the builtin resource table is keyed `fs.File`; no analysis change).
Commit: dd5b073ce

### Phase 2 — prove it changes nothing

- [x] `bash scripts/artifact-gate.sh target/release/mfb all` → 0 diffs (the pass has no callers).
      — run against a snapshot of the release build containing `last_use.rs`
      (`/tmp/p134-mfb-c`, so a concurrent `cargo test` relink could not swap the binary
      mid-gate): `artifact-gate [all]: 1437 tests, 1603 build(s), 2013 golden(s) checked, 0
      diff(s)`.

Acceptance: byte-identical emitted code on every target — this letter is provably neutral, so
byte-identity IS its gate. A diff is a bug in this letter (the pass must not touch lowering
state); localize it by building one fixture's `-ncode` before and after.
  Check: that command → `0 diff(s)` (est. 15 min: the gate is the only check that sees every
  target).
  Result: met — `2013 golden(s) checked, 0 diff(s)`. The release build's only warnings are the
  20 "never used" warnings in `last_use.rs` (`grep "^  --> " … | grep -vc last_use.rs` → 0),
  the expected consequence of no callers until letter D.
Commit: 7a3f67ff6

## Validation Plan

- Tests: the unit table and the decoder-site table.
- Runtime proof: none (no behaviour change).
- Doc sync: none until letter D wires it in.
- Final gate: plan-134-H.

## Open Decisions

- None beyond plan-134-A's.

## Corrections

- **Prerequisite re-run** (2026-09-13): `ls planning/completed/plan-134-B-*` →
  `planning/completed/plan-134-B-non-recursive-deep-copy.md` — MET.
- **Places, not locals: the analysis is field-sensitive.** The design's move rule ("its source
  operand is exactly `NirValue::Local(x)`") cannot meet this letter's own acceptance ("a move at
  each of the 5 append sites"). Read in the helper bodies: three of the five append a FIELD —
  `opts = collections::append(opts, nextc.node)` (`regex/helper_parse_alt.rs`) and
  `parts = collections::append(parts, q.node)` twice (`regex/helper_parse_concat.rs`), each
  followed by reads of `q.nxt`/`q.groups`/`q.names`, so `q` is live and a whole-local analysis
  moves nothing — and json's element is itself bound from a field, `LET item AS Json =
  parsed.value`, with `parsed.index` read on the next line. So a place is a local `x` or one field
  `x.f`: reading `x` reads every `x.f`, reading `x.f` reads only that field, binding/assigning `x`
  kills both. The consumer (letters D/E) decides what a site licenses: a `Local` store moves; a
  field store takes the edge (reads it and nulls the field word in the dead-for-that-field record,
  so a later drop of the record skips it — the F walker skips null edges).
- **Sites are keyed by the op's address, not an op index.** The lowering walks the same
  `function.body` the analysis reads, so `op as *const NirOp` identifies the op exactly; a counted
  index would have to reproduce the lowering's visit order. A desugar that lowers a synthesized op
  finds no site and copies (fail closed).
- **Home and signature.** `collect_last_use_moves(function, model) -> MoveSites` lives in
  `src/codegen/engine/analysis/last_use.rs` (the analysis directory the plan's own §2 names as
  having no liveness; `function_lowering.rs` holds the prescans it calls). It takes the
  `TypeModel` because the resource exclusion needs `type_contains_resource`. The query is
  `MoveSites::is_last_use(op, &Place)`. Test names keep the `collect_last_use_moves` filter.
- **Only simple statements produce sites.** `Bind`, `Assign`, `StoreGlobal`, `StateAssign`,
  `Eval`, `Return`, `Fail`, `ExitProgram`; a store inside an `IF`/loop/`MATCH` header is copied.
- **More fail-closed exclusions than §3 lists, each an alias the plan's list would miss:** a local
  bound from `UnionExtract` or `Capture` (an alias into another value / the closure env), a local
  read inside a `UnionExtract` or inside a borrow-`get` initializer (an alias of it may be live),
  a `FOR`/`FOR EACH` variable, a `TRAP` error name, a `STATE` resource, and any name the function
  never binds.
- **by-ref and resource rows are hand-built NIR.** No small program reliably lowers to a
  `Bind r = Capture { by_ref: true }` in a function a test can name, and a resource needs no
  program to classify; each hand-built test carries a positive control (the same shape with an
  ordinary value IS a move) so it cannot pass vacuously. Every other row is lowered from MFB source
  with `testutil::nir_for_src`.
- **No callers until letter D, by this letter's design** — so a non-test build warns that
  `collect_last_use_moves`/`MoveSites` are unused between this letter's commit and D's. No
  `#[allow]` is added; letter D wires the analysis in and must show the warning gone
  (`cargo check --release --all-targets`).

## Summary

A pure analysis with no callers, gated by byte-identity. Its correctness matters in D/E/G: a
wrong "move" is a shared graph that G later double-frees, so every unmodelled case fails
closed to "copy".
