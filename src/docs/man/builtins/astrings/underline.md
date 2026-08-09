# underline

Construct the underline flag `Attribute`.

## Synopsis

```
astrings::underline() AS Attribute
```

## Package

`astrings`

## Imports

```
IMPORT astrings
```

## Description

`underline` returns an `Attribute` wrapping the `AttrFlag` with `kind` `AttrTypeFlag.Underline`. Pass
it to `astrings::addAttribute` to mark a scalar range underlined. A flag attribute carries no value —
a scalar is underlined when any covering span carries the underline flag.
[[src/builtins/astrings_package.mfb]]

## Parameters

None.

## Return value

| Type | Description |
| --- | --- |
| `Attribute` | The underline flag attribute (`AttrFlag[AttrTypeFlag.Underline]`). |

## Errors

No errors.

## Examples

```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::underline())
END SUB
```

## See also

- `mfb man astrings addAttribute`
- `mfb man astrings package`
