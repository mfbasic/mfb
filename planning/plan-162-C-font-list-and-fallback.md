# plan-162-C: the public font list — `measureText` over a list, `canvas::Text.fonts`, fallback drawing

Last updated: 2026-09-27
Effort: large (3h–1d)
Depends on: plan-162-B. Also gated by bug-753 and bug-755 (see plan-162-A §Prerequisites).

Today a string is measured and drawn in exactly one font. A codepoint that font doesn't
map draws that font's `.notdef` glyph. This letter makes **a list of fonts** the unit
of text, and **list order is fallback order**. For each codepoint, the first font in
the list that maps it supplies the glyph. There is no separate "fallbacks" argument or
field (decided 2026-09-27).

- `canvas::measureText` gains an overload taking `List OF RES canvas::Font`, and the
  single-font form stays.
- `canvas::Text` names its fonts as `fonts AS List OF RES canvas::Font`, replacing
  `font`.

**The single behavioral outcome:** a string mixing Latin and CJK, drawn in
`[latinFace, cjkFace]`, shows every Latin glyph from `latinFace` and every CJK glyph
from `cjkFace`. `measureText([latinFace, cjkFace], size, s).width` equals the drawn
advance. A one-element list behaves exactly as the old single font did.

References:

- bug-756 (`__canvas_shapeRun(fonts, text)` already implements the fallback rule;
  this letter exposes it) and plan-162-A (face index; the Phase 1 findings on
  `List OF RES` record fields and geometry slot 2).
- `src/docs/spec/app/06_canvas.md` §"Images are named, not embedded", §"A named group is
  the exception" (the `2-203-0055` hand-off rule, and the close-what-nothing-live-names
  rule), and `.ai/canvas-threading.md` §7–8 (R17b, R18).
- `.ai/man-content.md` (banned memory vocabulary; `mfb man variable` owns the
  `RES` model).

## 1. Goal

- `canvas::measureText(fonts AS List OF RES canvas::Font, size AS Float, text AS String)`
  exists. `canvas::measureText(font, size, text)` is unchanged.
- `canvas::Text` has `fonts AS List OF RES canvas::Font` and no `font` field.
- Drawing and measuring both apply the fallback rule through `__canvas_shapeRun`.
- Every existing construction site is migrated, and man/spec/examples describe the
  list.

### Non-goals

- **No colour or bitmap glyphs.** `sbix`, `CBDT` and `COLR` aren't read. Outlines come
  from `glyf` only (`helper_glyph.rs:__canvas_glyphStart` reads `loca`/`glyf`). So a
  colour-emoji font in the list supplies glyph IDs that draw nothing. The man page
  says so plainly. Supporting colour glyphs is a separate plan.
- **No complex scripts.** Arabic joining, Indic reordering and other `GSUB`/`GPOS`
  contextual shaping are declared unsupported in the `loadFont` and `Text` man pages.
  Fallback picks a font per codepoint, not per cluster.
- **No automatic system fallback.** The program chooses the list. The runtime never
  appends fonts on its own.
- **No kerning across fonts** (plan-162-B Non-goals).
- **No change to `canvas::Picture`** or to image resources.

## 2. Current State

- `canvas::Text` is registered in `src/codegen/builtins/canvas/mod.rs` (the `"Text"` record,
  with a `font` field of type `ParameterType::res(ParameterType::named("canvas.Font"))`).
  The test `mod.rs:the_resource_naming_variants_hold_the_resource_itself` pins that type.
- Every MFBASIC read of `t.font` in the canvas helpers, re-measured 2026-10-14 after A
  and B: `grep -n "t\.font\b" src/codegen/builtins/canvas/*.rs` → 30 lines, 26 of them
  code (the rest are doc comments in `func_handle_bridge.rs` and `mod.rs`), in **7 files,
  not 2**:
  - `helper_geometry.rs` (6): glyph run, edges, run header (slot 2 and the ink box),
    the `Text` hash arm, the verify reference;
  - `helper_render.rs` (6): `__canvas_closeRetired`'s `Text` arms, and
    `__canvas_listNamesFont`;
  - `helper_picture.rs` (6): `__canvas_itemNamesFont` and the group-retire close, the
    `Picture`/`Text` ownership walk;
  - `helper_particles.rs` (4): the `Text` emitter box, the template's font handle,
    and the template close;
  - `helper_hit.rs` (1), `helper_autopaint.rs` (1): the text boxes.

  The count is the plan's 10 only because its grep named two files (Correction 1).
