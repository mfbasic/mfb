# numeric

Primitive numeric types, promotion, and checked-arithmetic rules

## Synopsis

```
Integer  Float  Fixed  Byte  Money
```

## Description

MFBASIC numeric primitives are checked values: arithmetic never silently wraps,
and failures are ordinary `Error` results that route through an inline or
function-level `TRAP` or propagate to the caller (see `mfb man errors`).

- **`Integer`** — 64-bit signed integer.
- **`Float`** — 64-bit IEEE 754 binary64. No user-accessible `Float` is
  non-finite (see Float rules).
- **`Fixed`** — 64-bit deterministic binary fixed-point with a signed 32/32
  split and resolution 1 / 2^32, ranging approximately -2147483648.0 through
  2147483647.9999999998. It is not exact decimal currency arithmetic, because
  most decimal fractions are rounded to binary fixed-point values.
- **`Byte`** — unsigned 8-bit integer, range 0 through 255.
- **`Money`** — exact base-10 fixed-point scaled to five decimal places, for
  auditable financial amounts (see Money rules).

`Boolean` and `String` are primitive but not numeric and do not participate in
numeric promotion.

## Literals

Numeric literals are initially untyped. An integer-looking literal (no `.`)
defaults to `Integer` and a decimal-looking literal defaults to `Float` when
there is no expected type. When the expected type is `Fixed`, a decimal literal
is rounded to the nearest representable `Fixed` value. Uppercase `F` selects
`Fixed`; lowercase `f` selects `Float`.
An `Integer` literal may initialize a `Byte` only when statically in range. A
`Money` literal uses an `m` or `M` suffix (`1.25m`) or an expected `Money` type.

```
LET x = 1.25             ' inferred Float
LET explicitFloat = 1.25f ' Float
LET explicitFixed = 1.25F ' Fixed
LET y AS Fixed = 1.25    ' Fixed
LET z = toFixed("1.25")  ' Fixed, fallible parse
LET price = 1.25m        ' Money
```

Bare numeric literals are also range-checked statically before any code runs; a
literal that cannot be represented in its target type is a compile error in the
`TYPE_*_LITERAL_*` family, distinct from the runtime conversion errors below.

A Fixed literal is rounded to the nearest value a Fixed can hold (ties away
from zero) and is accepted exactly when that rounded value is in range, so the
largest Fixed, `2147483647.99999999976716935634613037109375F`, and the smallest,
`-2147483648.0F`, can both be written. See `mfb spec language types` for the
precise literal acceptance rules.

## Promotion and result types

For mixed arithmetic operands the type is selected in the order
`Fixed > Float > Integer > Byte`. The result type of `+`, `-`, `*`, `^`, `/`, and
`MOD` follows this table; `DIV` always returns `Float`.

| OpA | OpB | `+ - * ^ / MOD` | `DIV` |
| --- | --- | --- | --- |
| `Byte` | `Byte` | `Byte` | `Float` |
| `Byte` | `Integer` | `Integer` | `Float` |
| `Byte` | `Fixed` | `Fixed` | `Float` |
| `Byte` | `Float` | `Float` | `Float` |
| `Integer` | `Integer` | `Integer` | `Float` |
| `Integer` | `Fixed` | `Fixed` | `Float` |
| `Integer` | `Float` | `Float` | `Float` |
| `Fixed` | `Fixed` | `Fixed` | `Float` |
| `Fixed` | `Float` | `Fixed` | `Float` |
| `Float` | `Float` | `Float` | `Float` |

The table is symmetric in the operand order. Numeric comparisons (`=`, `<>`, `<`,
`>`, `<=`, `>=`) return `Boolean` and convert their operands by the same
table; see `mfb man types comparisons`.

Promotion can fail: `Fixed` has a smaller range than `Integer` or `Float`.
Combining a Fixed with an Integer outside -2147483648 through 2147483647 raises
`ErrOverflow` before the operator runs. Float-to-Fixed conversion rounds to
nearest with ties away from zero and also raises `ErrOverflow` outside the
Fixed range. The Money rounding mode does not affect this conversion.

This table covers only the four dimensionless numerics. `Money` is **dimensioned**
and does not promote against them — it has its own restricted operator algebra and
result-type table; see `Money rules` below.

## Integer and Byte rules

