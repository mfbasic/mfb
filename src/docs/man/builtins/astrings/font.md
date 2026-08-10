# font

Construct a font-family `Attribute`.

## Synopsis

```
astrings::font(name AS String) AS Attribute
```

## Package

`astrings`

## Imports

```
IMPORT astrings
```

## Description

`font` returns an `Attribute` wrapping the `AttrText` with `kind` `AttrTypeText.Font` and `value`
`name`. Pass it to `astrings::addAttribute` to set the font family over a scalar range. Font is a
String-valued attribute: overlapping font spans resolve by higher-start-wins at read time.
[[src/builtins/astrings_package.mfb]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `name` | `String` | The font family name. |

## Return value

| Type | Description |
| --- | --- |
| `Attribute` | The font attribute (`AttrText[AttrTypeText.Font, name]`). |

## Errors

No errors.

## Examples

```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::font("Serif"))
END SUB
```

## See also

- `mfb man astrings fontSize`
- `mfb man astrings addAttribute`
- `mfb man astrings package`
