# bug-831: `toFixed(String)` parses through binary64 — wrong value near the top of the range, and rejects in-range text with ErrOverflow

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed
Regression Test: `tests/runtime/rt_tofixed_string_exact.rs`

## Cause (coordinator, bug-staging-24)

- Rule: `toFixed(String)` converts the decimal text exactly: nearest Q32.32 carrier, ties away from zero, then the range check, as the `Fixed` literal path does (`numeric::fixed_raw_from_decimal`).
- Where: `src/codegen/engine/convert/builder_conversions.rs:lower_to_fixed`, `ParameterType::String` arm: `emit_parse_decimal_string_to_double` then `emit_float_bits_to_fixed_value` rounds twice (binary64, then Q32.32).
- Fix: a decimal-text-to-Q32.32 parse that mirrors `fixed_raw_from_decimal` (integer part and fraction digits accumulated exactly, one rounding), replacing the binary64 detour on that arm.

`toFixed(String)` converts its text by first parsing it to an `f64`
(`emit_parse_decimal_string_to_double`), then converting that double to Fixed with
`emit_float_bits_to_fixed_value`. A Fixed has 32 fractional bits plus a 32-bit integer
part. An f64 has only 53 significant bits, so once the integer part is large, the double
cannot represent the text closely enough. Two observable failures follow:

1. **Wrong value.** `toFixed("2147483647.99")` gives `2147483647.990000009536`, about 41
   quanta above the correct result. The literal path gives `2147483647.989999999990`,
   which is the nearest Fixed.
2. **Spurious overflow.** `toFixed("2147483647.9999999")` raises `ErrOverflow`
   (77050010). The value is inside the Fixed range (`LET x AS Fixed = 2147483647.9999999`
   is accepted and stores `2147483647.999999900115`), but the nearest f64 is `2^31`,
   which falls outside the range.

The single correct behavior: `toFixed(String)` converts the decimal text exactly, rounding
to the nearest Q32.32 carrier with ties away from zero and then range-checking. This is
what the `Fixed` literal path does (`numeric::fixed_raw_from_decimal`), and what
`toMoney(String)` does since bug-449. The result is the same `Fixed` that
`LET x AS Fixed = <same text>` stores.

References:

- `src/docs/spec/language/04_types.md` §4.1: the Fixed range is "approximately
  -2147483648.0 through 2147483647.9999999998". The example `LET z = toFixed("1.25")` is
  the fallible text parse. §4.12 says Fixed literal validation is exact ("there is no
  `f64` step").
- `toFixed` man descriptor (`src/codegen/builtins/general/func_to_fixed.rs:register`):
  "A decimal fraction is rounded to the nearest representable `Fixed`." That claim is
  false for these inputs.
- Sibling: bugs/completed/bug-449-tomoney-string-parses-through-f64-not-exact-decimal.md
  fixed the same bug for Money. bug-813's Non-goals explicitly left `toFixed(String)`
  out of scope ("separate conversion paths, not covered").

## Failing Reproduction

```basic
IMPORT io
FUNC tx(s AS String) AS String
  RETURN toString(toFixed(s), toByte(12))
  TRAP(e)
    RETURN "err " & toString(e.code)
  END TRAP
END FUNC
FUNC main() AS Nothing
  LET lit AS Fixed = 2147483647.99
  io::print("literal  " & toString(lit, toByte(12)))
  io::print("toFixed  " & tx("2147483647.99"))
  LET lit2 AS Fixed = 2147483647.9999999
  io::print("literal2 " & toString(lit2, toByte(12)))
  io::print("toFixed2 " & tx("2147483647.9999999"))
END FUNC
```

`/tmp/hardening/mfb build /tmp/hardening/B01/repro/r_tofixed && ./build/r_tofixed.out`

- Observed:
  ```
  literal  2147483647.989999999990
  toFixed  2147483647.990000009536
  literal2 2147483647.999999900115
  toFixed2 err 77050010
  ```
- Expected: each `toFixed` line is identical to the `literal` line above it.

Contrast cases that already work: small magnitudes such as `toFixed("0.1")` and
`toFixed("1e-3")`. Their f64 has enough precision that rounding to Fixed hides the
double step.

## Root Cause

`src/codegen/engine/convert/builder_conversions.rs:lower_to_fixed`, in the
`ParameterType::String` arm: it calls `emit_parse_decimal_string_to_double`, then
`emit_double_overflow_check`, then `emit_float_bits_to_fixed_value`. The text is rounded
twice: first to binary64, then to Q32.32.

## Goal

- `toFixed(s)` equals the literal-path value for every decimal string `s` in range,
  including `2147483647.99999999976716935634613037109375` (carrier i64::MAX). It raises
  `ErrOverflow` only when the rounded carrier is outside the i64 range.

### Non-goals (must NOT change)

- `toFixed(Float)` semantics. They are binary64 by definition (§4.1).
- The `ErrInvalidFormat` contract for malformed text, `NaN`, and `Infinity`.

## Blast Radius

- `lower_to_fixed` String arm: fixed by this bug.
- Any `json`/`csv` path that decodes into Fixed via toFixed: audit with
  `grep -rn "toFixed" src/codegen/builtins`.

## Fix Design

Reuse the exact decimal-to-Q32.32 runtime routine, the same way bug-449 made toMoney
exact, as a runtime helper mirroring `numeric::fixed_conversion_from_decimal`.

## Validation Plan

- The rt fixture above, plus boundary strings (`2147483647.99999999976716935634613037109375`,
  `-2147483648`, the half-quantum ties from §4.12).
- Full suite.

## Summary

The engineering risk is concentrated in a runtime exact-decimal helper. Everything else
stays untouched.

## Fix

`lower_to_fixed`'s String arm calls the new `emit_parse_decimal_string_to_fixed_raw`
(`src/codegen/engine/convert/builder_fixed_parse.rs`), integer arithmetic only: the whole part
accumulates (bounded to 2^31), the first 33 fractional digits (the deciding digits of
`numeric::fixed_conversion_from_decimal`) go to a stack buffer that is doubled 33 times to read
off the fraction's binary digits exactly, the 33rd being the round bit (ties away from zero), then
sign and i64 range check. Exponents are applied exactly by moving the decimal point; the accepted
grammar is the one `_mfb_rt_string_to_float` accepted. Spec `language/04_types.md` and the
`toFixed` man description state the rule.

RED (main's compiler): `MFB_TEST_EXE=/tmp/main-dbg-target/release/mfb cargo test --release --test
rt_tofixed_string_exact` fails (`toFixed("2147483647.99")` = `2147483647.990000009536` vs the
literal `2147483647.989999999990`). GREEN with this branch.

Goldens expected to move: `byte-identity/general/general_codegen_cover_rt` (five `.ncode`
shas; measured by `artifact-gate general`, 5 diffs) - the fixture calls `toFixed(String)` and the
emitted parse changed. Any other fixture whose native code contains `toFixed(String)` moves the
same way (ncode kind only).
