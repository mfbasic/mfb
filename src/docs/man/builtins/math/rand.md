# rand

Uniform pseudo-random value in an inclusive range.

## Synopsis

```
math::rand(min AS Integer, max AS Integer) AS Integer
math::rand(min AS Money, max AS Money) AS Money
```

## Package

math

## Imports

```
IMPORT math
```

`math` is a built-in package, so no manifest dependency is required.
[[src/builtins/math.rs:is_math_call]]

## Description

`math::rand` returns a uniformly distributed pseudo-random value in the
**inclusive** range `[min, max]`. Both endpoints are reachable: when `min`
equals `max` that single value is returned, and otherwise every value from `min`
to `max` inclusive is equally likely. `min` must not exceed `max`; an empty
range fails with `ErrInvalidArgument`.
[[src/target/shared/code/builder_math.rs:lower_math_rand]]

The draw is taken from the **calling thread's** PCG64 generator, and it advances
that generator as a side effect, so successive calls return successive draws
from that thread's stream. Each thread owns an independent generator: the main
thread is seeded from OS entropy at program start, before any user code runs
(including global initializers), and a thread started with `thread::start` is
seeded from a draw taken on the spawning thread.
[[src/target/shared/code/entry.rs:lower_program_entry]] Call
`math::seed` first to make a thread's subsequent sequence reproducible.

The bounds may be negative, and the range may span the whole signed 64-bit
domain. The reduction is Lemire rejection sampling, which is **unbiased** over
the inclusive span rather than the modulo-biased `raw % span` shortcut; when
`min` is the minimum `Integer` and `max` the maximum, a single raw 64-bit draw is
already uniform and is returned directly.
[[src/target/shared/code/builder_math.rs:lower_math_rand]]

A second overload takes two `Money` bounds and returns a `Money`, drawn the same
way over the inclusive span of the underlying scaled amounts: a uniform amount
between two amounts is itself an amount. Both arguments must be the same type;
there is no mixed `Integer`/`Money` form, and there is no `Float` or `Fixed`
form. [[src/builtins/math.rs:resolve_call]]

Either argument may be given by name as well as positionally: the first accepts
`min` or `minimum`, the second `max` or `maximum`.
[[src/builtins/math.rs:call_param_names]]

`math::rand` and `math::seed` are the only `math::` members that import anything
from the platform — `getentropy`, for the startup seed. That is the RNG, not the
math kernels. [[src/docs/spec/architecture/18_math-kernels.md]]

## Overloads

**`math::rand(min AS Integer, max AS Integer) AS Integer`**

Uniform `Integer` in `[min, max]`, unbiased by Lemire rejection sampling.

**`math::rand(min AS Money, max AS Money) AS Money`**

Uniform `Money` amount in `[min, max]`, drawn over the raw scaled amounts by the
same sampling. The result stays in the `Money` dimension.
[[src/target/shared/code/builder_math.rs:lower_math_rand]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `min` | `Integer` or `Money` | The inclusive lower bound. Also accepted under the name `minimum`. Must not be greater than `max`. [[src/builtins/math.rs:call_param_names]] |
| `max` | Same type as `min` | The inclusive upper bound. Also accepted under the name `maximum`. Must not be less than `min`. [[src/builtins/math.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` or `Money` | A pseudo-random value `x` with `min <= x <= max`, drawn uniformly, in the same type as the bounds. When `min` equals `max`, that single value. [[src/builtins/math.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `min` is greater than `max`, so the requested range is empty. Checked before any draw is taken. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] [[src/target/shared/code/builder_math.rs:lower_math_rand]] |

## Type checking

`math::rand` takes exactly two arguments. [[src/builtins/math.rs:arity]] They must
be two `Integer`s or two `Money` amounts. A `Float`, `Fixed`, or `Scalar`
argument, a mixed `Integer`/`Money` pair, a list, or any non-numeric value such
as a `String`, `Boolean`, `Byte`, record, union, resource, thread, or function
value is a compile-time type error. [[src/builtins/math.rs:expected_arguments]]

## Examples

Roll a six-sided die and flip a coin:

```
IMPORT math
IMPORT io

SUB main()
  LET roll AS Integer = math::rand(1, 6)
  LET coin AS Integer = math::rand(0, 1)
  io::print(toString(roll))
  io::print(toString(coin))
END SUB
```

A reproducible sequence, a named-argument call, and a `Money` draw:

```
IMPORT math
IMPORT io

SUB main()
  math::seed(12345)
  LET first AS Integer = math::rand(minimum := 1, maximum := 100)
  LET price AS Money = math::rand(1.00m, 10.00m)
  io::print(toString(first))
  io::print(toString(price))
END SUB
```

## See also

- `mfb man math seed`
- `mfb man math min`
- `mfb man math max`
- `mfb man math clamp`
- `mfb man math`
