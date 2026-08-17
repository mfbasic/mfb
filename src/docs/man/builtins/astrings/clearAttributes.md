# clearAttributes

Clear all attributes, everywhere or over an inclusive scalar range.

## Synopsis

```
astrings::clearAttributes(value AS AttributedString) AS AttributedString
astrings::clearAttributes(value AS AttributedString, start AS Integer, endIndex AS Integer) AS AttributedString
```

## Package

`astrings`

## Imports

```
IMPORT astrings
```

## Description

`clearAttributes` returns a new `AttributedString` with attributes removed. The one-argument form
empties the entire attribute overlay. The three-argument form clears every attribute over the
inclusive range `[start, endIndex]`, **splitting** any span that straddles the range so its flanks
outside the range survive (regardless of member — unlike `removeAttribute`, no structural match is
required). [[src/codegen/builtins/astrings/package.mfb]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `AttributedString` | The attributed string to clear. |
| `start` | `Integer` | (ranged form) The first scalar index of the range (0-based). |
| `endIndex` | `Integer` | (ranged form) The last scalar index of the range (inclusive). |

## Return value

| Type | Description |
| --- | --- |
| `AttributedString` | A new attributed string with the cleared range/whole overlay; the input is unchanged. |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `7-705-0002` | `ErrInvalidArgument` | (ranged form) `start < 0` or `endIndex < start`. |
| `7-705-0001` | `ErrIndexOutOfRange` | (ranged form) `endIndex >= ` the visible scalar count. |

## Examples

```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello world")
  MUT b AS AttributedString = astrings::addAttribute(a, 0, 10, astrings::bold())
  LET ranged AS AttributedString = astrings::clearAttributes(b, 2, 7)
  LET whole AS AttributedString = astrings::clearAttributes(b)
END SUB
```

## See also

- `mfb man astrings removeAttribute`
- `mfb man astrings getAttributes`
- `mfb man astrings package`
