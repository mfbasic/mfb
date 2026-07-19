# math

Numeric constants and deterministic math helper functions

## Synopsis

```
IMPORT math
math::pi
math::abs(value)
math::clamp(value, low, high)
math::sqrt(value)
math::atan2(y, x)
math::rand(min, max)
```

## Description

The `math` package provides numeric constants and helper functions over the
`Integer`, `Float`, and `Fixed` numeric types: magnitude and rounding (`abs`,
`floor`, `ceil`, `round`), bounds (`min`, `max`, `clamp`), powers and roots
(`sqrt`, `pow`, `exp`, `log`, `log10`), trigonometry (`sin`, `cos`, `tan`,
`asin`, `acos`, `atan`, `atan2`), and pseudo-random integers (`rand`, `seed`).
`math` is a built-in package: `IMPORT math` needs no manifest dependency.
[[src/builtins/math.rs:is_math_call]]

Constants are `LET` values, each provided in a `Float` form and a `Fixed` form —
for example `math::pi` as `Float` and `math::piFixed` as `Fixed`.
[[src/builtins/math.rs:constant_type_name]] The functions are overloaded by the
exact numeric type of their arguments, and the return type matches that type:
`abs`, `min`, `max`, and `clamp` accept `Integer`, `Float`, `Fixed`, or `Money`
and return that same type; the rounding functions (`floor`, `ceil`, `round`)
accept `Float`, `Fixed`, or `Money`, returning `Integer` — for `Money` that is a
deliberate exit from the dimension, the count of whole units; the transcendental
functions accept `Float` or `Fixed` only; `seed` works on `Integer`, and `rand`
takes either two `Integer` bounds (returning `Integer`) or two `Money` bounds
(returning `Money`).
[[src/builtins/math.rs:expected_arguments]] There is no mixed-type or
automatic-promotion overload, so the arguments to a call must already share one
numeric type; convert explicitly before calling when they differ. Each scalar
function also has an array (SIMD) overload that maps element-wise over a
homogeneous `List OF` numeric list and returns a list of the matching type.
[[src/builtins/math.rs:resolve_call]]

Integer and Fixed computation is deterministic and identical across targets: the
Fixed transcendental and root functions use raw Q32.32 fixed-point arithmetic
rather than host floating point. [[src/target/shared/code/builder_math.rs:lower_fixed_external_math]]
The Float overloads return only finite values. Where an Integer or Fixed result
cannot be represented (such as negating the minimum value, or a rounding result
that exceeds the Integer range) the call raises `ErrOverflow` rather than
wrapping. [[src/target/shared/code/builder_math.rs:emit_float_rounding_integer_range_check]]

Domain failures are split by overload. A Float overload that has no real result
for its argument (square root or logarithm of a negative value, an inverse-trig
argument outside `[-1, 1]`) raises `ErrFloatDomain`, and a Float result that
would be NaN or infinite raises `ErrFloatNaN` or `ErrFloatInf`. The corresponding
Fixed overload reports the same out-of-domain argument as `ErrInvalidArgument`.
The range-checking helpers `clamp` and `rand` likewise raise `ErrInvalidArgument`
when their bounds are inverted (low greater than high, or min greater than max).
[[src/target/shared/code/builder_math.rs:lower_math_scalar_transcendental]]

`math::rand` and `math::seed` operate on the calling thread's PCG64 generator.
Each thread owns an independent generator and stream; the main thread is seeded
from the operating system at startup, and a thread started with `thread::start`
receives its own stream seeded from its parent. `math::rand` advances that state
as a side effect, and `math::seed` reseeds it so a subsequent sequence becomes
reproducible. [[src/target/shared/code/builder_math.rs:lower_math_rand]]

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | raised by the Fixed overloads of `sqrt`, `log`, `log10`, `asin`, `acos`, `pow`, and `tan` when the argument is outside the function's real domain, by `clamp` when `low` is greater than `high`, and by `rand` when `min` is greater than `max` [[src/target/shared/code/builder_math.rs:lower_math_clamp]] |
| `77050010` | `ErrOverflow` | raised by the Integer and Fixed overloads of `abs`, `floor`, `ceil`, `round`, `exp`, and `pow` when the result cannot be represented in that numeric type, such as negating the minimum value or a rounding result beyond the Integer range [[src/target/shared/code/builder_math.rs:emit_float_rounding_integer_range_check]] |
| `77050012` | `ErrFloatDomain` | raised by the Float overloads of `sqrt`, `log`, `log10`, `asin`, and `acos` when the argument has no real result, such as a negative square-root or logarithm argument or an inverse-trig argument outside the closed interval `[-1, 1]` [[src/target/shared/code/builder_math.rs:lower_math_scalar_transcendental]] |
| `77050013` | `ErrFloatNaN` | raised by the Float overloads of `exp` and `pow` when the result would be NaN [[src/target/shared/code/builder_math.rs:emit_float_result_check]] |
| `77050014` | `ErrFloatInf` | raised by the Float overloads of `exp`, `log10`, `pow`, and `tan` when the result would be positive or negative infinity [[src/target/shared/code/builder_math.rs:emit_float_result_check]] |
