# pi4

The mathematical constant `pi / 4` as a `Float`, an eighth turn (45 degrees) in radians.

## Synopsis

```
math::pi4 AS Float
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

`math::pi4` is a constant, not a callable function. It takes no arguments and
no parentheses: write the name wherever a `Float` expression is expected.
[[src/builtins/math.rs:constant_type_name]]

`math::pi4` is the closest 64-bit IEEE 754 double-precision `Float` to `pi / 4`, which is irrational and has no exact finite binary representation. Its decimal shorthand is `0.7853981633974483`. [[src/builtins/math.rs:constant_value]]

The constant is a compile-time value: it is substituted at the point of use, performs no computation and has no side effects, and evaluates to the same bit pattern on every reference and on every target. [[src/builtins/math.rs:is_math_constant]]

The same constant is also available as a `Fixed` under the name
`math::pi4Fixed`. There is no automatic conversion between the two forms, so pick
the one whose type matches the expression you are writing.
[[src/builtins/math.rs:constant_type_name]]

## Parameters

`math::pi4` is a constant and takes no parameters.
[[src/builtins/math.rs:is_math_constant]]

## Return value

| Type | Description |
| --- | --- |
| `Float` | The `Float` nearest to `pi / 4`, approximately `0.7853981633974483`. The same value on every reference. [[src/builtins/math.rs:constant_value]] |

## Errors

No errors. Referencing a constant performs no computation, so there is no failure path. [[src/builtins/math.rs:constant_value]]

## Examples

Read the constant into a `Float` binding:

```
IMPORT math
IMPORT io

SUB main()
  LET value AS Float = math::pi4
  io::print(toString(value))
END SUB
```

Take the tangent of an eighth turn:

```
IMPORT math
IMPORT io

SUB main()
  LET eighthTurn AS Float = math::tan(math::pi4)
  io::print(toString(eighthTurn))
END SUB
```

## See also

- `mfb man math pi4Fixed`
- `mfb man math pi`
- `mfb man math pi2`
- `mfb man math twoOverPi`
- `mfb man math tan`
- `mfb man math`