- Construction sites repo-wide, re-measured 2026-10-14:
  `grep -rn "font :=" src tests examples tools benchmark scripts | wc -l` → 76, and
  `grep -rho "canvas::Text\[" src tests examples benchmark tools scripts | wc -l` → 75,
  in 33 files (`grep -rlo …`). The directory is `benchmark`, not `benchmarks`. The
  `scripts/test-canvas-vulkan.sh` heredocs are among them, and nothing compiles them
  until the script runs (`.ai/testing-gates.md`: build them after the change).
- `measureText` call lines: `grep -rn "measureText(" src tests examples tools scripts | wc -l`
  → 43. They stay valid, because the single-font overload is kept.
- Group hand-off: the compiler rejects naming a font again after `canvas::setGroup`
  (`2-203-0055`). *Its analysis sees a font inside a `List OF RES` inside a `Text`
  literal* (read 2026-10-14). `setGroup`'s `items` is declared with
  `add_consuming_parameter("setGroup", "items")` (`func_set_group.rs`), and
  `src/ir/verify/link.rs:consumed_contained` resolves the argument through
  `held_resources` → `collect_held`. That walk recurses structurally through
  `IrValue::Constructor` args, `ListLiteral`/`SetLiteral` elements, map entries,
  `UnionWrap`/`Checked` and `WithUpdate`, and stops at a resource `Local` (or a local's
  recorded containment). So `canvas::Text[…, fonts := [f], …]` reaches `f` through
  Constructor → ListLiteral → Local. Phase 3 proves it with a diagnostic fixture and
  needs no compiler change.

## 3. Design Overview

1. **Registry.**
   - Add a second `Implementation` to `measureText`. `RegistryFunction.implementations`
     is a `Vec`, and the spec's audit format already describes builtin overloads
     (`src/docs/spec/tooling/04_audit-format.md`: "every overload of the name").
   - Change `Text.font` to `Text.fonts: ParameterType::list_of(ParameterType::res(named("canvas.Font")))`.
2. **Semantics (both paths share `__canvas_shapeRun`).**
   - *Fallback:* per codepoint, the first **open** font in the list whose cmap maps it.
     A closed font maps nothing, so it falls through to the next.
   - *Nothing maps it:* glyph 0 of the first open font.
   - *No open font:* the codepoint draws nothing and advances 0. With one font that is
     today's "a closed font draws nothing".
   - *Empty list:* `measureText` raises `ErrInvalidArgument`, and a `Text` with no fonts
     draws nothing. A drawn item never raises (§7).
   - *Closed fonts in `measureText`:* the list overload raises `ErrResourceClosed` if
     **any** listed font is closed, the same as the single form.
3. **Metrics for a list.**
   - `width` = Σ over glyphs of `advanceUnits × size / upem(fontOfGlyph)`, summed per
     font as Integers, then scaled per font and added. A one-element list is
     byte-identical to HEAD.
   - `ascent` and `descent` = the maxima over the fonts that supplied at least one glyph
     (fonts[0] when the string is empty).
   - `lineGap` = fonts[0]'s, and `height` = ascent + descent + lineGap.
   - This is CSS's line-box behavior for fallback fonts. It is an Open Decision below.
