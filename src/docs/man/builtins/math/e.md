# e

The mathematical constant `e` as a `Float`, Euler's number, the base of the natural logarithm.

## Synopsis

```
math::e AS Float
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

`math::e` is a constant, not a callable function. It takes no arguments and
no parentheses: write the name wherever a `Float` expression is expected.
[[src/builtins/math.rs:constant_type_name]]

`math::e` is the closest 64-bit IEEE 754 double-precision `Float` to `e`, which is irrational and has no exact finite binary representation. Its decimal shorthand is `2.718281828459045`. [[src/builtins/math.rs:constant_value]]

The constant is a compile-time value: it is substituted at the point of use, performs no computation and has no side effects, and evaluates to the same bit pattern on every reference and on every target. [[src/builtins/math.rs:is_math_constant]]

The same constant is also available as a `Fixed` under the name
`math::eFixed`. There is no automatic conversion between the two forms, so pick
the one whose type matches the expression you are writing.
[[src/builtins/math.rs:constant_type_name]]

## Parameters

`math::e` is a constant and takes no parameters.
[[src/builtins/math.rs:is_math_constant]]

## Return value

| Type | Description |
| --- | --- |
| `Float` | The `Float` nearest to `e`, approximately `2.718281828459045`. The same value on every reference. [[src/builtins/math.rs:constant_value]] |

## Errors

No errors. Referencing a constant performs no computation, so there is no failure path. [[src/builtins/math.rs:constant_value]]

## Examples

Read the constant into a `Float` binding:

```
IMPORT math
IMPORT io

SUB main()
  LET value AS Float = math::e
  io::print(toString(value))
END SUB
```

Check that `math::log` inverts the exponential at `e`:

```
IMPORT math
IMPORT io

SUB main()
  LET one AS Float = math::log(math::e)
  io::print(toString(one))
END SUB
```

## See also

- `mfb man math eFixed`
- `mfb man math exp`
- `mfb man math ln2`
- `mfb man math ln10`
- `mfb man math log`
- `mfb man math`
