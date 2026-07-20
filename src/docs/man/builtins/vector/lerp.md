# lerp

Linear interpolation between two vectors, clamped to the segment

## Synopsis

```
vector::lerp(a AS Float2, b AS Float2, t AS Float) AS Float2
vector::lerp(a AS Float3, b AS Float3, t AS Float) AS Float3
vector::lerp(a AS Float4, b AS Float4, t AS Float) AS Float4
vector::lerp(a AS Fixed2, b AS Fixed2, t AS Float) AS Fixed2
vector::lerp(a AS Fixed3, b AS Fixed3, t AS Float) AS Fixed3
vector::lerp(a AS Fixed4, b AS Fixed4, t AS Float) AS Fixed4
vector::lerp(a AS Integer2, b AS Integer2, t AS Float) AS Integer2
vector::lerp(a AS Integer3, b AS Integer3, t AS Float) AS Integer3
vector::lerp(a AS Integer4, b AS Integer4, t AS Float) AS Integer4
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

`vector::lerp` interpolates component-wise along the straight segment from `a` to
`b`, computing `a + (b - a) * t` for each component in declared field order. At
`t = 0` the result is `a`, at `t = 1` it is `b`, and at `t = 0.5` it is the
midpoint. The path traced as `t` sweeps is a straight line, and the speed along
it is constant — for interpolation that follows the arc between two directions
instead, use `vector::slerp`.
[[src/builtins/vector_package.mfb:__vector_lerp_float3]]

The defining difference from `vector::lerp_unclamped` is that `t` is **clamped to
the closed interval 0 through 1** with `math::clamp` before it is used. A `t` of
`2.0` therefore behaves exactly like `1.0` and returns `b`; a `t` of `-1.0`
behaves like `0.0` and returns `a`. The result is guaranteed to lie on the segment
between the two endpoints and can never overshoot them, which makes `lerp` the
safe choice when `t` comes from a source that may run past its expected range,
such as an elapsed-time ratio. [[src/builtins/vector_package.mfb:__vector_lerp_float2]]

`t` is a `Float` for **every** overload, including the `Fixed` and `Integer`
ones — it is not the vector's element type. This differs from
`vector::clamp_length`, whose scalar argument does follow the element type. On
the `Fixed` overloads the clamped `t` is converted with `toFixed` after the clamp,
and the interpolation then runs entirely in Q32.32. On the `Integer` overloads
each component is widened to `Float`, interpolated there, and rounded back with
`math::round`, half away from zero — so `lerp` on `Integer` vectors quantizes the
result to the integer lattice, and successive small steps of `t` can produce the
same output repeatedly. [[src/builtins/vector_package.mfb:__vector_lerp_integer2]]

Interpolation is strictly component-wise, so `lerp` preserves neither length nor
direction in general: the midpoint of two unit vectors pointing different ways is
shorter than either, because it cuts across the chord rather than following the
arc. [[src/builtins/vector_package.mfb:__vector_lerp_float4]]

## Overloads

**`vector::lerp(a AS Float2/Float3/Float4, b AS ..., t AS Float) AS ...`**

Clamps `t`, then interpolates each component in IEEE double arithmetic.

**`vector::lerp(a AS Fixed2/Fixed3/Fixed4, b AS ..., t AS Float) AS ...`**

Clamps `t` as a `Float`, converts it with `toFixed`, then interpolates each
component in deterministic Q32.32 arithmetic.

**`vector::lerp(a AS Integer2/Integer3/Integer4, b AS ..., t AS Float) AS ...`**

Clamps `t`, widens each component to `Float` for the interpolation, then rounds
each result back to `Integer` half away from zero.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | one of the nine vector types | The start vector, returned when `t` is `0` or less. [[src/builtins/vector.rs:call_param_names]] |
| `b` | the same type as `a` | The end vector, returned when `t` is `1` or more. Must be the same vector type as `a`. [[src/builtins/vector.rs:call_param_names]] |
| `t` | `Float` | The interpolation parameter, clamped to `0` through `1` before use. Always a `Float`, whatever the vector's element type. [[src/builtins/vector.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| the same type as `a` | A vector on the segment from `a` to `b`, at the clamped fraction `t` of the way from `a` to `b`. Exactly `a` for any `t` at or below `0` and exactly `b` for any `t` at or above `1`, up to the rounding of the element type. [[src/builtins/vector.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | On the `Fixed` overloads, a difference or product exceeds the checked Q32.32 range. On the `Integer` overloads, an interpolated component rounds outside the `Integer` range. [[src/target/shared/code/builder_math.rs:emit_float_rounding_integer_range_check]] |
| `77050015` | `ErrFloatOverflow` | On the `Float` overloads, a difference or interpolated component reaches infinity and is caught where it is bound. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_OVERFLOW_CODE]] |

## Type checking

`vector::lerp` is generic over the nine built-in vector record types. The first
two arguments must be the *same* one of the nine types, and the third must be a
`Float` for every overload — an `Integer` `t` is a compile-time error with no
implicit numeric promotion. The return type is always the first argument's own
type. [[src/builtins/vector.rs:resolve_call]] [[src/builtins/vector.rs:expected_arguments]]

## Examples

The midpoint of a segment:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::lerp(vector::Float2[0.0, 0.0], vector::Float2[10.0, 0.0], 0.5)))
END SUB
```

An out-of-range `t` is clamped, so this returns the endpoint rather than
overshooting it:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::lerp(vector::Float2[0.0, 0.0], vector::Float2[10.0, 0.0], 2.0)))
END SUB
```

## See also

- `mfb man vector lerp_unclamped`
- `mfb man vector slerp`
- `mfb man vector distance`
- `mfb man vector types`
