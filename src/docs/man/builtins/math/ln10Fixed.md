# ln10Fixed

The mathematical constant `ln(10)` as a `Fixed`, the natural logarithm of 10.

## Synopsis

```
math::ln10Fixed AS Fixed
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

`math::ln10Fixed` is a constant, not a callable function. It takes no arguments and
no parentheses: write the name wherever a `Fixed` expression is expected.
[[src/builtins/math.rs:constant_type_name]]

`math::ln10Fixed` is the closest Q32.32 `Fixed` to `ln(10)`, which is irrational and has no exact finite representation. Its decimal shorthand is `2.302585092994046`; the stored `Fixed` is that value rounded to the nearest representable Q32.32 step, so it carries less precision than the `Float` form `math::ln10`. [[src/builtins/math.rs:constant_value]]

The constant is a compile-time value: it is substituted at the point of use, performs no computation and has no side effects, and evaluates to the same bit pattern on every reference. Because `Fixed` is Q32.32 integer arithmetic, that bit pattern is identical on every target by construction, which makes `math::ln10Fixed` the right choice when a result must be reproducible across platforms. [[src/docs/spec/architecture/18_math-kernels.md]]

The same constant is also available as a `Float` under the name
`math::ln10`. There is no automatic conversion between the two forms, so pick
the one whose type matches the expression you are writing.
[[src/builtins/math.rs:constant_type_name]]

## Parameters

`math::ln10Fixed` is a constant and takes no parameters.
[[src/builtins/math.rs:is_math_constant]]

## Return value

| Type | Description |
| --- | --- |
| `Fixed` | The `Fixed` nearest to `ln(10)`, approximately `2.302585092994046`. The same value on every reference. [[src/builtins/math.rs:constant_value]] |

## Errors

No errors. Referencing a constant performs no computation, so there is no failure path. [[src/builtins/math.rs:constant_value]]

## Examples

Read the constant into a `Fixed` binding:

```
IMPORT math
IMPORT io

SUB main()
  LET value AS Fixed = math::ln10Fixed
  io::print(toString(value))
END SUB
```

Convert a natural logarithm to base 10, deterministically:

```
IMPORT math
IMPORT io

SUB main()
  LET x AS Fixed = 1000.0F
  LET log10 AS Fixed = math::log(x) / math::ln10Fixed
  io::print(toString(log10))
END SUB
```

## See also

- `mfb man math ln10`
- `mfb man math ln2Fixed`
- `mfb man math eFixed`
- `mfb man math log`
- `mfb man math piFixed`
- `mfb man math`
