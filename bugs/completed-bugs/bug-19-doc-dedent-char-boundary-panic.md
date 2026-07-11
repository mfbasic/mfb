# bug-19: DOC EXAMPLE `dedent` panics on a non-char-boundary string slice when indentation mixes multibyte whitespace

Last updated: 2026-07-08
Effort: small (<1h)

`src/ast/items.rs::dedent` (`:1302-1322`) strips the common leading indentation
from a DOC block's `EXAMPLE` body. It measures each line's indentation as a
**byte** count — `l.len() - l.trim_start().len()` (`:1306`) — takes the minimum
across lines as `min_indent`, then slices `l[min_indent..]` (`:1313`). `trim_start`
is Unicode-whitespace-aware (it strips U+00A0 NBSP, U+2003 EM SPACE, etc.), so when
different EXAMPLE lines are indented with whitespace characters of **different byte
widths**, `min_indent` can be a byte offset that falls **inside** a multibyte
leading-whitespace char on a more-deeply-indented line. `l[min_indent..]` then
panics with *"byte index N is not a char boundary"*, crashing `mfb build` /
`mfb test` / doc tooling instead of emitting a diagnostic.

The single correct behavior a fix produces: `dedent` strips the common leading
indentation without ever panicking, for any mixture of whitespace characters —
worst case it falls back to trimming that line.

Severity MEDIUM: a reachable compiler panic (SIGABRT) on ordinary user-authored
source; it aborts the whole compile/test/doc pass with a Rust backtrace rather
than a language-level error.

References:

- `src/ast/items.rs:1302-1322` (`dedent`) — `:1306` (byte-count indentation via
  Unicode `trim_start`), `:1313` (`l[min_indent..]` byte slice).
- Reached from `parse_doc_block` (`items.rs:1202`) on every parse of a DOC block
  with an EXAMPLE.
- Raw EXAMPLE lines preserve leading whitespace verbatim from the lexer
  (`src/lexer.rs`, `DocRawLine` capture ~`:790-820`).
- Found during goal-01 review of `src/ast/**`.

## Failing Reproduction

```
DOC
FUNC foo
EXAMPLE
 a
<NBSP><NBSP>b
END EXAMPLE
END DOC
```

where each `<NBSP>` is U+00A0 (2 bytes). `min_indent = min(1 [" a"], 4 [two NBSP])
= 1`. For the NBSP line `"\u{00A0}\u{00A0}b"`, `l.len() (5) >= min_indent (1)` is
true, so `l[1..]` is evaluated — byte index 1 is inside the first NBSP char (bytes
0–1) → panic.

- Observed: `thread 'main' panicked: byte index 1 is not a char boundary` — the
  compiler aborts.
- Expected: a clean de-indent (or, at worst, that line trimmed), no panic.

Contrast cases correct today:

- All-ASCII indentation (spaces/tabs): every byte offset is a char boundary, so
  `l[min_indent..]` never splits a char.
- A single-line or uniformly-indented EXAMPLE.
- Sibling slicers (`split_first_word`, `parse_header_signature`) slice only at
  ASCII delimiters and are unaffected.

## Root Cause

`dedent` conflates a **byte** offset with a **character** prefix length. `min_indent`
is a minimum of per-line leading-whitespace *byte* counts, but it is used to slice
every line, assuming it is a char boundary on all of them. That holds only when
every line's leading whitespace is byte-for-byte the same width; mixed-width
Unicode whitespace breaks it.

## Goal

- `dedent` strips the common indentation measured in a boundary-safe way and never
  panics for any whitespace mixture.

### Non-goals (must NOT change)

- The de-indent result for all-ASCII EXAMPLE bodies (the overwhelmingly common
  case) must be byte-identical.

## Blast Radius

- `dedent` only — the sole DOC EXAMPLE de-indenter. Other `items.rs` slicers use
  ASCII delimiters.

## Fix Design

Measure and strip indentation in **characters**, not bytes: compute `min_indent`
as the minimum count of leading whitespace *chars*, and strip that many chars via
`char_indices`/`chars()` rather than a byte slice. Minimal alternative: guard the
slice with `l.is_char_boundary(min_indent)` and fall back to `l.trim_start()` when
it is not (preserves current ASCII behavior exactly, fixes the panic). Recommended:
the char-count approach, since byte-minimum across mixed whitespace is not even the
semantically intended "common indentation."

## Phases

### Phase 1 — failing test + audit

- [ ] Add a `dedent` unit test with mixed ASCII + NBSP indentation asserting no
      panic and a sensible result. Confirm it panics today.
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Make `dedent` boundary-safe (char-count strip or `is_char_boundary` guard).

### Phase 3 — validation

- [ ] `scripts/test-accept.sh` — DOC/EXAMPLE goldens for ASCII-indented examples
      must be byte-identical.

## Validation Plan

- Regression test(s): the mixed-whitespace `dedent` test + build the reproduction
  and confirm a clean result instead of a panic.
- Doc sync: none (behavior for valid ASCII examples unchanged).
- Full suite: `scripts/test-accept.sh`.

## Summary

A byte-vs-char confusion in the DOC EXAMPLE de-indenter turns mixed-width
whitespace into a compiler panic; the fix is to strip a char prefix (or guard the
byte slice), keeping ASCII examples byte-identical.
