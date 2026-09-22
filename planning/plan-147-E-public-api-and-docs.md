# plan-147-E: Docs and the final gate

Last updated: 2026-09-21
Effort: medium (1h–2h)
Depends on: plan-147-D

Prerequisites: see plan-147-A. Additionally plan-147-D is archived
(`ls planning/completed/plan-147-D-*` → one file). If not, this letter cannot start,
full stop.

Documents the two public members (added in plan-147-B on top of `canvas::systemFaces`
and the `canvas::loadFont(path, face)` overload from A), rewrites the "no font
discovery" contract, and runs the feature's one full-suite gate. Checkable outcome: on this Mac,
`canvas::listSystemFonts()` contains `"Helvetica"` and is sorted with no duplicates, and
`canvas::loadSystemFont("Helvetica")` draws text; `loadSystemFont("No Such Font")`
fails `ErrNotFound` (77050004).

## 1. Goal

- `canvas::listSystemFonts() AS List OF String` — the `name` of every `systemFaces`
  entry, sorted (byte order, the `collections` sort), duplicates removed.
- `canvas::loadSystemFont(name AS String) AS canvas::Font` — among `systemFaces`
  entries with `name = name` (exact, case-sensitive), the first after sorting by
  `(path, postScript)` — deterministic when two files carry the same full name — is
  loaded with `canvas::loadFont(path, postScript)` (or `name` when the OS gave no
  PostScript name). None →
  `FAIL error(77050004, "no system font named: " & name)`. Errors: `ErrNotFound`,
  `ErrBadFontFile`, `ErrOutOfMemory`, and whatever `fs::readBytes` raises
  (`ErrPathNotFound` if the file vanished between listing and loading).
- Both are `Body::mfb` in a new `func_system_fonts.rs`; no new native code.
- No `Mode.Canvas` check (like `loadFont`).

### Non-goals

- No fuzzy/family matching, no fallback font, no default font.
- Text goldens keep using `loadFont` with fixture fonts; system fonts are never used in
  an exact-match golden.

## 2. Current State

- `loadFont`'s `DESC` (`func_load_font.rs`) ends: "There is no font *discovery*: `path`
  names a file … which is what makes text goldens exact-match." — replaced here.
- `src/docs/spec/app/06_canvas.md` has no font-discovery text
  (`grep -n -i 'discovery\|system font' src/docs/spec/app/06_canvas.md` → nothing).
- `ErrNotFound` = 77050004 (`src/codegen/errorcode/mod.rs`, md row) — no new code.

## Phases

> **NOTE — keep the checkboxes current as you go. An unticked box means NOT DONE.**

### Phase 1 — Members

- [x] ~~`func_system_fonts.rs`: both members, and their tests~~ — moot here: moved into
      plan-147-B (its Phase 1 tasks and Corrections), because the internal backend is
      unreachable from a test program without them.
- [ ] Man prose for `listSystemFonts` / `loadSystemFont` finished against
      `.ai/man-content.md`: the list depends on the machine; text in a system font can
      look different on another machine; `loadFont` is the reproducible path; no memory
      vocabulary.

Acceptance: Check: `scripts/man-census.sh --memory-scope` → 0 unclassified;
`mfb man canvas loadSystemFont` renders (est. 2 min).
Commit: —

### Phase 2 — Docs

- [ ] `func_load_font.rs` `DESC`: replace the "no font discovery" paragraph — `loadFont`
      is the reproducible path (same file, same pixels everywhere);
      `loadSystemFont` trades that for convenience.
- [ ] `06_canvas.md`: a *System fonts* section — `systemFaces` per OS (CoreText /
      fontconfig-dlopen / DirectWrite), the filters (glyf, not variable, not simulated,
      not a named instance), face resolution by PostScript name, empty list without
      fontconfig; `[[path:Symbol]]` provenance per `.ai/specifications.md`.
- [ ] Verify: `mfb man canvas listSystemFonts`, `mfb man canvas loadSystemFont`,
      `scripts/man-census.sh --memory-scope` → 0 unclassified,
      `scripts/man-run-examples.sh canvas --run` → examples compile and run.

Acceptance: Check: `cargo test --bin mfb spec` → pass; the man renders above show the
new pages (est. 5 min).
Commit: —

## Validation Plan

- Final gate (once, for all of plan-147): `cargo test` (full suite, est. per
  `.ai/testing-gates.md`) plus `cargo clippy --all-targets` → clean; C's and D's remote
  proofs recorded in their Corrections stand as the Linux/Windows runtime proof.
- Archive plan-147-A…E to `planning/completed/`.

## Open Decisions

- Name match exact and case-sensitive (recommended: the list is the vocabulary) vs.
  case-insensitive.

## Corrections

- The members themselves and their tests moved to plan-147-B (see B's Corrections);
  this letter keeps the prose, the spec, and the final gate.

## Summary

Thin MFBASIC layer over A–D plus the documentation turn from "no discovery" to "two
ways to get a font, one reproducible".
