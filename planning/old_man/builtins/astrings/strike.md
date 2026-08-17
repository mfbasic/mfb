# strike

Construct the strikethrough flag `Attribute`.

## Synopsis

```
astrings::strike() AS Attribute
```

## Package

`astrings`

## Imports

```
IMPORT astrings
```

## Description

`strike` returns an `Attribute` wrapping the `AttrFlag` with `kind` `AttrTypeFlag.Strike`. Pass it to
`astrings::addAttribute` to mark a scalar range struck through. A flag attribute carries no value — a
scalar is struck when any covering span carries the strike flag. [[src/codegen/builtins/astrings/package.mfb]]

## Parameters

None.

## Return value

| Type | Description |
| --- | --- |
| `Attribute` | The strike flag attribute (`AttrFlag[AttrTypeFlag.Strike]`). |

## Errors

No errors.

## Examples

```
IMPORT astrings

SUB main()
  LET a AS AttributedString = astrings::fromString("hello")
  LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::strike())
END SUB
```

## See also

- `mfb man astrings addAttribute`
- `mfb man astrings package`
