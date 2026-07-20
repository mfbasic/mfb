# toByte

Convert an `Integer`, `Money`, or `Scalar` value to an unsigned 8-bit `Byte`.

## Synopsis

```
toByte(value AS Integer) AS Byte
toByte(value AS Money) AS Byte
toByte(value AS Scalar) AS Byte
```

## Package

general

## Imports

None. `general` functions are always available without an `IMPORT` statement. [[src/builtins/general.rs:is_general_call]]

## Description

`toByte` narrows a supported value to an unsigned 8-bit `Byte`. A `Byte` holds a
whole number in the range `0` through `255` inclusive. Which overload is selected,
and how the argument is interpreted, depends on the argument type. [[src/builtins/general.rs:resolve_call]]

Every overload range-checks the value against `0` through `255` before producing
the `Byte`. A value that lands within range is moved into a `Byte` holding the same
numeric value; a value below `0` or above `255` cannot be represented and fails with
`ErrOverflow` rather than truncating, wrapping, or applying any modular reduction. [[src/target/shared/code/builder_conversions.rs:lower_to_byte]]

The `Integer` overload range-checks the signed 64-bit value directly. The `Money`
overload first reduces the value to its whole-unit count (`raw / 100000`, truncated
toward zero, discarding the fractional units), then range-checks that whole-unit
count exactly like an `Integer`. The `Scalar` overload range-checks the scalar's
Unicode code point; since a code point is never negative, only a code point above
`255` (any scalar beyond `U+00FF`) fails. `toByte(Scalar)` is the inverse of
`toScalar(Byte)`, which always succeeds. [[src/target/shared/code/builder_conversions.rs:lower_to_byte]]

`toByte` has no side effects beyond producing the result `Byte`; it never mutates
its argument.

## Overloads

**`toByte(value AS Integer) AS Byte`**

Narrows a signed 64-bit `Integer` to `Byte`, failing with `ErrOverflow` when `value`
is outside `0` through `255`.

**`toByte(value AS Money) AS Byte`**

Reduces `value` to its whole-unit count (`raw / 100000`, truncated toward zero) and
narrows that to `Byte`, failing with `ErrOverflow` when the whole-unit count is
outside `0` through `255`.

**`toByte(value AS Scalar) AS Byte`**

Narrows a `Scalar`'s Unicode code point to `Byte`, failing with `ErrOverflow` when
the code point exceeds `255`. The inverse of `toScalar(Byte)`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer` | A signed 64-bit value to narrow to a `Byte`. Must be in the range `0` through `255` inclusive; any value outside this range is rejected. |
| `value` | `Money` | A `Money` value whose whole-unit count (`raw / 100000`, truncated toward zero) is narrowed to a `Byte`; the whole-unit count must be `0` through `255`. |
| `value` | `Scalar` | A Unicode scalar value whose code point is narrowed to a `Byte`; the code point must be `0` through `255` (`U+0000` through `U+00FF`). |

## Return value

| Type | Description |
| --- | --- |
| `Byte` | The `Byte` holding the same numeric value as the (whole-unit or code-point) input when it is within `0` through `255`, so `toByte(0)` yields `0` and `toByte(255)` yields `255`. |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050010` | `ErrOverflow` | The value to narrow is less than `0` or greater than `255` and therefore cannot be represented as an 8-bit `Byte`. For `Money`, the checked value is the whole-unit count `raw / 100000`; for `Scalar`, it is the Unicode code point. [[src/target/shared/code/builder_conversions.rs:lower_to_byte]] [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Type checking

`toByte` accepts only `Integer`, `Money`, and `Scalar` values, each in the
one-argument form; any other argument type or arity is a compile-time error.
Convert unsupported values to one of these types explicitly first. [[src/builtins/general.rs:resolve_call]] [[src/builtins/general.rs:arity]]

## Examples

Narrow an Integer literal:

```
SUB main()
  LET value AS Byte = toByte(65)
END SUB
```

Narrow an arithmetic result back into a Byte:

```
SUB main()
  LET original AS Byte = 10
  LET bumped AS Byte = toByte(toInt(original) + 1)
END SUB
```

Narrow a Scalar's code point:

```
SUB main()
  LET letter AS Scalar = `A`
  LET code AS Byte = toByte(letter)
END SUB
```

## See also

- `mfb man general toInt`
- `mfb man general toScalar`
- `mfb man general toMoney`
- `mfb man general toString`
- `mfb man general typeName`
