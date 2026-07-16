# bug-196: generic instantiation binds type-params in source arg order, ignoring named-arg reordering

Last updated: 2026-07-14
Effort: medium (1h–2h)
Severity: MEDIUM
Class: correctness

Status: Fixed (2026-07-15) — before feeding `arg_types` to `instantiate_function`,
monomorph now reorders them into the template's declared-parameter order (new
`arg_types_in_param_order` helper, mirroring `normalize_local_call_arguments`) when
the call uses named arguments, so each template type-param binds against the type
of the argument that actually fills its slot. Only applied when every slot is
filled; positional calls are unaffected.
Regression Test: verified at runtime — `FUNC f OF A, B(x AS A, y AS B) AS A` called
as `f(y := "s", x := 1)` binds `A=Integer` and returns `1` (previously bound
`A=String` → spurious type error / wrong symbol).

`instantiate_function` zips a template's params (declaration order) against the
call's argument types in **source** order, ignoring named-argument reordering.
When named args are passed out of declaration order, template type-params bind to
the wrong argument types, producing a type-incorrect concrete function and a
wrong mangled symbol — while IR lowering (`normalize_local_call_arguments`)
correctly reorders the values to param order, so the two disagree.

## Failing Reproduction

```
FUNC f OF A, B(x AS A, y AS B) AS A
  RETURN x
END FUNC
...
LET r = f(y := "s", x := 1)
```
Observed: monomorph binds `A=String, B=Integer`, emits `f$String$Integer(x AS
String, y AS Integer)` → a spurious post-monomorph type error on valid code and
a wrong symbol. Expected: `A=Integer, B=String` (bind by declared-parameter
position after name resolution).

## Root Cause

`src/monomorph/lower.rs:1021-1027` builds `arg_types` in call/source order and
feeds them to `instantiate_function` (`:499`), which zips against
`template.params` without reordering named args into declared-parameter order.

## Non-goals

- Do not change positional-argument instantiation (already correct).
- Do not change `normalize_local_call_arguments` (the value-reorder is correct).

## Blast Radius

- `instantiate_function` / `arg_types` construction. Positional calls unaffected.

## Fix Design

Reorder `arguments`/`arg_types` into declared-parameter order (mirroring
`normalize_local_call_arguments`) before zipping with `template.params`.
