# rotate_2d

Rotate a 2D vector counterclockwise by an angle in radians

## Synopsis

```
vector::rotate_2d(v AS Float2, angle AS Float) AS Float2
vector::rotate_2d(v AS Fixed2, angle AS Float) AS Fixed2
vector::rotate_2d(v AS Integer2, angle AS Float) AS Integer2
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

`vector::rotate_2d` rotates `v` about the origin by `angle` **radians**,
counterclockwise, applying the standard 2D rotation matrix:
`(v.x*cos - v.y*sin, v.x*sin + v.y*cos)`. The sine and cosine are each computed
once and reused for both output components. A positive `angle` turns from the `+x`
axis toward the `+y` axis; a negative `angle` turns the other way. `angle` is
unbounded — it is passed straight to the trigonometric kernels with no range
reduction of its own — so multiple full turns are accepted and behave as the
equivalent angle. [[src/builtins/vector_package.mfb:__vector_rotate_2d_float2]]

This function is **2D only**. There are just three overloads, one per element
type, and there is no 3D or 4D form: rotation in higher dimensions needs an axis
or a plane, which a single scalar angle cannot specify. Passing a 3D or 4D vector
is a compile-time error. [[src/builtins/vector.rs:resolve_call]]

`angle` is a `Float` for **every** overload, including the `Fixed2` and `Integer2`
ones — it is not the vector's element type, in contrast to
`vector::clamp_length`, whose scalar does follow the element type. The `Float2`
overload uses the in-tree `Float` `math::sin` and `math::cos` directly. The
`Fixed2` and `Integer2` overloads convert `angle` with `toFixed` first and then use
the deterministic Q32.32 `sin` and `cos`, so their results are bit-identical on
every target; that conversion is also a range check, and an `angle` too large to
represent as a `Fixed` fails with `ErrOverflow`.
[[src/builtins/vector_package.mfb:__vector_rotate_2d_fixed2]]

The `Integer2` overload is the coarsest. It widens both components to `Fixed`,
applies the rotation in Q32.32, and rounds each result back with `math::round`,
half away from zero. Because a rotation generally maps lattice points off the
lattice, the result is snapped to the nearest integer coordinates and the rotation
is therefore not exactly invertible: rotating by an angle and then by its negative
need not return the original vector. Only the multiples of a quarter turn are
exact on `Integer2`, and even those depend on the `Fixed` sine and cosine landing
exactly on `0` and `1`. For an exact quarter turn counterclockwise, prefer
`vector::perpendicular`, which is a pure swap and negation with no trigonometry at
all. [[src/builtins/vector_package.mfb:__vector_rotate_2d_integer2]]

Rotation preserves magnitude on the `Float2` overload up to double-precision
rounding, and approximately on the other two.

## Overloads

**`vector::rotate_2d(v AS Float2, angle AS Float) AS Float2`**

Uses the in-tree `Float` `math::sin`/`math::cos` and IEEE double arithmetic.

**`vector::rotate_2d(v AS Fixed2, angle AS Float) AS Fixed2`**

Converts `angle` with `toFixed`, then rotates entirely in deterministic Q32.32
arithmetic; identical on every target.

**`vector::rotate_2d(v AS Integer2, angle AS Float) AS Integer2`**

Converts `angle` with `toFixed`, widens both components to `Fixed`, rotates in
Q32.32, then rounds each component back to `Integer` half away from zero.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `v` | `Float2`, `Fixed2`, or `Integer2` | The 2D vector to rotate about the origin. The zero vector is accepted and returns the zero vector. Also spelled `v` as a named argument. [[src/builtins/vector.rs:call_param_names]] |
| `angle` | `Float` | The rotation angle in radians, counterclockwise for a positive value. Unbounded; always a `Float`, whatever the vector's element type. Also spelled `angle` as a named argument. [[src/builtins/vector.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| the same type as `v` | The vector `v` rotated counterclockwise by `angle` radians about the origin, with the same magnitude up to the rounding of the element type. The zero vector maps to the zero vector. [[src/builtins/vector.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | On the `Fixed2` and `Integer2` overloads, converting `angle` with `toFixed` is out of the Q32.32 range, a product exceeds it, or an `Integer2` component rounds outside the `Integer` range. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |
| `77050015` | `ErrFloatOverflow` | On the `Float2` overload, a product or sum reaches infinity and is caught where the result component is bound. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_OVERFLOW_CODE]] |

## Type checking

`vector::rotate_2d` accepts only the three **2D** vector record types — `Float2`,
`Fixed2`, and `Integer2` — and its second argument must be a `Float` for all
three, with no implicit numeric promotion from `Integer`. A 3D or 4D first
argument, a non-`Float` second argument, or any arity other than two is rejected
by the syntax check with the message that a 2D vector and a `Float` angle were
expected. The return type is always the first argument's own type.
[[src/builtins/vector.rs:expected_arguments]] [[src/builtins/vector.rs:resolve_call]]

## Examples

Rotate the `+x` axis by a quarter turn to reach the `+y` axis:

```
IMPORT vector
IMPORT io
IMPORT math

SUB main()
  io::print(toString(vector::rotate_2d(vector::Float2[1.0, 0.0], math::pi2)))
END SUB
```

Rotate clockwise by negating the angle:

```
IMPORT vector
IMPORT io
IMPORT math

SUB main()
  LET cw AS vector::Float2 = vector::rotate_2d(vector::Float2[1.0, 0.0], 0.0 - math::pi2)
  io::print(toString(cw))
END SUB
```

## See also

- `mfb man vector perpendicular`
- `mfb man vector angle`
- `mfb man vector slerp`
- `mfb man vector types`
