# Money (money)

The `money` package controls how `Money` **arithmetic** settles the half case
and provides one explicit settling function. Called with the `money::` qualifier;
`IMPORT money` needs no manifest dependency. [[src/builtins/money.rs:is_money_call]]

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

Because the scale is a power of **ten**, every decimal amount within range is
stored with **no representation error**: `0.10`, `0.01`, `0.20` are the exact raw
integers `10000`, `1000`, `20000`. This is the property `Float` cannot provide —
an IEEE-754 binary carrier cannot represent `0.10` exactly, so repeated cent
accounting drifts. With `Money`, the only rounding that ever occurs is the
**deliberate** rounding of a division or an inexact scaling; addition, subtraction,
integer scaling, and comparison are exact integer operations on the raw i64 and
never round.

Because the carrier is an integer, all of `Money`'s exact operations are ordinary
integer arithmetic under overflow checking. Rounding enters only where a result
cannot be represented at five places — `M / k`, `M * Float`, `M * Fixed`, the
`toMoney` / `toFixed` conversions, and the explicit `money::round`. Every one of
those sites consults a single rounding rule, described below, so the two modes are
implemented exactly once. [[src/target/shared/code/builder_money_math.rs:emit_apply_rounding]]

## The rounding-mode state

There are two rounding modes, the members of the `Rounding` enum, whose
discriminants are exactly their stored values:

| Mode | Stored value | Half rule |
|------|-------------|-----------|
| `Commercial` | `0` | round half **away from zero** (the default) |
| `Banker` | `1` | round half to **even** (banker's rounding) |

`Commercial` is the **default** at program and thread start. `Banker` removes the
small upward bias of always rounding ties away, which matters when many rounded
amounts are summed. [[src/builtins/money_package.mfb:Rounding]]

The mode is **mutable, per-execution-context state**, not a global constant. It
lives in a single word of the per-thread arena state — the `moneyRoundingMode`
field, whose layout is owned by `./mfb spec memory arenas`.
[[src/target/shared/code/error_constants.rs:ARENA_ROUNDING_MODE_OFFSET]]

- `money::setRounding(mode)` writes the field. The stored value is masked to its
  low bit (`mode & 1`), so only the two defined modes are ever recorded.
  [[src/target/shared/code/builder_money.rs:lower_money_set_rounding]]
- `money::getRounding()` reads the field and returns it as a `Rounding` value.
  [[src/target/shared/code/builder_money.rs:lower_money_get_rounding]]

Because the field is part of the arena state, the mode is **per-thread**: each OS
thread owns its own arena and therefore its own mode word. A worker thread
**inherits** the spawning thread's mode at spawn — the child arena is zeroed to
`Commercial`, then the parent's mode is copied in — and thereafter changes
independently, exactly parallel to the per-thread RNG and other arena state.
Setting the mode on one thread never disturbs another.
[[src/target/shared/code/runtime_helpers.rs:455]]

Setting the mode affects **only** `Money` arithmetic. It does not change `Fixed`
or `Float` rounding, and — importantly — it does **not** change how
`toString(Money)` renders a value: presentation rounding is a fixed
half-away-from-zero rule, independent of the mode, so a displayed or logged amount
is a pure function of its value and requested precision. This decoupling enables
the common workflow of accumulating under one mode and presenting under another.

## The rounding rule

Every mode-sensitive site reduces to rounding a signed division. Given a
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
This is the sole implementation of both modes.
[[src/target/shared/code/builder_money_math.rs:emit_apply_rounding]]

The rule is fully **deterministic**: for a given operand, mode, and target scale
the rounded result is identical on every run and every target, because it is exact
integer arithmetic with no floating intermediate. (The one inherently inexact
`Money` operation, `Money * Float`, is inexact only because the `Float` factor is
itself approximate — the rounding step applied to it is still deterministic.)

## `money::round(value, decimals)`

`money::round` explicitly settles a `Money` to `decimals` fractional places under
the current mode — the "compute at five places, book at two" operation. It stays a
`Money` (contrast `math::round(Money)`, which exits the currency dimension to a
dimensionless whole-unit `Integer`). [[src/target/shared/code/builder_money.rs:lower_money_round]]

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

## Error and overflow model

- **`decimals` out of range** — `money::round` with `decimals` outside `0..5`
  reports `ErrInvalidArgument` (`77050002`).
- **Overflow on re-scale** — rounding can carry (`q → q+1`), so for a near-maximum
  `Money`, `(q+1) * divisor` can exceed `i64::MAX`. The re-multiply is
  **overflow-checked**: it traps `ErrOverflow` (`77050010`) rather than wrapping to
  a spuriously negative amount. [[src/target/shared/code/builder_money.rs:lower_money_round]]
- **`setRounding` / `getRounding`** — neither can fail; both are simple loads and
  stores of a single arena word, and `setRounding` masks its argument to the two
  valid modes.

These are consistent with the wider `Money` arithmetic error set (owned by
`./mfb spec language types`): overflow past the i64 raw range is `ErrOverflow`
(`77050010`), a non-`Float` divide or `MOD` by zero is `ErrInvalidArgument`
(`77050002`), and a non-finite `Float` operand in a scaling is `ErrInvalidFormat`
(`77050003`).

## See Also

- `./mfb man money` — the per-function API for `money::round`, `money::setRounding`, and `money::getRounding`.
- `./mfb spec language types` — the `Money` type: literals, range, dimensional algebra, and storage.
- `./mfb spec memory arenas` — the per-thread arena state, including the `moneyRoundingMode` word.
- `./mfb spec diagnostics error-codes` — `ErrInvalidArgument`, `ErrOverflow`, and the shared `7-705-*` runtime codes.
