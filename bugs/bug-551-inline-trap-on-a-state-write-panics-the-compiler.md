# bug-551: an inline `TRAP` on a `RES … STATE` write panics the compiler

Last updated: 2026-09-06 (FIXED)
Effort: small
Severity: HIGH — a compiler panic, on a program the parser accepts deliberately
Class: Correctness (internal panic) / Error model

Status: **FIXED.** Reproduced with the release compiler and attributed to
`main` (a clean `git archive main` build in `/tmp`), so it predates this branch.

## Reproduction

```basic
IMPORT fs
IMPORT io

TYPE Counter
  n AS Integer
END TYPE

FUNC risky(n AS Integer) AS Integer
  IF n < 0 THEN FAIL error(77050002, "risky")
  RETURN n * 2
END FUNC

FUNC main() AS Integer
  RES f AS fs::File STATE Counter = fs::createTempFile()
  f.state = Counter[0]
  f.state.n = risky(8) TRAP(e)
    RECOVER 0 - 1
  END TRAP
  io::print(toString(f.state.n))
  fs::close(f)
  RETURN 0
END FUNC
```

```
$ mfb build .
Building probe_statetrap (executable) for macos-aarch64

thread 'mfb' panicked at src/ir/lower.rs:4786:13:
internal error: entered unreachable code: inline TRAP must be lowered as a statement value
```

Both `RES … STATE` write forms panicked — the whole-state
`f.state = <expr> TRAP …` as well as the field form above.

## Why it happened

`ir::lower`'s `lower_statement` desugars an inline `TRAP` when it finds one at
the top of a statement's value. It had an arm for a binding, an assignment and a
bare expression statement. It had none for `HirStatement::StateAssign`, so the
`Trapped` fell through to `lower_expression`, where every inline trap is by
construction impossible:

```rust
HirExpression::Trapped { .. } => {
    // Inline traps are only constructed as the value of a binding,
    // assignment, or bare-expression statement, where `lower_statement`
    // desugars them directly; they never reach value lowering.
    unreachable!("inline TRAP must be lowered as a statement value")
}
```

The comment is the bug: a state write is a fourth position, and the parser
produces one. `ast::stmt` even skips the statement terminator for exactly this
shape (`if !matches!(value, Expression::Trapped { .. })`), so the grammar
accepts it on purpose. Two halves of the compiler disagreed about how many
statement positions carry a trap, and the disagreement was an `unreachable!`
rather than a diagnostic.

## The fix

A fourth `InlineTrapTarget::StateAssign { resource }`, emitting an
`IrOp::StateAssign` from the shared value slot after the branch, exactly as the
`Assign` target emits an `IrOp::Assign`.

The FIELD form needed one more decision. `ast::stmt` desugars
`f.state.n = <expr>` into `f.state = WITH f.state { n := <expr> }`, so the trap
ends up buried inside the update. The obvious fix — hoisting it at parse time to
cover the whole `WITH` — builds, and is wrong:

```
error[2-203-0067 TYPE_RECOVER_TYPE_MISMATCH]: RECOVER has type Integer,
expected Counter.
```

That makes `RECOVER` owe a whole STATE RECORD for a program that named one
field, which is the desugar leaking into the surface language — and for a state
record with several fields there is no way to write it that does not discard the
others. `mfb spec language error-model` §8.4 scopes an inline TRAP to "the whole
expression", and the whole expression the author wrote is the field's value.

So the trap stays on the field's value, and `lower_statement`'s `StateAssign`
arm recognises the single-field `WITH` with a `Trapped` inside: it binds the
trap's result to a temporary and applies the update to that. `RECOVER` supplies
the field's type, and the record's other fields survive.

Measured, both forms and both directions:

    field ok   n=16 tag=start
    field bad  n=-1 tag=start        <- the sibling field survives
    state ok   n=5  tag=fresh
    state bad  n=-2 tag=recovered

Covered by `tests/rt-behavior/control-flow/inline-trap-positions-rt`, which puts
an inline TRAP in every position the grammar allows and over every expression
shape the lift walks, in the in-process corpus.

## How it was found

`planning/tests.md` (the per-file coverage gate task). `ir/lower.rs` was 162
lines short and its gap is the inline-TRAP lifting internals — the `Eval` target
for a discarded trapped call, the `Checked` root operator, the closure and union
arms of `scan_trap_call`. Writing one fixture that puts a trap in every legal
position produced this panic on the first build.
