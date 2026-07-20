# twoOverPi

The mathematical constant `2 / pi` as a `Float`, the reciprocal of `pi / 2`.

## Synopsis

```
math::twoOverPi AS Float
```

## Package

math

## Imports

```
IMPORT math
```

`math` is a built-in package, so no manifest dependency is required.
[[src/builtins/math.rs:is_math_constant]]

## Description

`math::twoOverPi` is a constant, not a callable function. It takes no arguments and
no parentheses: write the name wherever a `Float` expression is expected.
[[src/builtins/math.rs:constant_type_name]]

`math::twoOverPi` is the closest 64-bit IEEE 754 double-precision `Float` to `2 / pi`, which is irrational and has no exact finite binary representation. Its decimal shorthand is `0.6366197723675814`. [[src/builtins/math.rs:constant_value]]

The constant is a compile-time value: it is substituted at the point of use, performs no computation and has no side effects, and evaluates to the same bit pattern on every reference and on every target. [[src/builtins/math.rs:is_math_constant]]

The same constant is also available as a `Fixed` under the name
`math::twoOverPiFixed`. There is no automatic conversion between the two forms, so pick
the one whose type matches the expression you are writing.
[[src/builtins/math.rs:constant_type_name]]

## Parameters

`math::twoOverPi` is a constant and takes no parameters.
[[src/builtins/math.rs:is_math_constant]]

## Return value

| Type | Description |
| --- | --- |
| `Float` | The `Float` nearest to `2 / pi`, approximately `0.6366197723675814`. The same value on every reference. [[src/builtins/math.rs:constant_value]] |

## Errors

No errors. Referencing a constant performs no computation, so there is no failure path. [[src/builtins/math.rs:constant_value]]

## Examples

Read the constant into a `Float` binding:

```
IMPORT math
IMPORT io

SUB main()
  LET value AS Float = math::twoOverPi
  io::print(toString(value))
END SUB
```

Scale an amplitude by `2 / pi`:

```
IMPORT math
IMPORT io

SUB main()
  LET amplitude AS Float = 3.0
  LET scaled AS Float = math::twoOverPi * amplitude
  io::print(toString(scaled))
END SUB
```

## See also

- `mfb man math twoOverPiFixed`
- `mfb man math pi`
- `mfb man math pi2`
- `mfb man math pi4`
- `mfb man math e`
- `mfb man math`
