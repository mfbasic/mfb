# seed

Reseed the calling thread's pseudo-random generator.

## Synopsis

```
math::seed(value AS Integer) AS Nothing
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

`math::seed` reseeds the calling thread's PCG64 generator from `value`, so that
the thread's subsequent `math::rand` draws follow a sequence determined solely by
that seed. Seeding with a fixed value makes the sequence deterministic and
reproducible, which is what tests and repeatable simulations want.
[[src/target/shared/code/builder_math.rs:lower_math_seed]]

Any `Integer` is a valid seed, including zero and negative values; there is no
rejected value and the call cannot fail.
[[src/target/shared/code/builder_math.rs:lower_math_seed]]

Seeding is **not required**. Every thread is seeded automatically before it runs:
the main thread from OS entropy at program start, before any user code including
global initializers, and a thread started with `thread::start` from a draw taken
on the spawning thread. Call `math::seed` only when reproducible output is
wanted. [[src/target/shared/code/entry_and_arena.rs:lower_program_entry]]

`math::seed` affects **only the calling thread**. The generator state lives in the
calling thread's own arena, so reseeding one thread does not disturb any other.
Note the consequence for reproducibility: because a child thread's seed is itself
a draw from the parent's generator, seeding the parent before spawning also makes
each child's stream reproducible.
[[src/target/shared/code/runtime_helpers.rs:lower_thread_helper]]

The call replaces the generator state and returns `Nothing` — it produces no draw
of its own, and it is a statement rather than an expression you bind.
[[src/builtins/math.rs:call_return_type_name]]

The argument may be given by name as well as positionally, under either the name
`value` or the name `seed`. [[src/builtins/math.rs:call_param_names]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer` | The seed for the calling thread's generator. Also accepted under the name `seed`. Every `Integer` across the full signed 64-bit range is valid, including zero and negative values; the same seed always yields the same subsequent `math::rand` sequence for a given build. [[src/builtins/math.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | No value is returned. The effect is the reseeding of the calling thread's generator. [[src/builtins/math.rs:call_return_type_name]] |

## Errors

No errors. Every `Integer` is a valid seed and the reseed writes two words of the calling thread's own arena state, so no failure path exists. [[src/target/shared/code/builder_math.rs:lower_math_seed]]

## Examples

Reseed for a reproducible sequence:

```
IMPORT math
IMPORT io

SUB main()
  math::seed(12345)
  LET first AS Integer = math::rand(1, 100)
  io::print(toString(first))
END SUB
```

Reseeding with the same value reproduces the same draws, and the argument may be
named:

```
IMPORT math
IMPORT io

SUB main()
  math::seed(seed := 42)
  LET a AS Integer = math::rand(1, 6)
  math::seed(seed := 42)
  LET b AS Integer = math::rand(1, 6)
  io::print(toString(a))
  io::print(toString(b))
END SUB
```

## See also

- `mfb man math rand`
- `mfb man math min`
- `mfb man math max`
- `mfb man math`
