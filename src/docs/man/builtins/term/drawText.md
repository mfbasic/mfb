# drawText

Draw a string at a position without moving the cursor

## Synopsis

```
term::drawText(x AS Integer, y AS Integer, text AS String) AS Nothing
term::drawText(x AS Integer, y AS Integer, text AS AttributedString) AS Nothing
```

## Package

term

## Imports

```
IMPORT term
```

`term` is a built-in package, so no manifest dependency is required.
[[src/builtins/term.rs:is_term_call]]

## Description

`term::drawText` stamps `text` onto the surface on row `y` starting at column `x`,
one grid cell per Unicode scalar, using the colours and attributes currently in
effect. Coordinates are **zero-based** from the top-left (`x` is the column, `y`
the row). Unlike `io::print`/`io::write`, it **does not move the shadow cursor**,
so it is the tool for placing a label, status line, or field value at a fixed
position without disturbing cursor-relative output.
[[src/target/shared/code/term.rs:emit_draw_text]]

The text is drawn on a **single row**: it does not wrap and does not scroll.
Characters that fall past the right edge are **clipped**, and columns before 0
(when `x` is negative) are skipped, so only the on-screen part is drawn. If `y` is
outside `0 .. rows-1` the call draws nothing. Control characters (below U+0020,
including newline and tab) are **skipped** — they advance one column but stamp
nothing — so a stray control character can never corrupt the presented frame; use
`io::write` for flowing text with newline handling. The run is shown on the next
`term::sync`. [[src/target/shared/code/term.rs:emit_draw_text]]

The call is gated: while TUI mode is off it does nothing and reports no error.
[[src/target/shared/code/term.rs:emit_gate_inactive]]

An overload accepts an `astrings::AttributedString` in the `text` position. It
stamps the same visible text as the `String` overload but honours the per-scalar
styling the value carries: the two attributes the terminal surface can represent —
**bold** and **underline** — are applied per run, and every other attribute
(italic, strikethrough, overline, font, font size) is silently ignored. The text
is drawn in maximal runs of a single (bold, underline) state, so each run renders
with those attributes and grapheme-cluster and wide-glyph handling is identical to
the `String` overload. The surface's current bold/underline are restored
afterwards, so like the `String` overload the call leaves the pen it found. Using
this overload requires `IMPORT astrings` (the only way to build an
`AttributedString`). [[src/builtins/term_astrings_bridge.mfb:__term_drawTextAttr]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `x` | `Integer` | Zero-based start column. Negative columns are skipped; the run clips at the right edge. [[src/builtins/term.rs:call_param_names]] |
| `y` | `Integer` | Zero-based row. Outside `0 .. rows-1` the call draws nothing. [[src/builtins/term.rs:call_param_names]] |
| `text` | `String` \| `AttributedString` | The text to stamp, one cell per Unicode scalar. Control characters are skipped. An `AttributedString` additionally applies its per-scalar bold/underline (other attributes ignored). [[src/builtins/term.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns nothing. The call is made for its side effect of stamping the text into the back buffer. [[src/builtins/term.rs:TERM]] |

## Errors

No errors.

## Examples

Draw a title and a status line at fixed positions:

```
IMPORT term

SUB main()
  term::on()
  term::drawText(2, 0, "My Application")
  LET size AS TermSize = term::terminalSize()
  term::drawText(0, size.rows - 1, "Press q to quit")
  term::sync()
  term::off()
END SUB
```

Draw styled text, applying its bold/underline attributes:

```
IMPORT term
IMPORT astrings

SUB main()
  term::on()
  MUT label AS AttributedString = astrings::fromString("Save  Quit")
  label = astrings::addAttribute(label, 0, 3, astrings::bold())
  label = astrings::addAttribute(label, 6, 9, astrings::underline())
  term::drawText(2, 0, label)
  term::sync()
  term::off()
END SUB
```

## See also

- `mfb man term drawGlyph`
- `mfb man term moveTo`
- `mfb man term setForeground`
- `mfb man term sync`
