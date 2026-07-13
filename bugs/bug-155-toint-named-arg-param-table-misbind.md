# bug-155 — `toInt`'s merged param-name table puts the 2-arg `text` at the wrong position → named-argument misbind

Last updated: 2026-07-12
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (named-argument binding)

Status: Open
Regression Test: _(none yet)_

`toInt` is overloaded: the 1-arg form is `toInt(value)`, the 2-arg form is
`toInt(text AS String, base AS Integer)` (documented at
`src/builtins/general.rs:224-230`). Its merged per-position param-name table is
`&[&["value"], &["text", "base"]]` — position 0 accepts only `value`, and both
`text` and `base` sit at position 1. Because the two overloads *disagree at
position 0* (`value` vs `text`), the correct merged table is
`&[&["value", "text"], &["base"]]`. As written, a documented named call
`toInt(text := "ff", base := 16)` cannot bind: `text` is not accepted at
position 0, and `text`+`base` collide at position 1. The single correct behavior
is that the 2-arg named form binds `text`→pos 0 and `base`→pos 1.

This is exactly the bug-94 "position-0 disagreement" hazard that
`datetime::fixedOffset` and `audio::openInput/openOutput` avoid via a dedicated
per-overload table (`call_param_name_overloads`, see `datetime.rs:149,185-190`
and `audio.rs:212-220`). `TO_STRING` (`general.rs:103`) is safe only because its
position 0 is `value` in *both* overloads. Positional calls
`toInt("ff", 16)` are unaffected.

References:

- bug-94 (position-0 param-name disagreement); the per-overload table mechanism.
- `src/builtins/general.rs:98-119` (`call_param_names`), `:224-230` (doc).
- goal-03 review.

## Failing Reproduction

```
LET n = toInt(text := "ff", base := 16)   ' expect 255
```

- Observed: named-argument resolution fails / misbinds (`text` unrecognized at
  position 0; `base`/`text` contend for position 1).
- Expected: binds `text := "ff"`, `base := 16` → 255, identical to the positional
  `toInt("ff", 16)`.

Contrast: `toString(value := x, precision := 2)` works because `toString`'s
position 0 is `value` in both overloads.

## Root Cause

`src/builtins/general.rs:104`:
`TO_INT => Some(&[&["value"], &["text", "base"]])`. The merged table is built
per-position but the 2-arg overload's first parameter (`text`) is placed at
position 1 instead of being unioned into position 0. `general.rs` has no
`call_param_name_overloads` mechanism (unlike `datetime`/`audio`), so overloads
with position-0 name disagreement cannot be represented correctly by a single
merged table.

## Goal

- `toInt(text := "ff", base := 16)` == `toInt("ff", 16)` == 255.
- The 1-arg `toInt(value := x)` still binds correctly.

### Non-goals (must NOT change)

- Positional `toInt` behavior; `toString`'s (already-correct) table.

## Blast Radius

- `TO_INT` entry (`general.rs:104`) — fixed by this bug.
- Audit other `general.rs` merged tables for position-0 disagreement: only
  `TO_INT` has overloads whose position 0 differs (`ERROR` is `code`/`message`
  across positions; the rest are single-name). No other in-scope site.

## Fix Design

Either (a) minimal: change the entry to `&[&["value", "text"], &["base"]]` so
`text` is accepted at position 0; or (b) robust: give `general.rs` a
per-overload `call_param_name_overloads` table like `datetime::fixedOffset` and
register `toInt`'s two overloads explicitly. Prefer (a) unless a future overload
forces (b).

## Validation Plan

- Add valid/invalid function tests under `tests/func_general_toInt_*` covering
  the 2-arg named form (both orders) and a bad name.
- Full acceptance.

## Summary

A one-entry table error blocks the documented 2-arg named form of `toInt`; fix
by unioning `text` into position 0 (or adopting the per-overload table pattern).
