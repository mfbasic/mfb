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
is rounded to the nearest representable `Fixed` value; there is no `Fixed` suffix.
An `Integer` literal may initialize a `Byte` only when statically in range. A
`Money` literal uses an `m` or `M` suffix (`1.25m`) or an expected `Money` type.

```
LET x = 1.25             ' inferred Float
LET x = 1.25f            ' Float
LET x = 1.25F            ' Fixed
LET y AS Fixed = 1.25    ' Fixed
LET z = toFixed("1.25")  ' Fixed, fallible parse
LET price = 1.25m        ' Money
```

Bare numeric literals are also range-checked statically before any code runs; a
literal that cannot be represented in its target type is a compile error in the
`TYPE_*_LITERAL_*` family, distinct from the runtime conversion errors below.

## Promotion and result types

For mixed numeric operands the wider type wins, in the order
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
`>`, `<=`, `>=`) use the same operand promotion but always return `Boolean`.

This table covers only the four dimensionless numerics. `Money` is **dimensioned**
and does not promote against them — it has its own restricted operator algebra and
result-type table; see `Money rules` below.

## Integer and Byte rules

Integer arithmetic is checked and never wraps. Overflow in `+`, `-`, `*`, unary
`-`, exponentiation (`^`), and the minimum-integer `MOD -1` case fails with
`ErrOverflow`. Byte arithmetic that returns `Byte` is checked too: a result above
255 fails with `ErrOverflow` and a result below 0 fails with `ErrUnderflow`.
Runtime conversion to `Byte` uses `toByte` and fails with `ErrOverflow` outside 0
through 255. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]]

## Division, DIV, and MOD

`/` uses the promoted result type. When it promotes to `Byte` or `Integer` it
truncates the quotient toward zero, and division by zero for a non-`Float` result
fails with `ErrInvalidArgument`. `DIV` is fractional division and always returns
`Float`. Division by zero in a `Float`-result `/` or `DIV` is **not** pre-checked:
`x / 0` yields ±infinity and `0.0 / 0.0` yields NaN, which are caught at the next
observation boundary (`ErrFloatOverflow` / `ErrFloatNaN`).

`MOD` uses the promoted result type and is defined for every numeric pairing.
`a MOD b` with `b = 0` fails with `ErrFloatDomain` for a `Float` result and
`ErrInvalidArgument` otherwise. Otherwise the remainder has the same sign as `a`
and `a = truncTowardZero(a / b) * b + (a MOD b)`. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]]

## Exponentiation

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
as `exp` overflow) fails with `ErrFloatInf`. An imported native `Float` that is
already NaN or infinite is rejected at the boundary with `ErrInvalidFormat`. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_NAN_CODE]]

## Fixed rules

`Fixed` is deterministic binary fixed-point arithmetic. Overflow fails with
`ErrOverflow`, and divide-by-zero or an invalid numeric domain fails with
`ErrInvalidArgument`.

## Money rules

`Money` is an exact base-10 `i64` scaled by 10^5 (five decimal places): one unit
is 0.00001, and every decimal amount in the range -92233720368547.75808 through
92233720368547.75807 is represented exactly. [[src/numeric.rs:MONEY_SCALE]] It is
a **dimensioned** numeric with a restricted algebra, enforced at compile time as
`TYPE_MONEY_OPERATION_INVALID`: [[src/ir/verify/values.rs:check_money_operands]]

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

`Money` is **not a currency and carries no unit**. Its "dimension" is only
money-vs-dimensionless — the type stops you from mixing an amount with a bare
number, not from mixing dollars with euros. Nothing records or checks a currency:
adding a USD amount to a EUR amount, or scaling an amount by a rate whose units
don't line up, compiles and runs silently. Tracking the real-world unit and
making sure every conversion and every step of the algebra is unit-consistent is
the program's responsibility.

## Conversions

Converting a `Float` or `Fixed` to `Integer` or `Byte` fails with `ErrOverflow`
when the value is outside the destination range. Converting text to a numeric
type fails with `ErrInvalidFormat` when the text is malformed or names a
non-finite value such as `NaN` or `Infinity`. [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]]

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | non-`Float` division or `MOD` by zero; a negative `Integer` exponent; a `Fixed` divide-by-zero or invalid domain; a zero divisor in `Money` scaling [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77050003` | `ErrInvalidFormat` | converting text to a numeric type when it is malformed or names a non-finite value, or an imported native `Float` that is already NaN or infinite [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |
| `77050010` | `ErrOverflow` | checked `Integer`, `Byte`, `Fixed`, or `Money` overflow, or a `Float`/`Fixed`→`Integer`/`Byte` conversion outside the destination range [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |
| `77050011` | `ErrUnderflow` | a `Byte`-returning operation whose result is below 0 [[src/target/shared/code/error_constants.rs:ERR_UNDERFLOW_CODE]] |
| `77050012` | `ErrFloatDomain` | `Float`-result `MOD` by zero, a non-whole or negative `Float` exponent, or a math-function domain error (negative `sqrt`, non-positive `log`, out-of-range `asin`/`acos`) [[src/target/shared/code/error_constants.rs:ERR_FLOAT_DOMAIN_CODE]] |
| `77050013` | `ErrFloatNaN` | a NaN result reaching an observation boundary (such as `0.0 / 0.0`) [[src/target/shared/code/error_constants.rs:ERR_FLOAT_NAN_CODE]] |
| `77050014` | `ErrFloatInf` | a math kernel producing an infinity, such as `exp` overflow [[src/target/shared/code/error_constants.rs:ERR_FLOAT_INF_CODE]] |
| `77050015` | `ErrFloatOverflow` | arithmetic overflow to infinity reaching an observation boundary (such as `x / 0.0`) [[src/target/shared/code/error_constants.rs:ERR_FLOAT_OVERFLOW_CODE]] |

## See also

- `mfb man types`
- `mfb man types comparisons`
- `mfb man money`
- `mfb man general toInt`
- `mfb man errors`
