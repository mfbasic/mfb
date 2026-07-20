# bug-363: float arithmetic on a record field aborts the build — the float error messages get no data object

Last updated: 2026-07-19
Effort: small
Severity: MEDIUM (a valid program fails to build; the message is an internal
error with no rule code, no file, and no line)
Class: Compiler / native codegen

Status: Open — found and root-caused, not fixed
Regression Test: none yet. One committed man-page example reproduces it
(`src/docs/man/flow/match.md:49`), which is how it was found; a fixture belongs
in `tests/rt-behavior/codegen/`.

## Reproduction

Nine lines. No `MATCH`, no union, no `math::` call — a Float multiply where one
operand is a *record field* is enough.

```
TYPE Circle
  radius AS Float
END TYPE

FUNC area(c AS Circle) AS Float
  RETURN 3.14159 * c.radius
END FUNC

SUB main()
END SUB
```

```
$ mfb build .
error: native code string literal 'Floating-point arithmetic overflowed to infinity.' has no data object while lowering return
```

Replacing `c.radius` with a plain `Float` local or parameter compiles. It is the
`MemberAccess` operand that does it.

## Root cause

`module_may_emit_float_numeric_error`
(`src/target/shared/code/module_analysis.rs:210`) decides whether the module
needs the four `ERR_FLOAT_*` message data objects emitted
(`data_objects.rs:92-99`). It answers by asking
`value_may_emit_float_arithmetic_error` whether any arithmetic *results in*
`Float`, which types both operands through `static_nir_value_type`.

`static_nir_value_type` cannot type a `NirValue::MemberAccess`: resolving
`c.radius` needs the module's record-type table, which the predicate is never
given. So the operand types as `None`, `numeric_binary_result_type` is never
consulted, the predicate answers **false**, and no `ERR_FLOAT_*` object is
emitted — while the *lowering*, which does know the field's type, goes on to
emit the overflow check that loads the message and aborts.

This is the same asymmetry as bug-361B (the collection pass's model of types
being strictly weaker than the builder's), but a different pass, a different
predicate, and a different missing fact — bug-361's fix does not touch it.

## Why it is narrow in practice

The flag is per **module**, not per expression, so any *other* float-typed
arithmetic anywhere in the module (or any `math::sqrt`-family call) turns the
messages on and masks this. It fires only when a module's sole
float-error-capable arithmetic runs through a record field. That is why it has
survived: it needs a small, focused program to surface.

## Suggested fix

Thread the module's record-type table into the predicate so
`static_nir_value_type` can resolve `MemberAccess` to its declared field type,
then confirm `3.14159 * c.radius` types as `Float`. Check the sibling walks in
the same file for the same gap before fixing (`ops_use_unicode_runtime_tables`
uses the same `static_nir_value_type` seam).

The `Err(String)` at `builder_emit_helpers.rs:134`/`:548` should also become a
coded diagnostic carrying the offending op's source location, rather than
surfacing to a user as a bare `error:` with no rule code, path, or line — the
same follow-up bug-361 records.

## Blast radius

Codegen only; no accepted-language change. Turning the predicate on for more
modules adds the four `ERR_FLOAT_*` data objects to programs that currently omit
them, so **expect golden churn** in any `.ncode`/`.nobj` fixture whose module
does float arithmetic solely through record fields. Verify each such diff is an
added data object and nothing else.

## Found by

`scripts/check-man-examples.py` during bug-361's fix (2026-07-19), alongside the
two shapes bug-361 documents. Filed separately because the root cause is a
different pass with a different missing fact.
