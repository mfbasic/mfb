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

Prerequisites: see plan-134-A; plan-134-B complete (`ls planning/completed/plan-134-B-*`).

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

- [ ] `src/codegen/engine/function/function_lowering.rs`: `MoveSites` type and
      `collect_last_use_moves`, exhaustive over `NirOp`.
- [ ] Unit tests (same file's test module), each a small NIR function with the expected set:
      a bind whose source is never read again (move); read again after (copy); read on the next
      loop iteration (copy); rebound each iteration then appended (move — the json shape);
      read in a `TRAP` handler after the store (copy); captured by a later lambda (copy);
      `FOR EACH` over it live (copy); parameter source (copy); `by_ref` (copy); resource-bearing
      type (copy); `RETURN` of a local (move, agreeing with `plan_returned_move`).
- [ ] A table test over the real decoder bodies: the NIR of `json`'s array-items helper and
      `regex`'s concat/alt helpers yields a move at each of the 5 append sites
      (plan-134-A §2.1).

Acceptance: the analysis gives the hand-derived answer for every shape, including the 5
decoder sites.
  Check: `cargo test --release --bin mfb -- collect_last_use_moves` → all passed (est. 3 min).
Commit: —

### Phase 2 — prove it changes nothing

- [ ] `bash scripts/artifact-gate.sh target/release/mfb all` → 0 diffs (the pass has no callers).

Acceptance: byte-identical emitted code on every target — this letter is provably neutral, so
byte-identity IS its gate. A diff is a bug in this letter (the pass must not touch lowering
state); localize it by building one fixture's `-ncode` before and after.
  Check: that command → `0 diff(s)` (est. 15 min: the gate is the only check that sees every
  target).
Commit: —

## Validation Plan

- Tests: the unit table and the decoder-site table.
- Runtime proof: none (no behaviour change).
- Doc sync: none until letter D wires it in.
- Final gate: plan-134-H.

## Open Decisions

- None beyond plan-134-A's.

## Corrections

(Filled in during execution.)

## Summary

A pure analysis with no callers, gated by byte-identity. Its correctness matters in D/E/G: a
wrong "move" is a shared graph that G later double-frees, so every unmodelled case fails
closed to "copy".
