# bug-449: toMoney(String) parses through f64, so it is not exact base-10

Last updated: 2026-08-22
Effort: medium (native/runtime exact-decimal parser)
Severity: MEDIUM
Class: Correctness (financial exactness — the whole point of Money — is lost on string input)

Status: Open
Regression Test: to be added (rt-behavior money-from-string fixture)

`toMoney(String)` lowers by parsing the text to an **f64**, then scaling by
100000.0 and mode-rounding (`src/codegen/engine/convert/builder_conversions.rs`,
the `"String"` arm of `lower_to_money` — comment: "Mirror `toFixed(String)`").
Money is an **exact base-10** type (`mfb spec language types` §4.1/§4.12: a Money
literal uses "the exact base-10 converter (no f64 bound — Money is exact
decimal)"), so routing its string parse through binary f64 reintroduces exactly
the drift Money exists to avoid, and it is wrong at the type's own boundaries:

- `toMoney("92233720368547.75807")` — the **exact max** — fails with `ErrOverflow`,
  even though the identical literal `92233720368547.75807m` is a valid Money. f64
  cannot hold 19 significant digits, so the parsed double rounds up past the range.
- `toMoney("1.234565")` returns `1.23456` under `Rounding.Commercial`, but the tie
  is a true base-10 half and half-away-from-zero must give `1.23457`. The exact
  converter already does this: `src/numeric.rs` test `money_raw_from_decimal("1.234565")
  == 123_457`. f64(1.234565) rounds just below the tie, so the runtime path loses it.

Mid-range values (≤ ~15 significant digits, no 6th-place tie) round-trip correctly,
which is why this hid — e.g. `toMoney("12345.67891") == 12345.67891m` is TRUE.

## Failing Reproduction

```mfbasic
IMPORT money
IMPORT io
SUB main
  LET maxm AS Money = 92233720368547.75807
  ' Valid literal, but the identical string overflows:
  LET a AS Money = toMoney("92233720368547.75807") TRAP(e)
    io::print("toMoney(max) trapped " & toString(e.code))   ' prints 77050010
    EXIT SUB
  END TRAP
  io::print(toString(a, toByte(5)))
END SUB
```

```mfbasic
IMPORT money
IMPORT io
SUB main
  money::setRounding(Rounding.Commercial)
  io::print(toString(toMoney("1.234565"), toByte(5)))   ' prints 1.23456, want 1.23457
END SUB
```

- Observed: `ErrOverflow` on the exact max; `1.23456` on the tie.
- Expected: the max parses to `92233720368547.75807m`; the tie rounds to `1.23457`
  (Commercial) / `1.23456` (Banker) — matching the literal converter and the mode.

## Root Cause

`lower_to_money`'s `"String"` arm calls `emit_parse_decimal_string_to_double`
(binary f64) then scales. The exact base-10 converter used for literals,
`src/numeric.rs::money_raw_from_decimal` / `money_conversion_from_decimal`, is a
compile-time Rust function and is not reachable from the emitted runtime path.

## Fix (options)

1. Emit a native exact decimal→scaled-i64 parser for the `String` arm: accumulate
   integer + up-to-5 fractional digits into i64 with overflow checks, settle the
   6th+ fractional digits through the shared mode-rounding helper, malformed →
   `ErrInvalidFormat`, out-of-range → `ErrOverflow`. Must define the accepted
   grammar (sign, `.5`/`5.` forms, leading zeros; decide on scientific notation —
   the f64 path currently accepts `1e3`).
2. Or add a `_mfb_rt_money_from_decimal` runtime helper (Rust, reusing
   `money_conversion_from_decimal` + the mode) and route the `String` arm to it —
   money currently has **no** runtime helpers (unlike `app::`), so this introduces
   the first, plus its link/ABI/error-signal wiring.

## Non-goals

- `toMoney(Float)` is inherently inexact (a Float is binary) and is out of scope;
  this bug is specifically the `String` path, which has an exact source of truth.
