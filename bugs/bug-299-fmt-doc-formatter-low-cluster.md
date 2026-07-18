# bug-299: formatter/doc LOW cluster (`mfb fmt` skips TESTING/TGROUP/TCASE + mid-line DOC; `mfb doc` duplicate `intro` id)

Last updated: 2026-07-17
Effort: small (<1h across items)
Severity: LOW
Class: Correctness (cosmetic)

Status: Open
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
