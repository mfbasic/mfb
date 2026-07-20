# setRounding

Set the rounding mode used by `Money` arithmetic on the calling thread

## Synopsis

```
money::setRounding(mode AS Rounding)
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

`money::setRounding` selects how `Money` arithmetic settles the exact half case.
`mode` is one of the two `Rounding` enum members: `Rounding.Commercial` (round
half **away from zero**, the default) or `Rounding.Banker` (round half to
**even**, which removes the small upward bias of always rounding ties away).
The call returns nothing. [[src/builtins/money_package.mfb:Rounding]]
[[src/builtins/money.rs:call_return_type_name]]

The call is lowered inline to a mask and a single store into the
per-execution-context rounding-mode field in the arena state region. The stored
value is the enum discriminant masked to its low bit, so exactly `0` or `1` is
ever written and a later `money::getRounding` reads back the same member.
[[src/target/shared/code/builder_money.rs:lower_money_set_rounding]]

The mode is per-execution-context state. A worker thread inherits the spawning
thread's mode at spawn and then changes independently, so setting the mode on one
thread never disturbs another. There is no scoped or automatic restore: the mode
stays as you set it until it is set again, so a routine that changes the mode for
one calculation should read the previous value with `money::getRounding` and put
it back.

The mode applies to every `Money` **arithmetic** rounding site — `money::round`,
dividing a `Money` by a scalar, scaling a `Money` by a `Float` or `Fixed`, and the
`toMoney` / `toFixed` conversions — all of which route through the one shared
rounding helper. Under either mode a result that is not an exact half is
unaffected: the difference appears only at a true tie.
[[src/target/shared/code/builder_money_math.rs:emit_apply_rounding]]

Two things the mode does **not** affect. It has no bearing on `Fixed` or `Float`
rounding, which are separate types with their own rules. And it does not change
how `toString(Money)` renders a value: presentation rounding is a fixed
half-away-from-zero rule, deliberately independent of the mode, so a displayed
amount is a pure function of the value.
[[src/target/shared/code/builder_money_math.rs:emit_apply_rounding]]

The `Rounding` enum is referenced bare, like every other builtin type: write
`Rounding.Banker`, not `money::Rounding.Banker`.
[[src/builtins/money.rs:is_builtin_type]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `mode` | `Rounding` | The mode to install for `Money` arithmetic on the calling thread: `Rounding.Commercial` or `Rounding.Banker`. Any other type is rejected at compile time. [[src/builtins/money.rs:call_param_names]] [[src/builtins/money.rs:resolve_call]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `money::setRounding` produces no value; call it as a statement. [[src/builtins/money.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Accumulate under banker's rounding, then restore the default:

```
IMPORT money
IMPORT io

SUB main
  money::setRounding(Rounding.Banker)
  io::print(toString(money::round(0.125m, 2)))
  money::setRounding(Rounding.Commercial)
  io::print(toString(money::round(0.125m, 2)))
END SUB
```

The two modes differ only at an exact tie:

```
IMPORT money
IMPORT io

SUB main
  money::setRounding(Rounding.Commercial)
  io::print(toString(money::round(0.125m, 2)))
  money::setRounding(Rounding.Banker)
  io::print(toString(money::round(0.125m, 2)))
  io::print(toString(money::round(0.135m, 2)))
END SUB
```

## See also

- `mfb man money getRounding`
- `mfb man money round`
- `mfb man money`
- `mfb man general toMoney`
