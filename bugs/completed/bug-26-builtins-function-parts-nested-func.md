# bug-26: `function_parts` mis-parses a `FUNC` type whose parameter is itself a multi-arg `FUNC`, causing false overload rejection of collection higher-order calls

Last updated: 2026-07-08
Effort: small (<1h)

`src/builtins/general.rs::function_parts` (`:591-600`) parses a `FUNC(...) AS ...`
type string non-recursively: `strip_prefix("FUNC(")`, then `split_once(") AS ")`
on the **first** occurrence, then `params.split(", ")`. This assumes the parameter
list contains no nested parentheses or commas. It is consumed by the
`collections::transform`/`filter`/`forEach`/`reduce` overload resolvers
(`general.rs:~519-565`) to match the mapper argument's type against the list
element type.

When the list element type is itself a **multi-parameter function value** — e.g.
`List OF FUNC(Integer, Integer) AS Integer` — the mapper type becomes
`FUNC(FUNC(Integer, Integer) AS Integer) AS X`. `function_parts` then splits at the
inner `") AS "`, producing `params = ["FUNC(Integer", "Integer"]` and
`returns = "Integer) AS X"` — garbage. `params[0]` no longer equals the extracted
element type, so `resolve_transform` (etc.) returns `None` and the compiler rejects
a well-typed call with "no matching overload".

The single correct behavior a fix produces: `function_parts` parses `FUNC(...)`
with paren-depth awareness so nested function-typed parameters are split correctly,
and higher-order collection calls over function-valued elements resolve.

Severity LOW / **latent**: this fires only if the language surface permits a
`List`/`Map` whose element/value type is a multi-argument function value. That
could not be confirmed from the builtins files alone; if such element types are
not expressible today, this is defense-in-depth against a future higher-order
extension. When triggered it is a deterministic compile-time **false rejection**
(no crash, no wrong runtime value).

References:

- `src/builtins/general.rs:591-600` (`function_parts`, non-recursive
  `split_once(") AS ")` + `split(", ")`).
- Consumers: `general.rs:~519-565` (`resolve_for_each`/`transform`/`filter`/
  `reduce`).
- Contrast: `map_parts` (`:586-589`) and `list_element` share the single-level
  assumption but split on ` TO `/element with no embedded comma, so they are safe;
  single-param FUNC element types (`FUNC(Integer) AS Integer`, no comma) also parse
  correctly.
- Found during goal-01 review of `src/builtins/**`.

## Failing Reproduction

Requires a `List`/`Map` whose element/value type is a `FUNC` with ≥2 params (if
expressible):

```
' hypothetical: a list of two-arg function values
LET fns AS List OF FUNC(Integer, Integer) AS Integer = ...
LET r = collections::transform(fns, someMapper)
```

- Observed: "no matching overload" — `function_parts` mangled the nested FUNC
  parameter type so the mapper's first param didn't match the element type.
- Expected: the call resolves (the element type is a valid function value).

Contrast (resolve correctly today): `List OF Integer`, `List OF String`,
`List OF Map OF String TO Integer` (Map value has no comma), and
`List OF FUNC(Integer) AS Integer` (single-param FUNC, no embedded comma).

## Root Cause

`function_parts` treats `FUNC(...) AS ...` as a flat string, splitting on the first
`") AS "` and on every `", "`. A parameter that is itself a parenthesized,
comma-bearing FUNC type breaks both assumptions.

## Goal

- `function_parts` correctly splits the parameter list and return type of a `FUNC`
  type even when a parameter is itself a multi-arg `FUNC` (paren-depth-aware).

### Non-goals (must NOT change)

- Parsing of non-nested FUNC types (correct today) — those results must be
  identical.

## Blast Radius

- `function_parts` and its four resolver consumers. A paren-depth-aware parse fixes
  them together.

## Fix Design

Parse `FUNC(` by scanning to its matching close paren (tracking `(`/`)` depth),
split the parameter substring on **top-level** commas only (depth 0), and take the
return type after the matching `) AS `. Alternatively normalize/reject nested FUNC
element types earlier with a clear diagnostic if they are intentionally
unsupported.

## Phases

### Phase 1 — failing test + audit

- [ ] Determine whether `List OF FUNC(A, B) AS C` is expressible in surface syntax.
      If yes, add a resolver test that a `transform` over it resolves; confirm it
      fails today. If no, record this as latent defense-in-depth.
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Make `function_parts` paren-depth-aware.

### Phase 3 — validation

- [ ] `scripts/test-accept.sh`; existing non-nested FUNC resolution byte-identical.

## Validation Plan

- Regression test(s): a `function_parts` unit test with a nested multi-arg FUNC
  parameter, plus (if expressible) a `transform`-over-function-list resolution test.
- Full suite: `scripts/test-accept.sh`.

## Summary

A flat-string parse of `FUNC(...) AS ...` mis-splits a nested multi-arg function
parameter, falsely rejecting higher-order collection calls. Latent pending
confirmation that such element types are expressible; the fix is a paren-depth-aware
parser, keeping non-nested parses identical.
