# bug-191: parse_type_name recurses without a depth cap → stack-overflow DoS on nested type annotation

Last updated: 2026-07-14
Effort: small (<1h)
Severity: HIGH
Class: memory-safety (compile-time DoS)

Status: Open
Regression Test: tests/syntax/ (deeply nested type annotation → bounded parse diagnostic, not abort)

`parse_type_name` recurses for grouped types, template args, Map/List/Result/
Thread element types, and function-type params/return, but never bumps the
`expr_depth`/`MAX_EXPR_DEPTH` guard that expression parsing uses. A deeply
nested **type** annotation therefore drives uncapped native recursion and
overflows the stack, aborting the compiler with `fatal runtime error: stack
overflow` and no MFB diagnostic.

This is a third parser asymmetry distinct from the known recursion gaps:
expression nesting (FE-01) is guarded, statement nesting is bug-183, and
**type-name** nesting is guarded by neither.

## Failing Reproduction

```
mfb build <project with> LET x AS List OF List OF List OF … Integer   (~100k OF)
```
or grouped `LET x AS (((((…)))))`, nested `Map OF K TO Map OF K TO …`, or a
nested `FUNC(...) AS FUNC(...) AS …` type. Observed: `thread 'main' has
overflowed its stack / fatal runtime error: stack overflow, aborting`, non-zero
abort, no diagnostic. Expected: a bounded parse diagnostic. Attacker vector: a
dependency package's source.

## Root Cause

`src/ast/expr.rs:539` `parse_type_name` — recursive descent at line 550
(grouped), 604–607 (template args), and the Map/List/Result/Thread/function-type
arms, none of which enter the depth guard.

## Non-goals

- Do not change accepted type syntax or the parsed AST for valid inputs.
- Do not lower `MAX_EXPR_DEPTH`; add a matching type-depth bound.

## Blast Radius

- `parse_type_name` only. Same crash *class* as bug-183 (statements) and the
  bug-191/193 sibling (TGROUP), but a distinct unguarded recursive function.

## Fix Design

Wrap the recursive `parse_type_name` entry in the same `enter_expr`/`leave_expr`
depth guard (or an analogous type-depth counter capped ~256) and emit a bounded
parse diagnostic when exceeded.
