# typeName

Return the display name of a value's static type as a string.

## Synopsis

```
typeName(value AS T) AS String
```

## Package

general

## Imports

None. `general` functions are always available without an `IMPORT` statement. [[src/builtins/general.rs:is_general_call]]

## Description

`typeName` returns a `String` holding the display name of the static type `T` of
`value`. It is intended for diagnostics, debugging, and human-readable messages.

The name reflects the static type known at compile time, not any runtime tag. The
argument is used only to determine its static type; its runtime contents are never
read, and `typeName` has no side effects. The result is fixed entirely at compile
time — `typeName` lowers to a string constant carrying the resolved type name — so
it never inspects, allocates, or fails at run time. [[src/target/shared/code/builder_values.rs:lower_value_inner]]

The name matches how the type is written in source. For a primitive type it is the
type keyword, such as `"Integer"`, `"String"`, or `"Byte"`. For a composite type it
spells out the structure, such as `"List OF Byte"` or `"Map OF String TO Integer"`.
The compiler must be able to determine the static type of `value`; a value whose
type cannot be resolved is a compile-time error, not a run-time one. [[src/target/shared/code/builder_value_semantics.rs:static_type_name]]

`typeName` takes exactly one argument of any type. Any other arity is a compile-time
error. [[src/builtins/general.rs:resolve_call]]

`typeName` is not a serialization or schema API and does not expose runtime values.
The exact spelling of a name may change between implementations, so programs must
not parse it or rely on it for security decisions or redaction. `typeName`,
`toString`, and diagnostic messages are not security boundaries; secret-safe output
requires explicit application-level formatting that omits or redacts sensitive
fields.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `T` | A value of any type `T`. Only its static type is used to compute the result; the value itself is never read. |

## Return value

| Type | Description |
| --- | --- |
| `String` | The display name of the static type `T` of `value`, such as `"Integer"`, `"String"`, `"List OF Byte"`, or `"Map OF String TO Integer"`. |

## Errors

No errors.

## Type checking

`typeName` is generic over its single argument and accepts a `value` of any type
`T`; the type parameter `T` is unconstrained. It takes exactly one argument; any
other arity is rejected at compile time. [[src/builtins/general.rs:resolve_call]]

## Examples

Show a primitive type:

```
SUB main()
  LET name AS String = typeName(42)
END SUB
```

Composite types spell out their structure:

```
SUB main()
  LET kind AS String = typeName([1, 2, 3])
END SUB
```

Use in a diagnostic message:

```
IMPORT io

SUB logType(value AS String)
  io::print("type=" & typeName(value))
END SUB

SUB main()
  logType("hello")
END SUB
```

## See also

- `mfb man general toString`
- `mfb man general error`
- `mfb man general len`
