# round

Settle a `Money` to a given number of decimal places under the current rounding mode

## Synopsis

```
money::round(value AS Money, decimals AS Integer) AS Money
```

## Package

`money`

## Imports

```
IMPORT money
```

`money` is a built-in package, so `IMPORT money` needs no manifest dependency.
[[src/builtins/money.rs:augmented_project]]

## Description

`money::round` settles `value` to `decimals` fractional places and returns the
result, still as a `Money`. It is the explicit "compute at five places, book at
two" operation: intermediate `Money` arithmetic keeps all five decimal places
that the type carries, and `money::round` is what settles a line item or an
allocation remainder to whole cents (`decimals` `2`) or another scale when it is
time to record it. [[src/builtins/money.rs:resolve_call]]

The computation is exact integer arithmetic on the underlying scaled value, with
no floating point anywhere: the raw is divided by `10^(5 - decimals)`, the
remainder is settled through the shared rounding helper, and the quotient is
multiplied back to `Money` scale.
[[src/target/shared/code/builder_money.rs:lower_money_round]]

How the remainder settles depends on the mode installed by `money::setRounding`.
A remainder that is not an exact half always goes to the nearer value, under
either mode. At an exact half, `Rounding.Commercial` (the default) rounds away
from zero and `Rounding.Banker` rounds to even — that is, it increments only when
the truncated quotient is odd. Negative amounts round symmetrically: the magnitude
is settled and the sign reapplied, so `money::round(-0.125m, 2)` under
`Rounding.Commercial` is `-0.13`.
[[src/target/shared/code/builder_money_math.rs:emit_apply_rounding]]

`decimals` must be in `0` through `5` inclusive; anything outside that range
fails with `ErrInvalidArgument`. The bounds are not arbitrary: `Money` is scaled
to exactly five decimal places, so `decimals` `5` is the identity and `decimals`
`0` settles to whole currency units — while remaining a `Money`, not an
`Integer`. [[src/target/shared/code/builder_money.rs:lower_money_round]]

Rounding can push a near-maximum `Money` past the representable range, because
settling upward returns a quotient one larger before it is scaled back. That
multiply is checked rather than allowed to wrap into a negative amount, so such a
call fails with `ErrOverflow` instead of returning a silently wrong figure.
[[src/target/shared/code/builder_money.rs:lower_money_round]]
[[src/target/shared/code/builder_numeric.rs:emit_checked_integer_multiply]]

`money::round` is distinct from two neighbouring operations that are easy to
reach for by mistake. `toString(Money)` applies presentation rounding, a fixed
half-away-from-zero rule that ignores the current mode, so formatting is not a
substitute for settling. And `math::round(Money)` leaves the `Money` dimension
entirely, yielding the dimensionless whole-unit `Integer` count;
`money::round(value, 0)` is the version that stays money.
[[src/target/shared/code/builder_money_math.rs:emit_apply_rounding]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Money` | The amount to settle. Any `Money` is accepted, including zero and negative amounts. [[src/builtins/money.rs:call_param_names]] |
| `decimals` | `Integer` | The number of fractional decimal places to keep. Must be `0` through `5` inclusive; `5` is the identity and `0` settles to whole currency units. [[src/target/shared/code/builder_money.rs:lower_money_round]] |

## Return value

| Type | Description |
| --- | --- |
| `Money` | `value` settled to `decimals` places under the current rounding mode, still a `Money` carrying five decimal places (the places below `decimals` are zero). [[src/builtins/money.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `decimals` is negative or greater than `5`. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] [[src/target/shared/code/builder_money.rs:lower_money_round]] |
| `77050010` | `ErrOverflow` | Settling rounds the magnitude up and the rescaled result no longer fits the `Money` range — reachable only for amounts near the representable maximum. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] [[src/target/shared/code/builder_money.rs:lower_money_round]] |

## Examples

Book a taxed line item to whole cents:

```
IMPORT money
IMPORT io

SUB main
  LET price AS Money = 19.99m
  LET line AS Money = price * 1.0825F
  LET booked AS Money = money::round(line, 2)
  io::print(toString(booked))
END SUB
```

The same tie settles differently under each mode:

```
IMPORT money
IMPORT io

SUB main
  money::setRounding(Rounding.Commercial)
  io::print(toString(money::round(0.125m, 2)))
  money::setRounding(Rounding.Banker)
  io::print(toString(money::round(0.125m, 2)))
END SUB
```

Settle to whole currency units without leaving the `Money` type:

```
IMPORT money
IMPORT io

SUB main
  io::print(toString(money::round(12.75m, 0)))
END SUB
```

## See also

- `mfb man money setRounding`
- `mfb man money getRounding`
- `mfb man money`
- `mfb man math round`
- `mfb man general toMoney`
