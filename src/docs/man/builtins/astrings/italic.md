# italic

Construct the italic flag `Attribute`.

## Synopsis

```
astrings::italic() AS Attribute
```

## Package

`astrings`

## Imports

```
IMPORT astrings
```

## Description

`italic` returns an `Attribute` wrapping the `AttrFlag` with `kind` `AttrTypeFlag.Italic`. Pass it to
`astrings::addAttribute` to mark a scalar range italic. A flag attribute carries no value — a scalar
is italic when any covering span carries the italic flag. [[src/builtins/astrings_package.mfb]]

## Parameters

None.

## Return value

| Type | Description |
| --- | --- |
| `Attribute` | The italic flag attribute (`AttrFlag[AttrTypeFlag.Italic]`). |

## Errors

No errors.

## Examples

```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::italic())
END SUB
```

## See also

- `mfb man astrings addAttribute`
- `mfb man astrings package`
