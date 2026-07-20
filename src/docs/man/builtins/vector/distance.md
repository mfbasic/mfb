# distance

Euclidean distance between two points

## Synopsis

```
vector::distance(a AS Float2, b AS Float2) AS Float
vector::distance(a AS Float3, b AS Float3) AS Float
vector::distance(a AS Float4, b AS Float4) AS Float
vector::distance(a AS Fixed2, b AS Fixed2) AS Fixed
vector::distance(a AS Fixed3, b AS Fixed3) AS Fixed
vector::distance(a AS Fixed4, b AS Fixed4) AS Fixed
vector::distance(a AS Integer2, b AS Integer2) AS Integer
vector::distance(a AS Integer3, b AS Integer3) AS Integer
vector::distance(a AS Integer4, b AS Integer4) AS Integer
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

`vector::distance` treats `a` and `b` as points and returns the straight-line
Euclidean distance between them: the square root of the sum of the squared
per-component differences, `sqrt((a.x-b.x)^2 + (a.y-b.y)^2 + ...)`. The result is
always non-negative and is symmetric in the arguments — `distance(a, b)` equals
`distance(b, a)`, because each difference is squared before it is summed.
`distance(a, a)` is zero for every input. [[src/builtins/vector_package.mfb:__vector_distance_float3]]

The differences are formed component by component into named locals first, in
declared field order, and only then squared and summed; the sum is accumulated
left to right, `x` before `y` before `z` before `w`. This fixed evaluation order
is what makes the result reproducible bit for bit across targets. The function is
mathematically equal to `vector::length` of the componentwise difference of the
two vectors, and shares its per-element-type behavior, but it is a distinct
implementation that never materializes that difference vector as a record.
[[src/builtins/vector_package.mfb:__vector_distance_float4]]

The `Float` overloads take the square root with `math::sqrt` over IEEE doubles.
The `Fixed` overloads use the deterministic Q32.32 square root. The `Integer`
overloads square and sum in exact checked integer arithmetic and then apply the
package's rounding integer square root, which returns the nearest integer to the
true root with halves rounded away from zero — so an `Integer` distance is a
rounded distance, not a truncated one, and `distance(Integer2[0,0], Integer2[3,4])`
is exactly `5`. [[src/builtins/vector_package.mfb:__vector_isqrtRound]]

Unlike `vector::normalize` or `vector::angle`, `distance` has no degenerate input
to reject: coincident points are a perfectly ordinary case returning zero. It
therefore never raises `ErrInvalidArgument`. It is not, however, error-free: the
squaring step is ordinary checked arithmetic in the element type and can overflow
for large coordinates, and on the `Integer` overloads the *difference* itself can
overflow before any squaring, when subtracting a large negative coordinate from a
large positive one. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]]

## Overloads

**`vector::distance(a AS Float2/Float3/Float4, b AS ...) AS Float`**

Differences, squares, and sum in IEEE doubles; root via `math::sqrt`.

**`vector::distance(a AS Fixed2/Fixed3/Fixed4, b AS ...) AS Fixed`**

Entirely in deterministic Q32.32 arithmetic, so the result is bit-identical on
every target.

**`vector::distance(a AS Integer2/Integer3/Integer4, b AS ...) AS Integer`**

Differences, squares, and sum in exact checked integer arithmetic; root via the
rounding integer square root, halves away from zero.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | one of the nine vector types | The first point. Also spelled `v` as a named argument. [[src/builtins/vector.rs:call_param_names]] |
| `b` | the same type as `a` | The second point, which must be the same vector type as `a`. Also spelled `n` as a named argument. [[src/builtins/vector.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| the element type of `a` (`Float`, `Fixed`, or `Integer`) | The non-negative Euclidean distance between the two points. Zero when the points coincide. The `Integer` overloads return the distance rounded to the nearest integer, halves away from zero. [[src/builtins/vector.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | On the `Fixed` and `Integer` overloads, a per-component difference, one of its squares, or the sum of squares exceeds the checked range of the element type. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |
| `77050015` | `ErrFloatOverflow` | On the `Float` overloads, a difference, square, or sum reaches infinity and is caught where it is bound or returned. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_OVERFLOW_CODE]] |

## Type checking

`vector::distance` is generic over the nine built-in vector record types. Both
arguments must be the *same* one of the nine types: there is no mixed-element-type
and no cross-dimension overload, and no implicit conversion is applied to a vector
argument. The return type is the element type of that vector type, not the vector
type itself. [[src/builtins/vector.rs:resolve_call]] [[src/builtins/vector.rs:same_vector]]

## Examples

The distance across a 3-4-5 triangle:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::distance(vector::Float2[0.0, 0.0], vector::Float2[3.0, 4.0])))
END SUB
```

The same measurement in exact integer coordinates:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::distance(vector::Integer2[0, 0], vector::Integer2[3, 4])))
END SUB
```

## See also

- `mfb man vector length`
- `mfb man vector dot`
- `mfb man vector normalize`
- `mfb man vector types`
