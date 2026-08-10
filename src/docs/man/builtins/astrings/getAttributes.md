# getAttributes

The resolved attributes active at a scalar index.

## Synopsis

```
astrings::getAttributes(value AS AttributedString, index AS Integer) AS List OF Attribute
```

## Package

`astrings`

## Imports

```
IMPORT astrings
```

## Description

`getAttributes` returns the attributes in effect at scalar `index`: for each enum member with any
covering span, the covering span with the **highest start** wins (ties break to the later insertion).
The result carries at most one `Attribute` per member — flags are present when any covering span
carries them; font/font-size take the winning span's value. Attributes are never merged on write, so
this read-time resolution is where overlaps are decided. [[src/builtins/astrings_package.mfb]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `AttributedString` | The attributed string to query. |
| `index` | `Integer` | The scalar index to resolve (0-based). |

## Return value

| Type | Description |
| --- | --- |
| `List OF Attribute` | The resolved attributes at `index`, one per active member; empty when none. |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `7-705-0001` | `ErrIndexOutOfRange` | `index < 0` or `index >= ` the visible scalar count (an empty string always errors). |

## Examples

```
IMPORT astrings
IMPORT io

SUB main()
  LET a AS AttributedString = astrings::fromString("hello world")
  MUT b AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::bold())
  FOR EACH attr IN astrings::getAttributes(b, 2)
    MATCH attr
      CASE AttrFlag(f)
        io::print("flag")
      CASE ELSE
    END MATCH
  NEXT
END SUB
```

## See also

- `mfb man astrings addAttribute`
- `mfb man astrings package`
