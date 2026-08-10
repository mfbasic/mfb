# toMarkdown

Render an `AttributedString` into a bespoke markdown-flavored format.

## Synopsis

```
astrings::toMarkdown(value AS AttributedString) AS String
```

## Package

`astrings`

## Imports

```
IMPORT astrings
```

## Description

`toMarkdown` flattens the resolved (higher-start-wins per member) attribute state
across the scalars into maximal runs and renders each run into a bespoke marker
vocabulary. It is a read-only projection — `value` is not modified.

**This is not CommonMark.** The format is read by the `astrings` toolchain, not a
standard markdown engine; do not treat `__`/`^^`/`::…::` as CommonMark.
[[src/builtins/astrings_package.mfb]]

- **Flags** wrap each run as nested pairs in canonical (enum-declaration) order —
  `**bold**`, `*italic*`, `__underline__`, `~~strike~~`, `^^overline^^` — opened in
  order and closed in reverse, so overlapping spans always produce valid nesting.
- **Font/size** switch forward via a minimal-delta `::font;size::` marker emitted
  at run boundaries where the state changes: a value sets, `-` resets to default,
  and an omitted slot leaves it unchanged (`::font::` font-only, `::;size::`
  size-only, `::-::` font reset).
- **Delimiter characters** (`\ * _ ~ ^ :`) in the visible text are backslash-
  escaped; font names additionally escape `;` (and a literal `-`).

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `AttributedString` | The attributed string to render. |

## Return value

| Type | Description |
| --- | --- |
| `String` | The rendered markdown-flavored text. |

## Errors

No errors.

## Examples

```
IMPORT astrings
IMPORT io

SUB main()
  MUT a AS AttributedString = astrings::fromString("hello world")
  a = astrings::addAttribute(a, 0, 4, astrings::bold())
  io::print(astrings::toMarkdown(a))
END SUB
```

## See also

- `mfb man astrings getAttributes`
- `mfb man astrings package`
