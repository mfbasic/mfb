# bug-744: a `FUNC … AS Nothing` call in value position panics native codegen

Last updated: 2026-09-26
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: tests/runtime/rt_nothing_func_value_position.rs

Owner report: `LET y = f(1)`, where `f` is `FUNC f(x AS Integer) AS Nothing`, makes
`mfb build` panic with `` `void` is not a register operand (bug-721) `` in
`src/target/shared/abi.rs`. The same happens for every position that consumes the
call's value: `MUT`, reassignment, an argument, an operand, `RETURN f(1)` followed by
a use, a record-field initializer, `LET y AS Nothing = …`, a lambda whose body is the
call, a call through a `FUNC` value (including a `SUB` named as a value), and a
built-in declared `AS Nothing` (`LET y = io::print("x")`). A sibling shape is silent
instead: `LET y = f(1) TRAP(e) … RECOVER END TRAP` builds and runs, but `y` is not
`NOTHING` (`IF y = NOTHING` is false) — it holds whatever the value register last held.

**The single correct behavior:** a `FUNC … AS Nothing` call is an ordinary value
expression. It runs the call and yields the unit value `NOTHING` in every value
position; the program builds and `y = NOTHING` is true. Only a *`SUB`* call is barred
from value position, and that stays `TYPE_SUB_HAS_NO_VALUE`.

This is not a front-end diagnostic. The spec makes it a legal program:

- `mfb spec language types` §4.6: "`Nothing` is the unit type. It has one value,
  written `NOTHING`." It is copyable, comparable and defaultable (§4 tables).
- `mfb spec language functions` §6: "Only `FUNC` (returns a value) and `SUB` (no
  value)." `f` is a `FUNC`.
- `mfb spec language subs` §7: "A `SUB` call may not be used in value position" —
  the only value-position ban, keyed on the callee's `sub` kind
  (`src/ir/verify/values.rs:check_value_depth`, `TYPE_SUB_HAS_NO_VALUE`). The same
  section says naming a `SUB` yields a `FUNC(...) AS Nothing` value, so a call through
  that value is a `FUNC` call.
- `mfb spec language error-model` §8 rule 13: `RECOVER` "carries a value iff that type
  is not `Nothing`" — the trapped `Nothing` expression still produces `NOTHING`.

References:

- `bugs/completed/bug-721-windows-arena-free-list-corrupts-with-a-worker-beside-canvas.md`
  — introduced the typed `abi::Reg` whose `From<&Operand>` panics on text. It did not
  cause this bug; it changed how it fails (see matrix).
- `.ai/codegen-invariants.md` "Registers are typed values, never text (bug-721)".

## Failing Reproduction

```
mfb init let && cd let
cat > src/main.mfb <<'EOF'
IMPORT io

FUNC f(x AS Integer) AS Nothing
  io::print("f")
END FUNC

SUB main()
  LET y = f(1)
  io::print("done")
END SUB
EOF
mfb build .
```

- Observed (HEAD 5c4ea72e3): `thread 'mfb' panicked at src/target/shared/abi.rs:243:14:`
  `` `void` is not a register operand (bug-721) ``, exit 101.
- Expected: builds; prints `f`, `done`.

