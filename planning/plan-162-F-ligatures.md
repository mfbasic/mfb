# plan-162-F: standard ligatures (`GSUB` `liga`)

Last updated: 2026-09-27
Effort: large (3h–1d)
Depends on: plan-162-E (prerequisites: see plan-162-A §Prerequisites)

This is the last letter of plan-162. Fonts with standard ligatures (fi, fl, ffi in most
text faces) draw them as separate glyphs today. This letter applies `GSUB` lookup type 4
(ligature substitution) under the `liga` feature in `__canvas_shapeRun`. It is the first
change where **one glyph can stand for several codepoints**, so the caret stops from
plan-162-D must split a ligature's advance among the graphemes it covers.

**The single behavioral outcome:** in a font with an `f,i → fi` ligature,
`"fi"` draws the single `fi` glyph. `measureText` uses its advance.
`caretStops` still reports a stop between `f` and `i`, placed by the font's `GDEF`
ligature caret if it has one, or at the evenly divided advance if not.

References:

- plan-162-A (face index versioning), plan-162-B (the `GPOS` walk; `GSUB` shares its
  script/feature/lookup-list structure and the Extension lookup), and plan-162-D (run
  columns `scalarIndex` and `gapAfter`; caret stops).
- OpenType `GSUB` lookup types 4 and 7 (Extension), and `GDEF` LigCaretList (caret
  value formats 1 and 2).

## 1. Goal

- Face index v4 carries the `liga` ligature sets.
- `__canvas_shapeRun` substitutes the longest matching ligature at each position, left
  to right, before kerning.
- `canvas::caretStops` places the interior carets of a ligature per `GDEF`, or evenly.
- Man pages state that standard ligatures are applied and that `letterSpacing <> 0.0`
  turns them off.

### Non-goals

- Only `liga`. No `clig`, `dlig`, `calt`, `rlig`, or contextual/chaining substitution
  (types 5, 6 and 8). Types 1–3 aren't applied.
- No ligatures across a font change or across a grapheme with `gapAfter`. With
  `letterSpacing <> 0.0` there are no ligatures at all (the CSS rule: spacing would
  split the glyph visually).
- No user-facing on/off switch in this letter (Open Decisions).
- No change for fonts without `liga`: byte-identical pixels.

## 2. Current State

- `__canvas_shapeRun` produces one glyph per codepoint, with the columns
  `fontIndex, glyphId, advanceUnits, scalarIndex, gapAfter` (plan-162-D).
- plan-162-B's `helper_kern.rs` walks `GPOS` ScriptList, FeatureList and LookupList.
  `GSUB`'s header is structurally identical up to the lookup level.
- Geometry consumers index glyphs by ordinal (`offset + __CANVAS_GEO_HEADER + c * 3` in
  `__canvas_geoRunReference`). **UNVERIFIED:** that no consumer assumes
  glyph count = codepoint count. Phase 1 task.

### Measured populations

| What | Count | Command |
|---|---|---|
| System fonts with a `GSUB` `liga` feature; ligature sets per font | UNMEASURED | Phase 1 probe |
| Fonts with a `GDEF` LigCaretList | UNMEASURED | same probe |

## 3. Design Overview

1. **Build.**
   - `helper_liga.rs` (new) reuses plan-162-B's feature/lookup walk with the table tag
     as a parameter. Refactor it to take the tag rather than copying it.
   - It flattens type-4 subtables into: first-glyph coverage runs → ligature sets →
     `(componentCount, components…, ligatureGid)`, in the font's order. The first
     match wins per the spec, which also makes it the longest match, because fonts
     order longer ligatures first.
   - `GDEF` LigCaretList formats 1 (coordinate) and 2 (contour point, treated as
     absent) are flattened to per-ligature caret coordinates.
2. **Apply.**
   - In `__canvas_shapeRun`, after glyph selection and before kerning: at each position
     whose font has `liga` and `letterSpacing = 0.0`, try the ligature sets for this
     glyph against the following same-font glyphs.
   - On a match, emit one glyph. Its `scalarIndex` is the first component's, and a new
     column, `componentCount`, records how many graphemes it covers. The ligature's
     `gapAfter` is taken from its last component.
3. **Carets.** `caretStops` splits a glyph with `componentCount > 1` at the `GDEF`
   caret coordinates, or at `advance × k / componentCount`.

**Gate:** behavior changes for fonts with `liga`.
- **Expected to change:** real-font text that contains ligatures.
- **Must not change:** PNG goldens, because the fixture font has no `GSUB`.

**Design uncertainty (first):** consumers assuming one glyph per codepoint.