Integer arithmetic is checked and never wraps. Overflow in `+`, `-`, `*`, unary
`-`, exponentiation (`^`), and the minimum-integer `MOD -1` case fails with
`ErrOverflow`. Byte arithmetic that returns `Byte` is checked too: a result above
255 fails with `ErrOverflow` and a result below 0 fails with `ErrUnderflow`.
Negating a nonzero Byte raises `ErrUnderflow`; negating Byte zero returns zero.
Runtime conversion from Integer to Byte uses `toByte` and fails with
`ErrOverflow` outside 0 through 255.

## Division, DIV, and MOD

`/` uses the promoted result type. When it promotes to `Byte` or `Integer` it
truncates the quotient toward zero, and division by zero for a non-`Float` result
fails with `ErrInvalidArgument`. `DIV` is fractional division and always returns
`Float`. Division by zero in a `Float`-result `/` or `DIV` is **not** pre-checked:
`x / 0` yields ±infinity and `0.0 / 0.0` yields NaN, which are caught at the next
observation boundary (`ErrFloatOverflow` / `ErrFloatNaN`).

`MOD` uses the promoted result type for the dimensionless types. Money permits
only `Money MOD Money`; its remainder is exact at five decimal places and is
independent of the Money rounding mode.
`a MOD b` with `b = 0` fails with `ErrFloatDomain` for a `Float` result and
`ErrInvalidArgument` otherwise. Otherwise the remainder has the same sign as `a`
and `a = truncTowardZero(a / b) * b + (a MOD b)`.

## Exponentiation

Byte exponentiation returns Byte and raises `ErrOverflow` if the result exceeds
255. Fixed exponentiation requires a whole, non-negative exponent; a fractional
or negative exponent raises `ErrInvalidArgument`. `0 ^ 0` returns one for Byte,
Integer, Fixed, and Float results.

`^` for an `Integer` result requires a non-negative integer exponent; a negative
exponent fails with `ErrInvalidArgument` and overflow fails with `ErrOverflow`. A
`Float`-result `^` requires a whole, non-negative exponent and fails with
`ErrFloatDomain` otherwise; overflow to infinity is caught at the observation
boundary, not at the operator. Note `^` is right-associative and unary `-` binds
tighter than `^`, so `-2 ^ 2` parses as `(-2) ^ 2`.

## Float rules

`Float` follows IEEE 754 binary64, but MFBASIC guarantees that no user-accessible
`Float` is non-finite. Finiteness is enforced at observation boundaries — where a
`Float` becomes observable by being bound, assigned, stored into a collection or
record, returned, passed as an argument, or printed/converted — not after each
operation, so an anonymous intermediate may be transiently non-finite and recover
to finite without trapping. At a boundary a NaN fails with `ErrFloatNaN` and an
infinity fails with `ErrFloatOverflow` ("arithmetic overflow to infinity").
Built-in math functions with a genuine domain error (a negative `sqrt`, a
non-positive `log`/`log10`, an out-of-range `asin`/`acos`) fail with
`ErrFloatDomain` at the call, and a math kernel that produces an infinity (such
as `exp` overflow) fails with `ErrFloatInf`. A native CDouble result that is NaN raises `ErrFloatNaN`; an infinite result
raises `ErrFloatInf`.

A direct Float multiply followed by addition or subtraction can be fused:
`a*b + c`, `c + a*b`, `a*b - c`, and `c - a*b` round the combined operation
once when the product is used only there. Parentheses around the product do
not stop fusion. Binding the product to a separate variable or passing it to
a conversion makes it observable first and prevents fusion across that step.
This applies even with optimization disabled. Consequently `a * 2.0 - a` can
stay finite for `a = 1.5e308`, while binding `a * 2.0` separately raises
`ErrFloatOverflow`. Do not expect algebraically rearranged expressions to round
identically; the full contraction rules are in `mfb spec language operators`.

## Fixed rules

`Fixed` is deterministic binary fixed-point arithmetic. Overflow fails with
`ErrOverflow`, and divide-by-zero or an invalid numeric domain fails with
`ErrInvalidArgument`.

## Money rules

`Money` represents monetary amounts with five decimal places: its smallest step
is 0.00001. Every amount with at most five fractional places in the range
-92233720368547.75808 through 92233720368547.75807 is represented exactly. It is
a **dimensioned** numeric with a restricted algebra, enforced at compile time as
`TYPE_MONEY_OPERATION_INVALID`:

- Add or subtract two `Money` amounts (`M + M`, `M - M`), and take a `Money`
  remainder (`M MOD M`) — both operands must be `Money`.
