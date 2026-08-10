# background

Construct a background-color `Attribute`.

## Synopsis

```
astrings::background(r AS Byte, g AS Byte, b AS Byte) AS Attribute
```

## Package

`astrings`

## Imports

```
IMPORT astrings
```

## Description

`background` returns an `Attribute` wrapping the `AttrNumber` with `kind`
`AttrTypeNumber.Background` and a `value` that packs the `(r, g, b)` channels into
a single `0xRRGGBB` Integer — `r` in the high byte, `b` in the low byte. Each
channel is a `Byte`, so the packing is lossless. Pass it to
`astrings::addAttribute` to set the text background color over a scalar range;
overlapping background spans resolve by higher-start-wins at read time.
[[src/builtins/astrings_package.mfb]]

When such an `AttributedString` is drawn with `term::drawText(x, y, value)` (both
`term` and `astrings` imported), the color is emitted as a truecolor background.
Renderers that do not model color — such as `astrings::toMarkdown` — ignore it.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `r` | `Byte` | The red channel (0–255). |
| `g` | `Byte` | The green channel (0–255). |
| `b` | `Byte` | The blue channel (0–255). |

## Return value

| Type | Description |
| --- | --- |
| `Attribute` | The background attribute (`AttrNumber[AttrTypeNumber.Background, 0xRRGGBB]`). |

## Errors

No errors.

## Examples

```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::background(0, 0, 255))
END SUB
```

## See also

- `mfb man astrings foreground`
- `mfb man astrings addAttribute`
- `mfb man astrings package`
