# addAttribute

Record an attribute over an inclusive scalar range.

## Synopsis

```
astrings::addAttribute(value AS AttributedString, start AS Integer, endIndex AS Integer, attr AS Attribute) AS AttributedString
```

## Package

`astrings`

## Imports

```
IMPORT astrings
```

## Description

`addAttribute` returns a new `AttributedString` with `attr` recorded over the **inclusive** scalar
range `[start, endIndex]` (length `endIndex − start + 1`; `start == endIndex` is a single scalar).
Spans are stored as-is and never merged; overlapping same-member spans resolve at read time by
higher-start-wins (see `getAttributes`). The end-of-range parameter is `endIndex` rather than `end`
because `end` is a reserved keyword. [[src/codegen/builtins/astrings/package.mfb]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `AttributedString` | The attributed string to add to. |
| `start` | `Integer` | The first scalar index of the range (0-based). |
| `endIndex` | `Integer` | The last scalar index of the range (inclusive). |
| `attr` | `Attribute` | The attribute to record (from a constructor such as `astrings::bold()`). |

## Return value

| Type | Description |
| --- | --- |
| `AttributedString` | A new attributed string carrying the added span; the input is unchanged. |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `7-705-0002` | `ErrInvalidArgument` | `start < 0` or `endIndex < start`. |
| `7-705-0001` | `ErrIndexOutOfRange` | `endIndex >= ` the visible scalar count (an empty string always errors). |

## Examples

```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello world")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::bold())
END SUB
```

## See also

- `mfb man astrings removeAttribute`
- `mfb man astrings getAttributes`
- `mfb man astrings package`
