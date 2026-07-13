# bug-156 — `RETURN <literal>` and `WITH { field := <literal> }` don't coerce the literal to the expected type (Fixed/Money read as raw Integer bits)

Last updated: 2026-07-12
Effort: small (<1h)
Severity: HIGH
Class: Correctness (silent wrong value from a core language construct)

Status: Open
Regression Test: _(none yet)_

Two IR-lowering sites lower a sub-expression with `lower_expression` (no expected
type) where the surrounding context has a known declared type, so an unsuffixed
numeric literal is classified as `Integer` and its raw bits are then
reinterpreted as the destination's type. For `Fixed` and `Money` (scaled
representations) this yields a wildly wrong value with no error. The single
correct behavior: the literal must be coerced to the declared type, exactly as
`LET`/constructor-arg lowering already does via `lower_expression_with_expected`.

**Runtime-confirmed** (this compiler, macOS aarch64):

```
IMPORT io
FUNC getf AS Fixed
  RETURN 5
END FUNC
FUNC main AS Integer
  LET v AS Fixed = getf()
  io::print(toString(v))     ' prints 0.00  (WRONG — expected 5.00)
  LET a AS Fixed = 5
  io::print(toString(a))     ' prints 5     (correct — LET coerces)
  RETURN 0
END FUNC
```

References:

- `src/ir/lower.rs` — `lower_expression_with_expected` threading in `LET`,
  constructor args (`lower_constructor_args`), and union wrapping.
- goal-03 review (runtime-verified).

## Failing Reproduction

Site 1 — `RETURN`: the program above. `getf()` → `0.00` (expected `5.00`);
`Money`-returning `RETURN 3` → `0.00`.

Site 2 — `WITH`-update:

```
TYPE Point
  x AS Fixed
  y AS Fixed
END TYPE
' LET p = Point(1, 2) ; LET q = WITH p { x := 9 }
' q.x prints 0.00 (expected 9.00); q.y prints 2.00 (untouched, correct)
```

- Observed: the updated/returned `Fixed`/`Money` field is `0.00`.
- Expected: `9.00` / `5.00`.

Contrast (correct today): `LET a AS Fixed = 5`, and constructor args
`Point(9, 2)` — both thread the expected type. `Byte` returns survive (raw ==
integer); suffixed literals (`5F`) survive (`classify_literal` types them).

## Root Cause

- `src/ir/lower.rs:905` (`Statement::Return`): lowers the value with
  `lower_expression(value, ...)` then `wrap_union_value` (which handles only
  union wrapping, not numeric coercion). No expected type is passed, so `5` is an
  `IrValue::Const { type_: "Integer", value: "5" }` inside a `Fixed` function;
  codegen reinterprets the Integer bits as a Fixed → `0.00`.
- `src/ir/lower.rs:3302` (`Expression::WithUpdate`): each update value is lowered
  with `lower_expression(&update.value, ...)`, never consulting the record
  field's declared type (constructor args do, via `lower_constructor_args`
  passing `field.type_`).

## Goal

- `RETURN <literal>` in a `Fixed`/`Money`/`Float` function coerces the literal to
  the declared return type (identical result to assigning it to a typed local
  and returning that).
- `WITH r { f := <literal> }` coerces the update value to field `f`'s type.

### Non-goals (must NOT change)

- Union wrapping behavior; suffixed-literal and `Byte`/`Integer` paths (already
  correct); the constructor-arg path.

## Blast Radius

- `Statement::Return` (`lower.rs:905`) — fixed here.
- `Expression::WithUpdate` (`lower.rs:3302`) — fixed here (same root cause).
- Audit any other `lower_expression(` call whose result flows into a
  declared-typed slot without a subsequent coercion; the LET/constructor/arg
  paths already use `lower_expression_with_expected` and are unaffected.

## Fix Design

Site 1: lower the return value with
`lower_expression_with_expected(value, context.current_return_type.as_deref(), locals, context)`
before `wrap_union_value`. Site 2: resolve the field type
(`context.type_index.record_field_type(&type_, &update.field)`) and lower each
update value with `lower_expression_with_expected` using it.

## Validation Plan

- Regression: runtime tests asserting `getf() == 5.00`, a `Money` `RETURN`, and a
  `WITH { x := 9 }` field read; plus the negative-control (`LET`/constructor still
  correct). Must fail pre-fix.
- Full `scripts/test-accept.sh` (IR goldens for RETURN/WITH shift — confirm the
  delta is only the added Const type coercions).

## Summary

A missing expected-type thread on exactly two lowering sites silently corrupts
`Fixed`/`Money` values returned or WITH-updated from unsuffixed literals. Both
share one fix (`lower_expression_with_expected`) and are runtime-observable.
