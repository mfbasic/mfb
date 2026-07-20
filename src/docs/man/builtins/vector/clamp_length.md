# clamp_length

Cap a vector's magnitude while preserving its direction

## Synopsis

```
vector::clamp_length(v AS Float2, max AS Float) AS Float2
vector::clamp_length(v AS Float3, max AS Float) AS Float3
vector::clamp_length(v AS Float4, max AS Float) AS Float4
vector::clamp_length(v AS Fixed2, max AS Fixed) AS Fixed2
vector::clamp_length(v AS Fixed3, max AS Fixed) AS Fixed3
vector::clamp_length(v AS Fixed4, max AS Fixed) AS Fixed4
vector::clamp_length(v AS Integer2, max AS Integer) AS Integer2
vector::clamp_length(v AS Integer3, max AS Integer) AS Integer3
vector::clamp_length(v AS Integer4, max AS Integer) AS Integer4
```

## Package

vector

## Imports

```
IMPORT vector
```

`vector` is a built-in package, so `IMPORT vector` needs no manifest dependency.
[[src/builtins/vector.rs:uses_package]]

## Description

`vector::clamp_length` limits the magnitude of `v` to at most `max` without
changing the direction it points. If `vector::length(v)` is already less than or
equal to `max`, `v` is returned unchanged, component for component. Otherwise
every component is multiplied by the ratio `max / length(v)`, producing a vector
that points the same way as `v` with a length of approximately `max`. This is the
standard "speed limit" operation: it is a no-op inside the ball of radius `max`
and a projection onto its surface outside it.
[[src/builtins/vector_package.mfb:__vector_clamp_length_float3]]

The zero vector is a special case. It has no direction, so it cannot be rescaled,
but it also never exceeds a non-negative `max`. The implementation checks
`len <= max OR len = 0` and returns `v` untouched in either case, so a zero
vector is passed through rather than raising an error — unlike
`vector::normalize`, which rejects it. Note that the length test is inclusive:
a vector already exactly at length `max` is returned unchanged with no division
performed. [[src/builtins/vector_package.mfb:__vector_clamp_length_float2]]

`max` must not be negative. A negative cap is meaningless, since no magnitude can
be below zero, and the implementation rejects it up front — before computing any
length — with `ErrInvalidArgument` and the message
`vector::clamp_length with negative max`. A `max` of exactly zero is accepted and
is not an error: the length test `len <= 0` matches only the zero vector, and any
non-zero vector is scaled by the ratio `0 / len`, collapsing it to the zero
vector. [[src/builtins/vector_package.mfb:__vector_clamp_length_fixed2]]

`max` is a scalar of the vector's own **element** type, not a `Float` for all
overloads: a `Fixed3` is capped by a `Fixed`, and an `Integer4` by an `Integer`.
This differs from `vector::lerp` and `vector::rotate_2d`, whose scalar parameter
is a `Float` for every element type. The compile-time check requires `max` to
match the element type exactly. [[src/builtins/vector.rs:resolve_call]]

The `Integer` overloads compute the length with the package's rounding integer
square root, then form the ratio and rescale each component in `Float` before
rounding back with `math::round`, half away from zero. Because the length itself
was already rounded, and each component is then rounded again, the resulting
vector's length is only approximately `max` — for small integer vectors it can
differ from `max` by a whole unit. [[src/builtins/vector_package.mfb:__vector_clamp_length_integer2]]

## Overloads

**`vector::clamp_length(v AS Float2/Float3/Float4, max AS Float) AS ...`**

Length via `math::sqrt` over IEEE doubles; components rescaled with
correctly-rounded floating-point multiplication.

**`vector::clamp_length(v AS Fixed2/Fixed3/Fixed4, max AS Fixed) AS ...`**

Length and rescaling entirely in deterministic Q32.32 arithmetic; identical on
every target.

**`vector::clamp_length(v AS Integer2/Integer3/Integer4, max AS Integer) AS ...`**

Length via the rounding integer square root, ratio and rescale in `Float`, each
component rounded back to `Integer` half away from zero.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `v` | one of the nine vector types | The vector whose magnitude is capped. The zero vector is accepted and returned unchanged. Also spelled `v` as a named argument. [[src/builtins/vector.rs:call_param_names]] |
| `max` | the element type of `v` (`Float`, `Fixed`, or `Integer`) | The maximum permitted magnitude. Must be greater than or equal to zero; `0` is valid and collapses any non-zero `v` to the zero vector. Also spelled `max` as a named argument. [[src/builtins/vector.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| the same type as `v` | `v` itself when `vector::length(v)` is at most `max` or when `v` is the zero vector; otherwise a vector in the same direction as `v` with magnitude approximately `max`. [[src/builtins/vector.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `max` is negative. Checked before any other work, so this is reported even for a zero-length `v`. [[src/builtins/vector_package.mfb:__vector_clamp_length_float2]] |
| `77050010` | `ErrOverflow` | On the `Fixed` and `Integer` overloads, a squared component or the sum of squares exceeds the checked range of the element type, or a rescaled `Integer` component rounds outside the `Integer` range. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |
| `77050015` | `ErrFloatOverflow` | On the `Float` overloads, a squared component or the sum of squares reaches infinity and is caught where the length is bound. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_OVERFLOW_CODE]] |

## Type checking

`vector::clamp_length` is generic over the nine built-in vector record types. The
first argument selects the overload by its exact record type, and the second must
be a scalar of exactly that vector type's element type — an `Integer` `max` for a
`Float3` is a compile-time error, with no implicit numeric promotion. The return
type is always the first argument's own type.
[[src/builtins/vector.rs:resolve_call]] [[src/builtins/vector.rs:expected_arguments]]

## Examples

Cap a length-5 vector at 2.5, halving it:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::clamp_length(vector::Float2[3.0, 4.0], 2.5)))
END SUB
```

A vector already within the cap passes through untouched:

```
IMPORT vector
IMPORT io

SUB main()
  LET small AS vector::Float2 = vector::clamp_length(vector::Float2[0.0, 1.0], 10.0)
  io::print(toString(small))
END SUB
```

## See also

- `mfb man vector length`
- `mfb man vector normalize`
- `mfb man vector scale`
- `mfb man vector types`
