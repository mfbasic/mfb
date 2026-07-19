# error

Construct an `Error` value from a numeric code and a message.

## Synopsis

```
error(code AS Integer, message AS String) AS Error
```

## Package

general

## Imports

None. `general` functions are always available without an `IMPORT` statement.

## Description

`error` builds an `Error` value from an `Integer` code and a `String` message. The
result is a read-only built-in record carrying the supplied `code` and `message`
together with a compiler-generated source location captured at the call site. It
takes exactly one `Integer` followed by one `String`; any other argument list is a
compile-time type error. [[src/builtins/general.rs:resolve_call]]

Neither argument is interpreted, validated, or constrained. Any `Integer` code and
any `String` message — including a zero code or an empty message — produce a valid
`Error`. `error` never fails and has no side effects beyond producing the value.

`error` is a reserved built-in name: unlike the other `general` functions it cannot
be overridden by a user-defined `FUNC`. [[src/builtins/general.rs:reserved_builtin_name]]

An `Error` is a read-only record. Programs create errors with `error` and inspect
their fields; the fields cannot be reassigned, and binding or propagating an `Error`
preserves them unchanged. An `Error` exposes three fields: [[src/ir/verify/mod.rs:builtin_type_fields]]

- `code` `AS Integer` — the code passed to `error`.
- `message` `AS String` — the message passed to `error`.
- `source` `AS ErrorLoc` — the call site where `error` was evaluated.

The `source` field is an `ErrorLoc` record, itself read-only, with three fields: [[src/ir/verify/mod.rs:builtin_type_fields]]

- `filename` `AS String` — the source file containing the `error` call.
- `line` `AS Integer` — the line of the `error` call.
- `char` `AS Integer` — the column of the `error` call.

The `source` location is filled in by the compiler at lowering time from the call
site; a program does not supply it. [[src/ir/lower.rs:build_error_value]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `code` | `Integer` | Numeric code stored in the `Error`'s `code` field. Any `Integer` is accepted and recorded verbatim; there are no reserved or invalid values. |
| `message` | `String` | Human-readable message stored in the `Error`'s `message` field. Any `String`, including an empty one, is accepted and recorded verbatim. |

## Return value

| Type | Description |
| --- | --- |
| `Error` | A read-only `Error` record whose `code` and `message` equal the supplied arguments and whose `source` holds the `ErrorLoc` of the call site. |

## Errors

No errors.

## Type checking

`error` is defined only for the single overload `error(code AS Integer, message AS
String) AS Error`. The first argument must be an `Integer` and the second a
`String`; any other argument count or type — including `(String, Integer)` — is a
compile-time type error, and no implicit conversion is performed. [[src/builtins/general.rs:resolve_call]]

## Examples

Construct an `Error` and inspect its fields:

```
LET value AS Error = error(123, "boom")
IF value.code <> 123 THEN RETURN 1
IF value.message <> "boom" THEN RETURN 2
```

Return an `Error` from a function:

```
FUNC makeError(code AS Integer, message AS String) AS Error
  RETURN error(code, message)
END FUNC
```

Report the source location of an `Error`:

```
LET value AS Error = error(500, "internal")
LET where AS ErrorLoc = value.source
io::print(where.filename & ":" & toString(where.line) & ":" & toString(where.char))
```

## See also

- `mfb man general typeName`
- `mfb man general toString`