**Correctness risk:** the caret split and the geometry verify reference with fewer
glyphs than codepoints.

Rejected alternatives:
- **A general `GSUB` engine.** Rejected: without complex-script support, the other
  lookup types have no in-scope consumer.
- **Ligatures on even with spacing.** Rejected: it produces visibly unspaced pairs
  inside spaced text.

## Phases

> **NOTE — keep the checkboxes current as you go** (rules as in plan-162-A).

### Phase 1 — measure and audit

- [ ] Probe the system fonts for `liga` and `GDEF` carets. Record the counts.
- [ ] Audit every consumer of the run and of `__CANVAS_GEO_TEXT` records for a
      glyph = codepoint assumption:
      `grep -rn "__CANVAS_RUN_\|__CANVAS_GEO_TEXT" src/codegen/builtins/canvas/`. Write a
      verdict per site here.

Acceptance: every site has a verdict and every UNMEASURED row has a number.
  Check: `grep -c "UNMEASURED\|UNVERIFIED" planning/plan-162-F-ligatures.md` → 0
  outside this list (est. 1 min).
Commit: —

### Phase 2 — `GSUB` build and fixture

- [ ] Refactor plan-162-B's walk to take a table tag. `GPOS` output must be unchanged
      (plan-162-B's tests pass).
- [ ] `helper_liga.rs`: type 4 and type 7, plus `GDEF` carets, in face index v4.
- [ ] Fixture: glyphs `f`, `i`, `l` and ligatures `fi`, `ffi`, with a `GDEF` caret on
      `ffi` only.
- [ ] RED tests: `measureText("ffi")` = `adv(ffi)`, `"fil"` = `adv(fi) + adv(l)`, and
      `"fi"` with `letterSpacing 1.0` = `adv(f) + adv(i) + 1.0`.

Acceptance: the fixture loads, the RED tests fail on the ligature term, and plan-162-B's
kerning tests still pass.
  Check: `cargo test --test rt_canvas_font kern liga` → kern passes, liga fails as
  predicted (est. 3 min).
Commit: —

### Phase 3 — apply, carets, docs

- [ ] `__canvas_shapeRun`: the substitution per §3.2, plus the `componentCount` column.
- [ ] `caretStops`: the interior split per §3.3.
- [ ] Tests:
  - the RED tests go GREEN;
  - the drawn `fi` shows one glyph's outline, checked by a pixel column unique to the
    ligature outline;
  - the caret stops for `"ffi"` sit at the `GDEF` coordinates, and for `"fi"` at the
    halves;
  - the last stop equals the width.
- [ ] Real-font oracle: the Phase 1 face compared with HarfBuzz
      (`hb-shape --features=liga,kern,-clig,-calt`), glyph IDs and summed advances for
      `"office fluffy"`, recorded with the command.
- [ ] Man: `measureText`, `Text` and `caretStops` (ligatures, and the spacing rule).
      `loadFont` DESC: the shaping sentence now lists pair kerning and standard
      ligatures, and still says complex scripts are unsupported. Check with
      `scripts/man-census.sh --memory-scope` → 0 unclassified.
- [ ] Spec `06_canvas.md`: the ligature rule, the `componentCount` column, and the caret
      split.

Acceptance: ligatures draw, measure and caret consistently. Unligated fonts are
byte-identical.
  Check: `cargo test --test rt_canvas_font --test rt_canvas_golden --test rt_canvas_geo_native`
  → pass and `git status --short tests/golden/canvas` → empty (est. 6 min).
Commit: —

## Validation Plan

- Tests: as listed, plus the HarfBuzz oracle.
- Runtime proof: `examples/emoji` caption text containing "fi" in a ligating system
  face. Take before/after screenshots.
- Doc sync: man and spec.
- Final gate for **all of plan-162**: `cargo test` (full), `scripts/artifact-gate.sh`,
  `scripts/check-generated.sh`, and `scripts/test-canvas-vulkan.sh` on the Vulkan box
  (est. 45 min).

## Open Decisions

- An on/off switch: none, with ligatures always on for fonts that have `liga`
  (recommended; the browser default) vs. a `Text` field (adds surface; a code editor
  would want it off). If added later, it is a boolean on the `Text` item and on
  `measureText`, not a font-list change.
- `GDEF` caret format 2 (contour point): treat as absent (recommended) vs. resolve the
  outline point.

## Corrections

(empty)

## Summary

The format work reuses letter B's walk. The real change is the one-glyph-many-codepoints
invariant break, which is why letter F is last, audits every consumer first, and ties
the caret split to the same run.
