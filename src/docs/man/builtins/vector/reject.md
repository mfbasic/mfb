# reject

Component of one vector orthogonal to another

## Synopsis

```
vector::reject(a AS Float2, b AS Float2) AS Float2
vector::reject(a AS Float3, b AS Float3) AS Float3
vector::reject(a AS Float4, b AS Float4) AS Float4
vector::reject(a AS Fixed2, b AS Fixed2) AS Fixed2
vector::reject(a AS Fixed3, b AS Fixed3) AS Fixed3
vector::reject(a AS Fixed4, b AS Fixed4) AS Fixed4
vector::reject(a AS Integer2, b AS Integer2) AS Integer2
vector::reject(a AS Integer3, b AS Integer3) AS Integer3
vector::reject(a AS Integer4, b AS Integer4) AS Integer4
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

`vector::reject` returns the part of `a` that is perpendicular to `b` — the vector
rejection, the complement of the vector projection. It is implemented directly as
`a - vector::project(a, b)`: the implementation calls the matching `project`
helper for the same type and then subtracts its components from `a`'s, in declared
field order. [[src/builtins/vector_package.mfb:__vector_reject_float3]]

Because `reject` is defined by that subtraction, the decomposition identity
`project(a, b) + reject(a, b) = a` holds **exactly**, on every element type,
including `Integer`. This is worth stating precisely: the projection itself is
rounded on the `Integer` overloads, so it is only an approximation of the true
parallel component, and consequently the `Integer` rejection is only approximately
orthogonal to `b`. What is exact is the round trip — whatever error the rounding
introduced into the projection is absorbed into the rejection, so the two always
sum back to `a` with no residue.
[[src/builtins/vector_package.mfb:__vector_reject_integer3]]

Delegating to `project` also means `reject` inherits its precondition. **`b` must
not be the zero vector**: the underlying `project` computes `dot(b, b)` and fails
with `ErrInvalidArgument` and the message
`vector::project onto a zero-length vector` when it is zero. Note that the message
names `project`, not `reject`, because the failure is raised inside the delegated
call. `a` is unconstrained — the zero vector rejects to the zero vector. Only the
direction of `b` matters, not its magnitude.
[[src/builtins/vector_package.mfb:__vector_project_float2]]

When `a` is already orthogonal to `b` the projection is the zero vector and
`reject` returns `a` unchanged; when `a` is parallel to `b` the projection is all
of `a` and `reject` returns the zero vector. The result is always orthogonal to
`b` on the `Float` and `Fixed` overloads, to within the precision of the element
type. [[src/builtins/vector_package.mfb:__vector_reject_float2]]

## Overloads

**`vector::reject(a AS Float2/Float3/Float4, b AS ...) AS ...`**

Delegates to the `Float` projection, then subtracts in IEEE double arithmetic.

**`vector::reject(a AS Fixed2/Fixed3/Fixed4, b AS ...) AS ...`**

Delegates to the `Fixed` projection, then subtracts in deterministic Q32.32
arithmetic.

**`vector::reject(a AS Integer2/Integer3/Integer4, b AS ...) AS ...`**

Delegates to the `Integer` projection — which rounds each component half away
from zero — then subtracts in exact checked integer arithmetic.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | one of the nine vector types | The vector to decompose. Unconstrained; the zero vector rejects to the zero vector. Also spelled `v` as a named argument. [[src/builtins/vector.rs:call_param_names]] |
| `b` | the same type as `a` | The direction to remove from `a`. Must have a non-zero squared length. Only its direction affects the result, not its magnitude. Also spelled `n` as a named argument. [[src/builtins/vector.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| the same type as `a` | The part of `a` perpendicular to `b`. Equal to `a` when `a` is already orthogonal to `b`, and the zero vector when `a` is parallel to `b`. Always exactly `a` minus `vector::project(a, b)`. [[src/builtins/vector.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `dot(b, b)` is zero, meaning `b` has no direction to remove. Raised by the delegated projection, so the message names `vector::project`. [[src/builtins/vector_package.mfb:__vector_project_float2]] |
| `77050010` | `ErrOverflow` | On the `Fixed` and `Integer` overloads, a dot-product term, a scaled component, or the final subtraction exceeds the checked range of the element type. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |
| `77050015` | `ErrFloatOverflow` | On the `Float` overloads, a dot product, a scaled component, or a difference reaches infinity and is caught where it is bound. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_OVERFLOW_CODE]] |

## Type checking

`vector::reject` is generic over the nine built-in vector record types. Both
arguments must be the *same* one of the nine types: there is no mixed-element-type
and no cross-dimension overload, and no implicit conversion is applied to a vector
argument. The return type is always the first argument's own type.
[[src/builtins/vector.rs:resolve_call]] [[src/builtins/vector.rs:same_vector]]

## Examples

The part of a diagonal vector that is not along the `+x` axis:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::reject(vector::Float2[2.0, 2.0], vector::Float2[1.0, 0.0])))
END SUB
```

Flatten a movement vector so it slides along a wall instead of passing through
it:

```
IMPORT vector
IMPORT io

SUB main()
  LET wall AS vector::Float3 = vector::normalize(vector::Float3[1.0, 0.0, 0.0])
  LET slide AS vector::Float3 = vector::reject(vector::Float3[1.0, 2.0, 0.0], wall)
  io::print(toString(slide))
END SUB
```

## See also

- `mfb man vector project`
- `mfb man vector reflect`
- `mfb man vector dot`
- `mfb man vector normalize`
