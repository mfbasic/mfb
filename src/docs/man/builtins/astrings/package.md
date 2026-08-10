# astrings

Attributed (styled) text: an opaque, value-semantic `AttributedString`

## Synopsis

```
IMPORT astrings
LET a AS AttributedString = astrings::fromString("hello")
LET text AS String = toString(a)
```

## Imports

The `AttributedString` type is always in scope (like `Error`) — a binding,
parameter, or return of that type needs no `IMPORT`. Building or operating on an
`AttributedString` through the `astrings::` functions requires `IMPORT astrings`.

## Description

The `astrings` package works with `AttributedString`, an opaque built-in that
pairs visible `String` text with an attribute overlay describing per-range style
(bold, italic, font, size, foreground/background color, …). The type is
**value-semantic**: it copies deeply,
drops with its owning scope, and defaults to empty text with no attributes. It is
**opaque** — it exposes no user-visible fields (`a.text` does not compile), cannot
be built with a record literal (`AttributedString[...]`), and cannot be
`WITH`-updated. It is copyable and defaultable but **not** comparable, so it is
never a `Map` key or `Set` element. [[src/builtins/astrings.rs:ASTRINGS]]

Reach the visible text with `toString(a)`; `io::print`/`io::write` emit it. The
text is never reached by an implicit coercion — only through `toString` or an
explicit overload. [[src/target/shared/code/builder_strings.rs:lower_to_string]]

`astrings::fromString(text)` constructs an `AttributedString` whose visible text
is `text` and whose attribute overlay is empty.
[[src/builtins/astrings.rs:is_astrings_call]]

## Errors

The `astrings` members in this release raise no runtime errors.
