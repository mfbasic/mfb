# bug-57: a `WHILE` condition constant-folds a loop-mutated local using its stale loop-entry value, emitted once above the back-edge — wrong result / infinite loop

Last updated: 2026-07-09
Effort: small (<1h)

The `WHILE` codegen arm lowers the loop condition **before** clearing the loop-entry
local-constant folds, and the loop back-edge target sits **above** the condition. So a
condition that folds a local through a string/primitive folder (`toString`, string
concat, `upper`/`lower`/`caseFold`/`normalizeNfc`) uses the local's *loop-entry constant*
— frozen — and that frozen comparison is emitted once, straight-line. The body then
reassigns the local, but the re-tested condition still sees the entry value, so the loop
never observes the change. For a termination test this means the loop never terminates.

Runtime-confirmed on macOS/aarch64: `WHILE toString(c) <> "3"` with `c` incremented in
the body loops forever (`c` reaches 21+ while the test still reads `"0"`), whereas the
integer form `WHILE c < 3` terminates at `c = 3`. The single correct behavior a fix
produces: a `WHILE` condition never folds a local that the loop body reassigns — it reads
the live value each iteration.

References:

- `src/target/shared/code/builder_control.rs`, `NirOp::While` arm (`:628-641`):
  `lower_value(condition)` (`:629`) precedes `clear_local_constants()` (`:632`), and the
  back-edge target `while_loop` label (`:628`) is above the condition; `branch(while_loop)`
  at `:641`.
- Correct sibling: `NirOp::DoUntil` (`:663-669`) clears constants (`:669`) **before**
  lowering its body+condition, with a comment stating the exact rule ("constants known at
  loop entry must not fold reads … they go stale once the body reassigns them").
- The folders that consult `local.constant`: `lower_value_inner` → `static_string_value`
  (`:207`); `Call toString` arm `builder_value_semantics.rs:516` → `static_primitive_text`.
- Integer/relational lowering (`builder_numeric`) does **not** consult `local.constant`,
  which is why numeric conditions are immune.
- Related constant-fold-scope work: plan-25-C (copy elision), local-constant save/restore
  in If/Match.
- Found during the goal-01 compiler source review of `src/target/shared/code/`.

## Failing Reproduction

```
IMPORT io
FUNC main AS Integer
  MUT c AS Integer = 0
  MUT n AS Integer = 0
  WHILE toString(c) <> "3"
    c = c + 1
    n = n + 1
    IF n > 20 THEN
      io::print("still looping at n=" & toString(n) & " c=" & toString(c))
      RETURN 1
    END IF
  WEND
  io::print("terminated, c=" & toString(c))
  RETURN 0
END FUNC
```

- Observed: prints `still looping at n=21 c=21` — the loop never terminates even though
  `c` passed 3, because the condition is frozen at `toString(0) <> "3"`.
- Expected: `terminated, c=3` after three iterations.

Contrast cases that work correctly today (verified):

- `WHILE c < 3` (integer relational condition) → terminates at `c = 3`. Arithmetic
  lowering ignores `local.constant`.
- `DO … UNTIL toString(c) = "3"` → correct (DoUntil clears constants before its
  condition).
- `typeName(x)` in a condition is safe (type-invariant, not value-folded).
- The numeric `FOR` induction variable is inserted with `constant: None`.

## Root Cause

In the `WHILE` arm, `clear_local_constants()` runs *after* the condition is lowered
(`builder_control.rs:632` vs `:629`), and the back-edge re-enters above the condition
(`:628`/`:641`). So on entry the local's `constant` fold (e.g. `c = Const 0` from its
initializer) is still live when the condition is lowered, and a string/primitive folder
collapses `toString(c)` to the rodata `"0"`. That comparison is emitted once; every
iteration branches back to it and re-tests the frozen value. `DoUntil` avoids this by
clearing constants before lowering the condition — the `WHILE` arm violates the rule its
sibling documents.

## Goal

- A `WHILE` condition reads the live value of any local the body reassigns; no
  loop-variant local is folded in the condition.
- The reproduction prints `terminated, c=3`.
- Numeric conditions and constant conditions over loop-invariant locals are unchanged.

### Non-goals (must NOT change)

- Folding in conditions over genuinely loop-invariant locals (still valid — do not
  over-clear in a way that pessimizes those).
- `DoUntil` (already correct), `FOR`, `IF`/`MATCH` fold scoping.
- The post-loop `clear_local_constants()`.

## Blast Radius

- `NirOp::While` arm (`builder_control.rs`) — fixed here.
- Any other construct that lowers a condition above its back-edge without a prior clear —
  `DoUntil` is correct; confirm no third loop form shares the ordering bug.

## Fix Design

