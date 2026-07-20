# project

Vector projection of one vector onto another

## Synopsis

```
vector::project(a AS Float2, b AS Float2) AS Float2
vector::project(a AS Float3, b AS Float3) AS Float3
vector::project(a AS Float4, b AS Float4) AS Float4
vector::project(a AS Fixed2, b AS Fixed2) AS Fixed2
vector::project(a AS Fixed3, b AS Fixed3) AS Fixed3
vector::project(a AS Fixed4, b AS Fixed4) AS Fixed4
vector::project(a AS Integer2, b AS Integer2) AS Integer2
vector::project(a AS Integer3, b AS Integer3) AS Integer3
vector::project(a AS Integer4, b AS Integer4) AS Integer4
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

`vector::project` returns the component of `a` that lies along `b`, computed as
`(dot(a, b) / dot(b, b)) * b`. The scalar ratio is formed once and then multiplies
each component of `b` in declared field order, so the result is always parallel to
`b` — a scalar multiple of it — and never has any component orthogonal to `b`.
Together with `vector::reject`, which returns the orthogonal remainder, it splits
`a` into two pieces that sum back to `a`.
[[src/builtins/vector_package.mfb:__vector_project_float3]]

The ratio's sign carries meaning: it is positive when `a` leans the same way as
`b`, zero when `a` is orthogonal to `b` (in which case the projection is the zero
vector), and negative when `a` leans against `b`, giving a projection that points
opposite to `b`. Note that only the *direction* of `b` matters for the result, not
its magnitude — the `dot(b, b)` in the denominator cancels the scaling — so
projecting onto `b` and onto `2 * b` gives the same answer.

**`b` must not be the zero vector.** The implementation computes `dot(b, b)` first
and, when it is zero, fails with `ErrInvalidArgument` and the message
`vector::project onto a zero-length vector` rather than dividing by zero. Note
that the guard is on the squared length rather than on the vector's components
directly; the two coincide for exact arithmetic, but on the `Fixed` overloads a
vector whose components are small enough that every square underflows to zero in
Q32.32 will also be rejected. `a`, by contrast, is unconstrained — the zero vector
is a perfectly ordinary `a` and projects to the zero vector.
[[src/builtins/vector_package.mfb:__vector_project_float2]]

The `Float` and `Fixed` overloads form the ratio and the products in their own
element type with correctly-rounded division. The `Integer` overloads are
**intentionally lossy**: the guard and the dot products are exact checked integer
arithmetic, but the ratio is computed in `Float` and each scaled component is
rounded back with `math::round`, half away from zero. An `Integer` projection is
therefore a lattice approximation, and the identity
`project(a, b) + reject(a, b) = a` still holds exactly only because
`vector::reject` is defined by subtracting the rounded projection from `a`.
[[src/builtins/vector_package.mfb:__vector_project_integer3]]

## Overloads

**`vector::project(a AS Float2/Float3/Float4, b AS ...) AS ...`**

Ratio and scaling in IEEE double arithmetic with correctly-rounded division.

**`vector::project(a AS Fixed2/Fixed3/Fixed4, b AS ...) AS ...`**

Ratio and scaling entirely in deterministic Q32.32 arithmetic; identical on every
target.

**`vector::project(a AS Integer2/Integer3/Integer4, b AS ...) AS ...`**

Dot products exact in `Integer`, ratio and scaling in `Float`, each component
rounded back to `Integer` half away from zero.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | one of the nine vector types | The vector to decompose. Unconstrained; the zero vector projects to the zero vector. Also spelled `v` as a named argument. [[src/builtins/vector.rs:call_param_names]] |
| `b` | the same type as `a` | The direction to project onto. Must have a non-zero squared length. Only its direction affects the result, not its magnitude. Also spelled `n` as a named argument. [[src/builtins/vector.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| the same type as `a` | The part of `a` parallel to `b`: a scalar multiple of `b`, pointing the same way as `b` when `dot(a, b)` is positive and the opposite way when it is negative. The zero vector when `a` is orthogonal to `b`. [[src/builtins/vector.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `dot(b, b)` is zero, meaning `b` has no direction to project onto. [[src/builtins/vector_package.mfb:__vector_project_float2]] |
| `77050010` | `ErrOverflow` | On the `Fixed` and `Integer` overloads, a dot-product term or a scaled component exceeds the checked range of the element type, or an `Integer` component rounds outside the `Integer` range. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |
| `77050015` | `ErrFloatOverflow` | On the `Float` overloads, a dot product or a scaled component reaches infinity and is caught where it is bound. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_OVERFLOW_CODE]] |

## Type checking

`vector::project` is generic over the nine built-in vector record types. Both
arguments must be the *same* one of the nine types: there is no mixed-element-type
and no cross-dimension overload, and no implicit conversion is applied to a vector
argument. The return type is always the first argument's own type.
[[src/builtins/vector.rs:resolve_call]] [[src/builtins/vector.rs:same_vector]]

## Examples

Project a diagonal vector onto the `+x` axis:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::project(vector::Float2[2.0, 2.0], vector::Float2[1.0, 0.0])))
END SUB
```

Projection and rejection sum back to the original vector:

```
IMPORT vector
IMPORT io

SUB main()
  LET a AS vector::Float3 = vector::Float3[2.0, 3.0, 4.0]
  LET b AS vector::Float3 = vector::Float3[0.0, 1.0, 0.0]
  io::print(toString(vector::project(a, b)))
  io::print(toString(vector::reject(a, b)))
END SUB
```

## See also

- `mfb man vector reject`
- `mfb man vector reflect`
- `mfb man vector dot`
- `mfb man vector normalize`
