# slerp

Spherical linear interpolation along the arc between two vectors

## Synopsis

```
vector::slerp(a AS Float2, b AS Float2, t AS Float) AS Float2
vector::slerp(a AS Float3, b AS Float3, t AS Float) AS Float3
vector::slerp(a AS Float4, b AS Float4, t AS Float) AS Float4
vector::slerp(a AS Fixed2, b AS Fixed2, t AS Float) AS Fixed2
vector::slerp(a AS Fixed3, b AS Fixed3, t AS Float) AS Fixed3
vector::slerp(a AS Fixed4, b AS Fixed4, t AS Float) AS Fixed4
vector::slerp(a AS Integer2, b AS Integer2, t AS Float) AS Integer2
vector::slerp(a AS Integer3, b AS Integer3, t AS Float) AS Integer3
vector::slerp(a AS Integer4, b AS Integer4, t AS Float) AS Integer4
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

`vector::slerp` interpolates along the great-circle arc between the directions of
`a` and `b` rather than along the straight chord between their tips. It first
computes `omega = vector::angle(a, b)` and `s = sin(omega)`, then returns
`(sin((1-t)*omega)/s) * a + (sin(t*omega)/s) * b`. The result sweeps the angle
between the two inputs at a constant angular rate, which is what makes `slerp` the
right choice for interpolating orientations and directions where
`vector::lerp` would slow down in the middle of the turn.
[[src/builtins/vector_package.mfb:__vector_slerp_float3]]

**`t` is not clamped.** Values below `0` or above `1` extrapolate along the same
great circle, past either endpoint, exactly as `vector::lerp_unclamped` does along
its line. Clamp `t` yourself if that is not wanted.
[[src/builtins/vector_package.mfb:__vector_slerp_float2]]

`slerp` interpolates *direction*, and it preserves magnitude only when
`vector::length(a)` equals `vector::length(b)`. The two weights are derived purely
from the angle, so for inputs of different lengths the intermediate magnitudes
follow the weighted blend rather than tracking a sphere. For a clean directional
interpolation, normalize both inputs first.

The formula divides by `sin(omega)`, which approaches zero as the inputs become
parallel or antiparallel. To stay stable there, the implementation tests
`abs(s) < 0.000001` — the literal threshold, in the `Float` overloads, and its
`toFixed` equivalent in the others — and when it is met **returns
`vector::lerp_unclamped(a, b, t)` instead**, taking the straight-line result. This
fallback is silent: nothing in the return value distinguishes the spherical path
from the linear one, and for nearly parallel inputs the two are in any case
indistinguishable. Note that the fallback is chosen for the *antiparallel* case as
well, where `sin(pi)` is also near zero; there is no unique great circle between
opposite directions, and `slerp` does not attempt to pick one — it interpolates
straight through the origin. [[src/builtins/vector_package.mfb:__vector_slerp_fixed3]]

Both inputs must be non-zero. The requirement is inherited from
`vector::angle`, which is called first and fails with `ErrInvalidArgument` when
either input has zero length; the message therefore names `vector::angle` rather
than `vector::slerp`. [[src/builtins/vector_package.mfb:__vector_angle_float2]]

As with `vector::lerp`, `t` is a `Float` for **every** overload. The `Float`
overloads use the in-tree `Float` `sin`; the `Fixed` overloads work in
deterministic Q32.32 throughout. The `Integer` overloads compute the angle and the
weights in `Fixed`, blend the components there, and round each result back with
`math::round`, half away from zero, so an `Integer` `slerp` is heavily quantized —
its degenerate-case fallback goes to the `Integer` `lerp_unclamped`, which rounds
in the same way. [[src/builtins/vector_package.mfb:__vector_slerp_integer3]]

## Overloads

**`vector::slerp(a AS Float2/Float3/Float4, b AS ..., t AS Float) AS ...`**

Angle and weights via the in-tree `Float` trigonometry; blend in IEEE double
arithmetic.

**`vector::slerp(a AS Fixed2/Fixed3/Fixed4, b AS ..., t AS Float) AS ...`**

Converts `t` with `toFixed`; angle, weights, and blend entirely in deterministic
Q32.32 arithmetic.

**`vector::slerp(a AS Integer2/Integer3/Integer4, b AS ..., t AS Float) AS ...`**

Angle and weights in `Fixed` through a dedicated helper, components widened to
`Fixed` for the blend, then each rounded back to `Integer` half away from zero.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | one of the nine vector types | The start vector, returned when `t` is `0`. Must have a non-zero length. [[src/builtins/vector.rs:call_param_names]] |
| `b` | the same type as `a` | The end vector, returned when `t` is `1`. Must be the same vector type as `a` and have a non-zero length. [[src/builtins/vector.rs:call_param_names]] |
| `t` | `Float` | The interpolation parameter, used verbatim with no clamping. Values outside `0` through `1` extrapolate along the arc. Always a `Float`, whatever the vector's element type. [[src/builtins/vector.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| the same type as `a` | A vector at the fraction `t` of the angular sweep from `a` to `b`. Magnitude is preserved only when `a` and `b` have equal lengths. When the inputs are nearly parallel or antiparallel, the linear `vector::lerp_unclamped` result is returned instead. [[src/builtins/vector.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | Either `a` or `b` has zero length. Raised by the delegated angle computation, so the message names `vector::angle`. [[src/builtins/vector_package.mfb:__vector_angle_float2]] |
| `77050010` | `ErrOverflow` | On the `Fixed` and `Integer` overloads, converting `t`, a squared component, a weight, or a blended component exceeds the checked range of the element type, or an `Integer` component rounds outside the `Integer` range. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |
| `77050015` | `ErrFloatOverflow` | On the `Float` overloads, a squared component, a weight, or a blended component reaches infinity and is caught where it is bound. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_OVERFLOW_CODE]] |

## Type checking

`vector::slerp` is generic over the nine built-in vector record types. The first
two arguments must be the *same* one of the nine types, and the third must be a
`Float` for every overload — an `Integer` `t` is a compile-time error with no
implicit numeric promotion. The return type is always the first argument's own
type. [[src/builtins/vector.rs:resolve_call]] [[src/builtins/vector.rs:expected_arguments]]

## Examples

Halfway along the arc between the two 2D axes — note that both components come
out equal, unlike the straight-line midpoint:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::slerp(vector::Float2[1.0, 0.0], vector::Float2[0.0, 1.0], 0.5)))
END SUB
```

Interpolating between two normalized directions keeps the result on the unit
circle:

```
IMPORT vector
IMPORT io

SUB main()
  LET start AS vector::Float3 = vector::normalize(vector::Float3[1.0, 0.0, 0.0])
  LET finish AS vector::Float3 = vector::normalize(vector::Float3[0.0, 0.0, 1.0])
  io::print(toString(vector::slerp(start, finish, 0.25)))
END SUB
```

## See also

- `mfb man vector lerp`
- `mfb man vector lerp_unclamped`
- `mfb man vector angle`
- `mfb man vector normalize`
