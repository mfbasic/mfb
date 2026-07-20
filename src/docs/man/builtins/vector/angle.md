# angle

Unsigned angle in radians between two vectors

## Synopsis

```
vector::angle(a AS Float2, b AS Float2) AS Float
vector::angle(a AS Float3, b AS Float3) AS Float
vector::angle(a AS Float4, b AS Float4) AS Float
vector::angle(a AS Fixed2, b AS Fixed2) AS Fixed
vector::angle(a AS Fixed3, b AS Fixed3) AS Fixed
vector::angle(a AS Fixed4, b AS Fixed4) AS Fixed
vector::angle(a AS Integer2, b AS Integer2) AS Integer
vector::angle(a AS Integer3, b AS Integer3) AS Integer
vector::angle(a AS Integer4, b AS Integer4) AS Integer
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

`vector::angle` returns the unsigned angle between the directions of `a` and `b`,
**in radians**, computed as `acos(clamp(dot(a, b) / (length(a) * length(b)), -1, 1))`.
The result lies in `0` through `pi`: it is `0` for vectors pointing the same way
and `pi` for vectors pointing opposite ways. The angle is unsigned and symmetric —
`angle(a, b)` equals `angle(b, a)` — so it carries no orientation or handedness
information and cannot distinguish a clockwise from a counterclockwise
separation. Magnitude is irrelevant: scaling either input by a positive factor
leaves the result unchanged. [[src/builtins/vector_package.mfb:__vector_angle_float3]]

The cosine is clamped to the closed interval `-1` through `1` with `math::clamp`
before `acos` is applied. This matters because the division can produce a value a
fraction of an ulp outside that interval for nearly parallel or nearly
antiparallel inputs; without the clamp `acos` would fail with a domain error. With
it, the function is total for every pair of non-zero inputs and never raises a
floating-point domain error. [[src/builtins/vector_package.mfb:__vector_angle_float2]]

Both inputs must be non-zero. A zero-length vector has no direction, so the
implementation checks each length before dividing and fails with
`ErrInvalidArgument` and the message `vector::angle with a zero-length vector` if
either is zero. The check is on the actual computed length, so the failure
happens before any division by zero can occur.
[[src/builtins/vector_package.mfb:__vector_angle_float4]]

The `Integer` overloads are the coarsest. They compute the angle internally in
`Fixed` (Q32.32) radians through a dedicated helper, then round that radian value
to an `Integer` with `math::round`, half away from zero. Because the full range of
the function is `0` through `pi`, the only possible `Integer` results are `0`,
`1`, `2`, and `3`. The `Integer` overload is therefore a very lossy quantization
of the angle and is rarely the right tool; prefer the `Float` or `Fixed`
overloads when the angle itself matters.
[[src/builtins/vector_package.mfb:__vector_angleFixed_integer3]]

## Overloads

**`vector::angle(a AS Float2/Float3/Float4, b AS ...) AS Float`**

Computes the lengths with `math::sqrt` over IEEE doubles and applies the in-tree
`Float` `acos`. Returns radians as a `Float`.

**`vector::angle(a AS Fixed2/Fixed3/Fixed4, b AS ...) AS Fixed`**

Computes the lengths and `acos` entirely in deterministic Q32.32 arithmetic, so
the result is bit-identical on every target. Returns radians as a `Fixed`.

**`vector::angle(a AS Integer2/Integer3/Integer4, b AS ...) AS Integer`**

Computes the dot products exactly in `Integer`, converts to `Fixed` for the
square roots and `acos`, then rounds the radian result to an `Integer` half away
from zero. The result is one of `0`, `1`, `2`, `3`.
[[src/builtins/vector_package.mfb:__vector_angle_integer2]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | one of the nine vector types | The first vector. Must have non-zero length. Also spelled `v` as a named argument. [[src/builtins/vector.rs:call_param_names]] |
| `b` | the same type as `a` | The second vector, which must be the same vector type as `a`. Must have non-zero length. Also spelled `n` as a named argument. [[src/builtins/vector.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| the element type of `a` (`Float`, `Fixed`, or `Integer`) | The unsigned angle in radians, in `0` through `pi`. `0` for parallel inputs, `pi` for antiparallel inputs, `pi / 2` for orthogonal inputs. The `Integer` overloads return the rounded radian value, so only `0`, `1`, `2`, `3` occur. [[src/builtins/vector.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | Either `a` or `b` has zero length, so the angle between them is undefined. [[src/builtins/vector_package.mfb:__vector_angle_float2]] |
| `77050010` | `ErrOverflow` | On the `Fixed` and `Integer` overloads, a squared component or a dot-product sum exceeds the checked range of the element type, or the `Integer` overload's conversion of a large squared sum into `Fixed` is out of range. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |
| `77050015` | `ErrFloatOverflow` | On the `Float` overloads, a squared component or dot-product sum reaches infinity and is caught where the length is bound. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_OVERFLOW_CODE]] |

## Type checking

`vector::angle` is generic over the nine built-in vector record types. Both
arguments must be the *same* one of the nine types: there is no mixed-element-type
and no cross-dimension overload, and no implicit conversion is applied to a vector
argument. The return type is the element type of that vector type, not the vector
type itself. [[src/builtins/vector.rs:resolve_call]] [[src/builtins/vector.rs:same_vector]]

## Examples

The right angle between the two 2D axes, in radians:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::angle(vector::Float2[1.0, 0.0], vector::Float2[0.0, 1.0])))
END SUB
```

The angle is unaffected by magnitude:

```
IMPORT vector
IMPORT io

SUB main()
  LET wide AS Float = vector::angle(vector::Float3[10.0, 0.0, 0.0], vector::Float3[0.0, 7.0, 0.0])
  io::print(toString(wide))
END SUB
```

## See also

- `mfb man vector dot`
- `mfb man vector slerp`
- `mfb man vector normalize`
- `mfb man vector rotate_2d`
