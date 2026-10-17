# plan-162-D: letter-spacing, decoration metrics, and caret stops

Last updated: 2026-09-27
Effort: large (3h–1d)
Depends on: plan-162-C (prerequisites: see plan-162-A §Prerequisites)

A layout engine built on the canvas needs three things the font list (plan-162-C)
still lacks:
- **letter-spacing**: extra space between user-perceived characters;
- **decoration metrics**: where the font's designer put the underline and the
  strikeout, and how thick they are, so an engine can draw them with `canvas::Line`;
- **caret stops**: the x offset of every grapheme boundary, which is all that caret
  drawing, click-to-index hit-testing and selection highlighting need.

All three come from the same shaped run, so they agree with drawing by construction.

**The single behavioral outcome:**
- For any text, font list, size and spacing, `canvas::caretStops(...)` returns one
  stop per grapheme boundary including both ends. Its last `x` equals
  `measureText(...).width` for the same arguments.
- Each stop's `x` is where the drawn glyph of the grapheme starting there begins
  (its pen position).

References:

- plan-162-A (face index versioning), plan-162-B (kerning in `__canvas_shapeRun`), and
  plan-162-C (the list overload, `Text.fonts`, list metrics).
- `strings::graphemes` (`src/codegen/builtins/strings/gen_graphemes.rs`). Grapheme
  segmentation already exists and follows utf8proc's properties
  (`src/docs/spec/unicode/01_tables-and-algorithms.md`).
- `mfb man strings`: positions are **zero-based Unicode scalar indices** for
  `find`/`mid`/`left`/`right`. Caret stop indices use the same unit, so they feed
  straight into `strings::mid`.
- OpenType `post` (`underlinePosition`, `underlineThickness`) and `OS/2`
  (`yStrikeoutSize`, `yStrikeoutPosition`).

## 1. Goal

- `canvas::Text` gains `letterSpacing AS Float` (pixels). Its zero value adds nothing, so
  every existing item is unchanged.
- `measureText` (both overloads) gains a trailing `letterSpacing AS Float` parameter with
  default `0.0`.
- `canvas::TextMetrics` gains `underlineOffset`, `underlineThickness`,
  `strikeoutOffset` and `strikeoutThickness` (pixels, with offsets measured downward
  from the baseline).
- `canvas::caretStops(font | fonts, size, text, letterSpacing = 0.0) AS List OF canvas::CaretStop`
  exists, where `CaretStop` is `{index AS Integer, x AS Float}`.

### Non-goals

- No word-spacing, no justification, and no tab stops. A layout engine does those by
  splitting runs.
- No drawn decorations. `canvas::Text` gains no underline field. The engine draws a
  `canvas::Line` from the metrics.
- No bidirectional caret logic. Stops are in logical order along a left-to-right pen,
  and RTL is a non-goal of plan-162 (complex scripts, plan-162-C).
- No change for `letterSpacing = 0.0`: pixels are byte-identical.

## 2. Current State

- `__canvas_shapeRun` returns `(fontIndex, glyphId, advanceUnits)` triples, one per
  codepoint (bug-756, with kerning from plan-162-B). It has no grapheme information.
- `canvas::TextMetrics` has 5 fields: width, height, ascent, descent, lineGap
  (`mfb man canvas types`).
- The four pen-walking callers (bug-756 blast radius) each add
  `toFloat(adv) * scale` per glyph.
- Neither `post` nor `OS/2` is read anywhere:
  `grep -rn '"post"\|"OS/2"' src/codegen/builtins/canvas/` → no matches (2026-09-27).

### Measured populations

