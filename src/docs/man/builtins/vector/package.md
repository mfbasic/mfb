# vector

Fixed-width math vectors with overloaded geometry, interpolation, and 2D helpers

## Synopsis

```
IMPORT vector
vector::length(v)
vector::normalize(v)
vector::dot(a, b)
vector::cross(a, b)
vector::lerp(a, b, t)
vector::upFloat3
```

## Description

The `vector` package provides nine small fixed-width math-vector value records
— `Float2`/`Float3`/`Float4`, `Fixed2`/`Fixed3`/`Fixed4`, and
`Integer2`/`Integer3`/`Integer4` — and a set of overloaded geometry,
interpolation, component-wise utility, and 2D functions over them, plus a set of
package-level direction constants. `vector` is a built-in package, so `IMPORT
vector` needs no manifest dependency.

Each type is an ordinary value record of N homogeneous 8-byte fields named `x`,
`y`, `z`, `w` (as many as the dimension): a 2-vector has `x` and `y`, a 3-vector
adds `z`, a 4-vector adds `w`. They copy by value, drop with no heap frees, and
are thread-sendable. Construct them positionally with bracket syntax, for example
`vector::Float3[3.0, 0.0, 4.0]`; the element type of every component matches the
type's name — `Float` components are IEEE doubles, `Fixed` components are Q32.32,
and `Integer` components are 64-bit signed integers. See `mfb man vector types`.

Every function is overloaded by the exact record type and arity of its arguments,
resolved at compile time, and the return type follows: `length`, `distance`,
`dot`, and `angle` return the element scalar; the geometry, interpolation, and
utility functions return a vector of the argument's type. Two vectors passed to a
binary function must be the same type — there is no mixed-type or cross-dimension
overload. `cross` is the generalized (n-1)-ary cross product, so its arity is
dimension-specific: one vector in 2D (the left perpendicular), two in 3D, three
in 4D. `perpendicular` and `rotate_2d` are 2D only.

Results are deterministic and identical across targets for every element type.
The algebraic functions use only correctly-rounded operations (hardware square
root and IEEE arithmetic for `Float`, Q32.32 for `Fixed`, and a deterministic
rounding integer square root for `Integer`); the three trigonometric members
(`angle`, `slerp`, `rotate_2d`) use `math::`'s deterministic `Fixed` and in-tree
`Float` trig, never libm. Evaluation is in a fixed left-to-right component order.

`Integer` is supported for every function where it is mathematically defined.
Every `Integer` result that comes from a real-valued computation — `length`,
`distance`, the `normalize` components, the `project`/`reject` quotient, `lerp`,
`slerp`, `rotate_2d`, `clamp_length` — rounds half away from zero (matching
`math::round`). `dot` and `cross` are exact. Most `Integer` unit vectors are
therefore degenerate (components in -1, 0, 1), but the rounding keeps them as
direction-faithful as integers allow. [[src/builtins/vector_package.mfb:__vector_normalize_integer3]]

`toString` over any vector renders `"(x, y, z)"` with each component formatted by
its own scalar `toString`. The package also exports 42 direction constants named
`<base><Type><N>`, referenced without parentheses like `math::pi`: `zeroFloat3`,
`oneInteger2`, `upFixed4`, `rightInteger3`, `forwardFloat4`, and so on. `zero`,
`one`, `up` (+y axis), and `right` (+x axis) exist for all nine types; `forward`
(+z axis) exists only for the 3D and 4D types (it is undefined in 2D).

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `normalize` of a zero-length vector; `project` or `reject` onto a zero-length vector; `angle` or `slerp` with a zero-length input; `clamp_length` with a negative maximum [[src/builtins/vector_package.mfb:__vector_normalize_float3]] |
| `77050010` | `ErrOverflow` | `abs` of the minimum representable `Integer` or `Fixed` value, and any `Integer` computation whose intermediate or result exceeds the `Integer` range, as in the scalar `math::` functions [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |
