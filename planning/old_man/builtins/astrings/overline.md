# overline

Construct the overline flag `Attribute`.

## Synopsis

```
astrings::overline() AS Attribute
```

## Package

`astrings`

## Imports

```
IMPORT astrings
```

## Description

`overline` returns an `Attribute` wrapping the `AttrFlag` with `kind` `AttrTypeFlag.Overline`. Pass it
to `astrings::addAttribute` to mark a scalar range overlined. A flag attribute carries no value — a
scalar is overlined when any covering span carries the overline flag.
[[src/codegen/builtins/astrings/package.mfb]]

## Parameters

None.

## Return value

| Type | Description |
| --- | --- |
| `Attribute` | The overline flag attribute (`AttrFlag[AttrTypeFlag.Overline]`). |

## Errors

No errors.

## Examples

```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::overline())
END SUB
```

## See also

- `mfb man astrings addAttribute`
- `mfb man astrings package`
