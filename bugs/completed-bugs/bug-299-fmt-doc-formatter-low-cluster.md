# bug-299: formatter/doc LOW cluster (`mfb fmt` skips TESTING/TGROUP/TCASE + mid-line DOC; `mfb doc` duplicate `intro` id)

Last updated: 2026-07-17
Effort: small (<1h across items)
Severity: LOW
Class: Correctness (cosmetic)

Status: Fixed
Regression Test: per-item (fmt/doc goldens)

Three LOW-severity, cosmetic formatter/doc-generator gaps found during goal-06.
Distinct root causes, one document per the repo's low-cluster convention. The
semantics-changing formatter bug (scalar-quote string rewrite) is filed separately
as bug-293.

References:

- `src/docs/spec/tooling/05_fmt.md`.
- Found during goal-06 review of `src/fmt.rs` and `src/doc.rs`.

## Items

### D1 — `mfb fmt` leaves TESTING/TGROUP/TCASE blocks unindented; `END` pops the block stack
- `src/fmt.rs:380-432` (`classify` — no arm for `Keyword::Testing`; `TGROUP`/`TCASE`
  are not keywords).
- `K::Testing` falls to `_ => None` so `TESTING` opens no block; `TGROUP`/`TCASE` are
  plain identifiers. Their `END X` lines still hit `K::End → Op::End(...)`, popping a
  never-pushed frame (harmless only because `Vec::pop` on empty is a no-op). Result:
  the formatter's core job — indentation — is skipped for the entire plan-18 test
  grammar (every line left at column 0).
- Fix: open a block for `TESTING`, and treat line-leading `TGROUP`/`TCASE`
  (word-based, like the LINK handler) as openers with their `END` forms as closers.
- Prior-work: new (fmt.rs was clean in goal-03, post plan-18).

### D2 — fmt/lexer disagreement on mid-line `DOC` (after `:`) recases prose and pops an unopened block
- `src/fmt.rs:490-501` (`is_doc_start`) vs `src/lexer.rs:892` (`is_statement_start`
  accepts `Colon`).
- The lexer starts a DOC capture when `DOC` follows a `:` statement separator
  mid-line; `is_doc_start` only recognizes a line whose first word is `DOC`. For
  `LET x = 1 : DOC`, fmt scans the following prose lines as code — uppercasing
  English words that spell keywords (`if`/`to`/`and`/`not`) and altering the
  documentation text spec 05_fmt.md says is preserved — and the `END DOC` line pops
  the enclosing block, mis-indenting the rest of the file.
- Fix: either restrict the lexer's DOC capture to line-leading position
  (spec-tightening), or make fmt track an "after `:`" DOC opener.
- Prior-work: new (LIKELY — reasoned from both recognizers, not run).

### D3 — `mfb doc` HTML emits a duplicate `id="intro"` when a declaration is named `intro`
- `src/doc.rs:126-145` (`anchor`) + `:601` (hardcoded `<section id="intro">`).
- `anchor()` de-duplicates only against other declaration anchors; the page-level
  `intro` id is hardcoded and not inserted into `used`, so `FUNC intro` slugifies to
  `intro` and produces a duplicate HTML id — its sidebar link scrolls to the page
  introduction instead of the declaration.
- Fix: seed `used` with `"intro"` before assigning declaration anchors (the bug-93.1
  fix pattern used in coverage.rs).
- Prior-work: new (doc.rs variant of bug-93.1's collision class).

## Goal

- `mfb fmt` indents test and mid-line-DOC blocks correctly and preserves their prose;
  `mfb doc` emits unique HTML ids.

### Non-goals (must NOT change)

- Formatting of non-test, non-DOC code.
- The doc page's introduction section id value.

## Blast Radius

Each item is a single site (cited). D1/D2 share the fmt block-stack model; fix
consistently.

## Fix Design / Phases

- [ ] Phase 1: fmt goldens for a TESTING file and a mid-line-DOC file; a doc golden
      for a `FUNC intro`.
- [ ] Phase 2: apply per-item fixes.
- [ ] Phase 3: regenerate fmt/doc goldens; confirm only intended deltas; full suite
      green.

## Validation Plan

- Regression: fmt idempotency/indent goldens; doc id-uniqueness golden.
- Doc sync: none.

## Summary

Three cosmetic formatter/doc gaps; each is a small classify/anchor change. No
semantic impact (the semantics-changing fmt bug is bug-293).

## Resolution

### D1 — superseded by bug-348

The `TESTING`/`TGROUP`/`TCASE` flattening was re-filed independently as
[[bug-348]] with a fuller treatment (MEDIUM rather than LOW, and with the
36-committed-sources evidence), and was fixed there: `Block` gained `Testing`,
`Tgroup` and `Tcase`, `classify` gained the `K::Testing` arm, and a new
`contextual_block_opener` recognizes the two contextual words that never scan as
keywords. The sibling `CSTRUCT`/`BIND IN` gap in `format_link_block` is
[[bug-356]]. All three share one root cause — the formatter's block model missing
constructs the language has — so they were fixed in one pass and committed
separately. Nothing was left for this item.

### D2 — the lexer was tightened, not the formatter

The report offered either restricting the lexer's DOC capture to line-leading, or
teaching fmt to track an "after `:`" opener. The deciding evidence is that the
**parser rejects a mid-line `DOC` outright** (`MFB_PARSE_UNEXPECTED_STATEMENT`),
which was confirmed by building one. So the lexer was capturing a verbatim block for
a construct that can never compile, and teaching fmt to format it would have been
teaching fmt about a shape no valid program contains.

`is_statement_start` also accepts a `:` separator; `DOC` now uses a stricter
`is_line_start`, which is the position the parser actually accepts and matches §21's
declaration-level placement. With that, the lexer and fmt agree — both treat a
`DOC` after `:` as an ordinary identifier — and the disagreement that caused the
prose recasing and the stray `END DOC` pop is gone at its root.

Verified that a line-leading `DOC` block's prose is still preserved verbatim
(`if`/`and`/`to` untouched), which is the behaviour that had to survive.

### D3 — seed the used-set

`PAGE_INTRO_ANCHOR` names the renderer-owned `intro` id, `reserved_anchors()`
returns a used-set with it already inserted, and **both** anchor-assignment sites
(the `.mfp` path and the AST path) use it — the fix is worthless if only one does.
The renderer's three literal `intro` occurrences now interpolate the constant, so
the id and its reservation cannot drift apart.

This is the bug-93.1 collision class and the same fix; the test pins that a
declaration named `intro` gets `intro-2`, that `Intro` collides too (case
slugifies onto the same base), and that an unrelated name is unaffected by the
seeding.

Full `cargo test` green; artifact gate 0 diffs.
