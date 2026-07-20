# perpendicular

Left perpendicular of a 2D vector

## Synopsis

```
vector::perpendicular(v AS Float2) AS Float2
vector::perpendicular(v AS Fixed2) AS Fixed2
vector::perpendicular(v AS Integer2) AS Integer2
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

`vector::perpendicular` returns `(-v.y, v.x)`, the *left* perpendicular of a 2D
vector: `v` rotated a quarter turn counterclockwise about the origin. The result
is orthogonal to `v` — their dot product is `(-v.y)*v.x + v.x*v.y`, identically
zero — and has exactly the same magnitude, because the two components are merely
swapped and one is negated.
[[src/builtins/vector_package.mfb:__vector_perpendicular_float2]]

This function is **2D only**. There are just three overloads, one per element
type, and there is no `Float3` or `Float4` form: in three or more dimensions a
single vector does not determine a unique perpendicular, so the operation is not
well defined. Passing a 3D or 4D vector is a compile-time error, not a runtime
one. For the higher-dimensional analogue use `vector::cross`, which takes the
`N - 1` operands needed to pin down a unique orthogonal direction.
[[src/builtins/vector.rs:resolve_call]]

The 2D unary form of `vector::cross` computes the same value. The two are
nevertheless **separate functions with separate implementations** in the companion
source — `__vector_perpendicular_float2` and `__vector_cross_float2` — rather than
one delegating to the other; the call dispatches to whichever name you wrote.
Prefer `vector::perpendicular` when the intent is a quarter turn and
`vector::cross` when the intent is the generalized product.
[[src/builtins/vector_package.mfb:__vector_cross_float2]]

Because the operation is a swap and a single negation, it does no multiplication,
division, or rounding, and is exact on every element type. It is not, however,
completely error-free: the negation `0 - v.y` is checked arithmetic on the
`Fixed` and `Integer` overloads, so a `y` component equal to the minimum
representable value of its type has no representable negation and fails with
`ErrOverflow`. The `Float` overload negates in IEEE arithmetic, where the
negation of any finite value is finite, so it never fails.
[[src/builtins/vector_package.mfb:__vector_perpendicular_integer2]]

Applying `perpendicular` four times returns the original vector, and applying it
twice returns `-v`. Two applications are therefore a cheap exact negation, and
`perpendicular(perpendicular(perpendicular(v)))` is the *right* perpendicular
`(v.y, -v.x)`, which the package does not provide directly.

## Overloads

**`vector::perpendicular(v AS Float2) AS Float2`**

Swaps the components and negates the new `x` in IEEE double arithmetic. Never
fails.

**`vector::perpendicular(v AS Fixed2) AS Fixed2`**

The same swap and negation in Q32.32. Fails with `ErrOverflow` if `v.y` is the
minimum representable `Fixed`.

**`vector::perpendicular(v AS Integer2) AS Integer2`**

The same swap and negation in checked 64-bit integer arithmetic. Fails with
`ErrOverflow` if `v.y` is the minimum representable `Integer`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `v` | `Float2`, `Fixed2`, or `Integer2` | The 2D vector to rotate a quarter turn counterclockwise. The zero vector is accepted and returns the zero vector. Also spelled `v` as a named argument. [[src/builtins/vector.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| the same type as `v` | The vector `(-v.y, v.x)`: orthogonal to `v`, the same magnitude as `v`, a quarter turn counterclockwise from it. The zero vector maps to the zero vector. [[src/builtins/vector.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | On the `Fixed2` and `Integer2` overloads, `v.y` is the minimum representable value of the element type, whose negation is not representable. The `Float2` overload never raises this. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Type checking

`vector::perpendicular` accepts only the three **2D** vector record types —
`Float2`, `Fixed2`, and `Integer2`. The overload is selected at compile time from
the exact record type of the single argument; a 3D or 4D vector, a non-vector
argument, or any arity other than one is rejected by the syntax check with the
message that a 2D vector was expected. The return type is always the argument's
own type. [[src/builtins/vector.rs:expected_arguments]] [[src/builtins/vector.rs:resolve_call]]

## Examples

The perpendicular of the `+x` axis is the `+y` axis:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::perpendicular(vector::Float2[1.0, 0.0])))
END SUB
```

Applying it twice negates the vector exactly:

```
IMPORT vector
IMPORT io

SUB main()
  LET back AS vector::Integer2 = vector::perpendicular(vector::perpendicular(vector::Integer2[3, 4]))
  io::print(toString(back))
END SUB
```

## See also

- `mfb man vector cross`
- `mfb man vector rotate_2d`
- `mfb man vector dot`
- `mfb man vector types`