- Scale an amount by a dimensionless number (`M * k` or `M / k`, where `k` is
  `Integer`, `Byte`, `Float`, or `Fixed`) — the result is `Money`.
- Take the ratio of two amounts (`M / M`) — the result is `Float`.
- Comparisons require both operands to be `Money`.

`Money` is dimensioned and does **not** share the promotion table above; its
operators do not all agree on a result type, so each is listed separately. A cell
marked *error* is a compile-time `TYPE_MONEY_OPERATION_INVALID` (`k` is any
dimensionless `Byte`, `Integer`, `Fixed`, or `Float`):

| Left | Right | `+ - MOD` | `*` | `/` | `DIV` |
| --- | --- | --- | --- | --- | --- |
| `Money` | `Money` | `Money` | error | `Float` | `Float` |
| `Money` | `Byte` | error | `Money` | `Money` | `Float` |
| `Money` | `Integer` | error | `Money` | `Money` | `Float` |
| `Money` | `Fixed` | error | `Money` | `Money` | `Float` |
| `Money` | `Float` | error | `Money` | `Money` | `Float` |
| `Byte` | `Money` | error | `Money` | error | error |
| `Integer` | `Money` | error | `Money` | error | error |
| `Fixed` | `Money` | error | `Money` | error | error |
| `Float` | `Money` | error | `Money` | error | error |

Exponentiation (`^`) is an error for every pairing that involves a `Money`.

Combining a `Money` with a bare number under `+`/`-`/`MOD`, multiplying two
amounts, dividing a non-`Money` by a `Money`, comparing an amount to a bare
number, or raising an amount to a power are all rejected. Crossing into or out of
`Money` is an explicit `toMoney`/`to*` call — there is no implicit conversion. At
run time a `Money` result that overflows fails with `ErrOverflow`, and a zero
divisor in `M / k` or a `Fixed`-scaled divide fails with `ErrInvalidArgument`.
`Money` arithmetic rounding follows a per-thread mode; see `mfb man money`.

`Money` distinguishes **monetary amounts from dimensionless quantities**, but
does not record a currency identity. Its distinction is amount-vs-dimensionless — the type stops you from mixing an amount with a bare
number, not from mixing dollars with euros. Nothing records or checks a currency:
adding a USD amount to a EUR amount, or scaling an amount by a rate whose units
don't line up, compiles and runs silently. Tracking the real-world unit and
making sure every conversion and every step of the algebra is unit-consistent is
the program's responsibility.

## Conversions

`toInt(Float)`, `toInt(Fixed)`, and `toInt(Money)` truncate toward zero.
Fixed and Money always fit Integer after this conversion; Float can raise
`ErrOverflow`. `toByte` accepts Integer, Money, and Scalar, with no direct Float
or Fixed conversion. For Money it first truncates to whole units, then checks
0 through 255: `toByte(-0.9m)` returns zero, while `toByte(-1m)` raises
`ErrOverflow`. Use `toByte(toInt(value))` when explicitly truncating a Float or
Fixed to Byte. Converting text to a numeric
type fails with `ErrInvalidFormat` when the text is malformed or names a
non-finite value such as `NaN` or `Infinity`.

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | non-`Float` division or `MOD` by zero; a negative Integer exponent or a negative/fractional Fixed exponent; a `Fixed` divide-by-zero or invalid domain; a zero divisor in `Money` scaling |
| `77050003` | `ErrInvalidFormat` | converting text to a numeric type when it is malformed or names a non-finite value, or converting a non-finite numeric representation |
| `77050010` | `ErrOverflow` | checked `Integer`, `Byte`, `Fixed`, or `Money` overflow, or a numeric conversion outside the destination range |
| `77050011` | `ErrUnderflow` | a `Byte`-returning operation whose result is below 0 |
| `77050012` | `ErrFloatDomain` | `Float`-result `MOD` by zero, a non-whole or negative `Float` exponent, or a math-function domain error (negative `sqrt`, non-positive `log`, out-of-range `asin`/`acos`) |
| `77050013` | `ErrFloatNaN` | a NaN result reaching an observation boundary (such as `0.0 / 0.0`) |
| `77050014` | `ErrFloatInf` | a math kernel or native CDouble result producing infinity |
| `77050015` | `ErrFloatOverflow` | arithmetic overflow to infinity reaching an observation boundary (such as `x / 0.0`) |

## See also

- `mfb man types`
- `mfb man types comparisons`
- `mfb man money`
- `mfb man general toInt`
- `mfb man errors`