Move `self.clear_local_constants();` to **before** `let condition =
self.lower_value(condition)?;` in the `WHILE` arm (mirroring `DoUntil`), keeping the
post-loop clear. Or, more surgically, clear the constants of exactly the locals the body
assigns before lowering the condition — a smaller invalidation that preserves folding of
truly invariant locals. The blanket pre-clear matches `DoUntil` and is lower-risk.

Rejected alternative: stop consulting `local.constant` in the string folders. Rejected —
that regresses the plan-25 constant-folding wins for straight-line code; the bug is the
*scope* of the fold across the loop back-edge, not folding itself.

## Phases

### Phase 1 — failing test

- [x] Add the reproduction (and a string-concat variant, `WHILE (log & …)`), asserting
      termination and correct output. Confirm the infinite loop today (bound with an
      iteration cap as above).
- [x] Add the `WHILE c < 3` and `DO…UNTIL` cases as passing guards.

### Phase 2 — the fix

- [x] Move the constant clear before condition lowering in the `WHILE` arm (or clear
      body-assigned locals).

### Phase 3 — validation

- [x] Regenerate codegen goldens; expect deltas only where a `WHILE` condition previously
      folded a loop-variant local. `scripts/artifact-gate.sh`, `scripts/test-accept.sh`.
- [x] Re-run the reproduction; it must terminate with `c=3`.

## Validation Plan

- Regression test(s): the `toString`-condition termination test + string-concat variant.
- Runtime proof: build and run — `terminated, c=3`.
- Doc sync: none expected.
- Full suite: `scripts/artifact-gate.sh`, `scripts/test-accept.sh`.

## Summary

The `WHILE` arm folds the loop condition before clearing loop-entry constants and re-tests
that frozen fold every iteration, so a condition like `WHILE toString(c) <> "3"` never
sees `c` change and loops forever. The fix is `DoUntil`'s ordering — clear constants before
lowering the condition. Numeric and invariant-local conditions are unaffected.

## Resolution

Fixed. `NirOp::While` in `src/target/shared/code/builder_control.rs` now calls
`self.clear_local_constants()` **immediately after emitting `while_loop` and before**
`self.lower_value(condition)?` (the post-loop clear is kept), mirroring the `DoUntil`
sibling. A string-producing condition therefore reads the live local each iteration
instead of folding the loop-entry constant. A block comment records the rule.

Sibling-loop audit (all other loop forms are safe, no change needed):

- **`DoUntil`** — already clears constants before its body+condition (documented at the
  arm). Guard test `native_do_until_tostring_condition_reads_live_local` confirms it was
  and stays correct.
- **numeric `FOR`** (`lower_numeric_for`) — the induction variable is inserted with
  `constant: None`, and the synthesized condition is purely numeric (`i <= end`,
  `step >= 0`, `AND`/`OR`). The only folders that consult `local.constant` are
  `static_string_value` / `static_primitive_text`, which reject non-`String` `Const`s, so
  a numeric condition never folds a local — `FOR` cannot exhibit the defect even though its
  `clear_local_constants()` also runs after the condition. Left unchanged (matches the
  doc's non-goal).
- **`FOR EACH`** (`lower_for_each`) — the iterable is lowered once *above* the back-edge,
  the loop var is inserted `constant: None`, and `clear_local_constants()` runs before the
  body; the loop test is a slot-loaded `remaining == 0` count, never a user string fold.
  Safe.
- General local reads are also immune: `lower_value_inner`'s `NirValue::Local` arm always
  loads from the slot — only the two string folders consult `local.constant`, so the defect
  is confined to string-producing conditions emitted above a back-edge, which only `WHILE`
  did.

Tests (in `tests/native_loop_runtime.rs`, all build+run the produced binary):
`native_while_tostring_condition_reads_live_local`,
`native_while_concat_condition_reads_live_local` (the two that FAILED pre-fix with
`looping c=21` / now print `terminated c=3`), plus guards
`native_while_integer_condition_terminates` and
`native_do_until_tostring_condition_reads_live_local`. Each guards exit-0 with a bounded
iteration counter so a buggy binary yields a well-formed wrong result rather than hanging.

Runtime proof: the doc's reproduction prints `terminated, c=3` (exit 0) after the fix;
`cargo test --test native_loop_runtime` → 12 passed. Reverting the one-line reorder made
exactly the two new `WHILE` tests fail (`looping c=21`), the guards still passed.

Goldens: no acceptance `.mfb` has a `WHILE` whose condition folds a local
(`toString`/`&`/`upper`/`lower`/`caseFold`/`normalizeNfc`), and `clear_local_constants()`
emits no instructions itself, so no codegen/native/IR goldens are expected to shift.
