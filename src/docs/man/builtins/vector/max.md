# max

Component-wise maximum of two vectors

## Synopsis

```
vector::max(a AS Float2, b AS Float2) AS Float2
vector::max(a AS Float3, b AS Float3) AS Float3
vector::max(a AS Float4, b AS Float4) AS Float4
vector::max(a AS Fixed2, b AS Fixed2) AS Fixed2
vector::max(a AS Fixed3, b AS Fixed3) AS Fixed3
vector::max(a AS Fixed4, b AS Fixed4) AS Fixed4
vector::max(a AS Integer2, b AS Integer2) AS Integer2
vector::max(a AS Integer3, b AS Integer3) AS Integer3
vector::max(a AS Integer4, b AS Integer4) AS Integer4
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

`vector::max` returns a new vector whose every component is the larger of the two
corresponding components, each computed by the scalar `math::max` in declared
field order. The result is assembled into a fresh record; neither argument is
modified. [[src/builtins/vector_package.mfb:__vector_max_float3]]

The comparison is made **per component and independently**, so the returned
vector is generally not equal to either input: `max(Float2[2.0, 3.0], Float2[4.0, 1.0])`
is `(4.0, 3.0)`, which is neither operand. This is the corner-wise upper bound of
the two vectors, not a selection of the longer one — `vector::max` does not compare
magnitudes and is not related to `vector::length`. Paired with `vector::min` it is
the standard way to build an axis-aligned bounding box: `min` gives the low corner
and `max` the high corner. [[src/builtins/vector_package.mfb:__vector_max_integer4]]

The operation is a comparison and a select on every element type — it does no
arithmetic at all, so it cannot overflow, performs no rounding, and never fails.
This makes `vector::max` one of only two functions in this package (with
`vector::min`) that raise no errors whatsoever, on any overload. `Float`
comparisons use the hardware maximum instruction; `Fixed`, `Integer`, and `Money`
comparisons are a signed 64-bit compare and select over the underlying
representation. [[src/target/shared/code/builder_math.rs:lower_math_min_max]]

## Overloads

**`vector::max(a AS Float2/Float3/Float4, b AS ...) AS ...`**

Per-component maximum of IEEE double components via the hardware maximum
instruction.

**`vector::max(a AS Fixed2/Fixed3/Fixed4, b AS ...) AS ...`**

Per-component maximum of Q32.32 components by signed 64-bit compare and select.

**`vector::max(a AS Integer2/Integer3/Integer4, b AS ...) AS ...`**

Per-component maximum of 64-bit signed components by compare and select.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | one of the nine vector types | The first vector. Also spelled `v` as a named argument. [[src/builtins/vector.rs:call_param_names]] |
| `b` | the same type as `a` | The second vector, which must be the same vector type as `a`. Also spelled `n` as a named argument. [[src/builtins/vector.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| the same type as `a` | A new vector of the same type and dimension whose `i`-th component is the larger of `a`'s and `b`'s `i`-th components. Equal to an operand only when that operand dominates the other in every component. [[src/builtins/vector.rs:resolve_call]] |

## Errors

No errors.

## Type checking

`vector::max` is generic over the nine built-in vector record types. Both
arguments must be the *same* one of the nine types: there is no mixed-element-type
and no cross-dimension overload, and no implicit conversion is applied to a vector
argument. The return type is always the first argument's own type.
[[src/builtins/vector.rs:resolve_call]] [[src/builtins/vector.rs:same_vector]]

## Examples

The component-wise maximum of two 2D vectors:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::max(vector::Float2[2.0, 3.0], vector::Float2[4.0, 1.0])))
END SUB
```

The high corner of a bounding box around two points:

```
IMPORT vector
IMPORT io

SUB main()
  LET hi AS vector::Integer3 = vector::max(vector::Integer3[1, 7, 3], vector::Integer3[4, 2, 9])
  io::print(toString(hi))
END SUB
```

## See also

- `mfb man vector min`
- `mfb man vector abs`
- `mfb man vector clamp_length`
- `mfb man vector types`
