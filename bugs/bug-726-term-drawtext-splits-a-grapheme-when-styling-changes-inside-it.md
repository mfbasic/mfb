# bug-726: `term::drawText`'s `AttributedString` overload splits a grapheme when styling changes inside it

Last updated: 2026-09-19
Effort: medium
Severity: MEDIUM
Class: Correctness (text renders differently depending on styling)

Status: Open
Regression Test: none yet — see Phase 1

`term::drawText(row, column, value AS AttributedString)` draws the value one styled
run at a time. A run boundary can fall *inside* a grapheme cluster, and when it
does, the parts are stamped into separate cells: the base character lands in one
cell and its combining mark in the next, so the terminal shows two things where the
`String` overload shows one.

Probe `/tmp/p125-ex/termattr`, captured from a real pty. The text is the decomposed
`e` + U+0301 (one grapheme, two scalars), with bold applied to scalar 1 only:

```
...m e \x1b[0m\x1b[1m\x1b[38;2;255;255;255m\x1b[48;2;0;0;0m ́ ...
```

The base `e` is emitted, then the attribute switch, then the combining acute — the
two are never adjacent bytes (`b"e\xcc\x81" in output` is `False`). A `|` drawn at
column 4 shows the cluster consumed two columns rather than one. Drawing the same
text as a plain `String` renders `é` in a single cell.

`astrings::toMarkdown` on the same value reports `e**́**`, so the styling itself is
recorded exactly as asked; only the terminal rendering splits.

**The single correct behavior a fix produces:** a grapheme cluster occupies the same
cells whether or not styling changes inside it. Either the run splitter refuses to
break a cluster (the whole cluster takes one run's style — the first covering span's,
say), or the cluster is stamped as a unit and the mid-cluster style change is
dropped. Whichever is chosen, the page states it, and no styled value renders text
the plain overload would not.

Found by plan-125-D Phase 3 while triaging the `term::drawText` review.
**Filed, not fixed**, by user instruction during a documentation-only plan ("file
all bugs, make no fixes").

## Reproduction

```basic
IMPORT term
IMPORT astrings
IMPORT io

SUB main()
  LET plain AS String = "é"          ' decomposed: "e" + U+0301
  LET a AS AttributedString = astrings::fromString(plain)
  LET styled AS AttributedString = astrings::addAttribute(a, 1, 1, astrings::bold())
  term::on()
  term::clear()
  term::drawText(0, 0, styled)
  term::drawText(0, 4, "|")
  term::sync()
  term::off()
  io::print("markdown=" & astrings::toMarkdown(styled))
END SUB
```

Run on a pty (an ordinary pipe raises `7-705-0007` before drawing). Observed: `e`,
an attribute switch, then the combining mark in the next cell. Expected: one cell
holding `é`.

## Root cause

`src/codegen/builtins/term/helper_astrings_bridge.rs:__term_drawTextAttr` walks the
value scalar by scalar, cuts a new run wherever the resolved style changes, and draws
each run with `strings::mid`, advancing the column by that run's
`strings::displayWidth`. The walk knows about scalars and display width but not about
grapheme boundaries, so a style boundary inside a cluster becomes a cell boundary.

## Non-goals

- Changing the `String` overload, which already renders clusters correctly.
- Changing how `astrings` records or resolves attributes (the overlay is right; see
  the `toMarkdown` output above).
- Wide-glyph (`WIDE_TRAIL`) handling in `term::drawGlyph`, which is bug-725's area.

## Blast-radius audit

- `astrings::toMarkdown`: same per-run splitting, but markers are text, so confirm
  whether a mid-cluster split changes what it emits.
- The macOS and Linux GTK app term backends: they receive the already-split runs, so
  check whether they show the same break.
- `strings::displayWidth` on a lone combining scalar: the bridge's column advance
  depends on what it reports for a run that is only a mark.

## Fix

Phase 1 — RED test drawing the decomposed cluster styled and unstyled, asserting both
occupy the same cells. Commit:

Phase 2 — make the run splitter respect grapheme boundaries (or stamp clusters
whole); re-render the `term` pages; full suite and term runtime goldens. Commit:
