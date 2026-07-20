# getRounding

Read the rounding mode currently in effect for `Money` arithmetic

## Synopsis

```
money::getRounding() AS Rounding
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

`money::getRounding` returns the `Money` arithmetic rounding mode currently in
effect, as a `Rounding` value. It takes no arguments and always succeeds.
[[src/builtins/money.rs:call_return_type_name]]

The mode is not a call into a runtime helper — it is lowered inline to a single
load of the per-execution-context rounding-mode field held in the arena state
region, so reading it is as cheap as reading a local. The stored value is exactly
the enum discriminant: `0` for `Rounding.Commercial`, `1` for `Rounding.Banker`,
and only those two values are ever stored, because `money::setRounding` masks its
argument to the low bit before writing.
[[src/target/shared/code/builder_money.rs:lower_money_get_rounding]]
[[src/target/shared/code/builder_money.rs:lower_money_set_rounding]]

The mode is per-execution-context state, so `getRounding` reports the mode of the
thread that calls it: the value most recently written by `money::setRounding` on
this thread, or — if this thread has never set it — the mode it inherited from its
spawning thread. A program that has never called `money::setRounding` observes
`Rounding.Commercial`, the default.
[[src/builtins/money_package.mfb:Rounding]]

The returned mode governs `Money` **arithmetic** rounding only: `money::round`,
division of a `Money` by a scalar, scaling a `Money` by a `Float` or `Fixed`, and
the `toMoney` / `toFixed` conversions. It does **not** describe how
`toString(Money)` renders a value — presentation rounding is a fixed
half-away-from-zero rule that ignores the mode entirely, so `getRounding` is not
a way to predict formatted output.
[[src/target/shared/code/builder_money_math.rs:emit_apply_rounding]]

The `Rounding` enum is referenced bare, like every other builtin type: write
`Rounding.Banker`, not `money::Rounding.Banker`.
[[src/builtins/money.rs:is_builtin_type]]

## Parameters

`money::getRounding` takes no parameters. [[src/builtins/money.rs:arity]]

## Return value

| Type | Description |
| --- | --- |
| `Rounding` | The mode in effect on the calling thread: `Rounding.Commercial` (round half away from zero, the default) or `Rounding.Banker` (round half to even). [[src/builtins/money.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Branch on the mode currently in effect:

```
IMPORT money
IMPORT io

SUB main
  IF money::getRounding() = Rounding.Banker THEN
    io::print("banker's rounding is active")
  END IF
END SUB
```

Save the mode, switch it for one calculation, then restore what was there before:

```
IMPORT money
IMPORT io

SUB main
  LET previous AS Rounding = money::getRounding()
  money::setRounding(Rounding.Banker)
  io::print(toString(money::round(0.125m, 2)))
  money::setRounding(previous)
END SUB
```

## See also

- `mfb man money setRounding`
- `mfb man money round`
- `mfb man money`
- `mfb man types numeric`
