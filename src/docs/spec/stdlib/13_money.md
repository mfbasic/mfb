# Money (money)

The `money` package controls how `Money` **arithmetic** settles the half case
and provides one explicit settling function. Called with the `money::` qualifier;
`IMPORT money` needs no manifest dependency. [[src/codegen/builtins/money/mod.rs:register]]

This topic specifies the *model behind* the package: the exact base-10 fixed-point
representation that makes decimal amounts exact, the rounding-mode state the
package reads and writes, and the rounding rule `money::round` applies. The
`Money` **type** itself — its literals, range, dimensional algebra, and 8-byte
storage — is owned by `./mfb spec language types`; it is cross-referenced here,
not re-specified. The per-function API — signatures, parameters, errors — is owned
by `./mfb man money`.

## The exact base-10 model

`Money` is a 64-bit **signed integer** carrier interpreted as a base-10
fixed-point value scaled to **five decimal places**. One raw unit is `0.00001`;
`1.00000` is the raw integer `100000`, i.e. the scale factor is `10^5`. The
representable range follows directly from the i64 carrier: `-92233720368547.75808`
through `92233720368547.75807`. [[src/ir/verify/values.rs:check_const_literal]]

Because the scale is a power of **ten**, every decimal amount within range
with at most five fractional places is stored with **no representation error**: `0.10`, `0.01`, `0.20` are the exact raw
integers `10000`, `1000`, `20000`. This is the property `Float` cannot provide —
an IEEE-754 binary carrier cannot represent `0.10` exactly, so repeated cent
accounting drifts. With `Money`, the only rounding that ever occurs is the
**deliberate** rounding of a division or an inexact scaling; addition, subtraction,
integer scaling, and comparison are exact integer operations on the raw i64 and
never round.

A rate to multiply a `Money` by, such as an exchange rate, has the same need for
an exact decimal value but more places than five. The package's `money::Rate`
holds one at twelve places; see `money::Rate` below.

## Operation formulas

Let `m` and `n` be signed Money raw carriers, `S = 100000`, `Q = 2^32`,
`f` a signed Fixed raw carrier, and `k` an Integer/Byte value. `trunc` means
truncation toward zero; `floor` means toward negative infinity. `R` is nearest
integer rounding under the calling thread's mode, applied once to the exact
rational value. The following describes the current implementation.

| Operation | Result calculation |
| --- | --- |
| Money add/subtract | checked `m + n` / `m - n` |
| Money times Integer/Byte | checked `m * k` |
| Money divided by Integer/Byte | `R(m / k)` |
| Money MOD Money | `m - trunc(m / n) * n`, exact signed remainder |
| Money times Fixed (either order) | `R(m * f / Q)` |
| Money divided by Fixed | `R(m * Q / f)` |
| `toMoney(Fixed)` | `R(f * S / Q)` |
| `toFixed(Money)` | `R(m * Q / S)` as a Fixed raw carrier |
| `toInt(Money)` | `trunc(m / S)` |
| `toByte(Money)` | `trunc(m / S)`, then range check `0..255` |
| `money::convert(m, from, to)` | `R(m * from / to)`, with `from` and `to` two `Rate` scaled values |
| `money::scale(m, r)` | `R(m * r / P)`, with `r` a `Rate` scaled value and `P = 10^12` |

`money::convert` and `money::scale` are exact as well, and round once. The product
`|m| * |r|` of a Money raw carrier and a `Rate` scaled value is at most `2^126`, so it
is formed as a full unsigned 128-bit magnitude, then divided by the 64-bit
magnitude of the divisor (`to`, or `P`) by a restoring long division that keeps the
exact remainder. In `convert` the two rates' `10^12` factors cancel, so no scale
constant enters. The quotient magnitude is rounded once under the mode, the sign
applied, and the rounded carrier range-checked: a result outside the Money range
raises `ErrOverflow`. Because nothing rounds before the end, one `convert` can
differ in the last place from scaling twice. Each rate's `scaled` must be above
zero, checked on every call, or the call raises `ErrInvalidArgument`; this also
covers a zero `to`, the only zero divisor. For example,
`money::convert(10000000000m, money::toRate("1"), money::toRate("0.0067"))` is
`1492537313432.83582m`, and `money::scale(0.00001m, money::toRate("0.5"))` is the
tie: `0.00001m` under `Commercial`, `0m` under `Banker`.
[[src/codegen/builtins/money/gen_money_math.rs:emit_money_rescale]]
[[src/codegen/builtins/money/func_convert.rs:lower_money_convert]]
[[src/codegen/builtins/money/func_convert.rs:emit_positive_rate_scaled]]
[[src/codegen/builtins/money/func_scale.rs:lower_money_scale]]

