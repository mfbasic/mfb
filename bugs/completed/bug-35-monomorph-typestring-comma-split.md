# bug-35: monomorph `split_top_level_commas` / `func_type_parts` split type-argument strings on `", "` with no paren-depth tracking, mangling nested-comma type args

Last updated: 2026-07-08
Effort: small (<1h)

`src/monomorph/helpers.rs::split_top_level_commas` (`:231`) is
`value.split(", ").map(...).collect()` — **no** bracket/paren depth tracking,
despite the "top_level" name. `func_type_parts` (`:145-152`) splits its FUNC
parameter list the same way (`params.split(", ")`) and uses
`rest.split_once(") AS ")` which matches the *first* `") AS "`. So a type argument
that itself contains a comma — a multi-parameter FUNC type or a nested user
template — is shredded: `Pair OF FUNC(Integer, String) AS Boolean, Integer` splits
the single first argument `FUNC(Integer, String) AS Boolean` into
`["FUNC(Integer", "String) AS Boolean"]`, so a 2-arg template is seen as 3 args,
and `FUNC(FUNC(A, B) AS C) AS D` mis-splits at the inner `) AS `. Downstream
`unify_type`, `substitute_type_params`, and `concrete_type_name` then operate on
wrong sub-strings, producing a wrong mangled type name or a failed unification —
i.e. a false rejection or (worse) a wrong specialization/mangled symbol.

This is the **same root class** as bug-26 (`builtins/general.rs::function_parts`):
flat comma/`") AS "` splitting of type strings without paren-depth awareness,
replicated in the monomorphization type machinery.

The single correct behavior a fix produces: type-argument and FUNC-parameter
strings are split only on top-level (paren-depth-0, outside `OF`/`TO`/`AS` groups)
separators, so nested-comma type args unify and mangle correctly.

Severity LOW: reachable only from a generic type/function whose type argument is a
multi-parameter FUNC type or a nested user template — narrow in practice — and it
is a compile-time misparse (false rejection or wrong mangle), not a runtime memory
hazard. Filed because a wrong mangled symbol could in principle collide or mis-link.

References:

- `src/monomorph/helpers.rs:231` (`split_top_level_commas`, flat `split(", ")`),
  `:145-152` (`func_type_parts`, flat param split + first-match `") AS "`).
- Contrast (safe): `split_top_level_to` (`:225-229`) uses `split_once(" TO ")`, and
  `List OF …`/`Map OF K TO V` peel their prefix first, so single-level shapes parse
  correctly. The bug bites only when a comma-bearing type is itself an *argument*.
- Same class, different file: bug-26 (`builtins/general.rs::function_parts`).
- Found during goal-01 review of `src/monomorph/**`.

## Failing Reproduction

A generic instantiation whose type argument is a multi-param FUNC (if expressible):

```
' e.g. Pair OF FUNC(Integer, String) AS Boolean, Integer
```

- Observed: `split_top_level_commas` yields 3 args instead of 2 (the FUNC arg is
  split at its inner comma), so unification/mangling operate on garbage substrings
  → false "no matching template"/wrong mangled name.
- Expected: the FUNC type is treated as one type argument; the template unifies.

Contrast: `List OF FUNC(Integer, String) AS Boolean` (peeled `List OF ` prefix, then
one FUNC) works; single-arg FUNC type args work.

## Root Cause

`split_top_level_commas` and `func_type_parts` split on every `", "` (and the first
`") AS "`), ignoring paren/`OF`/`TO`/`AS` nesting — so a comma inside a nested type
argument is treated as a top-level separator.

## Goal

- Type-argument and FUNC-parameter splitting respects nesting; nested-comma type
  args unify and mangle correctly.

### Non-goals (must NOT change)

- Splitting of non-nested type strings (correct today) — results must be identical.

## Blast Radius

- `split_top_level_commas`, `func_type_parts`, and their consumers (`unify_type`,
  `substitute_type_params`, `concrete_type_name`). The same fix pattern applies to
  bug-26's `function_parts`.

## Fix Design

Make `split_top_level_commas` a depth-aware scanner: split on `, ` only at
paren-depth 0 and outside `OF …`/`TO …`/`AS …` groupings (mirror the care in
`split_top_level_to`). Parse `func_type_parts` by scanning to the matching close
paren of `FUNC(`, splitting params on top-level commas, and taking the return type
after the matching `) AS `. Consider a shared helper reused by bug-26's fix.

## Phases

### Phase 1 — failing test + audit

- [ ] `split_top_level_commas`/`func_type_parts` unit tests with a nested multi-arg
      FUNC type arg; confirm they mis-split today.
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Depth-aware split in both functions (shared helper with bug-26 if practical).

### Phase 3 — validation

- [ ] `scripts/test-accept.sh` — non-nested generic mangling byte-identical.

## Validation Plan

- Regression test(s): the depth-aware split tests + a generic-instantiation
  end-to-end if the nested type arg is expressible.
- Full suite: `scripts/test-accept.sh`.

## Summary

Flat comma/`") AS "` splitting of type strings (same class as bug-26) mis-parses
nested-comma type args in monomorphization; a depth-aware split fixes it, ideally
sharing one helper with bug-26.
