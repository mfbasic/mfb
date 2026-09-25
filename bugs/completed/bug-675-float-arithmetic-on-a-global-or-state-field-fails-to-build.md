# bug-675: `Float` arithmetic on a global or a `STATE` field fails to build ("has no data object")

Last updated: 2026-09-21
Effort: small (<1h)
Severity: HIGH
Class: Correctness (a valid program does not compile)

Status: Fixed
Regression Test: `tests/rt-behavior/scope/float-arith-on-global-and-state-field-valid`

**STATUS: FIXED.** Found by plan-145-A Phase 2: the field-kind harness's `Float`
line failed to build at S5 and at every `STATE` site.

## Failing Reproduction

```basic
IMPORT fs
IMPORT io

TYPE P
  a AS Float
  b AS Float
  n AS Integer
END TYPE

FUNC main() AS Integer
  RES h AS fs::File STATE P = fs::openFile("/dev/null")
  h.state.b = h.state.b + 0.5
  io::print(toString(h.state.b))
  RETURN 0
END FUNC
```

- Observed (at `4e0c50a8b`): `error: native code string literal 'Floating-point
  arithmetic overflowed to infinity.' has no data object while lowering state assign
  h`. The same with a module-level `MUT gR AS Rec` and `gR = WITH gR { b := gR.b +
  0.5 }` in a `SUB` ("while lowering store global gR").
- Expected: `0.50`.

A program escapes the failure only when some other `Float` expression in it happens
to register the messages (a `LET f AS Float = f0 + 0.5` anywhere does).

## Root Cause

A module that may raise a float-arithmetic error needs the error's message strings
as data objects; `module_may_emit_float_numeric_error`
(`src/codegen/engine/analysis/module_analysis.rs`) decides that by typing each
`Binary` operand with `static_nir_value_type`
(`src/codegen/engine/types/type_utils.rs`). That oracle could type neither operand
above:

- a global read inside a function reaches NIR as `Global { type_: "" }` (untyped),
  so `gR.b` had no type;
- `h.state` had no member arm, so `h.state.b` had no type.

The `Binary` was therefore not known to be `Float`, no message was registered, and
the lowering — which does know the types — then emitted the overflow check and found
no data object.

## Fix

- `FieldTypes` records every module global's declared type (`module_field_types`),
  and the oracle types an untyped `Global` from it.
- The oracle's `MemberAccess` arm types `h.state` as the handle's `STATE` payload
  (the same arm the IR and, since bug-671, the monomorphizer have).

Both only let the recognizer register strings it used to miss; nothing it
registered before changes.

## Validation

- RED: the regression fixture fails to build on `main`'s compiler with the message
  above; a `STATE`-only variant fails the same way ("while lowering state assign h"),
  so both halves are needed. GREEN: it prints `1.50 3.50 4.00` / `-0.25 1.50`.
- `scripts/artifact-gate.sh target/release/mfb all` → "1483 tests, 1658 build(s),
  2096 golden(s) checked, 0 diff(s)": no committed fixture's golden moved.
- The full suites run in plan-145's final gate (plan-145-I Phase 3).
