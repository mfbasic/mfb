# abs

Component-wise absolute value of a vector

## Synopsis

```
vector::abs(v AS Float2) AS Float2
vector::abs(v AS Float3) AS Float3
vector::abs(v AS Float4) AS Float4
vector::abs(v AS Fixed2) AS Fixed2
vector::abs(v AS Fixed3) AS Fixed3
vector::abs(v AS Fixed4) AS Fixed4
vector::abs(v AS Integer2) AS Integer2
vector::abs(v AS Integer3) AS Integer3
vector::abs(v AS Integer4) AS Integer4
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

`vector::abs` returns a new vector of the same type whose every component is the
absolute value of the corresponding component of `v`. Each component is computed
by the scalar `math::abs` of that component, evaluated in declared field order
(`x`, then `y`, then `z`, then `w`), and the results are assembled into a fresh
record. `v` is not modified — like every `vector` type these records copy by
value. [[src/builtins/vector_package.mfb:__vector_abs_float3]]

This is a purely component-wise operation with no cross-component interaction:
`abs` reflects the vector into the all-positive orthant, so it is not a
direction-preserving operation and the result generally does not point the same
way as `v`. The magnitude is preserved, however, because negating individual
components does not change the sum of their squares — `vector::length(vector::abs(v))`
always equals `vector::length(v)`.

The three element types differ only in how the scalar absolute value is taken.
The `Float` overloads clear the sign bit with the hardware floating-point
absolute value, which cannot overflow and performs no rounding or domain check,
so the `Float` overloads never fail. The `Fixed` and `Integer` overloads operate
on the underlying signed 64-bit representation, whose negative range extends one
step further than its positive range; negating the minimum representable value
has no positive counterpart and is reported as `ErrOverflow` rather than
silently wrapping. This is exactly the scalar `math::abs` behavior, inherited
per component. [[src/target/shared/code/builder_math.rs:lower_math_abs]]

## Overloads

**`vector::abs(v AS Float2/Float3/Float4) AS Float2/Float3/Float4`**

Clears the sign bit of each IEEE double component. Exact, never rounds, and
never fails.

**`vector::abs(v AS Fixed2/Fixed3/Fixed4) AS Fixed2/Fixed3/Fixed4`**

Negates each negative Q32.32 component with checked fixed-point arithmetic.
Fails with `ErrOverflow` if any component is the minimum representable `Fixed`.

**`vector::abs(v AS Integer2/Integer3/Integer4) AS Integer2/Integer3/Integer4`**

Negates each negative 64-bit signed component with checked integer arithmetic.
Fails with `ErrOverflow` if any component is the minimum representable
`Integer`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `v` | one of the nine vector types | The vector whose components are taken in absolute value. Any finite value is accepted except the minimum representable `Integer`/`Fixed`. Also spelled `v` as a named argument. [[src/builtins/vector.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| the same type as `v` | A new vector of the same type and dimension whose components are the absolute values of `v`'s components, in the same order. A vector that is already all non-negative is returned with identical components. [[src/builtins/vector.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | A `Fixed` or `Integer` component is the minimum representable value of its type, whose negation is not representable. The `Float` overloads never raise this. [[src/target/shared/code/builder_math.rs:lower_math_abs]] |

## Type checking

`vector::abs` is generic over the nine built-in vector record types. The overload
is selected at compile time from the exact record type of the single argument;
no implicit conversion or numeric promotion is applied to a vector argument, and
a non-vector argument or any arity other than one is rejected by the syntax
check. The return type is always the argument's own type.
[[src/builtins/vector.rs:resolve_call]] [[src/builtins/vector.rs:arity]]

## Examples

Absolute value of a `Float2`:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::abs(vector::Float2[0.0 - 2.0, 3.0])))
END SUB
```

Absolute value of an `Integer3`:

```
IMPORT vector
IMPORT io

SUB main()
  LET a AS vector::Integer3 = vector::abs(vector::Integer3[0 - 3, 4, 0 - 5])
  io::print(toString(a))
END SUB
```

## See also

- `mfb man vector min`
- `mfb man vector max`
- `mfb man vector scale`
- `mfb man vector types`