Integer operations check the signed carrier range. The four Money/Fixed rows
are exact integer arithmetic with no Float intermediate, and they apply `R` to
the exact ratio, not to a truncated one: a product forms the full unsigned
128-bit magnitude `|m| * |f|` (or `|f| * S`) and keeps the 32 low bits it
divides away as the remainder over `Q`; a quotient builds the integer part and
32 fraction bits of `|m| * Q / |f|` (or `/ S`) by long division and keeps the
final remainder over the divisor. The magnitude is rounded once under the mode,
the sign applied, and the **rounded** carrier range-checked, so a rounding carry
past the maximum raises `ErrOverflow` while a result rounding to exactly the
signed minimum is accepted. For example, `0.00003m * 0.5F` is `0.00002m` under
either mode, and `0.00005m * 0.5F` is `0.00003m` under `Commercial` and
`0.00002m` under `Banker`. A zero Fixed divisor raises `ErrInvalidArgument`;
`toMoney(Fixed)` cannot overflow (its magnitude is below `2^48` raw units).
These rows round under the Money mode because a Money operand or result is
involved; ordinary `Fixed * Fixed` and `Fixed / Fixed` do not read the mode and
are unchanged by it. Money divided by Integer/Byte rounds the signed
quotient/remainder through the rule below; the one divisor whose magnitude a
signed register cannot hold, the minimum Integer (`2^63`), is rounded on
unsigned magnitudes instead, so `50000000000000m / -9223372036854775808` is
`-0.00001m` and raw `2^62` divided by it is the tie (`-0.00001m` under
`Commercial`, `0m` under `Banker`).
[[src/codegen/builtins/money/gen_money_math.rs:emit_money_multiply]]
[[src/codegen/builtins/money/gen_money_math.rs:emit_money_divide_scalar]]
[[src/codegen/builtins/money/gen_money_math.rs:emit_money_fixed_multiply]]
[[src/codegen/builtins/money/gen_money_math.rs:emit_money_fixed_divide]]
[[src/codegen/builtins/money/gen_money_math.rs:emit_apply_rounding_magnitude]]
[[src/codegen/engine/convert/builder_conversions/mod.rs:lower_to_money]]
[[src/codegen/engine/convert/builder_conversions/mod.rs:lower_to_fixed]]

The Money remainder has the dividend's sign or is zero. A zero divisor raises
`ErrInvalidArgument`; minimum raw Money `MOD -0.00001m` raises `ErrOverflow`
because the checked quotient would overflow, even though its mathematical
remainder is zero. `toInt(Money)` cannot overflow; `toByte(Money)` checks after
truncation, so `toByte(-0.9m)` is zero while `toByte(-1m)` raises `ErrOverflow`.
[[src/codegen/builtins/money/gen_money_math.rs:emit_money_binary]]
[[src/codegen/engine/operators/builder_numeric/binary_emit.rs:emit_integer_binary]]
[[src/codegen/engine/convert/builder_conversions/to_int.rs:emit_money_to_int_value]]
[[src/codegen/engine/convert/builder_conversions/to_byte_scalar.rs:lower_to_byte]]

Money times/divided by Float first converts `m` to binary64, performs binary64
multiplication/division with the Float operand, then rounds that binary64 result
to a Money raw carrier under the mode. This can round during conversion and
arithmetic as well as at final settlement; it is not exact rational arithmetic
with only one final rounding. Money/Money ratio instead converts each raw
carrier to binary64 and divides, returning Float; the scale cancels. `DIV`
converts operands to their numerical Float values before division. A zero
divisor for a Money result raises `ErrInvalidArgument`; a Float-result ratio
uses the Float observation-boundary rules.
[[src/codegen/builtins/money/gen_money_math.rs:emit_money_scale_float]]
[[src/codegen/builtins/money/gen_money_math.rs:emit_money_ratio]]
[[src/codegen/builtins/money/gen_money_math.rs:emit_money_div_to_float]]

## The rounding-mode state

There are two rounding modes, the members of the `Rounding` enum, whose
discriminants are exactly their stored values:

