# bug-937: one unknown record field produces cascading `TYPE_CALL_ARGUMENT_MISMATCH` (overloaded builtin) and a duplicated `TYPE_UNKNOWN_VALUE`

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (diagnostic cascade)

Status: Open
Regression Test: tests/syntax/types/unknown-field-no-cascade-invalid (new; name suggested — golden pins exactly one diagnostic per bad line)

`architecture type-inference` says an `Unknown` operand is accepted permissively "so a single upstream
error does not cascade into spurious operator-mismatch diagnostics", and both compatibility predicates
"accept an `Unknown` operand to suppress cascading diagnostics". That holds for operators
(`"a" & p.nope` reports only the unknown field), but not for an overloaded builtin argument or an
unannotated binding:

- `toString(p.nope)` reports `TYPE_UNKNOWN_FIELD` **and** `TYPE_CALL_ARGUMENT_MISMATCH` ("Call to
  `toString` has argument type(s) (Unknown), expected Integer, Float[, Byte], …").
- `LET q = p.nope + 1` reports `TYPE_UNKNOWN_FIELD` **and two** identical
  `TYPE_UNKNOWN_VALUE` ("value type could not be determined") on the same line.
- In a function-level `TRAP` handler, `RETURN "…" & toString(e.source.column)` (a field `ErrorLoc` does
  not have) gives three errors: the unknown field, the `toString` mismatch on `(Unknown)`, and
  `TYPE_UNKNOWN_VALUE` "RETURN value does not have a known type".

**Correct behavior:** one diagnostic per root cause — `TYPE_UNKNOWN_FIELD` on each line — with the
builtin-overload check and the binding/return type check treating `Unknown` as permissive, as the
operator checks already do.

References:

- `src/docs/spec/architecture/22_type-inference.md` "Structural Assignability" and "Numeric and
  Ordering Predicates" (`Unknown` flows through every predicate as permissive).
- Found in hardening batch B12 (architecture/22_type-inference).

## Failing Reproduction

```
IMPORT io
TYPE P
  x AS Integer
END TYPE
FUNC main() AS Nothing
  LET p AS P = P[1]
  io::print(toString(p.nope))
  LET q = p.nope + 1
  io::print("a" & p.nope)
END FUNC
```

`mfb build` (macOS aarch64, /tmp/hardening/mfb at HEAD) — error lines only:

- Observed:
  ```
  main.mfb:7 error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]
  main.mfb:8 error[2-203-0043 TYPE_UNKNOWN_VALUE]
  main.mfb:7 error[2-203-0045 TYPE_UNKNOWN_FIELD]
  main.mfb:8 error[2-203-0045 TYPE_UNKNOWN_FIELD]
  main.mfb:8 error[2-203-0043 TYPE_UNKNOWN_VALUE]
  main.mfb:9 error[2-203-0045 TYPE_UNKNOWN_FIELD]
  ```
- Expected: exactly `TYPE_UNKNOWN_FIELD` on lines 7, 8 and 9.

Contrast: line 9 (`&` operator with an `Unknown` operand) already emits only the root cause.

## Root Cause

Hypotheses, in order:

1. The builtin call checker (`src/ir/shape.rs:check_call_shape` for builtins / the registry overload
   matcher) selects among `toString`'s overloads by exact argument types and has no `Unknown` escape,
   so `(Unknown)` matches no overload and the mismatch fires. The user-function path and the operator
   path short-circuit on `Unknown`.
2. `TYPE_UNKNOWN_VALUE` for an unannotated `LET` is emitted both by the shape pass and by
   `src/ir/verify` (or twice by one pass over the binding and its initializer) without checking whether
   the initializer's type is `Unknown` because of an already-reported error.

## Goal

- The reproduction yields exactly three diagnostics, all `TYPE_UNKNOWN_FIELD`.

### Non-goals (must NOT change)

- A genuinely untypeable unannotated binding with no upstream error still gets `TYPE_UNKNOWN_VALUE`.
- A real `toString` mismatch (e.g. `toString(1, 2, 3)`) still reports `TYPE_CALL_ARGUMENT_MISMATCH`.

## Blast Radius

- Every overloaded builtin (`toString`, `len`, `math::*`, …) fed an `Unknown` — same cascade, latent.
- `RETURN <unknown-typed>` — measured (the `e.source.column` case above).
