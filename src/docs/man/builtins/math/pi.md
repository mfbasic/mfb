# pi

The mathematical constant `pi` as a `Float`, the ratio of a circle's circumference to its diameter.

## Synopsis

```
math::pi AS Float
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

`math::pi` is a constant, not a callable function. It takes no arguments and
no parentheses: write the name wherever a `Float` expression is expected.
[[src/builtins/math.rs:constant_type_name]]

`math::pi` is the closest 64-bit IEEE 754 double-precision `Float` to `pi`, which is irrational and has no exact finite binary representation. Its decimal shorthand is `3.141592653589793`. [[src/builtins/math.rs:constant_value]]

The constant is a compile-time value: it is substituted at the point of use, performs no computation and has no side effects, and evaluates to the same bit pattern on every reference and on every target. [[src/builtins/math.rs:is_math_constant]]

The same constant is also available as a `Fixed` under the name
`math::piFixed`. There is no automatic conversion between the two forms, so pick
the one whose type matches the expression you are writing.
[[src/builtins/math.rs:constant_type_name]]

## Parameters

`math::pi` is a constant and takes no parameters.
[[src/builtins/math.rs:is_math_constant]]

## Return value

| Type | Description |
| --- | --- |
| `Float` | The `Float` nearest to `pi`, approximately `3.141592653589793`. The same value on every reference. [[src/builtins/math.rs:constant_value]] |

## Errors

No errors. Referencing a constant performs no computation, so there is no failure path. [[src/builtins/math.rs:constant_value]]

## Examples

Read the constant into a `Float` binding:

```
IMPORT math
IMPORT io

SUB main()
  LET value AS Float = math::pi
  io::print(toString(value))
END SUB
```

Compute a circle's circumference from its radius:

```
IMPORT math
IMPORT io

SUB main()
  LET radius AS Float = 2.5
  LET circumference AS Float = 2.0 * math::pi * radius
  io::print(toString(circumference))
END SUB
```

## See also

- `mfb man math piFixed`
- `mfb man math pi2`
- `mfb man math pi4`
- `mfb man math twoOverPi`
- `mfb man math e`
- `mfb man math`
