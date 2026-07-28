# bug-34: A `LINK CONST`/`SUCCESS_ON` pin with a 64-bit value whose bit 63 is set is silently lowered to 0

Last updated: 2026-07-08
Effort: small (<1h)

Native-`LINK` constant pins and `SUCCESS_ON`/`RESULT` comparison literals are
lowered by `eval_link_const` (`src/ir/lower.rs:376-393`) and `lower_link_expr`
(`:398-405`), both of which parse the canonicalized literal as **signed i64** and
swallow the error to 0: `…parse::<i64>().unwrap_or(0)` (`:382-383`, `:402-403`). A C
flag/mask constant that uses the full 64-bit width — e.g. `CONST flags =
0xFFFFFFFFFFFFFFFF`, any value `>= 2^63`, or `-9223372036854775808` — canonicalizes
to a decimal string (`18446744073709551615`) that exceeds `i64::MAX`, so
`parse::<i64>` fails and `unwrap_or(0)` yields **0**. The FFI thunk is emitted with
a NULL/zero immediate instead of the intended 64-bit mask, with **no diagnostic**.

The single correct behavior a fix produces: a 64-bit LINK constant is lowered to
its exact bit pattern (the ABI cares about the bits, not the sign), so a bit-63-set
mask reaches the native call intact; only a genuinely malformed literal defaults.

Severity LOW (footgun / silent-wrong-value): reachable via user-authored
`bindings/*.mfb` source, but only for the narrow `>= 2^63` pin range. When
triggered it silently corrupts an FFI argument (NULL instead of the mask), which
can turn a `dlopen`/flag/mode argument into the wrong behavior.

References:

- `src/ir/lower.rs:376-393` (`eval_link_const`; `parse::<i64>().unwrap_or(0)` at
  `:382-383`), `:398-405` (`lower_link_expr`; same at `:402-403`).
- Lexer canonicalizes radix literals via `u128::from_str_radix` (`src/lexer.rs:~675`),
  so the value reaching here is the exact unsigned magnitude.
- `bindings/sqlite3/src/lib.mfb:155` uses `CONST` pins (flag constants).
- NOT bug-11: `expand_scientific_notation` here receives a canonical decimal (no
  large exponent), so bug-11's blow-up does not apply — this is a distinct i64-parse
  loss of a valid unsigned 64-bit value.
- Found during goal-01 review of `src/ir/lower.rs`.

## Failing Reproduction

```
' a native LINK binding pinning a full-width 64-bit flag
LINK "libfoo" ...
  CONST flags = 0xFFFFFFFFFFFFFFFF
  ...
```

- Observed: `flags` is lowered to `0` (the `parse::<i64>` fails and defaults); the
  native call receives NULL/no bits.
- Expected: `flags` is lowered to the exact 64-bit pattern
  `0xFFFFFFFFFFFFFFFF` (`-1` as i64 bits).

Contrast: any constant `<= i64::MAX` (e.g. `0x7FFFFFFF`, a combined
`SQLITE_OPEN_READWRITE|CREATE`, or the `NOTHING`/`0` NULL pin) parses and is
honored — only the high-bit-set 64-bit range is corrupted.

## Root Cause

`eval_link_const`/`lower_link_expr` parse into signed `i64` and default the
out-of-range case to 0. C flag/mask constants are conventionally unsigned, so any
value with bit 63 set is unrepresentable as `i64` and lost.

## Goal

- A LINK constant is lowered to its exact 64-bit pattern for the full unsigned
  range; only truly malformed input defaults (ideally with a diagnostic).

### Non-goals (must NOT change)

- Lowering of constants `<= i64::MAX` (correct today).

## Blast Radius

- `eval_link_const` (`:382-383`) and `lower_link_expr` (`:402-403`) — both share the
  `parse::<i64>().unwrap_or(0)` pattern; fix both.

## Fix Design

On `parse::<i64>` failure, fall back to `parse::<u64>().map(|u| u as i64)` to
preserve the exact bit pattern before defaulting. Consider surfacing a diagnostic
(rather than `unwrap_or(0)`) when both parses fail, so a malformed pin is not
silently zeroed. Apply to both functions.

## Phases

### Phase 1 — failing test + audit

- [ ] Lower/codegen test: a `CONST` pin of `0xFFFFFFFFFFFFFFFF` produces the
      `0xFFFFFFFFFFFFFFFF` immediate, not 0. Confirm it is 0 today.
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Add the `u64` fallback (and optional diagnostic) to both functions.

### Phase 3 — validation

- [ ] `scripts/artifact-gate.sh`; a runtime FFI test pinning a bit-63 flag observes
      the correct native behavior.

## Validation Plan

- Regression test(s): the LINK-const lowering test.
- Runtime proof: an FFI call with a bit-63 flag pin behaves per the flag.
- Full suite: `scripts/artifact-gate.sh`.

## Summary

A signed-i64 parse silently zeroes any 64-bit LINK constant with bit 63 set; a
`u64` fallback preserving the bit pattern fixes both lowering sites.