| What | Count | Command |
|---|---|---|
| System fonts missing `post` / `OS/2` strikeout fields | Of 638 TrueType faces: every one has `post`, 632 have `OS/2` (6 do not). A zero `post.underlineThickness`: 12; a zero `OS/2.yStrikeoutSize`: 3. `OS/2` versions: 76 × v1, 55 × v2, 286 × v3, 215 × v4 (`yStrikeoutSize`/`yStrikeoutPosition` sit at +26/+28 in every version). Medians as a fraction of the em: underline position −0.100, thickness 0.050; strikeout position +0.262, size 0.050 | `python3 -I /tmp/p162/probe.py` over the four font directories, summarised from `/tmp/p162/probe.json` (2026-10-14) |
| `TextMetrics[` literal constructions that name every field | 2, both in `func_measure_text.rs`'s own body | `grep -rn "TextMetrics\[" src tests examples \| wc -l` → 2 |

## 3. Design Overview

1. **Grapheme-aware shaping.**
   - `__canvas_shapeRun` gains a **fourth column**, `scalarIndex`: the zero-based scalar
     index of the codepoint the glyph came from.
   - It also gains a **fifth column**, `gapAfter` (0 or 1): 1 on the last glyph of each
     grapheme except the final one.
   - Graphemes come from `strings::graphemes(text)`, walked alongside the codepoints.
   - The stride becomes 5. Callers read columns by name-offset constants, so no caller
     hard-codes positions.
2. **Spacing in one place.** Each caller's pen step becomes
   `toFloat(adv) * scale + toFloat(gapAfter) * letterSpacing`. The *decision* of where
   gaps go stays in `__canvas_shapeRun`. `measureText` adds
   `gaps × letterSpacing` to its per-font scaled sum.
3. **Decoration metrics.** Face index v3 appends four words, read at load:
   - `post.underlinePosition` and `post.underlineThickness`;
   - `OS/2.yStrikeoutPosition` and `OS/2.yStrikeoutSize`.

   A missing table, or a zero thickness, is stored as the flag value −32768, and the
   reader substitutes a documented default (Open Decisions). For a list, the metrics come
   from fonts[0], because a single decoration line needs one position.
