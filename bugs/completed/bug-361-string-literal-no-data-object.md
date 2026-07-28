# bug-361: two string-literal shapes reach codegen with no data object and abort the build with an uncoded internal error

Last updated: 2026-07-19
Effort: small
Severity: MEDIUM (a valid program fails to build; the message is an internal
error with no rule code, no file, and no line)
Class: Compiler / native codegen

Status: Open — found and root-caused, not fixed
Regression Test: none yet. Two committed man-page examples reproduce it verbatim
(`src/docs/man/flow/match.md` and `src/docs/man/builtins/general/typeName.md`),
which is how it was found; a fixture per shape belongs in `tests/rt-behavior/`.

`data_objects.rs` walks the NIR to collect every string literal that needs a
read-only data object emitted for it. Two shapes are missed. Each aborts the
build with

```
error: native code string literal '<text>' has no data object while lowering <op>
```

— no rule code, no path, no line, so a user has nothing to act on and no way to
tell which of their string literals is at fault.

## Reproductions

Both are valid MFBASIC and both fail on `macos-aarch64` at `d2a0aec` (and are
ISA-independent — the gap is in shared NIR collection, above instruction
selection).

**A — a multi-literal `CASE`:**

```
IMPORT io

SUB main()
  LET g AS String = "A"
  MATCH g
    CASE "A"
      io::print("a")
    CASE "B", "C"
      io::print("bc")
    CASE ELSE
      io::print("?")
  END MATCH
END SUB
```

```
$ mfb build .
error: native code string literal 'B' has no data object while lowering match
```

A single-literal `CASE "A"` compiles. Only the multi-literal form fails.

**B — a constant-folded concatenation whose operand folded from a call:**

```
IMPORT io

SUB logType(value AS String)
  io::print("type=" & typeName(value))
END SUB

SUB main()
  logType("hello")
END SUB
```

```
$ mfb build .
error: native code string literal 'type=String' has no data object while lowering eval call io.print
```

Note the literal in the message — `type=String` — is a string that appears
nowhere in the source. `typeName(value)` folds to `"String"` at compile time and
the `&` then folds the pair into one literal, which is created *after* the
collection pass has run.

Neither `"type=" & someRuntimeString` nor `typeName(v)` alone fails; it is the
fold-to-a-new-literal that does.

## Root cause

**A** is a missing match arm. `src/target/shared/code/data_objects.rs:827-838`
walks `NirOp::Match`, and its pattern walk is:

```rust
if let NirMatchPattern::Value(value) = &case.pattern {
    collect_string_values_from_value(value, values, constants, types);
}
```

`NirMatchPattern` has three variants (`src/target/shared/nir/mod.rs:234-238`):
`Else`, `Value`, and **`OneOf(Vec<NirValue>)`**. `OneOf` is the multi-literal
form and is never walked, so none of its literals is collected. The `if let`
silently skips it — an exhaustive `match` here would have been a compile error
the day `OneOf` was added.

Note the neighbouring comment: the `WHEN` guard walk immediately below was added
by bug-118 for exactly this class of miss. This is the same bug, one variant over.

**B** is an ordering problem, not a missing walk: the collection pass runs before
(or without re-running after) the constant fold that produces the combined
literal, so the folded value is never a candidate. Confirm which by checking
whether the fold happens in `ir::lower` or later in NIR lowering — if the folded
literal exists in the NIR the pass sees, this is also a walk gap; if it is
produced during code lowering, the pass must run after it or the lowering must
register the object on demand.

## Suggested fix

- **A:** make the pattern walk an exhaustive `match` over `NirMatchPattern` and
  handle `OneOf` by walking every element. Exhaustiveness is the real fix — it
  makes the next variant a build error rather than a silent miss.
- **B:** determine where the fold happens and either collect after it or have
  `builder_emit_helpers.rs:134`/`:548` register a missing literal on demand
  instead of erroring.
- **Both:** the error at `builder_emit_helpers.rs:134` and `:548` should be a
  coded diagnostic. As an internal invariant failure it should arguably be an
  ICE with the offending op's source location, not a bare `Err(String)` that
  surfaces to a user as `error:` with no location.

## Blast radius

Codegen only. Nothing about the accepted language changes: both programs are
already valid and already type-check — they fail at the last stage. No golden
churn is expected for programs that build today, since the fix only adds data
objects for literals that currently abort the build.

## Not caused by the man-page migration

Found during bug-336's example-compilation sweep, but pre-existing: both repros
fail identically on a compiler built before that work.
