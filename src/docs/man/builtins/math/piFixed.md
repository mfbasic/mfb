# piFixed

The mathematical constant `pi` as a `Fixed`, the ratio of a circle's circumference to its diameter.

## Synopsis

```
math::piFixed AS Fixed
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

`math::piFixed` is a constant, not a callable function. It takes no arguments and
no parentheses: write the name wherever a `Fixed` expression is expected.
[[src/builtins/math.rs:constant_type_name]]

`math::piFixed` is the closest Q32.32 `Fixed` to `pi`, which is irrational and has no exact finite representation. Its decimal shorthand is `3.141592653589793`; the stored `Fixed` is that value rounded to the nearest representable Q32.32 step, so it carries less precision than the `Float` form `math::pi`. [[src/builtins/math.rs:constant_value]]

The constant is a compile-time value: it is substituted at the point of use, performs no computation and has no side effects, and evaluates to the same bit pattern on every reference. Because `Fixed` is Q32.32 integer arithmetic, that bit pattern is identical on every target by construction, which makes `math::piFixed` the right choice when a result must be reproducible across platforms. [[src/docs/spec/architecture/18_math-kernels.md]]

The same constant is also available as a `Float` under the name
`math::pi`. There is no automatic conversion between the two forms, so pick
the one whose type matches the expression you are writing.
[[src/builtins/math.rs:constant_type_name]]

## Parameters

`math::piFixed` is a constant and takes no parameters.
[[src/builtins/math.rs:is_math_constant]]

## Return value

| Type | Description |
| --- | --- |
| `Fixed` | The `Fixed` nearest to `pi`, approximately `3.141592653589793`. The same value on every reference. [[src/builtins/math.rs:constant_value]] |

## Errors

No errors. Referencing a constant performs no computation, so there is no failure path. [[src/builtins/math.rs:constant_value]]

## Examples

Read the constant into a `Fixed` binding:

```
IMPORT math
IMPORT io

SUB main()
  LET value AS Fixed = math::piFixed
  io::print(toString(value))
END SUB
```

Compute a circumference with deterministic `Fixed` arithmetic:

```
IMPORT math
IMPORT io

SUB main()
  LET radius AS Fixed = 2.5F
  LET circumference AS Fixed = 2.0F * math::piFixed * radius
  io::print(toString(circumference))
END SUB
```

## See also

- `mfb man math pi`
- `mfb man math pi2Fixed`
- `mfb man math pi4Fixed`
- `mfb man math twoOverPiFixed`
- `mfb man math eFixed`
- `mfb man math`
