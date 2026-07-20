# dot

Dot (inner) product of two vectors

## Synopsis

```
vector::dot(a AS Float2, b AS Float2) AS Float
vector::dot(a AS Float3, b AS Float3) AS Float
vector::dot(a AS Float4, b AS Float4) AS Float
vector::dot(a AS Fixed2, b AS Fixed2) AS Fixed
vector::dot(a AS Fixed3, b AS Fixed3) AS Fixed
vector::dot(a AS Fixed4, b AS Fixed4) AS Fixed
vector::dot(a AS Integer2, b AS Integer2) AS Integer
vector::dot(a AS Integer3, b AS Integer3) AS Integer
vector::dot(a AS Integer4, b AS Integer4) AS Integer
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

`vector::dot` returns the sum of the products of corresponding components:
`a.x*b.x + a.y*b.y + a.z*b.z + a.w*b.w`, taking as many terms as the dimension.
The products are formed and accumulated strictly left to right in declared field
order, which is what makes the result reproducible bit for bit across targets.
The dot product is symmetric — `dot(a, b)` equals `dot(b, a)` — and is a scalar,
so the return type is the vector type's element type rather than a vector.
[[src/builtins/vector_package.mfb:__vector_dot_float3]]

Geometrically the dot product equals `length(a) * length(b) * cos(angle(a, b))`,
which makes its **sign** the useful part in most code: positive when the two
vectors point broadly the same way (their angle is under a quarter turn), zero
when they are exactly orthogonal, and negative when they point broadly opposite
ways. `dot(v, v)` is the squared length of `v`, which is why several other
functions in this package — `project`, `reject`, `angle`, and the `Integer`
`normalize` — use it to test for a zero-length vector without paying for a square
root. [[src/builtins/vector_package.mfb:__vector_project_float3]]

The implementation is multiplication and addition only: no division, no square
root, and no trigonometry. It therefore performs **no rounding** on any element
type. The `Integer` overloads are exact checked integer arithmetic, so
`vector::dot` is one of the few members of this package (with `cross` and `scale`)
whose `Integer` results carry no approximation at all. The `Fixed` overloads are
exact within the Q32.32 grid, and the `Float` overloads are ordinary IEEE
double arithmetic. [[src/builtins/vector_package.mfb:__vector_dot_integer4]]

Because the terms are ordinary checked arithmetic, `dot` can overflow. Squaring
a large coordinate is the common way to hit this: `dot(v, v)` on an `Integer3`
whose components approach the square root of the `Integer` maximum will exceed
the range and fail with `ErrOverflow`. There are no other failure modes — `dot`
never rejects an input, and the zero vector is an entirely ordinary argument
returning zero. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]]

## Overloads

**`vector::dot(a AS Float2/Float3/Float4, b AS ...) AS Float`**

Products and sum in IEEE double arithmetic, accumulated left to right.

**`vector::dot(a AS Fixed2/Fixed3/Fixed4, b AS ...) AS Fixed`**

Products and sum in deterministic Q32.32 arithmetic; identical on every target.

**`vector::dot(a AS Integer2/Integer3/Integer4, b AS ...) AS Integer`**

Products and sum in exact checked 64-bit integer arithmetic, with no rounding of
any kind.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | one of the nine vector types | The first vector. The zero vector is accepted and yields zero. Also spelled `v` as a named argument. [[src/builtins/vector.rs:call_param_names]] |
| `b` | the same type as `a` | The second vector, which must be the same vector type as `a`. Also spelled `n` as a named argument. [[src/builtins/vector.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| the element type of `a` (`Float`, `Fixed`, or `Integer`) | The dot product. Positive when the vectors point broadly the same way, zero when they are orthogonal or either is the zero vector, negative when they point broadly opposite ways. [[src/builtins/vector.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | On the `Fixed` and `Integer` overloads, a component product or the running sum exceeds the checked range of the element type. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |
| `77050015` | `ErrFloatOverflow` | On the `Float` overloads, a product or the sum reaches infinity and is caught where the result is returned. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_OVERFLOW_CODE]] |

## Type checking

`vector::dot` is generic over the nine built-in vector record types. Both
arguments must be the *same* one of the nine types: there is no mixed-element-type
and no cross-dimension overload, and no implicit conversion is applied to a vector
argument. The return type is the element type of that vector type, not the vector
type itself. [[src/builtins/vector.rs:resolve_call]] [[src/builtins/vector.rs:same_vector]]

## Examples

The dot product of two 3D vectors:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::dot(vector::Float3[1.0, 2.0, 3.0], vector::Float3[4.0, 5.0, 6.0])))
END SUB
```

Using the sign to test whether two directions broadly agree:

```
IMPORT vector
IMPORT io

SUB main()
  LET facing AS Float = vector::dot(vector::Float2[1.0, 0.0], vector::Float2[0.0 - 1.0, 0.0])
  IF facing < 0.0 THEN
    io::print("opposite")
  END IF
END SUB
```

## See also

- `mfb man vector cross`
- `mfb man vector angle`
- `mfb man vector project`
- `mfb man vector length`
