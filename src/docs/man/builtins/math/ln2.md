# ln2

The mathematical constant `ln(2)` as a `Float`, the natural logarithm of 2.

## Synopsis

```
math::ln2 AS Float
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

`math::ln2` is a constant, not a callable function. It takes no arguments and
no parentheses: write the name wherever a `Float` expression is expected.
[[src/builtins/math.rs:constant_type_name]]

`math::ln2` is the closest 64-bit IEEE 754 double-precision `Float` to `ln(2)`, which is irrational and has no exact finite binary representation. Its decimal shorthand is `0.6931471805599453`. [[src/builtins/math.rs:constant_value]]

The constant is a compile-time value: it is substituted at the point of use, performs no computation and has no side effects, and evaluates to the same bit pattern on every reference and on every target. [[src/builtins/math.rs:is_math_constant]]

The same constant is also available as a `Fixed` under the name
`math::ln2Fixed`. There is no automatic conversion between the two forms, so pick
the one whose type matches the expression you are writing.
[[src/builtins/math.rs:constant_type_name]]

## Parameters

`math::ln2` is a constant and takes no parameters.
[[src/builtins/math.rs:is_math_constant]]

## Return value

| Type | Description |
| --- | --- |
| `Float` | The `Float` nearest to `ln(2)`, approximately `0.6931471805599453`. The same value on every reference. [[src/builtins/math.rs:constant_value]] |

## Errors

No errors. Referencing a constant performs no computation, so there is no failure path. [[src/builtins/math.rs:constant_value]]

## Examples

Read the constant into a `Float` binding:

```
IMPORT math
IMPORT io

SUB main()
  LET value AS Float = math::ln2
  io::print(toString(value))
END SUB
```

Convert a natural logarithm to base 2:

```
IMPORT math
IMPORT io

SUB main()
  LET x AS Float = 8.0
  LET log2 AS Float = math::log(x) / math::ln2
  io::print(toString(log2))
END SUB
```

## See also

- `mfb man math ln2Fixed`
- `mfb man math ln10`
- `mfb man math e`
- `mfb man math log`
- `mfb man math log10`
- `mfb man math`
