# length

Euclidean length (magnitude) of a vector

## Synopsis

```
vector::length(v AS Float2) AS Float
vector::length(v AS Float3) AS Float
vector::length(v AS Float4) AS Float
vector::length(v AS Fixed2) AS Fixed
vector::length(v AS Fixed3) AS Fixed
vector::length(v AS Fixed4) AS Fixed
vector::length(v AS Integer2) AS Integer
vector::length(v AS Integer3) AS Integer
vector::length(v AS Integer4) AS Integer
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

`vector::length` returns the Euclidean magnitude of `v`: the square root of the
sum of its squared components, `sqrt(x*x + y*y + ...)`, taking as many terms as
the dimension. The squares are accumulated strictly left to right in declared
field order, `x` before `y` before `z` before `w`, which is what makes the result
reproducible bit for bit across targets. The result is always non-negative, and
is zero exactly when every component of `v` is zero.
[[src/builtins/vector_package.mfb:__vector_length_float3]]

The return type is the vector type's **element** type, not the vector type: a
`Float4` measures to a `Float`, a `Fixed2` to a `Fixed`, an `Integer3` to an
`Integer`. The zero vector is an entirely ordinary argument here — `length` has no
degenerate input to reject and never raises `ErrInvalidArgument`, in contrast to
`vector::normalize`, which needs a direction and refuses the zero vector.
[[src/builtins/vector.rs:resolve_call]]

The `Float` overloads sum in IEEE doubles and take the root with `math::sqrt`.
The `Fixed` overloads work entirely in deterministic Q32.32 arithmetic. The
`Integer` overloads square and sum in exact checked integer arithmetic and then
apply the package's rounding integer square root: it first derives a seed from
the hardware `Float` square root of the sum, then corrects that seed to the exact
`floor` of the true root using only integer comparisons and divisions, and finally
rounds up when the remainder exceeds the floor. The floating-point seed is only a
starting point — the integer correction loops guarantee the exact floor
regardless of how the seed rounded — so the `Integer` result is deterministic and
independent of the host's floating-point behavior.
[[src/builtins/vector_package.mfb:__vector_isqrtFloor]]

The rounding rule for the `Integer` overloads is half away from zero, matching
`math::round`. Because `(f + 0.5)^2` is never an integer, no exact tie can ever
occur, so the rule is unambiguous in practice: the result rounds up exactly when
the remainder above the floor exceeds the floor itself. An `Integer` length is
therefore the nearest integer to the true magnitude, not a truncation —
`length(Integer2[3, 4])` is exactly `5`, and `length(Integer2[1, 1])` is `1`.
[[src/builtins/vector_package.mfb:__vector_isqrtRound]]

## Overloads

**`vector::length(v AS Float2/Float3/Float4) AS Float`**

Squares and sum in IEEE doubles; root via `math::sqrt`.

**`vector::length(v AS Fixed2/Fixed3/Fixed4) AS Fixed`**

Squares, sum, and root entirely in deterministic Q32.32 arithmetic, so the result
is bit-identical on every target.

**`vector::length(v AS Integer2/Integer3/Integer4) AS Integer`**

Squares and sum in exact checked integer arithmetic; root via the rounding
integer square root, halves away from zero.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `v` | one of the nine vector types | The vector to measure. The zero vector is accepted and measures zero. Also spelled `v` as a named argument. [[src/builtins/vector.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| the element type of `v` (`Float`, `Fixed`, or `Integer`) | The non-negative Euclidean magnitude of `v`, zero exactly when every component is zero. The `Integer` overloads return the magnitude rounded to the nearest integer, halves away from zero. [[src/builtins/vector.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | On the `Fixed` and `Integer` overloads, a squared component or the sum of squares exceeds the checked range of the element type. [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |
| `77050015` | `ErrFloatOverflow` | On the `Float` overloads, a squared component or the sum reaches infinity and is caught where the result is returned. [[src/target/shared/code/error_constants.rs:ERR_FLOAT_OVERFLOW_CODE]] |

## Type checking

`vector::length` is generic over the nine built-in vector record types. The
overload is selected at compile time from the exact record type of the single
argument; no implicit conversion or numeric promotion is applied to a vector
argument, and a non-vector argument or any arity other than one is rejected by the
syntax check. The return type is the element type of that vector type, not the
vector type itself. [[src/builtins/vector.rs:resolve_call]] [[src/builtins/vector.rs:arity]]

## Examples

The length of a 3-4-5 vector:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::length(vector::Float3[3.0, 0.0, 4.0])))
END SUB
```

An `Integer` length rounds to the nearest whole unit:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::length(vector::Integer2[3, 4])))
END SUB
```

## See also

- `mfb man vector distance`
- `mfb man vector normalize`
- `mfb man vector clamp_length`
- `mfb man vector dot`