| Mode | Stored value | Half rule |
|------|-------------|-----------|
| `Commercial` | `0` | round half **away from zero** (the default) |
| `Banker` | `1` | round half to **even** (banker's rounding) |

`Commercial` is the **default** at program start; a worker inherits its parent's mode as described below. `Banker` removes the
small upward bias of always rounding ties away, which matters when many rounded
amounts are summed. [[src/codegen/builtins/money/mod.rs:Rounding]]

The mode is **mutable, per-execution-context state**, not a global constant. It
lives in a single word of the per-thread arena state — the `moneyRoundingMode`
field, whose layout is owned by `./mfb spec memory arenas`.
[[src/codegen/error/constants/error_constants.rs:ARENA_ROUNDING_MODE_OFFSET]]

- `money::setRounding(mode)` writes the field. The stored value is masked to its
  low bit (`mode & 1`), so only the two defined modes are ever recorded.
  [[src/codegen/builtins/money/func_set_rounding.rs:lower_money_set_rounding]]
- `money::getRounding()` reads the field and returns it as a `Rounding` value.
  [[src/codegen/builtins/money/func_get_rounding.rs:lower_money_get_rounding]]

Because the field is part of the arena state, the mode is **per-thread**: each OS
thread owns its own arena and therefore its own mode word. A worker thread
**inherits** the spawning thread's mode at spawn — the child arena is zeroed to
`Commercial`, then the parent's mode is copied in — and thereafter changes
independently, exactly parallel to the per-thread RNG and other arena state.
Setting the mode on one thread never disturbs another.
[[src/codegen/runtime/thread/runtime_helpers.rs:lower_thread_start_helper]]

Setting the mode affects **only** `Money` arithmetic. It does not change `Fixed`
or `Float` rounding, and — importantly — it does **not** change how
`toString(Money)` renders a value: presentation rounding is a fixed
half-away-from-zero rule, independent of the mode, so a displayed or logged amount
is a pure function of its value and requested precision. This decoupling enables
the common workflow of accumulating under one mode and presenting under another.

## The rounding rule

For exact integer divisions that apply the mode, settlement reduces to rounding a signed division. Given a
magnitude quotient `q` truncated toward zero, the signed remainder `r`, the
positive divisor magnitude `|div|`, and the true sign of the result, the decision
is made on the remainder magnitude:

```text
half := |div| - |rem|          ; the tie threshold, in [1, |div|]
if |rem| < half:  keep q        ; below the halfway point
if |rem| > half:  round q away  ; past the halfway point
if |rem| == half:               ; exact tie
    Commercial -> round q away
    Banker     -> round away only when q is odd (reach an even result)
```

"Round away" moves the magnitude away from zero: `+1` when the result is positive,
`-1` when negative. Comparing `|rem|` against `|div| - |rem|` rather than doubling
`|rem|` avoids overflow near `i64::MAX`. The result carries the true sign, which is
tracked separately because a truncated quotient of `0` has no sign of its own.
This is the shared integer quotient/remainder implementation of both modes.
The Money/Fixed rows and a minimum-Integer divisor apply the same rule to unsigned magnitudes — `q`, `|rem|`
and `|div|` with the sign applied afterward — comparing unsigned, so a divisor
magnitude of `2^63` (the Fixed or Integer minimum) is decided exactly.
[[src/codegen/builtins/money/gen_money_math.rs:emit_apply_rounding]]
[[src/codegen/builtins/money/gen_money_math.rs:emit_apply_rounding_magnitude]]

For the same exact quotient, remainder, and mode, this integer rule gives the
same result on every target. Float scaling instead settles an already-rounded
binary64 result; its earlier precision loss is not repaired by settlement.
[[src/codegen/builtins/money/gen_money_math.rs:emit_apply_rounding]]
[[src/codegen/builtins/money/gen_money_math.rs:emit_money_scale_float]]

## `money::round(value, decimals)`

`money::round` explicitly settles a `Money` to `decimals` fractional places under
the current mode — the "compute at five places, book at two" operation. It stays a
`Money` (contrast `math::round(Money)`, which converts the monetary amount to a
dimensionless whole-unit `Integer`). [[src/codegen/builtins/money/func_round.rs:lower_money_round]]

`decimals` must be in the range `0` through `5`:

- `decimals = 5` is the **identity** — `Money` already carries five decimal
  places, so nothing changes.
- `decimals = 2` books to whole cents; `decimals = 0` settles to whole currency
  units while remaining a `Money`.
- Any `decimals` outside `0..5` fails with `ErrInvalidArgument` (`77050002`).

The algorithm is exact integer arithmetic. With `divisor = 10^(5 - decimals)`
(built by a bounded multiply loop of at most five steps), it computes the
truncated quotient `raw / divisor` and remainder, rounds the remainder through the
shared rule above under the current mode, then re-multiplies by `divisor` to
return to the five-place `Money` scale. The tie `0.125` illustrates the mode
split: rounding to two places yields `0.13` under `Commercial` and `0.12` under
`Banker`.

## `money::Rate`

`money::Rate` is an exported record with one field, `scaled AS Integer`. The rate
it holds is `scaled / 10^12`: the scale is the constant `10^12` and no function
takes it as a parameter. The smallest positive rate is `0.000000000001` (`scaled`
1) and the largest is `9223372.036854775807` (`scaled` 9223372036854775807, the
largest `Integer`). It exists so a rate such as an exchange rate is held exactly:
a `Fixed` or `Float` rate is binary, and `0.842` written as either is stored as a
slightly different value. [[src/codegen/builtins/money/mod.rs:RATE_SCALE]]

`Rate` has no operators of its own. Two rates compare with `=` and `<>` as any
record whose fields are all comparable does (`./mfb spec language types`), so two
rates are equal exactly when their `scaled` values are.

**Valid rates and check-on-use.** A usable rate is above zero. `scaled` is an
ordinary exported field, so `money::Rate[scaled := 0]` and `money::Rate[scaled :=
-5]` compile and hold those values; nothing prevents building one. Every `money`
function that computes with a `Rate` checks `scaled > 0` on each call instead and
raises `ErrInvalidArgument` (`77050002`) otherwise.

**`money::toRate(String)`** reads decimal text exactly, using integer arithmetic
only. [[src/codegen/builtins/money/func_to_rate_string.rs:BODY]]

- The text is an optional leading `-`, then one or more digits, then optionally a
  `.` followed by one or more digits. There is no leading or trailing whitespace,
  no `+`, no exponent and no digit separator; a `.` needs a digit on each side.
  Leading zeros are allowed. Anything else is `ErrInvalidFormat` (`77050003`).
- More than 12 fraction digits is `ErrInvalidFormat`, trailing zeros included:
  `"0.8420000000000"` has 13 places and is rejected. Nothing is rounded.
- A value at or below zero is `ErrInvalidArgument` (`77050002`), for any
  magnitude: `"-99999999999999999999"` is `ErrInvalidArgument`, not `ErrOverflow`.
- A value above `9223372.036854775807` is `ErrOverflow` (`77050010`).
- The checks apply in that order: format, then the sign, then the range.

**`money::toRate(Integer)`** makes the whole-number rate `n`: `scaled` is
`n × 10^12`. `n` at or below zero is `ErrInvalidArgument`; `n` above `9223372` is
`ErrOverflow`. [[src/codegen/builtins/money/func_to_rate_integer.rs:BODY]]

There is no `toRate(Float)` or `toRate(Fixed)`: each would convert a binary value
and store a rate other than the one written.

**`toString(Rate)`** is the package's override of the general `toString`. It
prints a `-` when `scaled` is negative, then the whole part, then a `.` and the
fraction digits with trailing zeros removed. The `.` is omitted when the fraction
is zero. `toString(money::toRate("0.84200"))` is `0.842`, `toString(money::toRate(1))`
is `1`. It does not validate: a record built with `scaled` zero prints `0`, and one
built with `-5` prints `-0.000000000005`. Its output is always text that
`money::toRate(String)` reads back to the same `scaled` when the rate is valid.
[[src/codegen/builtins/money/func_to_string_rate.rs:BODY]]

## Error and overflow model

- **`decimals` out of range** — `money::round` with `decimals` outside `0..5`
  reports `ErrInvalidArgument` (`77050002`).
- **Overflow on re-scale** — rounding can carry (`q → q+1`), so for a near-maximum
  `Money`, `(q+1) * divisor` can exceed `i64::MAX`. The re-multiply is
  **overflow-checked**: it traps `ErrOverflow` (`77050010`) rather than wrapping to
  a spuriously negative amount. [[src/codegen/builtins/money/func_round.rs:lower_money_round]]
- **Rates** — `money::toRate` raises `ErrInvalidFormat` (`77050003`) for bad text
  or more than 12 places, `ErrInvalidArgument` (`77050002`) for a rate at or below
  zero, and `ErrOverflow` (`77050010`) for one above `9223372.036854775807`.
  `money::convert` and `money::scale` raise `ErrInvalidArgument` for a `Rate`
  argument whose `scaled` is at or below zero, and `ErrOverflow` for a result
  outside the Money range.
- **`setRounding` / `getRounding`** — neither can fail; both are simple loads and
  stores of a single arena word, and `setRounding` masks its argument to the two
  valid modes.

These are consistent with the wider `Money` arithmetic error set (owned by
`./mfb spec language types`): overflow past the i64 raw range is `ErrOverflow`
(`77050010`), a zero divisor in a Money-result division or `MOD` is `ErrInvalidArgument`
(`77050002`), and a non-finite `Float` operand in a scaling is `ErrInvalidFormat`
(`77050003`).

## See Also

- `./mfb man money` — the per-function API for `money::round`, `money::setRounding`, `money::getRounding`, `money::toRate`, `money::convert` and `money::scale`.
- `./mfb man money types` — the `money::Rate` record and the `money::Rounding` enum.
- `./mfb spec language types` — the `Money` type: literals, range, dimensional algebra, and storage.
- `./mfb spec memory arenas` — the per-thread arena state, including the `moneyRoundingMode` word.
- `./mfb spec diagnostics error-codes` — `ErrInvalidArgument`, `ErrOverflow`, and the shared `7-705-*` runtime codes.