4. **Geometry.**
   - The glyph-cache key already includes the font handle
     (`__canvas_glyphKey(font, sizeQ, gid)`), so glyphs from different fonts share one
     run.
   - The run header's ink box uses the maximum ascent/descent over the fonts used.
   - Slot 2 (the run's single font handle) is handled per plan-162-A's Phase 1 finding:
     if nothing reads it, write fonts[0]'s handle; if something does, it changes to
     per-glyph (Phase 2 task).
5. **Identity and ownership.**
   - The `Text` hash folds in the list length, then each handle in order.
   - `__canvas_closeRetired`, `__canvas_anythingNamesFont` and `__canvas_listNamesFont`
     iterate over the list, and close each distinct handle at most once. That preserves
     R17b/R18.

**Gate:** behavior changes (a public API). Correctness comes from new rt tests.
- **Expected to change:** every migrated fixture's source; `app-mouse-surface`
  sentinels.
- **Must not change:** `tests/golden/canvas/*.png`. They are migrated to one-element
  lists, and byte-identity there is the proof that a one-font list equals the old field.

**Design uncertainty (first):**
- a `List OF RES` field in a builtin union variant (plan-162-A Phase 1 recorded the
  answer; re-check it here);
- the compiler's `2-203-0055` analysis through the list.

**Correctness risk (last):** closing and retirement with lists, since a missed handle
means text that silently vanishes or a font that leaks.

Rejected alternatives:
- **A separate `fallbacks` field or argument.** Rejected by decision (2026-09-27).
- **Keep `font` and add `fonts`.** Rejected: two sources of truth, and an ambiguity when
  both are set.
- **`measureRun(t AS canvas::Text)`.** Rejected in favour of the list overload, which is
  the user's choice. It would couple measuring to the drawing record.
- **Per-cluster fallback.** Deferred: meaningful only with complex shaping, which is a
  non-goal.

## Compatibility / Format Impact

- **Breaking, source level:** `canvas::Text[font := f]` → `canvas::Text[fonts := [f]]`. All
  in-repo sites are migrated here: 75 `canvas::Text[` literals in 33 files, re-measured.
  *No external-package corpus to notify:* `grep -rln "canvas::Text\|canvas.Text"
  repository/ packages/` → nothing (2026-10-14).
- **Additive:** the `measureText` list overload.
- **Unchanged:** `canvas::Font`, `loadFont`, `systemFonts`, `destroyFont`, and all
  non-text items.

## Phases

> **NOTE — keep the checkboxes current as you go** (rules as in plan-162-A).

### Phase 1 — premises

- [x] Re-run plan-162-A's prerequisite table. The bug-753 and bug-755 rows must be MET.
      `ls bugs/completed/bug-753-* bugs/completed/bug-755-*` → both (2026-10-14).
- [x] Re-measure the `t.font` reads and the construction sites. Update §2.
- [x] Read the `2-203-0055` implementation. Find it with
      `grep -rn "0055\|setGroup" src/semantics src/codegen/builtins/canvas | head`, then
      follow the analysis. Record whether it walks into a `List OF RES` inside a record
      literal. If not, extending it is Phase 3 scope, because it is the compiler half of
      this feature. It does (§2). `src/semantics` does not exist; the analysis is
      `src/ir/verify/link.rs`, reached from the consuming-parameter declaration.
- [x] Re-confirm the plan-162-A Phase 1 finding on `List OF RES` record fields in a union
      variant and through `present`'s deep copy. `/tmp/p162/cpremise`, rewritten
      2026-10-14 as a user `UNION Item` of `Chain` (`fonts AS List OF RES canvas::Font`)
      and `Plain`, built per frame from `chainOf(a, b)` and read through `MATCH … CASE
      Chain(c)` / `FOR EACH f IN c.fonts`. Three frames print `2 15.57 17.50` / `plain`,
      a `Text` presented each frame, then both `destroyFont`s succeed and it prints
      `done`, exit 0. A *builtin* variant through `present` cannot exist before Phase 3
      makes the field; `rt_canvas_present_deep_copy` and the Phase 3 ownership tests
      prove it there.

Acceptance: every UNVERIFIED item in §2 and §Compatibility has a recorded answer.
  Check: `grep -c UNVERIFIED planning/plan-162-C-font-list-and-fallback.md` → 0 outside
  this list (est. 1 min).
  Result: → 2, both in the acceptance text above.
Commit: 0f3eaf7ed

### Phase 2 — the `measureText` list overload (additive, no field change yet)

- [x] `func_measure_text.rs`: a second `Implementation` whose MFBASIC body checks every
      font with `canvas::fontOpenHandle` (raising `ErrResourceClosed`), raises
      `ErrInvalidArgument` on an empty list, shapes with the handle list, and computes
      metrics per §3.3. `__canvas_measureTextFonts`, over the shared
      `__canvas_measureHandles` helper.
- [x] Refactor the single-font body to call the list body with `[font]`, so there is one
      metric computation. `__canvas_measureText` is
      `__canvas_measureHandles([canvas::fontOpenHandle(font)], size, text)`.
      `__canvas_shapeRun` also now gives `.notdef` to the first **open** font, per §3.2's
      "nothing maps it" rule, rather than to `fonts[0]` unconditionally.
- [x] Tests (`rt_canvas_font.rs`), using two fixture fonts with disjoint cmaps:
  - the mixed-string width equals the sum computed per font;
  - a codepoint mapped by neither font takes fonts[0]'s glyph-0 advance;
  - ascent/descent are the maxima over the fonts used;
  - an empty list raises `ErrInvalidArgument`;
  - a closed second font raises `ErrResourceClosed`;
  - `measureText([f], …)` equals `measureText(f, …)` field for field.

  `measure_text_over_a_font_list_sums_each_font_at_its_own_scale` covers the first
  three bullets and fallback order: `"AVB"` in `[latin, fallback]` →
  `675000 900000 300000 100000 1300000` (thousandths). `X` → the first font's glyph 0,
  500 units. `measure_text_over_a_one_font_list_is_the_single_font_form` covers the
  other three: identical metrics for `[f]` and `f` in both fixtures, `empty: 77050002`,
  and `closed: 77030004`. The closed case closes the font behind a resource parameter;
  closing it directly is the compile-time `2-203-0055` this plan's Phase 3 relies on.
- [x] Man: `measureText` page. Describe the overload and the fallback rule in developer
      terms, with a runnable example. Check with
      `scripts/man-run-examples.sh canvas --run` → the new example runs.
      A "Measuring in a list of fonts" paragraph, plus `measureText([face], …)` in the
      example. `scripts/man-run-examples.sh canvas --run measureText` → `examples: 1
      built: 1 ran: 1 … failed: 0`, printing `width 60.00 height 26.40` / `same width
      TRUE`. The runner uses `target/release/mfb`, which must be rebuilt first
      (Correction 2).

Acceptance: the overload works, and the single form is byte-identical.
  Check: `cargo test --test rt_canvas_font measure` → pass (est. 3 min).
  Result: 6 passed, 0 failed. That includes the single-form
  `measure_text_sums_a_mixed_run_in_font_units_then_scales_once` (the last-bit
  Integer-sum test) and `measure_text_scales_the_fonts_own_metrics`, now running through
  the list body.
Commit: 5da31a307

### Phase 3 — `Text.fonts`, drawing, ownership (blast radius last)

- [x] `mod.rs`: `Text.font` → `Text.fonts` (`List OF RES canvas::Font`), with a
      description giving the fallback order.
- [x] Update `the_resource_naming_variants_hold_the_resource_itself` to expect
      `list_of(res(named("canvas.Font")))`. AGENTS.md test-edit proof:
  - (1) written by plan-116-I (`git log -S "the_resource_naming_variants_hold_the_resource_itself"`);
  - (2) it protects "the field aliases the live resource rather than copying it";
  - (3) its only dependant is itself;
  - (4) proof the old line is wrong: this plan's decision replaces the field.

  The protected property is kept: the element stays `RES`, not a value. Cite this in
  the commit. The test now carries `(owner, field, resource, in_list)` and expects
  `list_of(res(named("canvas.Font")))` for `Text.fonts`, and `res(named(…))` for
  `Picture.image` as before.
- [x] `helper_geometry.rs`: glyph run, edges, run header (ink box over the fonts used;
      slot 2 per Phase 1), verify reference, and the hash arm all switch to `t.fonts`.
      Outline bytes are fetched per glyph for the font its `fontIndex` names. A font's
      file is re-fetched only when the run moves to another font, so a one-font run
      fetches it once, as before. Shared helpers: `__canvas_textHandles`,
      `__canvas_fontScales`, `__canvas_runDrawable`, `__canvas_runLine`, `__canvas_runPen`.
      A glyph from a font with no scale is left out of the run, never given a `-1`
      entry, so no draw path or GPU publisher sees one. Slot 2 is `fonts[0]`'s handle.
      The hash folds the font count, then each handle.
- [x] `helper_render.rs`: `__canvas_closeRetired`, `__canvas_anythingNamesFont` and
      `__canvas_listNamesFont` iterate over the list and close distinct handles once.
      Through `__canvas_closeTextFonts` and `__canvas_textNamesFont` (in
      `helper_geometry.rs`, which every program carries; `helper_picture.rs` is
      injected only with `Picture`). A repeat closes once because a closed font's handle
      reads 0.
- [x] (Correction 1) `helper_picture.rs`, `helper_particles.rs`, `helper_hit.rs` and
      `helper_autopaint.rs` read `t.fonts`. Group ownership and the particle template
      walk every listed font. The text boxes take their metrics as `measureText`'s list
      form does. `grep -n "t\.font\b" src/codegen/builtins/canvas/*.rs` now hits only
      prose.
- [x] ~~Compiler: extend `2-203-0055` through the list if Phase 1 found it doesn't see
      it.~~ — moot: Phase 1 found `collect_held` already walks Constructor → ListLiteral
      → Local. Add the diagnostic test in the existing `2-203-0055` test file. Done as
      the sibling fixture `tests/syntax/resources/canvas-setgroup-consumes-text-fonts`,
      beside `canvas-setgroup-consumes-items`, since each fixture is one program. It
      hands `[title, body]` to one group and names `body` in a second, and the golden is
      `main.mfb:27 error[2-203-0055 TYPE_USE_AFTER_MOVE] … Binding \`body\` was moved`
      (`scripts/test-accept.sh target/debug/mfb /tmp/p162/accept-c3
      canvas-setgroup-consumes-text-fonts` → passed).
- [x] Migrate every construction site (48, re-measured in Phase 1) to `fonts := [f]`:
      `tests/`, `examples/`, `src/docs/spec/app/06_canvas.md`, registry examples,
      `tools/canvas-bench/gen/text.sh`, and the three `scripts/test-canvas-vulkan.sh`
      heredocs. 76 sites in 34 files (`/tmp/p162/edits/c3migrate.py`, a word-bounded
      `font := X` → `fonts := [X]`; every right-hand side was a plain identifier), after
      which `grep -rnE "(^|[^A-Za-z])font := " src tests examples tools benchmark scripts`
      → nothing.
- [x] Tests:
  - a two-font drawn string: pixel columns show each glyph from its font (fixture
    fonts with different outlines);
  - closing the first font draws the whole string from the second;
  - a group naming `[a, b]` closes both on `removeGroup` unless something live names
    them;
  - a `Text` whose list repeats a font closes it once;
  - an R18-style rebuilt-each-frame group keeps drawing.

  `rt_canvas_font`: `a_two_font_string_draws_each_glyph_from_its_own_font` (`AB` in
  `[latin, fallback]` lights columns 110..204, `[latin]` alone 110..139) and
  `closing_the_first_font_draws_the_string_from_the_next` (125..204). Both ok.
  `rt_canvas_group_ownership`: `retiring_a_group_closes_every_font_its_text_lists`
  (`a=FALSE b=FALSE`),
  `retiring_a_group_keeps_a_listed_font_the_scene_still_names` (`a=FALSE b=TRUE`),
  `a_font_listed_twice_closes_once`, and
  `a_group_rebuilt_each_frame_from_a_font_list_keeps_drawing` (glyphs > 0 every
  frame, both open). All ok.
- [x] Man: the `Text` page (the `fonts` field, fallback, what isn't supported: colour
      glyphs, complex scripts), and `loadFont` (unsupported scripts). Check with
      `scripts/man-census.sh --memory-scope` → 0 unclassified, and
      `cargo test --test cli_canvas_man_examples_compile` → pass. `Text`'s record
      description and its `fonts` field (`mfb man canvas types`) carry the fallback
      rule, colour/bitmap glyphs drawing nothing, and complex scripts drawing unjoined.
      `loadFont`'s "Kerning." paragraph (plan-162-B) already names the unsupported
      shaping. The package overview's resource paragraph now says closing a font moves
      the text to the next font. Census → 0 unclassified;
      `cli_canvas_man_examples_compile` → 3 passed.
- [x] Spec `06_canvas.md`: the `Text` contract, list ownership in groups, and metrics
      over a list. "Images are named, not embedded" (`Text.fonts`), "A named group is
      the exception" (per-font close, repeat once, move checker through the list),
      "Every text path reads one shaped run" (first open font's `.notdef`, per-font
      scale, list metrics, hash). `cargo test --bin mfb spec` → 46 passed.

Acceptance: fallback draws and measures consistently, the PNG goldens are
byte-identical after migration to one-element lists, and ownership tests pass.
  Check: `cargo test --test rt_canvas_font --test rt_canvas_golden --test rt_canvas_group_ownership --test rt_canvas_present_deep_copy`
  → pass, and `git status --short tests/golden/canvas` → empty (est. 8 min).
  Result: 46 + 28 + 11 + 9 passed (`/tmp/p162/c3-accept1.log`, before the new tests
  were added), then the 2 + 4 new tests, all ok. `git status --short
  tests/golden/canvas` → empty: the PNG goldens, migrated to one-element lists, are
  byte-identical. The other migrated suites: cli_canvas_package 7, rt_canvas_autopaint 23, rt_canvas_hit 14, rt_canvas_metal 71, rt_canvas_particles 29, rt_canvas_rasteriser 107 (2 ignored, as at HEAD) passed (`/tmp/p162/c3-migrated.log`).
Commit: — (this commit)

## Validation Plan

- Tests: as listed, including negative cases (empty list, closed fonts, repeated font).
- Coverage check: `rt_canvas_group_ownership` covers the close paths, and
  `rt_canvas_present_deep_copy` covers the list crossing `present`.
- Runtime proof: `examples/emoji` switched to a two-font list (a UI face, then a CJK
  face from `listSystemFonts`), with a caption mixing both. Take a screenshot per
  platform, then build and run the Vulkan script heredocs.
- Doc sync: man (`measureText`, `Text`, `loadFont`) and spec `06_canvas.md`.
- Final gate: `cargo test` (full), `scripts/artifact-gate.sh`, and
  `scripts/test-canvas-vulkan.sh` on the Vulkan box (est. 45 min).

## Open Decisions

- List metrics:
  - ascent/descent as the maxima over the fonts used (recommended, CSS line-box
    behavior, so a CJK fallback doesn't clip) vs. fonts[0]'s only (simpler; a mixed line
    overlaps the line above).
- Field name:
  - `fonts` (recommended, reads as plural at every site) vs. keeping the name `font`
    with a list type (smaller diff, misleading name).

## Corrections

1. **The `t.font` population is 26 code lines in 7 files, not 10 in 2.** The plan's grep
   named only `helper_geometry.rs` and `helper_render.rs`. The group-ownership walk
   (`helper_picture.rs`), the particle template (`helper_particles.rs`) and the hit and
   AutoPaint text boxes read the field too. Phase 3's tasks name `helper_geometry.rs`
   and `helper_render.rs`, and gain one for the other four files.

2. **`man-run-examples.sh` runs `target/release/mfb`** (`MFB=${MFB:-./target/release/mfb}`).
   The first run printed only the old example's line because the release binary
   predated the page edit. After `cargo build --release` it ran the new example.

## Summary

The shaping rule already exists (bug-756). This letter is API surface plus ownership.
The real risk is resource lifetime with a list (retirement, groups, the compiler's
hand-off check), not glyph selection. One-element-list migration of the PNG goldens is
what proves the old behavior survived.
