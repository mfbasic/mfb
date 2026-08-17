# bold

Construct the bold flag `Attribute`.

## Synopsis

```
astrings::bold() AS Attribute
```

## Package

`astrings`

## Imports

```
IMPORT astrings
```

## Description

`bold` returns an `Attribute` wrapping the `AttrFlag` with `kind` `AttrTypeFlag.Bold`. Pass it to
`astrings::addAttribute` to mark a scalar range bold. A flag attribute carries no value — a scalar is
bold when any covering span carries the bold flag. [[src/codegen/builtins/astrings/package.mfb]]

## Parameters

None.

## Return value

| Type | Description |
| --- | --- |
| `Attribute` | The bold flag attribute (`AttrFlag[AttrTypeFlag.Bold]`). |

## Errors

No errors.

## Examples

```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::bold())
END SUB
```

## See also

- `mfb man astrings addAttribute`
- `mfb man astrings package`
