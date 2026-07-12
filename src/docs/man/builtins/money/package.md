# money

Rounding-mode control for Money arithmetic

## Synopsis

```
IMPORT money
money::setRounding(Rounding.Banker)
money::getRounding()
money::round(value, decimals)
```

## Description

The `money` package controls how `Money` **arithmetic** settles the half case and
provides an explicit settling function. `Money` itself is a built-in scalar type
(see `mfb man types money`): an exact base-10 fixed-point value scaled to five
decimal places. Its arithmetic (`M / k`, `M * Float`, `M * Fixed`, and the
`toMoney`/`toFixed` conversions) rounds under a per-execution-context mode that
this package reads and writes. `money` is a built-in package: `IMPORT money` needs
no manifest dependency. [[src/builtins/money.rs:is_money_call]]

The mode is one of the `Rounding` enum members:

- `Commercial` — round half **away from zero** (the default).
- `Banker` — round half to **even** (banker's rounding), which removes the small
  upward bias of always rounding ties away.

The mode is per-thread state: a worker thread inherits the spawning thread's mode
and then diverges independently, consistent with the per-thread RNG and other
arena state. It affects only `Money` arithmetic — it does not change `Fixed` or
`Float` rounding, and it does **not** change how `toString(Money)` renders a
value. `toString` presentation rounding is a fixed half-away-from-zero rule
independent of the mode, so a logged or displayed amount is a pure function of its
value and precision. This decoupling enables the common workflow of accumulating
under one mode and presenting under another.

`money::round(value, decimals)` explicitly settles an amount to `decimals` places
under the current mode ("compute at five places, book at two"). It stays a
`Money`; contrast `math::round(Money)`, which exits the dimension to the
dimensionless whole-unit `Integer` count with a fixed half-away rule.

## Functions

- `setRounding(mode)` — set the current Money rounding mode.
- `getRounding()` — read the current Money rounding mode.
- `round(value, decimals)` — settle a Money to `decimals` places under the mode.

## See Also

- `mfb man types money`
- `mfb man general toMoney`
- `mfb man math round`
