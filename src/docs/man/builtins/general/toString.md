# toString

Convert a supported primitive value or a UTF-8 byte list to a `String`.

## Synopsis

```
toString(value AS Integer) AS String
toString(value AS Byte) AS String
toString(value AS Boolean) AS String
toString(value AS String) AS String
toString(value AS Scalar) AS String
toString(value AS Float, precision AS Byte = 2) AS String
toString(value AS Fixed, precision AS Byte = 2) AS String
toString(value AS Money, precision AS Byte = 2) AS String
toString(value AS List OF Byte) AS String
```

## Package

general

## Imports

None. `general` functions are always available without an `IMPORT` statement. [[src/builtins/general.rs:is_general_call]]

## Description

`toString` converts a value of one of the supported built-in types to a `String`.
It is the explicit text-formatting seam used before handing a value to
text-oriented APIs such as `io::print`, `io::write`, `io::printError`, and
`io::writeError`; MFBASIC never implicitly stringifies a value for output. [[src/target/shared/code/builder_strings.rs:lower_to_string]]

`Integer` and `Byte` values render as base-10 text. `Byte` is unsigned and always
in the range `0` through `255`, so it renders without a sign; `Integer` renders
with a leading `-` when negative. `Boolean` renders as `"TRUE"` or `"FALSE"`. A
`String` is returned unchanged. A `Scalar` renders as the one-character UTF-8
encoding of its code point. [[src/target/shared/code/builder_strings.rs:lower_to_string]] [[src/target/shared/code/builder_conversions.rs:emit_scalar_to_string_value]]

`Float`, `Fixed`, and `Money` render as decimal text with `precision` digits after
the decimal point, where `precision` is an optional `Byte` that defaults to `2` and
may also be passed by the name `decimals`. [[src/builtins/general.rs:call_param_names]] `Fixed` is binary fixed-point, so its
decimal output reflects the representable value, not necessarily the original
decimal spelling used to create it. Rendering a `Float` is an observation boundary:
when the argument is a fresh arithmetic expression that evaluates to a non-finite
value (for example `x / 0.0`), `toString` traps rather than emitting `"inf"` or
`"nan"`. [[src/target/shared/code/builder_math.rs:observe_float]]

A `List OF Byte` is validated as UTF-8 and, when valid, decoded into the resulting
`String`; an invalid byte sequence fails with `ErrEncoding`. [[src/target/shared/code/builder_strings.rs:emit_byte_list_to_string_value]] Every overload
that builds a fresh `String` allocates it from the arena and fails with
`ErrOutOfMemory` if that allocation cannot be satisfied; the `String` overload
(returned unchanged) and the `Boolean` overload (a static constant) allocate
nothing. [[src/target/shared/code/builder_collection_layout.rs:emit_materialize_string_from_bytes]]

`toString` performs no numeric parsing and has no side effects beyond producing the
result `String`. It is defined only for the overloads listed here; calling it on
records, unions, enums, resources, threads, functions, lambdas, `Map` values, or
`List` values other than `List OF Byte` is a compile-time type error, with no
implicit conversion. [[src/builtins/general.rs:resolve_call]]

`toString` and `typeName` are diagnostic and formatting conveniences, not security
boundaries. Do not rely on them to redact secrets or decide whether a value is safe
to log; secret-safe output requires explicit application-level formatting that omits
or redacts sensitive fields.

## Overloads

**`toString(value AS Integer) AS String`**

Renders the `Integer` as base-10 text, with a leading `-` when negative.

**`toString(value AS Byte) AS String`**

Renders the unsigned `Byte` (`0`–`255`) as base-10 text.

**`toString(value AS Boolean) AS String`**

Returns `"TRUE"` for `TRUE` and `"FALSE"` for `FALSE`.

**`toString(value AS String) AS String`**

Returns `value` unchanged.

**`toString(value AS Scalar) AS String`**

Returns the single-character `String` that is the UTF-8 encoding of the `Scalar`
code point.

**`toString(value AS Float, precision AS Byte = 2) AS String`**

Renders a finite `Float` as decimal text with `precision` digits after the decimal
point (default `2`). A non-finite arithmetic result reaching this boundary traps
with `ErrFloatOverflow`.

**`toString(value AS Fixed, precision AS Byte = 2) AS String`**

Renders a `Fixed` value as decimal text with `precision` digits after the decimal
point (default `2`). Output reflects the representable binary fixed-point value.

**`toString(value AS Money, precision AS Byte = 2) AS String`**

Renders a `Money` value as decimal text with `precision` digits after the decimal
point (default `2`).

**`toString(value AS List OF Byte) AS String`**

Validates the bytes as UTF-8 and returns the decoded `String`; invalid UTF-8 fails
with `ErrEncoding`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer`, `Byte`, `Boolean`, `String`, `Scalar`, `Float`, `Fixed`, `Money`, or `List OF Byte` | The value to convert. Its type selects the overload and determines how the result is formatted. A `List OF Byte` is interpreted as a UTF-8 encoded string. |
| `precision` | `Byte` | Present only on the `Float`, `Fixed`, and `Money` overloads. The number of digits to emit after the decimal point. Defaults to `2` when omitted, and may also be passed by the name `decimals`. |

## Return value

| Type | Description |
| --- | --- |
| `String` | The text representation of `value`: base-10 digits for `Integer` and `Byte`, a fixed-precision decimal for `Float`, `Fixed`, and `Money`, `"TRUE"` or `"FALSE"` for `Boolean`, the original `String` unchanged, the one-character UTF-8 encoding of a `Scalar`, or the UTF-8 decoding of a `List OF Byte`. |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77020004` | `ErrEncoding` | The `List OF Byte` overload receives a byte sequence that is not valid UTF-8. [[src/target/shared/code/builder_strings.rs:emit_byte_list_to_string_value]] |
| `77050015` | `ErrFloatOverflow` | The `Float` overload renders a non-finite arithmetic result (for example `x / 0.0`) at the observation boundary. [[src/target/shared/code/builder_math.rs:observe_float]] |
| `77010001` | `ErrOutOfMemory` | An arena allocation for the result `String` fails, on every overload that builds a fresh string (all except the unchanged `String` and the static `Boolean`). [[src/target/shared/code/builder_collection_layout.rs:emit_materialize_string_from_bytes]] |

## Type checking

`toString` is defined only for the overloads listed on this page: a single argument
of `Integer`, `Byte`, `Boolean`, `String`, `Scalar`, or `List OF Byte`; or a
`Float`, `Fixed`, or `Money` with an optional trailing `Byte` precision. Any other
argument type, or any other arity, is rejected at compile time; no implicit
conversion is performed. Convert to a supported type explicitly, or provide a
domain-specific formatter. [[src/builtins/general.rs:resolve_call]]

## Examples

Render an integer:

```
IMPORT io

SUB main()
  LET count AS Integer = 42
  io::print(toString(count))
END SUB
```

Format a Boolean:

```
SUB main()
  LET enabled AS Boolean = TRUE
  LET label AS String = "enabled=" & toString(enabled)
END SUB
```

Format a Float with explicit precision:

```
SUB main()
  LET ratio AS Float = 3.14159
  LET text AS String = toString(ratio, toByte(4))
END SUB
```

Decode UTF-8 bytes:

```
SUB main()
  LET bytes AS List OF Byte = [104, 101, 108, 108, 111]
  LET text AS String = toString(bytes)
END SUB
```

## See also

- `mfb man general toInt`
- `mfb man general toFloat`
- `mfb man general toFixed`
- `mfb man general toByte`
- `mfb man general toMoney`
- `mfb man general typeName`
