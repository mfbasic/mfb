# cross

Generalized (n-1)-ary cross product

## Synopsis

```
vector::cross(a AS Float2) AS Float2
vector::cross(a AS Fixed2) AS Fixed2
vector::cross(a AS Integer2) AS Integer2
vector::cross(a AS Float3, b AS Float3) AS Float3
vector::cross(a AS Fixed3, b AS Fixed3) AS Fixed3
vector::cross(a AS Integer3, b AS Integer3) AS Integer3
vector::cross(a AS Float4, b AS Float4, c AS Float4) AS Float4
vector::cross(a AS Fixed4, b AS Fixed4, c AS Fixed4) AS Fixed4
vector::cross(a AS Integer4, b AS Integer4, c AS Integer4) AS Integer4
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

`vector::cross` returns a vector orthogonal to all of its operands. It is the
*generalized* cross product, which in an N-dimensional space takes `N - 1`
operands, so the arity of this call is fixed by the dimension of the vector type:
one operand in 2D, two in 3D, three in 4D. Passing the wrong number of operands
for the dimension — a single `Float3`, or two `Float4` values — is a compile-time
error, not a runtime one. [[src/builtins/vector.rs:resolve_call]]

In 2D the unary form returns the *left perpendicular* `(-v.y, v.x)`, which is `v`
rotated a quarter turn counterclockwise. In 3D it is the familiar binary product
`a x b`, whose components are `(a.y*b.z - a.z*b.y, a.z*b.x - a.x*b.z, a.x*b.y - a.y*b.x)`;
it follows the right-hand rule, so `cross(right, up)` yields `forward`. In 4D it
is the ternary product built from the six 2x2 minors of `b` and `c` expanded
against `a`, in the cofactor pattern
`(a.y*mZW - a.z*mYW + a.w*mYZ, a.z*mXW - a.x*mZW - a.w*mXZ, a.x*mYW - a.y*mXW + a.w*mXY, a.y*mXZ - a.x*mYZ - a.z*mXY)`.
Note the sign convention this particular expansion implies: `cross` of the `x`,
`y`, and `z` basis vectors yields the **negated** `w` axis, `(0, 0, 0, -1)`, not
`(0, 0, 0, 1)`. [[src/builtins/vector_package.mfb:__vector_cross_float4]]

Every form is built from multiplications and subtractions only — there is no
division, no square root, and no trigonometry anywhere in any overload. As a
result `cross` performs **no rounding** on any element type: the `Integer`
overloads are exact integer arithmetic and the `Fixed` overloads are exact within
the Q32.32 grid, in contrast to `normalize`, `project`, and the interpolation
functions, which all round on `Integer`. `cross` is also the only geometry
function here that never raises `ErrInvalidArgument`: it has no degenerate input
to reject, and the cross product of parallel operands is simply the zero vector.
[[src/builtins/vector_package.mfb:__vector_cross_integer3]]

The unary 2D form computes the same value as `vector::perpendicular`, but the two
are separate functions with separate implementations in the companion source —
`__vector_cross_float2` and `__vector_perpendicular_float2` — rather than one
delegating to the other. Use whichever name reads better at the call site.
[[src/builtins/vector_package.mfb:__vector_perpendicular_float2]]

## Overloads

**`vector::cross(a AS Float2/Fixed2/Integer2) AS ...`**

Unary. Returns the left perpendicular `(-a.y, a.x)`, a quarter turn
counterclockwise.

**`vector::cross(a AS Float3/Fixed3/Integer3, b AS ...) AS ...`**

Binary. The classical right-handed 3D cross product `a x b`, orthogonal to both
operands, with magnitude equal to the area of the parallelogram they span. Zero
when the operands are parallel.

**`vector::cross(a AS Float4/Fixed4/Integer4, b AS ..., c AS ...) AS ...`**

Ternary. The 4D vector orthogonal to all three operands, from the cofactor
expansion of the 2x2 minors of `b` and `c`. Zero when the operands are linearly
dependent.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | one of the nine vector types | The first (and in 2D, only) operand. Its record type selects the overload and fixes the required arity. Also spelled `v` as a named argument. [[src/builtins/vector.rs:call_param_names]] |
| `b` | the same type as `a` | The second operand. Required for the 3D and 4D forms, rejected for the 2D form. [[src/builtins/vector.rs:call_param_names]] |
| `c` | the same type as `a` | The third operand. Required for the 4D form only. [[src/builtins/vector.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| the same type as `a` | A vector of the same type and dimension, orthogonal to every operand. The zero vector when the operands are linearly dependent (in 3D, when `a` and `b` are parallel). [[src/builtins/vector.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | On the `Fixed` and `Integer` overloads, a component product or the difference of two products exceeds the checked range of the element type. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |
| `77050015` | `ErrFloatOverflow` | On the `Float` overloads, a component product or difference reaches infinity and is caught where the result component is bound. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_OVERFLOW_CODE]] |

## Type checking

`vector::cross` is generic over the nine built-in vector record types, and is the
only member of this package whose accepted arity varies: `1` for a 2D type, `2`
for a 3D type, `3` for a 4D type. The declared arity span is therefore `1` through
`3`, with the exact requirement enforced against the first argument's dimension
during overload resolution. Every operand must be the *same* one of the nine
types; there is no mixed-element-type and no cross-dimension overload.
[[src/builtins/vector.rs:arity]] [[src/builtins/vector.rs:resolve_call]]

## Examples

The 3D cross product of the `x` and `y` basis vectors is the `z` axis:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::cross(vector::Float3[1.0, 0.0, 0.0], vector::Float3[0.0, 1.0, 0.0])))
END SUB
```

The unary 2D form is a quarter turn counterclockwise:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::cross(vector::Float2[1.0, 0.0])))
END SUB
```

The ternary 4D form, orthogonal to all three basis operands:

```
IMPORT vector
IMPORT io

SUB main()
  LET n AS vector::Float4 = vector::cross(vector::Float4[1.0, 0.0, 0.0, 0.0], vector::Float4[0.0, 1.0, 0.0, 0.0], vector::Float4[0.0, 0.0, 1.0, 0.0])
  io::print(toString(n))
END SUB
```

## See also

- `mfb man vector perpendicular`
- `mfb man vector dot`
- `mfb man vector normalize`
- `mfb man vector types`