4. **Caret stops.**
   - Built from the shaped run: one stop at index 0, x 0.0; then one per grapheme
     start, at the pen position before its first glyph; then a final stop at
     `(scalarCount, width)`.
   - `x` is relative to the run start (the `Text`'s `x`), so the program adds its own
     origin.
   - Hit-testing is a binary search over `x`. The man page example shows it. No
     separate `caretAt` member is added (Open Decisions).

**Gate:** additive public API. Correctness comes from rt tests.
- **Expected to change:** `app-mouse-surface` sentinels.
- **Must not change:** PNG goldens, since `letterSpacing` defaults to 0.

**Design uncertainty (first):** whether a codepoint walk and `strings::graphemes` agree
on scalar counts for every input, including unpaired edge cases. The Phase 2 property
test settles it.

**Correctness risk (last):** the stride change to 5 across the four callers and the
geometry verify reference.

Rejected alternatives:
- **Spacing folded into `advanceUnits`.** Rejected: spacing is in pixels, and converting
  to font units per font loses exactness, which would break the byte-identity of
  `letterSpacing = 0`.
- **Spacing after every grapheme including the last.** Rejected as the default. It
  inflates `width` and leaves a trailing gap in centred text (Open Decisions).
- **`canvas::caretAt(stops, x)`.** Rejected for now: it is a four-line binary search
  over a public list, better shown in the example than added as a member.

## Phases

> **NOTE — keep the checkboxes current as you go** (rules as in plan-162-A).

### Phase 1 — measure

- [x] `/tmp` probe: per system face, record whether `post` and `OS/2` exist, the
      `OS/2` version, and whether the thickness fields are zero. Record the counts.
      The shared plan-162 probe; counts and medians in §2.
- [x] Census of `TextMetrics[` literals, since new fields are additive and named
      construction keeps them compiling. Record the count with its command. 2, both
      in the `measureText` body itself.

Acceptance: §2 rows are measured.
  Check: `grep -c UNMEASURED planning/plan-162-D-spacing-decoration-carets.md` → 0
  outside this list (est. 1 min).
  Result: → 1, the line above.
Commit: 1a87d0917

### Phase 2 — the five-column run

- [x] `helper_font.rs:__canvas_shapeRun`: add the `scalarIndex` and `gapAfter` columns
      from `strings::graphemes`, and define the `__CANVAS_RUN_*` column constants and
      the stride. canvas now imports `strings` (Correction 1).
- [x] Migrate the four callers to the constants. There is no spacing yet, so
      `gapAfter` is ignored. Every reader of a shaped run: the three geometry builders,
      the run helpers (`__canvas_runDrawable`/`runLine`/`runPen`), the hit and particle
      boxes, `__canvas_kernRun`, and `__canvas_measureHandles` (Correction 2).
- [x] Tests: a property test over strings mixing combining marks, ZWJ emoji sequences,
      CRLF and flags. The number of glyphs with `gapAfter = 1` must be
      `graphemesCount(s) − 1`, and `scalarIndex` must be non-decreasing and end at
      `len(toScalars(s)) − 1`.
      `shape_run_marks_every_grapheme_boundary_and_scalar` → ok, over combining marks, a
      ZWJ family, CRLF, two flags, Hangul jamo, a skin-tone emoji, a Devanagari conjunct,
      a leading combining mark and the empty string. The run is read through the new
      `--debug` `MFB_CANVAS_SHAPE_DUMP` (spec `tooling/09_debug-report.md`).

Acceptance: identical pixels, and the property test passes.
  Check: `cargo test --test rt_canvas_font shape_run --test rt_canvas_golden` → pass and
  `git status --short tests/golden/canvas` → empty (est. 5 min).
  Result: `shape_run` → 1 passed; `rt_canvas_golden` → 28 passed;
  `git status --short tests/golden/canvas` → empty. A full `rt_canvas_font` run made
  alongside a separate 210 s test run had 3 of its 30-second-watchdog tests
  (`a_cmap_group_count_*`, `a_glyph_whose_bitmap_would_exceed_the_raster_cap_is_skipped`)
  time out. Run alone, the three pass in 10.8 s together; they are wall-clock bounded,
  and the box was loaded (Correction 3).
Commit: 78cef082d

### Phase 3 — letter-spacing

- [x] `mod.rs`: add a `Text.letterSpacing` field, described in pixels, applied between
      characters, and not after the last one. A record literal must name every field,
      so all 78 `canvas::Text[` literals gained `letterSpacing := 0.0` (Correction 4).
- [x] `func_measure_text.rs`: add a defaulted `letterSpacing` parameter to both
      implementations. `DefaultValue::Fill { Float, "0.0" }`; the width adds
      `gaps × spacing` only when the spacing is non-zero.
- [x] The callers' pen steps and the `Text` hash arm fold in `letterSpacing`.
      Forgetting the hash is the geometry-cache stale-glyph trap, so a test changes
      only the spacing between two presents. Glyph run, edges, verify reference and
      `__canvas_runPen` (the hit, particle and AutoPaint boxes) add it after a
      `gapAfter` glyph when it is non-zero. The hash folds it only when non-zero, so an
      unspaced `Text` keeps its old key.
- [x] Tests: the width grows by `(graphemes − 1) × spacing`, the drawn second-grapheme
      column moves by `spacing`, negative spacing works, and a spacing-only change
      re-renders. `measure_text_spacing_adds_between_graphemes_only` (`AVA` 800 → 820 at
      +10, 792 at −4 over a list, `A`+U+0301 750 either way),
      `letter_spacing_moves_the_next_character_by_the_spacing` (fill and stroke, +7 and
      −5 px), `a_spacing_only_change_redraws` → all ok.

Acceptance: measured and drawn spacing agree.
  Check: `cargo test --test rt_canvas_font spacing` → pass (est. 3 min).
  Result: 3 passed, 0 failed.
Commit: d15df3399

### Phase 4 — decoration metrics

- [x] Face index v3: four words (plan-162-A §4.1 versioning). Header words 10–13
      (`__canvas_faceLine`); the header is 14 words. The plan-A index tests moved to v3,
      as their own comment predicts.
- [x] `TextMetrics`: four new fields in `mod.rs`, computed in the `measureText` body
      from fonts[0], with defaults per Open Decisions when flagged. Offsets run down
      from the baseline to the line's **top edge**, as OpenType defines both positions
      (Correction 5). Defaults, locked to §2's measured medians: underline 0.10 em below
      at 0.05 em, strikeout 0.26 em above (not the 0.3 the Open Decision guessed) at the
      underline's thickness.
- [x] Tests: a fixture with `post`/`OS/2` values gives exact pixels, and a fixture with
      neither gives the documented defaults.
      `decoration_metrics_come_from_post_and_os2_or_the_defaults` → ok: `150 60 -280 45`
      from the tables, `100 50 -260 50` from none, the same for zero thicknesses, and
      `120 70 -260 70` with an underline but no strikeout.

Acceptance: metrics match the fixture tables.
  Check: `cargo test --test rt_canvas_font decoration` → pass (est. 3 min).
  Result: `decoration` 1 passed. With it, the v3-dependent `face_index_of_*`,
  `face_kern_reads_each_fixtures_pairs` and the exact live-bytes test (now 23 index
  words) passed: 5 passed, 0 failed.
Commit: d4ce4670b

### Phase 5 — caret stops, docs

- [x] New `func_caret_stops.rs`: `CaretStop` record, and two implementations (a single
      font, and a list), sharing one MFBASIC body over `__canvas_shapeRun`.
      `__canvas_caretStopsHandles`; the last stop takes `__canvas_measureHandles`'s width.
      Listed in each backend's `RUNTIME_CALLS` beside `measureText`.
- [x] Tests:
  - the last stop's `x` equals `measureText(...).width` for 50 generated strings,
    including kerned pairs and spacing;
  - stop indices equal the grapheme start scalar indices;
  - the empty string gives one stop `(0, 0.0)`;
  - a closed font raises `ErrResourceClosed`.

  `caret_stops_agree_with_measuring_and_with_graphemes`: 50 generated strings over
  `A`, `V`, space and `A`+U+0301 in the kerning fixture, with spacings 0, 3.5 and −1.25.
  Every one prints `TRUE TRUE` (last `x` = `measureText` width; indices = grapheme
  starts). `caret_stops_place_kerned_characters_and_refuse_closed_fonts`: `AV` →
  `0:0 1:130 2:430`, with spacing 10 over a list `0:0 1:140 2:440`, `""` → `0:0`,
  `empty: 77050002`, `closed: 77030004`. Both ok.
- [x] Man pages: `caretStops` (with a hit-test example and a caret-drawing example),
      `measureText` (`letterSpacing`), the `Text` page (`letterSpacing`), and the
      `TextMetrics` fields (an underline example using `canvas::Line`). Check with
      `scripts/man-run-examples.sh canvas --run` and
      `scripts/man-census.sh --memory-scope` → 0 unclassified. `caretStops` (a
      nearest-stop hit-test example; the caret is drawn at a stop's `x`), `measureText`
      ("Letter-spacing." and "Underline and strikeout." paragraphs, and the example draws
      the underline `canvas::Line`), `Text.letterSpacing` and the four `TextMetrics`
      field descriptions. `scripts/man-run-examples.sh canvas --run measureText
      caretStops` → `examples: 2 built: 2 ran: 2 … failed: 0`; census → 0.
- [x] Spec `06_canvas.md`: the run columns, where gaps go, and the caret stop contract.
      Under "Every text path reads one shaped run", with the decoration words.
      `cargo test --bin mfb spec` → 46 passed; no leaked `[[`.

Acceptance: caret stops agree with measuring and drawing.
  Check: `cargo test --test rt_canvas_font caret --test cli_canvas_man_examples_compile`
  → pass (est. 4 min).
  Result: `caret` 2 passed; `cli_canvas_man_examples_compile` 3 passed.
Commit: a1049a79a

## Validation Plan

- Tests: as listed, including negative spacing, the empty string and closed fonts.
- Runtime proof: a `/tmp` text-field demo that draws a caret at the clicked position
  using `canvas::pollMouse` and `caretStops`, and underlines the text with
  `canvas::Line` from the metrics. Take a screenshot.
  Runtime proof (2026-10-17): `/tmp/p162/proof` wraps a mixed English/Japanese paragraph
  with `strings::lineBreaks` + `canvas::measureText([latin, cjk], …)` (Arial, STHeiti
  Light), draws each line as a `Text` over `[latin, cjk]`, underlines the first line
  from `underlineOffset`/`underlineThickness`, and draws a caret at its sixth
  `caretStops` stop. The dumped frame (`MFB_CANVAS_DUMP`, converted to PNG) shows Latin
  from Arial and kana/kanji from STHeiti, `AVAWATo` kerned, lines broken between CJK
  characters and never inside `“quotes”`/`(brackets)`, the underline exactly the first
  line's measured width (502.62 px), and the caret between `Wrapp` and `ing` (scalar 5,
  x 81.96).
- Doc sync: man and spec, as above.
- Final gate: `cargo test` (full) and `scripts/artifact-gate.sh` (est. 35 min).

## Open Decisions

- Where spacing goes: between graphemes only (recommended, so `width` and centring stay
  honest) vs. after every grapheme, the CSS behavior (easy to add later as a sign
  convention).
- Default decoration metrics when a table is missing: underline at 0.1 em below the
  baseline and 0.05 em thick; strikeout 0.3 em above the baseline with the underline's
  thickness (recommended, measured against the probe's median real-font values before
  locking) vs. reporting 0.0 (forces every engine to invent its own).
- A `caretAt` convenience member: none (recommended) vs. add one.

## Corrections

1. **canvas imports `strings`.** `strings::graphemes` is unreachable from canvas's
   injected source until `canvas` lists `strings` in `add_imports`
   (`error[2-201-0014 SYMBOL_UNKNOWN_IMPORT]: Package \`strings\` is used but not
   imported`). The import itself costs 32 bytes: a one-`present` canvas program built by
   `main`'s compiler is 3,409,096 bytes, and 3,409,128 with `IMPORT strings` and a
   `strings::graphemes` call. The same program built on this branch after plan-162-A–D's
   helpers is 3,821,896 bytes (+412,800, the new MFBASIC: face index, kerning, measure,
   run helpers).
2. **More than four callers read the run.** By Phase 2 the run is read in 9 places
   (§ the task above); all go through `__CANVAS_RUN_*`.
3. **Wall-clock watchdog tests flake under load.** Three `rt_canvas_font` tests bound a
   hostile font's run with a 30 s timeout. They timed out once while another test binary
   ran in parallel and passed alone (10.8 s for all three). The checks below run the
   suite alone.

4. **Every `Text` literal names `letterSpacing`.** §1 said the zero value leaves
   existing items unchanged. That holds for drawing, but a record literal must name
   every field (`error[2-203-0024 TYPE_CONSTRUCTOR_ARITY_MISMATCH]: Constructor
   \`canvas.Text\` has 7 argument(s), expected 8`), so adding the field is a source
   change at every construction site. All 78 were updated with `letterSpacing := 0.0`
   (`/tmp/p162/edits/d3migrate.py`, inserted after `fontSize := …`).

5. **A decoration offset is to the line's top edge.** OpenType defines
   `post.underlinePosition` and `OS/2.yStrikeoutPosition` as the top of the stroke. So
   the metrics report the distance down to the top edge, and a program draws the band
   `thickness` tall below it. The man pages say so, and their example centres a
   `canvas::Line` at `offset + thickness / 2`.

## Summary

Every feature here is a projection of the one shaped run. The risk is the stride
change across the four callers and remembering to hash `letterSpacing`, not the
features themselves.
