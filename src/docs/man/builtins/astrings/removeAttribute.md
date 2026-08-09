# removeAttribute

Remove a matching attribute over an inclusive scalar range, splitting straddlers.

## Synopsis

```
astrings::removeAttribute(value AS AttributedString, start AS Integer, endIndex AS Integer, attr AS Attribute) AS AttributedString
```

## Package

`astrings`

## Imports

```
IMPORT astrings
```

## Description

`removeAttribute` returns a new `AttributedString` with `attr` removed over the inclusive range
`[start, endIndex]`. A stored span is affected only when its attribute **structurally matches** `attr`
(same member and, for font/size, same value). A matching span that straddles the range is **split**:
its surviving left flank `[s.start, start−1]` and/or right flank `[endIndex+1, s.last]` are kept and
the overlap dropped. Because overlapping spans resolve by higher-start-wins, removing a covering
winner can reveal a lower-start loser at read time. [[src/builtins/astrings_package.mfb]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `AttributedString` | The attributed string to remove from. |
| `start` | `Integer` | The first scalar index of the range (0-based). |
| `endIndex` | `Integer` | The last scalar index of the range (inclusive). |
| `attr` | `Attribute` | The attribute to remove (matched structurally). |

## Return value

| Type | Description |
| --- | --- |
| `AttributedString` | A new attributed string with matching spans removed/split; the input is unchanged. |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `7-705-0002` | `ErrInvalidArgument` | `start < 0` or `endIndex < start`. |
| `7-705-0001` | `ErrIndexOutOfRange` | `endIndex >= ` the visible scalar count. |

## Examples

```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello world here")
  MUT b AS AttributedString = astrings::addAttribute(a, 5, 15, astrings::bold())
  b = astrings::removeAttribute(b, 8, 10, astrings::bold())
END SUB
```

## See also

- `mfb man astrings addAttribute`
- `mfb man astrings clearAttributes`
- `mfb man astrings package`
