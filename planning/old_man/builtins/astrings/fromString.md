# fromString

Construct an `AttributedString` from plain text with no attributes.

## Synopsis

```
astrings::fromString(text AS String) AS AttributedString
```

## Package

`astrings`

## Imports

```
IMPORT astrings
```

`astrings` is a built-in package, so `IMPORT astrings` needs no manifest
dependency. The `AttributedString` type itself is always in scope.

## Description

`fromString` builds an `AttributedString` whose visible text is a deep copy of
`text` and whose attribute overlay is empty. The result is value-semantic: it
copies deeply, drops with its owning scope, and shares no storage with `text`.
[[src/codegen/builtins/astrings/mod.rs:register]] [[src/target/shared/code/builder_astrings.rs:lower_astrings_from_string]]

Recover the visible text with `toString(a)`; `io::print`/`io::write` emit it. The
constructed value has no attributes until `astrings::addAttribute` (and the other
mutation members) records some.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `text` | `String` | The visible text. Copied into the new value. [[src/codegen/builtins/astrings/func_from_string.rs:text]] |

## Return value

| Type | Description |
| --- | --- |
| `AttributedString` | A new attributed string with visible text equal to `text` and an empty attribute overlay. [[src/codegen/builtins/astrings/mod.rs:register]] |

## Errors

No errors.

## Examples

Build an attributed string and print its visible text:

```
IMPORT astrings
IMPORT io

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  io::print(toString(a))
END SUB
```

## See also

- `mfb man astrings package`
