# Attributed Strings (astrings)

The `astrings` package models styled (attributed) text: the opaque, value-semantic
`AttributedString` type, the open `Attribute` model, the mutation/query surface,
the attribute-aware `strings::` overloads, and the `toMarkdown` renderer. This
topic specifies the *semantic model* behind the package; the per-function API —
signatures, parameters, return types, errors — is owned by `./mfb man astrings`.

## The `AttributedString` value

`AttributedString` is an always-in-scope built-in (like `Error`), opaque and
value-semantic: it copies deeply, drops with its owning scope, defaults to empty
text with an empty overlay, and is **not** comparable (it wraps a list overlay, so
it is never a `Map` key or `Set` element). It exposes **no** user-visible fields —
`a.text` does not compile, `AttributedString[...]` cannot be constructed, and
`WITH a { }` cannot update it. The visible text is reached only through
`toString(a)` (or an explicit overload), never by implicit coercion.

Internally it pairs a visible `String` with an attribute overlay — a list of
stored spans. The overlay element is a codegen-internal flat record and is not
user-visible; the model the user sees is the `Attribute` union.

## The `Attribute` model

Imported with the package, the open model is three enums, three wrapper records,
and a union:

- `AttrTypeFlag` — `Bold`, `Italic`, `Underline`, `Strike`, `Overline`; a boolean
  flag with no value.
- `AttrTypeText` — `Font`; a String-valued attribute.
- `AttrTypeNumber` — `FontSize`, `Foreground`, `Background`; an Integer-valued
  attribute. `Foreground`/`Background` carry a packed `0xAARRGGBB` color — alpha in
  the high byte, `b` in the low one, which is exactly `color::toPacked`'s order, so
  `color::fromPacked` reads the value back as a whole `color::Color`.
- `AttrFlag { kind }`, `AttrText { kind, value }`, `AttrNumber { kind, value }`.
- `UNION Attribute` over the three wrappers.

Convenience constructors return an `Attribute`: `bold()`, `italic()`,
`underline()`, `strike()`, `overline()`, `font(name)`, `fontSize(size)`,
`foreground(base)`, `background(base)`. The color constructors take a single
`color::Color` and pack it into the numeric attribute's `0xAARRGGBB` value, so a
colour round-trips through an attribute exactly, **alpha included**. A program that
names a `color::Color` must `IMPORT color` as well as `astrings`: imports are not
transitive and a package cannot re-export another's types.

`term::drawText` renders the colours as truecolor foreground/background, and
`toMarkdown` (which has no color notation) ignores them entirely.

**The terminal has no alpha, and the bridge ignores it.** A half-transparent
foreground draws exactly the cells an opaque one draws: `term::drawText` reads only
the red, green and blue channels, so the emitted escape is byte-identical whatever
the alpha was. The alpha is kept in the attribute rather than discarded at
construction, so a renderer that *can* model transparency still receives the whole
colour; synthesizing a blend against the cell's current background would disagree
with what a canvas surface draws for the same colour, which is why the terminal
does not attempt one.

## Ranges and storage

All attribute ranges are **inclusive** scalar ranges `[start, endIndex]`: length is
`endIndex − start + 1`, `start == endIndex` is a single scalar, and there is no
empty-range form. Ranges are validated against the visible scalar count:
`start < 0 || endIndex < start` raises `ErrInvalidArgument` (`7-705-0002`);
`endIndex >=` the scalar count raises `ErrIndexOutOfRange` (`7-705-0001`). An
operation on an empty `AttributedString` therefore always raises.

`addAttribute` appends a span; attributes are **never merged or coalesced** on
write. `removeAttribute` and the ranged `clearAttributes` **split** any covered
span, keeping the flanks outside the removed range (`removeAttribute` matches spans
structurally — same member and value; ranged `clearAttributes` matches all). Whole
`clearAttributes(a)` empties the overlay.

## Read-time resolution

`getAttributes(a, index)` resolves the attributes active at a scalar: for each enum
member with any covering span, the covering span with the **highest start** wins
(ties break to the later insertion). The result carries at most one `Attribute` per
member — flags are present when any covering span carries them; font/size take the
winning span's value. Because resolution happens at read time and losers are never
trimmed on write, removing a covering winner can reveal a lower-start loser.

## Attribute-aware `strings::` overloads

`strings::` functions split by whether they interrogate or modify the text:

- **Tier-A (interrogators)** — `byteLen`, `contains`, `count`, `displayWidth`,
  `startsWith`/`endsWith`(`Any`), `find`, `graphemes`, `graphemesCount`,
  `graphemeAt`, `split`, `toBytes`, `toScalars` — accept an `AttributedString` and
  return exactly what the `String` overload returns (same value, type, and errors),
  computed on the visible text.
- **Tier-B (modifiers)** — `left`, `right`, `mid`, `trim`/`trimStart`/`trimEnd`/
  `trimChars`, `stripPrefix`/`stripSuffix`, `padLeft`/`padRight`, `repeat`,
  `replace` — accept an `AttributedString` and return an `AttributedString` whose
  text is transformed exactly as the `String` overload's (the text invariant
  `toString(t(a)) == strings::t(toString(a))`) and whose spans are remapped by the
  same edit: slice/trim clip-and-shift to the kept window, `padLeft` shifts,
  `repeat` replicates per copy, `replace` remaps piecewise (spans inside a match are
  dropped, straddlers clip/split, the inserted replacement is plain, and everything
  after each match shifts by the cumulative length delta). `upper`, `lower`,
  `caseFold`, and `normalizeNfc` transform the text but **drop** attributes — full
  case mapping and NFC change scalar counts within a span, so a 1:1 remap is
  impossible.

`AttributedString & AttributedString` concatenates: the text joins and both
overlays are kept, the right operand's spans shifted by the left's scalar length.
There is no mixing with `String`.

## `toMarkdown`

`toMarkdown(a)` flattens the resolved state across scalars into maximal runs and
renders each run into a bespoke marker vocabulary. **It is not CommonMark** — the
format is read by the `astrings` toolchain.

- **Flags** wrap each run as nested pairs in canonical (enum-declaration) order:
  `**bold**`, `*italic*`, `__underline__`, `~~strike~~`, `^^overline^^`, opened in
  order and closed in reverse, so overlapping spans always nest validly.
- **Font/size** switch forward via a minimal-delta `::font;size::` marker emitted
  at run boundaries where the state changes: a value sets, `-` resets to default,
  and an omitted slot is left unchanged (`::font::` font-only, `::;size::`
  size-only, `::-::` font reset).
- **Escaping**: `\ * _ ~ ^ :` in the visible text are backslash-escaped; font names
  additionally escape `;` and a literal `-`.

## See Also

* ./mfb man astrings — the per-function API reference
* ./mfb spec architecture frontend — how the `astrings` source companion is injected