| Compiler | Result |
| --- | --- |
| HEAD 5c4ea72e3 (after bug-721) | panic, exit 101 ✗ |
| eee5df136 (bug-721's parent chain, before the typed-register foundation c42ab29e9) | `error: unknown AArch64 register 'void'`, exit 1 ✗ |

So the defect predates bug-721: the call has never worked in value position.

Contrast cases that already work: the same call as a statement (`f(1)`); `LET y =
NOTHING`; `LET y = sb(1)` for a `SUB` is correctly rejected (`TYPE_SUB_HAS_NO_VALUE`).

## Root Cause

A `Nothing`-typed call lowers to a `ValueResult` whose location is the text `void`
— no register. There are three producers, one per call shape, all with the same arm
`if result_type == ParameterType::Nothing { … location: Operand::from("void") }`:

- `src/codegen/engine/builder/builder_emit_helpers.rs:emit_call` (direct user call),
- `…:emit_function_value_call` (a call through a `FUNC` value / lambda),
- `…:emit_runtime_helper_call` (a runtime built-in such as `io.print`).

The callee's ABI agrees: `emit_return_exit_inner`
(`src/codegen/engine/control/builder_exits.rs`) does not set
`RESULT_VALUE_REGISTER` for a `Nothing` return, so there is no value to move out. That
is right for a statement, which is the only place the author expected the call. But
`lower_value` (`src/codegen/engine/value/builder_values.rs`) returns the `void`
result to *every* caller, and a value-position caller hands `&result.location` to a
register-operand constructor: `store_value_at` → `abi::store_u64` → `abi::reg` →
`<Reg as From<&Operand>>::from`, which panics on any text that is not a register
(bug-721's enforcement). Before bug-721 the text went into the instruction stream and
the backend rejected it (`unknown AArch64 register 'void'`).

No earlier stage rejects the program because it is valid (see above): the front end
checks only the `sub` kind.

The inline-`TRAP` sibling reaches a different consumer. `lower_inline_trap` binds the
trapped call as `Result OF Nothing` (`callResult`), and
`materialize_current_result` (`src/codegen/memory/arena/builder_arena_transfer.rs`)
boxes `RESULT_VALUE_REGISTER` as the payload — uninitialized for a `Nothing` callee.
`NirValue::ResultValue` then loads that payload (`ldr [result, #16]`), so `y` is
garbage instead of 0 and `y = NOTHING` (a register compare) is false.

## Goal

- Every value-position shape above builds and runs, and the value it reads back is
  `NOTHING` (`= NOTHING` is true).
- A statement-position `Nothing` call and a `Nothing` function's `RETURN` emit exactly
  what they emit today (no new instruction).
- `LET y = sb(1)` still reports `TYPE_SUB_HAS_NO_VALUE`.

### Non-goals (must NOT change)

- The `Nothing` callee ABI (no `RESULT_VALUE_REGISTER` write on a `Nothing` return):
  changing it would touch every `SUB` and every `Nothing` function in every binary.
- The front end. Rejecting the program with a new diagnostic is the tempting wrong fix
  — it would outlaw a program the spec defines.
- Statement-position codegen: materializing `NOTHING` for every `Nothing` call would
  add a dead instruction after every `SUB` call in every golden.

## Blast Radius

Found by `grep -rn 'from("void")' src/codegen` and `grep -rn '"void"' src/codegen`:

- `builder_emit_helpers.rs:emit_call` / `emit_function_value_call` /
  `emit_runtime_helper_call` — the three `void` producers. Fixed centrally in `lower_value`
  (below), so every consumer of these is covered at once.
- `builder_values.rs:NirValue::ResultValue` — reads a `Nothing` payload that was never
  written. Fixed (produces 0).
- `builder_values.rs:free_call_closure_temps` — skips parking a `Nothing` result.
  Unaffected: the materialization runs after it, so the `NOTHING` register is not
  live across its closure drops.
- `builder_error_emission.rs:store_pending_success_result` — already writes 0 for a
  `Nothing` result. Unaffected.
- `function_lowering.rs` ABI-inline epilogue (`result.location.render() != "void"`),
  `builder_values.rs` raw-inline `forEach` (`success.location == "void"`),
  `collections/func_for_each.rs` and `gen_flow.rs` (`action.location == "void"`) —
  these read a result that never goes through `lower_value`'s exit, or a `FUNC`-typed
  action. Unaffected; `LET y = collections::forEach(xs, sb)` was probed and binds
  `NOTHING` after the fix.
- ~40 built-in bodies in `src/codegen/builtins/**` returning `Operand::from("void")`
  — ABI-inline bodies consumed by the function-lowering epilogue above, not by a
  value-position caller. Unaffected.
- `List OF Nothing` / `Map OF K TO Nothing` / `Set OF Nothing` — the collection-literal
  sibling (`[f(1), f(2)]`) still fails, but NOT through this bug: `[NOTHING, NOTHING]`
  fails identically (`native code collection type 'List OF Nothing' is not
  supported`), at HEAD and before bug-721, because
  `type_utils.rs:collection_type_code` maps `Nothing` to `None`. Filed and fixed
  separately as **bug-745**.
- `collections::contains(List OF Nothing, NOTHING)`, `collections::transform` with a
  `Nothing` callback, `thread::start` of a worker returning `Nothing` — rejected in the
  front end with `TYPE_CALL_ARGUMENT_MISMATCH`, because strict validation
  (`src/codegen/registry/mod.rs:unify`) never binds a type variable to `Nothing`
  (bug-443's deliberate rule). A proper diagnostic, not a crash; unaffected here.

## Fix Design

Materialize at the one chokepoint every value passes through, and only where the
value is used. `lower_value` now ends with: if the result is `Nothing`-typed with a
`void` location and this frame is not a discarded value, allocate a register and
`mov` 0 into it — exactly what `lower_default_value(Nothing)` and a `NOTHING` constant
produce. "Discarded" is a one-shot flag, `CodeBuilder::value_discarded_armed`, set by
the `Eval` statement and by `emit_return_exit_inner` immediately before they lower
their value and consumed (`mem::take`) by the root frame, so every operand frame below
is a value position — the same one-frame discipline as `borrow_get_armed` (bug-592).
Both sites clear it after the call so it can never leak to an unrelated frame. Arming
it for *every* `RETURN` is exact because it changes only a `Nothing`/`void` result,
which the return path never reads.

`NirValue::ResultValue` of a `Result OF Nothing` emits `mov 0` instead of loading the
never-written payload.

Rejected: materializing in the three producers (dead `mov` after every `SUB` call,
tree-wide golden churn); setting `RESULT_VALUE_REGISTER = 0` in every `Nothing`
callee (ABI change, churn in every binary); a front-end diagnostic (contradicts §4.6/§7).

Expected golden shift: none for code that built before — the new `mov` appears only
where the build used to fail. The `ResultValue` change replaces one `ldr` with one
`mov` wherever a trapped `Nothing` call's payload is extracted.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] `tests/runtime/rt_nothing_func_value_position.rs`, one test per shape (12) plus
      two contrast tests. At HEAD (`MFB_TEST_EXE=target/debug/mfb cargo test --test
      rt_nothing_func_value_position`): 12 failed — 11 on the panic, the inline-`TRAP`
      one on its output (`f1\ndone\n`, no `eq`); the 2 contrasts pass.
- [x] Blast-radius audit above.

Acceptance: the new tests fail for the documented reason; the audit list is complete.
Commit: —

### Phase 2 — the fix

- [x] `lower_value` materializes `NOTHING` for a used `Nothing`/`void` result;
      `value_discarded_armed` set by `Eval` and `RETURN`.
- [x] `NirValue::ResultValue` of `Result OF Nothing` produces 0.

Acceptance: all 14 tests pass.
Commit: —

### Phase 3 — spec + full validation

- [ ] `mfb spec language types` §4.6 / `subs` §7: state that a `FUNC … AS Nothing`
      call is a value expression yielding `NOTHING`.
- [ ] Full `cargo test --release --no-fail-fast` and
      `scripts/artifact-gate.sh target/release/mfb all`.
- [ ] Re-run the owner's reproduction.

Acceptance: full suite green; any golden delta is exactly the intended change.
Commit: —

## Validation Plan

- Regression tests: `tests/runtime/rt_nothing_func_value_position.rs`.
- Runtime proof: each test builds, runs, and checks the program's stdout (`eq` printed
  only when the value read back equals `NOTHING`).
- Doc sync: `src/docs/spec/language/04_types.md` §4.6 and `07_subs.md`.
- Full suite: `cargo test --release --no-fail-fast`;
  `scripts/artifact-gate.sh target/release/mfb all`.

## Open Decisions

- Diagnostic vs. valid program. **Decided by the owner, 2026-09-26: valid program.**
  `Nothing` is a first-class type: `LET a = NOTHING` works, `LET y = f(1)` binds
  `NOTHING`, and a `Nothing` value may be passed as an argument (`LET b = same(a)`,
  test `a_nothing_value_round_trips_through_a_nothing_func`). plan-159-A (main,
  78e0eb6f0) had planned the opposite: a `2-203-0074` error for every `Nothing` call in
  value position. That diagnostic is dropped. The coordinator amended plan-159-A/B/D on main
  (038959645) to say so. The owner briefly chose plan-159-A's rule instead. That work was abandoned
  uncommitted.
- Correction to an earlier note in this doc: it claimed the `Eval`/`RETURN` discard
  marking "keeps `LAMBDA(x) -> io::print(x)` legal". That was wrong. The return path
  never read a `Nothing` location, and the lambda compiled at HEAD. The marking only
  stops `lower_value` from materializing a `NOTHING` that nobody reads.

## Summary

The risk was in choosing the behavior, not the code: the report reads like a missing
diagnostic, but the spec defines the program. The fix is two small codegen edits at
the chokepoint; statement calls, the callee ABI, and the front end are untouched.
