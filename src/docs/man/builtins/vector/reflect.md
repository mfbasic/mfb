# reflect

Reflect a vector about a plane through the origin with the given normal

## Synopsis

```
vector::reflect(v AS Float2, n AS Float2) AS Float2
vector::reflect(v AS Float3, n AS Float3) AS Float3
vector::reflect(v AS Float4, n AS Float4) AS Float4
vector::reflect(v AS Fixed2, n AS Fixed2) AS Fixed2
vector::reflect(v AS Fixed3, n AS Fixed3) AS Fixed3
vector::reflect(v AS Fixed4, n AS Fixed4) AS Fixed4
vector::reflect(v AS Integer2, n AS Integer2) AS Integer2
vector::reflect(v AS Integer3, n AS Integer3) AS Integer3
vector::reflect(v AS Integer4, n AS Integer4) AS Integer4
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

`vector::reflect` returns `v - 2 * dot(v, n) * n`, the mirror image of `v` across
the hyperplane through the origin whose normal is `n`. The scalar `2 * dot(v, n)`
is formed once and then multiplies each component of `n` in declared field order,
with the product subtracted from the corresponding component of `v`. This is the
classic bounce formula: the component of `v` along `n` is negated while the
component within the plane is left untouched.
[[src/builtins/vector_package.mfb:__vector_reflect_float3]]

**`n` is used exactly as given and is never normalized.** The formula is only a
true reflection when `n` is a unit vector; if `n` has length `k`, the term
`2 * dot(v, n) * n` scales by `k^2` and the result is not a mirror image but a
skewed vector whose magnitude generally differs from `v`'s. Callers are
responsible for passing a unit normal — typically the output of
`vector::normalize` — and this function will not do it for them. In exchange,
`reflect` never rejects an input: unlike `vector::project` and `vector::reject`,
it has no division and therefore no zero-vector guard, so a zero `n` is accepted
and simply returns `v` unchanged. [[src/builtins/vector_package.mfb:__vector_reflect_float2]]

Because the implementation is multiplication and subtraction only — no division,
no square root, no trigonometry — it performs **no rounding** on any element type.
The `Integer` overloads are exact checked integer arithmetic and the `Fixed`
overloads are exact within the Q32.32 grid. This puts `reflect` in the small group
of exact members of this package alongside `dot`, `cross`, `scale`, and
`perpendicular`, and means that reflecting an `Integer` vector about an `Integer`
unit axis such as `(0, 1)` is exact.
[[src/builtins/vector_package.mfb:__vector_reflect_integer4]]

Reflection is its own inverse for a unit normal: applying `reflect` twice with the
same `n` returns the original vector. It also preserves magnitude for a unit
normal, and reverses the sign of `dot(v, n)` while leaving every in-plane
component fixed.

## Overloads

**`vector::reflect(v AS Float2/Float3/Float4, n AS ...) AS ...`**

Dot product, doubling, and subtraction in IEEE double arithmetic.

**`vector::reflect(v AS Fixed2/Fixed3/Fixed4, n AS ...) AS ...`**

The same operations in deterministic Q32.32 arithmetic; identical on every
target.

**`vector::reflect(v AS Integer2/Integer3/Integer4, n AS ...) AS ...`**

The same operations in exact checked 64-bit integer arithmetic, with no rounding
of any kind.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `v` | one of the nine vector types | The incoming vector to be reflected. Also spelled `v` as a named argument. [[src/builtins/vector.rs:call_param_names]] |
| `n` | the same type as `v` | The plane normal. Used verbatim and **not** normalized, so pass a unit vector for a true, length-preserving reflection. A zero `n` is accepted and returns `v` unchanged. Also spelled `n` as a named argument. [[src/builtins/vector.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| the same type as `v` | The reflection of `v` about the hyperplane with normal `n`. Equal in magnitude to `v` when `n` is a unit vector; `v` itself when `n` is the zero vector or when `v` lies entirely in the plane. [[src/builtins/vector.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | On the `Fixed` and `Integer` overloads, a dot-product term, the doubling, or a scaled component exceeds the checked range of the element type. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |
| `77050015` | `ErrFloatOverflow` | On the `Float` overloads, a dot product or a result component reaches infinity and is caught where it is bound. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_OVERFLOW_CODE]] |

## Type checking

`vector::reflect` is generic over the nine built-in vector record types. Both
arguments must be the *same* one of the nine types: there is no mixed-element-type
and no cross-dimension overload, and no implicit conversion is applied to a vector
argument. The return type is always the first argument's own type.
[[src/builtins/vector.rs:resolve_call]] [[src/builtins/vector.rs:same_vector]]

## Examples

Bounce a downward-moving vector off a floor whose normal is the `+y` axis:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::reflect(vector::Float2[1.0, 0.0 - 1.0], vector::Float2[0.0, 1.0])))
END SUB
```

Normalize the surface normal first when it is not already a unit vector:

```
IMPORT vector
IMPORT io

SUB main()
  LET n AS vector::Float3 = vector::normalize(vector::Float3[0.0, 3.0, 4.0])
  io::print(toString(vector::reflect(vector::Float3[1.0, 0.0 - 1.0, 0.0], n)))
END SUB
```

## See also

- `mfb man vector project`
- `mfb man vector reject`
- `mfb man vector normalize`
- `mfb man vector dot`
