# general

Always-in-scope core functions for length, conversion, type inspection, predicates, and Error construction

## Synopsis

```
len(value)
toString(value)
toInt(text, base)
typeName(value)
error(code, message)
```

## Imports

`general` is always in scope. Its functions need no `IMPORT` statement and no
manifest dependency. [[src/builtins/general.rs:is_general_call]]

## Description

The `general` package collects the core built-in functions that every program
can call without qualification: length queries, numeric and text conversions,
static type inspection, value predicates, and `Error` construction. They operate
on the primitive types (`Integer`, `Float`, `Fixed`, `Byte`, `Boolean`,
`String`) and on the generic `List OF T` and `Map OF K TO V` containers.
[[src/builtins/general.rs:resolve_call]]

Most `general` functions are overloaded on the static type of their argument, and
the overload is resolved at compile time; an argument whose type a function does
not accept is rejected during type checking rather than at run time. `len`
reports a size as an `Integer`: the Unicode scalar count of a `String`, the
element count of a `List OF T`, or the entry count of a `Map OF K TO V`.
`typeName` returns a display name for a value's static type and never reads the
value itself. [[src/builtins/general.rs:expected_arguments]]

The conversion family moves between the numeric and text types. `toInt`,
`toFloat`, `toFixed`, `toByte`, `toMoney`, and `toScalar` produce the named
type; `toString` renders `Integer`, `Float`, `Fixed`, `Money`, `Boolean`,
`String`, `Byte`, `Scalar`, and `List OF Byte` values as text. Each accepts
`Money` where the conversion is meaningful: `toInt(Money)`, `toFloat(Money)`,
`toFixed(Money)`, and `toByte(Money)` all compile, and `toMoney` is the
explicit crossing *into* `Money` from a `String`, `Integer`, `Float`, `Fixed`,
or `Byte`. `toScalar` builds a `Scalar` (a 32-bit Unicode scalar value) from an
`Integer` code point, a one-scalar `String`, or a `Byte`; `toInt(Scalar)` and
`toString(Scalar)` are its infallible inverses, and `toByte(Scalar)` narrows a
code point below 256. `toString` on `Float`, `Fixed`, and `Money` takes an
optional precision (also spelled `decimals`) that defaults to 2, and `toInt`
takes an optional second `base` argument (2 through 36) as a separate arity, not
a default parameter. None of the conversions mutate their argument.
[[src/builtins/general.rs:call_param_names]]

The predicates return a `Boolean` and inspect their argument without side
effects: `isNumeric` tests whether a `String` would parse as a number, and the
numeric and emptiness predicates (`isEven`, `isOdd`, `isPositive`, `isNegative`,
`isZero`, `isEmpty`, `isNotEmpty`) classify a value by parity, sign, or size.
`isEmpty` and `isNotEmpty` use the same length rules as `len`. These predicates
are inlined builtins, so they cannot be passed as function values directly; wrap
one in a `FUNC` where a predicate argument is needed. The predicates are also
exposed through the `filters` package. [[src/builtins/general.rs:filter_predicate_type]]

`error` constructs a read-only `Error` value from an `Integer` code and a
`String` message, capturing the call-site source location in an `ErrorLoc`. An
`Error` exposes `code` (`Integer`), `message` (`String`), and `source`
(`ErrorLoc`) fields; an `ErrorLoc` exposes `filename` (`String`), `line`
(`Integer`), and `char` (`Integer`). Both are read-only built-in records whose
fields cannot be reassigned. Unlike the other `general` functions, `error` is a
reserved language primitive and cannot be overridden by a user `FUNC`.
[[src/builtins/general.rs:is_overridable]]

Every `general` function except `error` may be overridden by a user- or
package-defined `FUNC` of the same name for its own value types; a package
override yields the built-in's conventional result type, while a user override
yields its own declared return type. `List` and `Map` helpers live in the
`collections` package, and string slicing and search helpers live in the
`strings` package. [[src/builtins/general.rs:override_result_type]]

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | raised by `toInt`, `toFloat`, and `toFixed` when a `String` argument is not well-formed text for the target type, or when a `Float` argument to `toInt` or `toFixed` is NaN or infinite [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |
| `77050010` | `ErrOverflow` | raised by `toInt`, `toFloat`, `toFixed`, and `toByte` when a value is outside the representable range of the target type, such as text too large for `Integer` or an `Integer` outside 0 through 255 for `toByte` [[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |
| `77020004` | `ErrEncoding` | raised by the `toString` `List OF Byte` overload when the byte sequence is not valid UTF-8 [[src/target/shared/code/error_constants.rs:ERR_ENCODING_CODE]] |
