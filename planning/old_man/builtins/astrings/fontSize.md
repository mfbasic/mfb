# fontSize

Construct a font-size `Attribute`.

## Synopsis

```
astrings::fontSize(size AS Integer) AS Attribute
```

## Package

`astrings`

## Imports

```
IMPORT astrings
```

## Description

`fontSize` returns an `Attribute` wrapping the `AttrNumber` with `kind` `AttrTypeNumber.FontSize` and
`value` `size`. Pass it to `astrings::addAttribute` to set the font size (e.g. in points) over a
scalar range. Font size is an Integer-valued attribute: overlapping font-size spans resolve by
higher-start-wins at read time. [[src/codegen/builtins/astrings/package.mfb]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `size` | `Integer` | The font size. |

## Return value

| Type | Description |
| --- | --- |
| `Attribute` | The font-size attribute (`AttrNumber[AttrTypeNumber.FontSize, size]`). |

## Errors

No errors.

## Examples

```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::fontSize(14))
END SUB
```

## See also

- `mfb man astrings font`
- `mfb man astrings addAttribute`
- `mfb man astrings package`
