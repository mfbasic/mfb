# bug-198: a top-level (global) FUNC-valued binding is rejected in call position

Last updated: 2026-07-14
Effort: small (<1h)
Severity: MEDIUM
Class: correctness

Status: Open
Regression Test: tests/ (calling a global FUNC-valued binding builds & runs)

`resolve_callable` accepts a callee that is a local binding or a visible
top-level function, but never checks `top_level_visible_in_file`, so a **global**
variable holding a function value is rejected as an unknown callable — even though
the identical pattern with a local binding compiles and runs, and passing the same
global as a value works.

## Failing Reproduction

```
LET GLOBAL_ADDER AS FUNC(Integer) AS Integer = add1   ' top level
FUNC add1(n AS Integer) AS Integer ... END FUNC
SUB main()
  LET r = GLOBAL_ADDER(5)     ' error 2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER
END SUB
```
Observed: `Callable \`GLOBAL_ADDER\` is not a top-level function.` The same code
with a local `LET localAdder ...` builds and runs; `LET g = GLOBAL_ADDER` (value
position) also works. Expected: the call resolves and defers callability to
typecheck.

## Root Cause

`src/resolver/resolution.rs:1143` `resolve_callable` (fall-through at 1160-1169)
checks `locals.contains_key(callee)` and `function_visible_in_file(callee)` but
not `top_level_visible_in_file(callee)`. `resolve_identifier` (`:1182`) does
consult it, which is why value position works and only call position is broken.

## Non-goals

- Do not make non-callable globals callable — defer the callability check to
  typecheck, as the local-binding path already does.

## Blast Radius

- `resolve_callable` only.

## Fix Design

Add `|| self.top_level_visible_in_file(file, callee)` to the acceptance set in
`resolve_callable`, mirroring `resolve_identifier`.
