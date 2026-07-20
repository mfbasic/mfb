# scale

Component-wise (Hadamard) product of two vectors

## Synopsis

```
vector::scale(a AS Float2, b AS Float2) AS Float2
vector::scale(a AS Float3, b AS Float3) AS Float3
vector::scale(a AS Float4, b AS Float4) AS Float4
vector::scale(a AS Fixed2, b AS Fixed2) AS Fixed2
vector::scale(a AS Fixed3, b AS Fixed3) AS Fixed3
vector::scale(a AS Fixed4, b AS Fixed4) AS Fixed4
vector::scale(a AS Integer2, b AS Integer2) AS Integer2
vector::scale(a AS Integer3, b AS Integer3) AS Integer3
vector::scale(a AS Integer4, b AS Integer4) AS Integer4
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

`vector::scale` returns the component-wise product of two vectors —
`(a.x*b.x, a.y*b.y, ...)`, taking as many terms as the dimension, evaluated in
declared field order. This is the Hadamard product, also called the elementwise
product. Neither argument is modified; a fresh record is returned.
[[src/builtins/vector_package.mfb:__vector_scale_float3]]

Despite its name, `scale` is **not** multiplication by a scalar: this package
provides no vector-times-scalar function, and both arguments must be full vectors
of the same type. To multiply a whole vector by one number, build a vector whose
components are all that number and pass it as `b` — for a uniform factor of `3`,
`vector::scale(v, vector::Float3[3.0, 3.0, 3.0])`. The usual application of the
general form is non-uniform axis scaling, where each axis is stretched by its own
factor. [[src/builtins/vector.rs:resolve_call]]

It is also not the dot product: `vector::scale` returns a *vector* of the pairwise
products, whereas `vector::dot` sums those same products into a *scalar*. The two
are related by `dot(a, b) = scale(a, b).x + scale(a, b).y + ...`, but they have
different return types and the compiler will not confuse them.
[[src/builtins/vector_package.mfb:__vector_dot_float3]]

The implementation is multiplication only — no addition beyond that, no division,
no square root, no trigonometry — so it performs **no rounding** on any element
type. The `Integer` overloads are exact checked integer arithmetic and the `Fixed`
overloads are exact within the Q32.32 grid, putting `scale` in the small exact
group alongside `dot`, `cross`, `reflect`, and `perpendicular`. Multiplication is
still ordinary checked arithmetic, however, so a product that leaves the range of
the element type fails with `ErrOverflow` rather than wrapping.
[[src/builtins/vector_package.mfb:__vector_scale_integer4]]

## Overloads

**`vector::scale(a AS Float2/Float3/Float4, b AS ...) AS ...`**

Per-component product in IEEE double arithmetic.

**`vector::scale(a AS Fixed2/Fixed3/Fixed4, b AS ...) AS ...`**

Per-component product in deterministic Q32.32 arithmetic; identical on every
target.

**`vector::scale(a AS Integer2/Integer3/Integer4, b AS ...) AS ...`**

Per-component product in exact checked 64-bit integer arithmetic, with no
rounding of any kind.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | one of the nine vector types | The first vector. Also spelled `v` as a named argument. [[src/builtins/vector.rs:call_param_names]] |
| `b` | the same type as `a` | The second vector, one factor per axis. Must be the same vector type as `a`. Also spelled `n` as a named argument. [[src/builtins/vector.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| the same type as `a` | A new vector of the same type and dimension whose `i`-th component is the product of `a`'s and `b`'s `i`-th components. The zero vector when either argument is the zero vector. [[src/builtins/vector.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | On the `Fixed` and `Integer` overloads, a component product exceeds the checked range of the element type. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |
| `77050015` | `ErrFloatOverflow` | On the `Float` overloads, a component product reaches infinity and is caught where the result component is bound. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_OVERFLOW_CODE]] |

## Type checking

`vector::scale` is generic over the nine built-in vector record types. Both
arguments must be the *same* one of the nine types: there is no mixed-element-type
and no cross-dimension overload, and — importantly — no overload takes a bare
scalar as its second argument. The return type is always the first argument's own
type. [[src/builtins/vector.rs:resolve_call]] [[src/builtins/vector.rs:same_vector]]

## Examples

The component-wise product of two 2D vectors:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::scale(vector::Float2[2.0, 3.0], vector::Float2[4.0, 5.0])))
END SUB
```

Uniform scaling, expressed by repeating the factor in every component:

```
IMPORT vector
IMPORT io

SUB main()
  LET tripled AS vector::Float3 = vector::scale(vector::Float3[1.0, 2.0, 3.0], vector::Float3[3.0, 3.0, 3.0])
  io::print(toString(tripled))
END SUB
```

## See also

- `mfb man vector dot`
- `mfb man vector min`
- `mfb man vector max`
- `mfb man vector clamp_length`
