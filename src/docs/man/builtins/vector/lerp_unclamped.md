# lerp_unclamped

Linear interpolation between two vectors, extrapolating outside 0 through 1

## Synopsis

```
vector::lerp_unclamped(a AS Float2, b AS Float2, t AS Float) AS Float2
vector::lerp_unclamped(a AS Float3, b AS Float3, t AS Float) AS Float3
vector::lerp_unclamped(a AS Float4, b AS Float4, t AS Float) AS Float4
vector::lerp_unclamped(a AS Fixed2, b AS Fixed2, t AS Float) AS Fixed2
vector::lerp_unclamped(a AS Fixed3, b AS Fixed3, t AS Float) AS Fixed3
vector::lerp_unclamped(a AS Fixed4, b AS Fixed4, t AS Float) AS Fixed4
vector::lerp_unclamped(a AS Integer2, b AS Integer2, t AS Float) AS Integer2
vector::lerp_unclamped(a AS Integer3, b AS Integer3, t AS Float) AS Integer3
vector::lerp_unclamped(a AS Integer4, b AS Integer4, t AS Float) AS Integer4
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

`vector::lerp_unclamped` computes `a + (b - a) * t` component-wise in declared
field order, using `t` **verbatim** with no clamping. It is otherwise identical to
`vector::lerp` and shares its per-element-type behavior; the only difference
between the two implementations is the missing `math::clamp` call on `t`.
[[src/builtins/vector_package.mfb:__vector_lerp_unclamped_float3]]

That single difference changes what the function is for. Because `t` is not
restricted to `0` through `1`, values outside that range **extrapolate** along the
infinite line through `a` and `b` rather than saturating at an endpoint: `t = 2.0`
lands as far beyond `b` as `b` is beyond `a`, and `t = -1.0` lands the same
distance before `a`. Use this when the parameter legitimately runs past the
endpoints — projecting a trajectory forward, or overshooting deliberately for an
easing effect — and use `vector::lerp` when an out-of-range `t` should be treated
as a mistake and pinned to the segment.
[[src/builtins/vector_package.mfb:__vector_lerp_unclamped_float2]]

Extrapolation is also where this function's failure modes come from. Since `t` is
unbounded, so is the result: a large `t` scales the difference `b - a` without
limit and can drive a component past the range of the element type, which the
clamped `vector::lerp` cannot do for finite endpoints. On the `Integer` overloads
this surfaces as `ErrOverflow` from the final rounding back to `Integer`.
[[src/target/shared/code/builder_math.rs:emit_float_rounding_integer_range_check]]

As with `vector::lerp`, `t` is a `Float` for **every** overload, including the
`Fixed` and `Integer` ones. The `Fixed` overloads convert `t` with `toFixed` and
interpolate in Q32.32; the `Integer` overloads widen each component to `Float`,
interpolate there, and round back with `math::round`, half away from zero.
`vector::slerp` falls back to this function, not to `vector::lerp`, when its two
inputs are too nearly parallel for the spherical formula to be stable — which is
why an out-of-range `t` passed to `slerp` still extrapolates in that degenerate
case. [[src/builtins/vector_package.mfb:__vector_slerp_float3]]

## Overloads

**`vector::lerp_unclamped(a AS Float2/Float3/Float4, b AS ..., t AS Float) AS ...`**

Interpolates each component in IEEE double arithmetic, with `t` used as given.

**`vector::lerp_unclamped(a AS Fixed2/Fixed3/Fixed4, b AS ..., t AS Float) AS ...`**

Converts `t` with `toFixed`, then interpolates each component in deterministic
Q32.32 arithmetic.

**`vector::lerp_unclamped(a AS Integer2/Integer3/Integer4, b AS ..., t AS Float) AS ...`**

Widens each component to `Float` for the interpolation, then rounds each result
back to `Integer` half away from zero.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | one of the nine vector types | The start vector, returned when `t` is exactly `0`. [[src/builtins/vector.rs:call_param_names]] |
| `b` | the same type as `a` | The end vector, returned when `t` is exactly `1`. Must be the same vector type as `a`. [[src/builtins/vector.rs:call_param_names]] |
| `t` | `Float` | The interpolation parameter, used verbatim with no clamping. Values below `0` or above `1` extrapolate beyond the endpoints. Always a `Float`, whatever the vector's element type. [[src/builtins/vector.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| the same type as `a` | A vector on the infinite line through `a` and `b`, at parameter `t`. On the segment for `t` in `0` through `1`, beyond `b` for `t` above `1`, and before `a` for `t` below `0`. [[src/builtins/vector.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | On the `Fixed` overloads, converting `t`, or a difference or product, exceeds the checked Q32.32 range. On the `Integer` overloads, an extrapolated component rounds outside the `Integer` range. [[src/target/shared/code/builder_math.rs:emit_float_rounding_integer_range_check]] |
| `77050015` | `ErrFloatOverflow` | On the `Float` overloads, a difference or extrapolated component reaches infinity and is caught where it is bound. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_OVERFLOW_CODE]] |

## Type checking

`vector::lerp_unclamped` is generic over the nine built-in vector record types.
The first two arguments must be the *same* one of the nine types, and the third
must be a `Float` for every overload — an `Integer` `t` is a compile-time error
with no implicit numeric promotion. The return type is always the first argument's
own type. [[src/builtins/vector.rs:resolve_call]] [[src/builtins/vector.rs:expected_arguments]]

## Examples

Extrapolate twice as far as the endpoint:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::lerp_unclamped(vector::Float2[0.0, 0.0], vector::Float2[10.0, 0.0], 2.0)))
END SUB
```

Extrapolate backwards, before the start point:

```
IMPORT vector
IMPORT io

SUB main()
  LET back AS vector::Float2 = vector::lerp_unclamped(vector::Float2[0.0, 0.0], vector::Float2[10.0, 0.0], 0.0 - 0.5)
  io::print(toString(back))
END SUB
```

## See also

- `mfb man vector lerp`
- `mfb man vector slerp`
- `mfb man vector scale`
- `mfb man vector types`
