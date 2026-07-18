# bug-292: `PUBLIC ISOLATED <garbage>` is silently accepted as a FUNC declaration

Last updated: 2026-07-17
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (accepts-invalid)

Status: Open
Regression Test: tests/syntax (new) — `PUBLIC ISOLATED BOGUS …` is rejected with `MFB_PARSE_UNEXPECTED_TOKEN`

`check_top_level_item_start`'s visibility arm only checks that the token after the
visibility keyword is `SUB|FUNC|ISOLATED`; it never checks what follows `ISOLATED`.
`parse_function` then consumes `ISOLATED`, blindly `advance()`s the next token as
`kind_token`, and defaults `kind = Func` for anything that is not `SUB` — with no
check that it was actually `FUNC`. So `PUBLIC ISOLATED BOGUS weird AS Integer`
compiles to an executable, as does `PUBLIC ISOLATED ISOLATED …`. The spec grammar
requires the `FUNC` keyword (`funcDecl = declVis funcIso "FUNC" …`).

The single correct behavior a fix produces: a declaration after `ISOLATED` must be
`FUNC` (or `SUB`); anything else is a parse error.

References:

- `src/docs/spec/language/19_grammar.md` (`funcDecl` requires `FUNC`).
- Found during goal-06 review of `src/ast/items.rs`.

## Failing Reproduction

```
PUBLIC ISOLATED BOGUS weird AS Integer
  RETURN 1
END FUNC
```

- Observed: compiles cleanly (`Wrote executable to ./build/…`).
- Expected: `MFB_PARSE_UNEXPECTED_TOKEN` (or similar) at `BOGUS`.

Contrast (correct today): bare `ISOLATED BOGUS …` (no visibility) is correctly
rejected because the no-visibility arm requires `ISOLATED`+`FUNC`.

## Root Cause

`src/ast/items.rs:43-57` (`parse_function`) defaults `kind = Func` for a non-`SUB`
`kind_token` without verifying it is `FUNC`; `src/ast/items.rs:430-447`
(`check_top_level_item_start`) does not check the token after `ISOLATED`.

## Goal

- After matching `ISOLATED`, `parse_function` verifies `kind_token` is
  `Keyword::Func`/`Keyword::Sub` and reports an error otherwise; or
  `check_top_level_item_start` requires `ISOLATED` be followed by `FUNC`.

### Non-goals (must NOT change)

- Valid `PUBLIC ISOLATED FUNC …` / `ISOLATED FUNC …` declarations.

## Blast Radius

- `parse_function` / `check_top_level_item_start` — fixed here.
- No other caller relies on the permissive default (verified: `SUB` path is
  explicit).

## Fix Design

Add the `kind_token` keyword check in `parse_function` (loud, minimal) — this also
covers the `PUBLIC ISOLATED ISOLATED` case. Rejected alternative: only tightening
`check_top_level_item_start` — leaves the permissive default in `parse_function`.

## Phases

### Phase 1 — failing test
- [ ] Test that `PUBLIC ISOLATED BOGUS …` compiles today.
### Phase 2 — the fix
- [ ] Reject a non-FUNC/SUB `kind_token`.
### Phase 3 — validation
- [ ] Full suite green; valid ISOLATED FUNC/SUB still parse.

## Validation Plan

- Regression: the syntax-reject test + a valid `ISOLATED FUNC` contrast.
- Doc sync: none.

## Summary

A permissive default silently accepts a garbage keyword between `ISOLATED` and the
signature; a one-line keyword check rejects it. Minimal risk.
