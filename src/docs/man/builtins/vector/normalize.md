# normalize

Unit vector pointing the same way as the given vector

## Synopsis

```
vector::normalize(v AS Float2) AS Float2
vector::normalize(v AS Float3) AS Float3
vector::normalize(v AS Float4) AS Float4
vector::normalize(v AS Fixed2) AS Fixed2
vector::normalize(v AS Fixed3) AS Fixed3
vector::normalize(v AS Fixed4) AS Fixed4
vector::normalize(v AS Integer2) AS Integer2
vector::normalize(v AS Integer3) AS Integer3
vector::normalize(v AS Integer4) AS Integer4
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

`vector::normalize` divides every component of `v` by `vector::length(v)`,
yielding a vector of magnitude `1` pointing in the same direction. The length is
computed once, in declared field order, and then each component is divided by it
in turn, so the direction is preserved and only the magnitude changes. The
argument is not modified — a fresh record is returned.
[[src/builtins/vector_package.mfb:__vector_normalize_float3]]

**A zero-length vector is rejected.** It has no direction, so there is no unit
vector to return, and dividing by its length would be a division by zero. The
implementation computes the length first and fails with `ErrInvalidArgument` and
the message `vector::normalize of a zero-length vector` when it is zero, rather
than returning the zero vector or a vector of `NaN` components. This is a
deliberate contrast with `vector::clamp_length`, which accepts the zero vector and
passes it through unchanged. Callers that want a zero-safe normalize must test the
length themselves, or trap the error.
[[src/builtins/vector_package.mfb:__vector_normalize_float2]]

The `Float` and `Fixed` overloads divide with the correctly-rounded division of
their element type, giving a result whose length is `1` to within the precision of
that type. The `Integer` overloads are a different matter and are **intentionally
lossy**. They test the squared length for zero, take the rounding integer square
root of it, widen that integer length to `Float`, divide each component there, and
round each quotient back with `math::round`, half away from zero. Because a true
unit vector's components lie between `-1` and `1`, every rounded `Integer`
component collapses to `-1`, `0`, or `1`. `vector::normalize(Integer2[3, 4])`, for
example, returns `(1, 1)` — the exact quotients `0.6` and `0.8` both round to `1`
— which is not a unit vector at all. The `Integer` overloads are best read as
"snap to the nearest lattice direction", and code that needs a real unit vector
should use the `Float` or `Fixed` overloads.
[[src/builtins/vector_package.mfb:__vector_normalize_integer3]]

Note that the zero test differs slightly across element types: the `Float` and
`Fixed` overloads compare the computed square root against zero, while the
`Integer` overloads compare the squared sum against zero before taking any root.
Both reject exactly the all-zero vector.
[[src/builtins/vector_package.mfb:__vector_normalize_integer2]]

## Overloads

**`vector::normalize(v AS Float2/Float3/Float4) AS ...`**

Length via `math::sqrt`, then correctly-rounded IEEE double division per
component. The result has magnitude `1` to within double precision.

**`vector::normalize(v AS Fixed2/Fixed3/Fixed4) AS ...`**

Length and division entirely in deterministic Q32.32 arithmetic; identical on
every target.

**`vector::normalize(v AS Integer2/Integer3/Integer4) AS ...`**

Length via the rounding integer square root, division in `Float`, each quotient
rounded back to `Integer` half away from zero. Components are always `-1`, `0`,
or `1`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `v` | one of the nine vector types | The vector to normalize. Must have a non-zero length. Also spelled `v` as a named argument. [[src/builtins/vector.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| the same type as `v` | A new vector of the same type and dimension pointing in the same direction as `v`. Magnitude `1` on the `Float` and `Fixed` overloads; on the `Integer` overloads a lattice-rounded approximation whose components are each `-1`, `0`, or `1`. [[src/builtins/vector.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `v` has zero length, so it has no direction to preserve. [[src/builtins/vector_package.mfb:__vector_normalize_float2]] |
| `77050010` | `ErrOverflow` | On the `Fixed` and `Integer` overloads, a squared component or the sum of squares exceeds the checked range of the element type. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |
| `77050015` | `ErrFloatOverflow` | On the `Float` overloads, a squared component or the sum of squares reaches infinity and is caught where the length is bound. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_OVERFLOW_CODE]] |

## Type checking

`vector::normalize` is generic over the nine built-in vector record types. The
overload is selected at compile time from the exact record type of the single
argument; no implicit conversion or numeric promotion is applied to a vector
argument, and a non-vector argument or any arity other than one is rejected by the
syntax check. The return type is always the argument's own type.
[[src/builtins/vector.rs:resolve_call]] [[src/builtins/vector.rs:arity]]

## Examples

Normalize a 3-4-5 vector to unit length:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::normalize(vector::Float3[3.0, 0.0, 4.0])))
END SUB
```

The `Integer` overload snaps to the nearest lattice direction rather than
producing a unit vector:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::normalize(vector::Integer2[3, 4])))
END SUB
```

## See also

- `mfb man vector length`
- `mfb man vector clamp_length`
- `mfb man vector project`
- `mfb man vector angle`
