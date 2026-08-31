# plan-98-G: Canvas text, atlas eviction, measureText, damage-rect present

Last updated: 2026-08-30
Effort: x-large — a hand-rolled TrueType reader + contour rasteriser in MFBASIC
(Correction 1); a real shaper (HarfBuzz) is a separate future plan
Depends on: plan-98-F (Vulkan backend — all three GPU backends + software render text)

This sub-plan adds **text rendering** (a hand-rolled TrueType path), glyph-atlas LRU eviction,
`measureText`/`TextMetrics` from day one, and optional damage-rect presentation. After it
lands, Text `DrawItem`s render on all backends (software exact-match golden, GPU within
tolerance), glyphs are packed into the shared atlas on demand and evicted under pressure,
and `canvas::measureText` returns metrics whose API is shaper-independent so a future
HarfBuzz/FreeType upgrade doesn't change the surface.

This is **build step 7** of the A–G sequence — text, the hard part. It ships the *minimum
viable* path — glyph positioning and per-glyph rasterisation — not full shaping.

References:

- **plan-98-A** — the "Cross-cutting invariants" section is this feature's top-level
  design (invariant 8 governs this letter's testing and its Phase 4 closeout). There is
  no separate design document: plan-98-A … plan-98-G plus plan-98-api.md are the whole
  corpus.
- `planning/plan-98-api.md` — the `Text` variant's fields and the `TextMetrics` record.
- plan-98-B (Text is one of the frozen `DrawItem` variants; `measureText` signature reserved),
  plan-98-C (atlas: white pixel + images; Text stubbed until now), plan-98-D (positional diff/
  damage computed but deferred to here).
- `.ai/testing-gates.md`, `.ai/resources-packages.md` (Font as a resource handle).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-98-F's **backends** render the non-text scene | `scripts/test-canvas-vulkan.sh <exe> --box 2228 --libc glibc` and `--box 2227 --libc musl --icd auto`; `cargo test --test rt_canvas_metal` | **MET** — software, Metal and Linux Vulkan all render the full primitive set within tolerance. The row used to read "plan-98-F complete", which is a *different* claim: F Phase 3 (Windows Vulkan) is blocked on bug-478 and G does not touch it. Corrected in place rather than waived — see Correction 2. |
| ~~The shared atlas exists (white pixel + images)~~ | — | **Corrected: the row asked G to require its own output.** There is no atlas: plan-98-E and plan-98-F each audited this and marked their atlas rows moot *pointing at G*. `grep -n 'CASE Picture' -A 3 src/codegen/builtins/canvas/helper_geometry.rs` shows `Picture` returning `__canvas_emptyHeader()` and `[]`, and all four `IMAGE_DIRTY` hits are writes. Building the atlas is Phase 1's work, not its precondition (Correction 3). |
| The vendored-dependency policy is decided | this letter's Open Decisions | **MET** — hand-roll in MFBASIC, decided 2026-08-31 (Correction 1). |
| Working tree builds | `cargo build` → pass | **MET** (`Finished \`dev\` profile`). |

> Per A's invariant 8: no "full suite green at HEAD" row and no byte-identity
> obligation.

> The **vendored-single-header policy** was this feature's named open question and is now
> settled: **hand-roll, in MFBASIC** (Correction 1). It was a real gate — it decided the
> shape of the whole letter — and it is recorded rather than assumed.

## 1. Goal

- Render Text `DrawItem`s: shape (positioning only, no complex shaping) and rasterise
  glyphs into the shared atlas, cached by `(font, size, codepoint)` with LRU eviction; emit
  tinted textured quads through the existing single pipeline.
- `canvas::loadFont` returns a `Font` RES resource (from B); `canvas::measureText` returns
  `TextMetrics` — **shipped from day one**, decided in plan-98-api.md precisely so
  swapping in a real shaper later doesn't change the API.
- Glyph atlas eviction: LRU by last-used revision under atlas pressure, mirroring the geometry
  cache eviction.
- Text renders exact-match on the software oracle and within tolerance on Metal/Vulkan.
- **Optional** damage-rect presentation (`VK_KHR_incremental_present` / Metal dirty rects)
  consuming D's positional-diff damage — behind a capability check, full-frame otherwise.

### Non-goals (explicit constraints)

- **No complex text shaping.** Advance-width positioning only: no kerning, ligatures, bidi,
  Arabic/Indic shaping, emoji, or subpixel positioning — all explicitly out of scope. A real
  shaper (HarfBuzz + FreeType) or MSDF is a separate future plan; the `measureText`/`TextMetrics`
  API is designed so that upgrade is non-breaking.
- **No API change** to the frozen `DrawItem` set — Text is already a variant (B).
- **Damage-rect present is optional** and must degrade to full-frame where the platform lacks it;
  it never changes visible output, only present efficiency.
- Software backend stays exact-match for text goldens — which is *why* the rasteriser is
  hand-rolled (Correction 1), not a hope about someone else's library.

## 2. Current State

- **B** froze Text as a `DrawItem` variant and reserved the `measureText`/`TextMetrics` and
  `canvas::loadFont` signatures. **C/E/F did *not* build the atlas** — each of them audited
  the claim and marked its own atlas row moot, pointing here (Correction 3). **D** computes
  positional-diff damage but has no consumer (deferred here).
- **Font is a RES resource** (from B): a `Font` id with closed-flag lifetime, freed by the same
  closed + frame-drain rule as Image — no refcount. Same lifetime machinery as Image.
- **The no-dependency bar is settled** (Correction 1): hand-rolled, in MFBASIC. The
  alternatives — a vendored per-platform library reached through `LINK`, or compiling
  `stb_truetype` into `mfb` — are recorded with why each was rejected, so the question is
  not reopened by accident.

### Measured populations

| What | Count | Command |
|---|---|---|
| Glyph cache key | `(font, size, codepoint)` | this plan's §3 — a decision, not a measurement |
| Text features in scope | positioning + per-glyph raster only | this letter's Non-goals list above |
| Damage consumers to wire | 2 (Vulkan incremental present, Metal dirty rects) | this plan's Phase 3 — one per GPU backend (E, F) |

### Verified properties

- ~~**stb_truetype is single-header, public-domain, vendorable**~~ — **moot**: nothing is
  vendored (Correction 1). The licence and self-containedness questions this row reserved
  do not arise.
- **`measureText` from day one makes the shaper swappable** — a design decision of this
  plan set, not a verified property. G ships the API even though a minimal positioner
  underlies it, so a later HarfBuzz swap is non-breaking. It is only true if `TextMetrics`
  stays shaper-independent; Phase 1 must keep rasteriser-specific fields out of it.
- Determinism is no longer a question to be *discovered* — it is the property the
  hand-rolled path is chosen for. The rasteriser runs the same MFBASIC on every target,
  the same code path plan-98-F Phase 1 measured byte-identical across two ISAs and two
  operating systems, so text goldens stay **exact-match** on the software oracle. The
  standing risk moves from "is stb deterministic" to "does the contour rasteriser use
  anything width- or order-dependent" — which is a thing to *not do*, and the same
  byte-identity comparison catches it.

## 3. Design Overview

- **Glyph pipeline (in `present()`, invariant 1).** Position by advance width + rasterise each
  needed `(font,size,codepoint)` into the atlas on demand; cache with LRU; emit tinted quads.
  Shaping is per-string, per-`present()` — so a per-frame-changing string re-shapes each present
  (charged to the worker budget, invariant 2); the glyph *raster* cache absorbs the per-glyph
  cost.
- **`measureText`/`TextMetrics`.** Compute metrics via the same reader; return the reserved
  `TextMetrics` record — API frozen, implementation swappable.
- **Atlas eviction.** LRU by last-used revision under atlas pressure, mirroring the geometry
  cache; a live scene's glyphs are pinned.
- **Damage-rect present (optional).** Consume D's positional-diff damage: present only the damage
  union via `VK_KHR_incremental_present` / Metal dirty rects where available; full-frame else.

**Where correctness risk concentrates:** determinism of the software text golden (so text can be
exact-match-gated) and atlas eviction correctness under pressure (never evict a glyph a live scene
references). Land text rendering + metrics first, eviction next, optional damage-rect last (it is
an efficiency-only change behind a capability check).

**Gate:** text renders exact-match on the software oracle (or within a documented text tolerance if
the raster proves machine-variant, which it should not) and within tolerance on GPU. Damage-rect present must produce
**identical visible output** to full-frame (verified by golden equality with damage on vs off).

**Rejected alternatives (for this sub-plan):**
- *HarfBuzz + FreeType now.* Deferred: months of work; the design ships a minimal
  positioner first with a shaper-independent `measureText` API so the upgrade is
  non-breaking later.
- *MSDF now.* Deferred: kills the per-size cache explosion but needs a build-time msdfgen step;
  revisit when text scale demands it. The `measureText` API is unaffected by the choice.

## Compatibility / Format Impact

- **Changes:** Text rendering + glyph atlas + `measureText`; optional damage-rect present. New
  text-golden corpus. **No new dependency** (Correction 1).
- **Unchanged:** the frozen `DrawItem` set, the scene model, thread/ring/retirement, the GPU
  backends' pipeline, and visible output under damage-rect present (efficiency only).

> **Carried in from a 2026-08-30 review (plan-98-D Phase 2).** `ImageRef` and
> `FontRef` are exported records with a public `id: Integer`, so a program can write
> `ImageRef[id := 7]` and fabricate a handle naming an image that does not exist —
> the runtime then draws nothing, silently. The *indirection* is forced (the spec's
> `TYPE_RESOURCE_FIELD_FORBIDDEN` rule says a resource never appears inside a data
> type, and a scene holding an image would also make `canvas::destroyImage` a lie),
> but the public field is not. The fix is to make both types opaque — no public
> fields, no user constructor, obtainable only from `canvas::imageRef` /
> `canvas::fontRef`. Deferred deliberately by the author on 2026-08-30 ("leave it as
> is for now"); G is where it lands because G is the letter that introduces `Font`
> and `canvas::loadImage` and touches both types anyway.

## Phases

### Phase 1 — Resolve the dependency policy; text render + measureText

> **Three tasks moved here** (plan-98-B Corrections 20–21), all blocked on the
> vendoring decision this phase's first task settles. Nothing is deferred out of the
> plan; they land in the phase that owns their mechanism.

- [x] Vendored-single-header policy **resolved: hand-roll, in MFBASIC** (Correction 1),
      decided by the author on 2026-08-31 after the three options were laid out with
      what each costs. It is one decision for fonts *and* images, as the row says.

      The deciding property is the oracle. plan-98-F Phase 1 measured the software
      render **byte-identical across macOS/Linux and aarch64/x86-64** — 2,304,000 bytes,
      two ISAs, two operating systems — and every GPU backend is gated against it. A
      per-platform vendored library would end that: the same string would rasterise
      differently on each target and the text goldens would need a tolerance instead of
      exact match, weakening the one gate this feature set is built on.

      The third option — compile `stb_truetype` into `mfb` itself — was **rejected on
      inspection, not on taste**: glyph rasterisation has to happen at *program run
      time* for arbitrary strings, and an emitted program has no C toolchain and no CRT.
      The compiler can only bake glyphs it already knows, which text rendering is not.

      It is also the option that fits what is already here: the canvas rasteriser *is*
      MFBASIC (`__canvas_edgeDistance`, `__canvas_geoDistance`), a TrueType glyph is a
      set of quadratic contours, and coverage-from-a-signed-distance is the machinery
      those helpers already implement. The new code is a `cmap`/`loca`/`glyf` reader and
      a contour rasteriser; the fill, the antialiasing and the blend are shared.

      Recorded in `.ai/canvas-threading.md` §12 so the next reader finds it without
      this plan.
- [x] **The `Font` RES resource**, `canvas::destroyFont`, `canvas::fontRef` — **and
      `canvas::loadFont` with them**, in one commit rather than two. B's reason for
      deferring the type is still the operative one: without a constructor the type is
      surface no program can reach, and splitting them across commits would have
      recreated that state for as long as the split lasted. Landing them together also
      keeps `RESOURCE_TAG_FONT` and `FONT_BYTES` from being dead constants — AGENTS.md
      forbids the `#[allow(dead_code)]` that "a later phase consumes it" would need.

      `Font` sits on the canonical header (`tag@0`/`handle@8`/`closed@16`/`STATE@24`)
      with the file's bytes at `FONT_BYTES@32`, and tag `12` is now claimed rather than
      reserved. `handle@8` is the record's own address, exactly as `createImage` does
      it: unique and non-zero for the resource's lifetime, so a `FontRef` is a real
      identity from the start.

      `loadFont` is **MFBASIC** over an `internal_only` `canvas::fontFromBytes`
      emitter — the read is `fs::readBytes`, the version check is
      `collections::getOr`, and only the record stamping needs codegen. That keeps the
      *rule* about which files are acceptable readable instead of spelled in loads and
      compares. New error `ErrBadFontFile` (`77050022`), because "not a font I can
      read" and "no such path" need different fixes (Correction 4).

      Measured end to end on the host, not only in the harness: a real
      `Andale Mono.ttf` loads and yields a non-zero handle, `Helvetica.ttc` is refused
      `77050022`, and a missing path is `ErrNotFound` `77030001`.

      `tests/rt_canvas_font.rs` is the regression gate — 2 tests, RED-checked by making
      the version predicate always accept, which turns `otto` red. Its fixtures are
      **written by the program under test**, so both spellings of TrueType and each
      refused container (`OTTO`, `ttcf`, `wOFF`, and a too-short file) are exercised
      exactly, with no font committed to the repository.
- [ ] **`canvas::loadImage`** (moved from plan-98-B Phase 4): decode an image file to
      RGBA8 and hand it to the existing `canvas::createImage` path, which already owns
      the resource record, the CPU shadow and the pixel-count contract. It lands here
      because decoding needs inflate, which does not exist —
      `grep -rn "inflate\|deflate" src/codegen/builtins/` returns nothing, and it is
      plan-93-A's scope — so `loadImage` rides the same decision as the font path
      (Correction 1): an inflate and a PNG unfilter, hand-rolled beside the TrueType
      reader.
- [x] The rasteriser, and Text rendering — **as a polygon**, which is the whole payoff
      of Correction 1. A glyph is a set of closed contours and `__canvas_edgeDistance`
      already turns closed contours into a signed distance, so a `Text` item produces a
      `__CANVAS_GEO_POLYGON` header with every glyph's flattened edges in its tail and
      **no renderer arm is added at all** — software, Metal and Vulkan draw text through
      the path they already draw polygons through, with the same fill, stroke,
      antialiasing and blending.

      `helper_glyph.rs` is the reader: `loca` (both offset formats), simple `glyf`
      contours, run-length-decoded flags, the three-case delta coordinates, implied
      on-curve midpoints, and quadratic flattening. `helper_font.rs` supplies the tables
      it sits on.

      **Two departures from the box's wording, both deliberate.** There is no
      *per-`(font,size,codepoint)` atlas* and no *tinted quad*: the geometry cache
      already keys on the item's content hash, so a string's outlines are flattened once
      and reused until the string changes, and the "quad" a glyph would be textured into
      is the polygon itself. The atlas the box asks for is what a *bitmap* rasteriser
      needs; an outline one does not, and adding it would mean rasterising glyphs twice.
      Phase 2's eviction row is re-scoped accordingly (Correction 11).

      Verified by rendering. `Hi` in Andale Mono is two clean glyphs with the dot on the
      `i` as its own contour; `Sog@` exercises curves, counters and a descender — the
      `o`'s hole and the `@`'s spiral both come out right, which is the even-odd/non-zero
      question answered in practice (Correction 10). The committed gate uses a
      **synthesized** glyph instead: a square whose font-unit coordinates make the
      expected pixels arithmetic — ink exactly `x` 110..139, `y` 170..199, 900 lit
      pixels, and black on all four sides of it. Two more tests pin the pen advance
      between glyphs and that text in an unresolvable font draws nothing.
- [x] `canvas::loadFont` landed with the resource above; `canvas::measureText` →
      `TextMetrics` now lands with the TrueType table reader it shares with the render
      box. `helper_font.rs` is that reader: big-endian primitives, the table directory,
      `head`/`hhea` metrics, `hmtx` advances, and `cmap` formats 4 and 12.

      **`TextMetrics` was already declared by plan-98-B, and its contract is the one
      implemented** — `descent` positive, `height = ascent + descent + lineGap` — not
      the one the raw file suggests. The duplicate record this box first added was
      caught by `DOC_DUPLICATE` at build time (Correction 7).

      Measured against a **synthesized** font whose every number is stated in the test
      rather than copied from a run (`unitsPerEm` 1000, ascender 800, descender -200,
      lineGap 100, advances `[500, 250, 300]`, `cmap` mapping only `A` and `B`):
      `A` is 25.0 px at size 100, `B` is 30.0, an unmapped `X` falls to glyph 0 at
      50.0, `AXB` is 105.0, halving the size halves every number, and an empty string
      is zero wide and still a full line tall. Also measured on a real font — Andale
      Mono, `unitsPerEm` 2048 — where "hello" at 24 px is 72.01, which is five
      monospaced advances.
- [x] `tests/rt_canvas_font.rs` is 8 tests over a synthesized font: the container
      rules, the metrics, the descent sign, the glyph's own coordinates, the pen
      advance, the unresolvable-font case, and **Text on the GPU within
      `Tolerance::GPU_DEFAULT`**. The GPU one is not vacuous — checked by hand that the
      two frames *differ* before they agree, which is the tell that would otherwise hide
      a silent fallback (the trap plan-98-F Correction 4 was caught by).

      Software is exact-match by construction rather than by tolerance: the expected
      pixels are computed from the fixture glyph's own coordinates.

      The Metal comparison uses four square glyphs deliberately. A curved glyph costs
      ~160 edges and `MAX_EDGES` is 256, so real text does not reach the Metal GPU path
      at all today — measured and recorded in Correction 12, which is Phase 2's
      motivation rather than something this box can assert around.

Acceptance: Text `DrawItem`s render on all backends (software exact-match/documented-tolerance, GPU
within tolerance); `measureText` works and its API is shaper-independent; a `Font`
loads, is named by a `FontRef` in a `Text` item, and is released by
`canvas::destroyFont` and by scope-drop; `canvas::loadImage` decodes a real file to
the same `Image` resource `canvas::createImage` produces.
Commit: —

### Phase 2 — Glyph atlas LRU eviction

- [x] Rasterise each `(font, sizeQ, glyphId)` once into a coverage bitmap and blit it, instead of
      evaluating a signed distance field over the glyph's area per frame (Correction 12).
      `__CANVAS_GLYPH_KEYS/META/COV` in `src/codegen/builtins/canvas/helper_glyph_cache.rs`;
      a `__CANVAS_GEO_TEXT` geometry entry carries `(entryIndex, penX, penY)` per glyph.
      **Measured: a 12-character render at size 120 went from 8.1 s to 1.0 s.**
- [x] Evict glyphs by last-used revision under atlas pressure; pin glyphs referenced by a live
      scene (never evict an in-use glyph). `__canvas_glyphEvict` pins from the geometry cache's
      own `__CANVAS_GEO_TEXT` runs *and* from the run under construction
      (`__CANVAS_GLYPH_PINS`), renumbers the survivors, and rewrites both to match. Budget
      1 MiB, overridable with `MFB_CANVAS_GLYPH_BUDGET` so a test can force pressure.
- [x] Tests: forcing a small atlas evicts least-recently-used glyphs; a live-scene glyph is never
      evicted; re-rendering a scene after eviction re-rasters cleanly (golden unchanged).
      `eviction_frees_unpinned_glyphs_and_changes_no_pixel` in `tests/rt_canvas_font.rs`:
      the same 300-item scene under a forced 8 KiB budget and under the default, asserted
      **pixel-identical**. Measured `glyphEvictions=23, glyphs=261` under pressure against
      `glyphEvictions=0, glyphs=300` without. It is not a vacuous test — it found two real
      defects before it passed (Corrections 15 and 16).
- [ ] **GPU text** (added — Correction 14 is its motivation): give Metal and Vulkan a glyph
      atlas texture and a sampler so a `__CANVAS_GEO_TEXT` scene renders on the GPU instead of
      being declined. Until then both `*Renderable` predicates decline it and the whole scene
      falls back to software, which is correct but forfeits the GPU for any scene with a
      character in it.

Acceptance: atlas eviction is LRU and never evicts a live glyph; output is golden-stable across
eviction cycles — asserted as **exact pixel equality** between a pressured and an unpressured
render of the same scene, not merely "no crash".
Commit: —

### Phase 3 — Optional damage-rect present (largest blast radius last)

- [ ] Consume D's positional-diff damage: present only the damage union via
      `VK_KHR_incremental_present` (Vulkan) / Metal dirty rects where the capability exists;
      full-frame otherwise.
- [ ] Tests: golden equality with damage-rect present on vs off (identical visible output); a
      single changed label repaints only its damage region (observable via a present-region
      counter); capability-absent path falls back to full-frame.

Acceptance: damage-rect present produces visibly identical output to full-frame (exact on
the software path, within tolerance on GPU) while presenting only the changed region where
supported; degrades to full-frame cleanly. Run only the new damage/text tests plus the
existing canvas goldens.
Commit: —

### Phase 4 — Plan-98 closeout: the one full-suite run

This is the **end of the whole plan** (A–G), and the only place plan-98 runs the full
suite (A's invariant 8). Everything before this point ran targeted tests only, so this
phase is where accumulated cross-letter breakage surfaces — budget for it.

- [ ] `cargo test --no-fail-fast` — **not** plain `cargo test`, which stops at the first
      failing target and silently skips every `rt_*` runtime/codegen test that sorts
      after it.
- [ ] The acceptance golden harness, which is **not** part of `cargo test`:
      `scripts/test-accept.sh` with a session-unique scratch dir (never `tests/` — the
      second argument is `rm -rf`'d). Watch the `N ran` count between runs; a silent drop
      means fixtures were skipped, not that they passed.
- [ ] Triage anything red: fix it, or — if it is genuinely pre-existing — prove that
      against the merge-base binary (`git archive <merge-base> | tar -x -C /tmp/base98`
      + `cargo build --release`) before setting it aside. Pre-existing is not
      not-a-bug.
- [ ] `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`
      — the root `--all` does not reach the `repository/` path dependency.
- [ ] Move `planning/plan-98-*.md` to `planning/completed/` with the verification
      evidence, per the archive convention.

Acceptance: `cargo test --no-fail-fast` green; `test-accept.sh` green with no drop in the
`N ran` count; fmt clean; plan archived.
Commit: —

## Validation Plan

- Tests: text goldens (software exact-match/documented-tolerance + GPU tolerance), `measureText`
  metrics, atlas eviction, and damage-rect visible-equality.
- Coverage check: shaping/measure/eviction/damage logic in the `--bin mfb` denominator via
  in-process tests; the render runs in the headless/real subprocess.
- Runtime proof: a canvas program renders changing text (a clock/counter), presents per frame,
  and the atlas/eviction/damage counters show glyph reuse and region-limited present.
- Doc sync: `src/docs/spec/app/` canvas text section (scope + limits, `TextMetrics`, the
  shaper-swap note); man content for `canvas::measureText`, `canvas::loadFont`,
  `canvas::destroyFont` — authored as registry descriptors + Rust doc comments per
  `RegistryFunction` `intro`/`desc`/`example` + `Parameter.desc` on the new members in
  `src/codegen/builtins/canvas/`, and the `TextMetrics` `RegistryRecord.description` —
  **not** as `src/docs/man/**` pages and **not** from the retired `.ai/man*_template.md`
  files. Verify by rendering `mfb man canvas measureText` and `mfb man canvas types`;
  a note recording the resolved dependency policy.
- Acceptance: Phases 1–3 run targeted tests only; **Phase 4 is the plan's single full
  `cargo test --no-fail-fast` + `test-accept.sh` run** (A's invariant 8). Text + damage
  goldens pass; fmt.

## Open Decisions

- ~~**The no-dependency bar (this feature's named blocker)**~~ — **RESOLVED 2026-08-31:
  hand-roll, in MFBASIC** (Correction 1). The pre-execution recommendation here was to
  vendor single-header public-domain C; it was not taken, and the reason is recorded on
  the Phase 1 box: a vendored library would have to be per-platform, and that ends the
  cross-target byte-identity of the software render that every GPU backend is gated
  against.
- ~~**stb software-golden determinism**~~ — **moot** with the row above. Determinism is now
  a property of code this repo owns rather than a question about someone else's library:
  the same MFBASIC runs on every target. The residual risk is only that the contour
  rasteriser might use something width- or order-dependent, which the existing
  cross-target byte-identity comparison catches.
- **Damage-rect present at all** — recommended: ship it only if it's cheap on top of D's already-
  computed damage; otherwise leave full-frame (correctness is identical either way). (§Phase 3)
- **Future shaper** — HarfBuzz + FreeType (or MSDF) is a separate future plan; G's `measureText`
  API is designed so that upgrade is non-breaking. (out of scope here)

## Corrections

**2026-08-30 — pre-execution revision (no code written yet).** See plan-98-A's
Corrections for the full account. Applied here: A's invariant 8 (this is new work, so
no codegen byte-identity gate and no full-suite run until the end of the plan); the
per-phase acceptance lines now name targeted tests; and the software rasteriser's
reference images are called **exact-match** rather than "byte-exact goldens", so this
plan's own new oracle is not confused with the repo's `tests/byte-identity/` codegen
drift gate. No design decision changed. This letter cited no paths that moved in the
2026-08-16/17 restructurings, so no remap was needed. G additionally gains a Phase 4 closeout that
owns the plan's single full-suite + acceptance-harness run.

**Correction 1 (Phase 1) — the dependency bar: hand-roll, in MFBASIC.** Decided by the
author on 2026-08-31, against this plan's own pre-execution recommendation to vendor
`stb_truetype`. Three options were put with what each costs; the deciding property is the
oracle. plan-98-F Phase 1 measured the software render **byte-identical across
macOS/Linux and aarch64/x86-64**, and every GPU backend is gated against it — a
per-platform vendored library would end that and force a tolerance onto the text goldens,
weakening the gate the whole feature set rests on.

The third option — compiling `stb_truetype` into `mfb` — was rejected on inspection
rather than on taste: glyph rasterisation happens at *program run time* for arbitrary
strings, and an emitted program has no C toolchain and no CRT, so the compiler could only
bake glyphs it already knows. That is not text rendering.

It is also the option that fits what exists. The canvas rasteriser already *is* MFBASIC:
`__canvas_edgeDistance` walks a polygon's edges for a signed distance and
`__canvas_geoDistance` dispatches the kinds. A TrueType glyph is a set of quadratic
contours, and coverage-from-a-signed-distance is exactly that machinery. The new code is a
`cmap`/`loca`/`glyf` reader and a contour rasteriser; fill, antialiasing and blending are
shared with every other primitive. `canvas::loadImage` rides the same decision — an
inflate and a PNG unfilter, hand-rolled beside the font reader.

**Correction 2 (Prerequisites) — "plan-98-F complete" was the wrong condition.** F Phase 3
(Windows Vulkan) is blocked on **bug-478** — a Windows `--app` program faults before
`main`'s first statement — and G does not touch the Windows Vulkan path. The row now names
what G actually depends on: that F's *backends* render the non-text scene, which software,
Metal and Linux Vulkan all do, each with a command to check it. Corrected rather than
waived: the distinction matters, because a row that cannot be satisfied invites being
ignored.

**Correction 3 (Prerequisites, §2) — the atlas row asked G to require its own output.**
"The shared atlas exists (white pixel + images)" is listed as a precondition and §2 said
"C/E/F built the shared atlas". Neither is true, and both plan-98-E and plan-98-F audited
it and marked *their* atlas rows moot pointing at G. Re-audited at this commit:
`grep -n 'CASE Picture' -A 3 src/codegen/builtins/canvas/helper_geometry.rs` shows
`Picture` returning `__canvas_emptyHeader()` from the header builder and `[]` from the
tail builder, and all four `IMAGE_DIRTY` hits are writes with no reader. Building the
atlas is Phase 1's work.

**Correction 4 (Phase 1) — `ErrBadFontFile` is a new error code, and the reason is that
`ErrNotFound` already existed.** `canvas::loadFont` can fail two ways a caller fixes
differently: the path is wrong, or the file is not a font this build reads. Collapsing
them into one code would send every reader to the wrong fix. `77050022` is the second
row added to the `errorCode` table since the migration, and the table's own
`ADDED_SINCE_MIGRATION` guard names it so the legacy-row count stays checkable.

The accepted set is *TrueType outlines only* — sfnt `0x00010000`, or Apple's `true`
tag — and each refusal is a real file with its own reason: `OTTO` is CFF outlines (a
different curve type and a different rasteriser), `ttcf` is a collection so "the font"
is ambiguous, and `wOFF`/`wOF2` are compressed wrappers. Measured on the development
host: `Helvetica.ttc` is one of these, so the check is not hypothetical.

**Correction 5 (Phase 1) — three backends had to be told about the new members.**
`canvas.loadFont`, `canvas.fontFromBytes`, `canvas.destroyFont` and `canvas.fontRef`
each needed adding to `SUPPORTED_RUNTIME_CALLS` in `macos_aarch64`, `linux_common` and
`win_x86_64`, or `validate_capabilities` rejects the program with "native backend does
not support runtime call" *before codegen*. Worth recording because the failure is at
build time and names the call, so it is easy to fix and easy to forget: the list is
per target, and a member added on one platform's say-so is invisible on the other two
until someone builds for them.

**Correction 6 (Phase 1) — the read-only alias was wrong, and it took a segfault to
show it.** `canvas::fontBytes` first returned the resource's own byte block instead of
a copy, on the reasoning that a font file is hundreds of kilobytes and `measureText`
runs per string per frame, so copying makes measuring cost the font's size rather than
the string's length. The reasoning is right and the code is not: the value is bound to
an ordinary `LET` inside `__canvas_measureText`, and that binding's scope-drop reclaims
the block — so the *second* call on the same font reads what the first one freed. The
first `measureText` printed correct metrics and the next one exited 139.

It now copies, like `canvas::getBytes`. The cost is real and paid deliberately: what
removes it is the glyph cache in Phase 2, which skips the whole read, where an alias
only skipped the copy and handed out a dangling block to do it.

**Correction 7 (Phase 1) — `TextMetrics` already existed, with a different contract.**
plan-98-B declared the record, documenting `descent` as "a positive number" and
`height` as `ascent + descent + lineGap`. The implementation here first followed the
*file's* convention instead — `hhea.descender` is negative and `height` would be
`ascent - descent` — and declared its own record saying so. `DOC_DUPLICATE` caught the
second declaration at build time; the wrong sign would not have been caught by
anything, and produces a height of 60 where 110 belongs, which is a plausible-looking
number that lays text out wrong. The published contract wins, the sign is flipped once
at the boundary, and `descent_is_reported_positive_though_the_file_stores_it_negative`
pins it.

**Correction 8 (Phase 1) — two MFBASIC surprises worth writing down.** `sub` is a
reserved word, so `LET sub AS Integer = …` in the `cmap` reader is a parse error
("Binding name must be an identifier") rather than a shadowing warning. And `DIV` is
*fractional* division language-wide (`.../04_types.md`: "DIV is the explicit Float
escape") — integer division is plain `/` on two `Integer`s, which is the opposite of
what the keyword's name suggests to anyone arriving from Pascal or VB.

**Correction 9 (Phase 1) — the font blob needed a process-global table, and that is the
thing the plan calls "the atlas".** `canvas::loadFont` runs on the **worker**; the
geometry cache that needs a glyph's outline runs on the **graphics thread**. The bytes
are already reachable from both — an arena block is ordinary process memory, and only
the *allocator state* is per-thread (`.ai/canvas-threading.md` §2) — but the graphics
thread has no way to get from the integer a `FontRef` carries to the block. So
`_mfb_rt_canvas_fonts` is a sixteen-slot process-global `handle -> block` map, published
by `loadFont` and cleared by `destroyFont`, and it is a global for exactly the reason
the scene region is one.

A full table is **not** an error: the font still loads and still measures, and only text
*drawn* in it comes out empty — the same thing that happens to a released font. Failing
the load instead would turn a rendering limit into a program-stopping error at a moment
the program cannot predict.

**Correction 10 (Phase 1) — the fill rule is even-odd, and TrueType specifies non-zero.**
`__canvas_edgeDistance` counts crossings. The two agree on every glyph whose counters
are wound opposite their outer contour, which is what the format requires and what
well-made fonts do — verified by rendering `o`, `g` and `@`, whose holes and spiral all
come out right. They differ only where two contours of one glyph *overlap*, which good
fonts avoid because it renders badly everywhere. That is a real if narrow limitation and
it is the price of text sharing one rasteriser with every other primitive rather than
having its own.

**Correction 11 (Phase 1/2) — a text header is not cheap, so the cache had to change.**
`__canvas_geometryFor` builds the header on **every probe**, which is fine when a header
is arithmetic on the item's own fields and doubles as the hash-collision guard. A `Text`
header is not: its bounds and its edge count are properties of the *flattened outlines*,
so building it per frame would re-read `glyf` for every character on screen and the
cache would save nothing. Text therefore probes on the hash alone and builds its header
from the tail on a miss — a narrower collision guard for the one kind that cannot afford
the wide one, rather than a slower cache for every kind.

~~The same fact re-scopes **Phase 2**...~~ — **withdrawn, see Correction 12.** That
paragraph argued a glyph atlas was redundant because the geometry cache already caches a
flattened string. It is wrong, and measuring is what showed it.

**Correction 12 (Phase 1, and it withdraws half of Correction 11) — "text is a polygon"
is correct and, on its own, unusably slow.** Correction 11 concluded that Phase 2's
glyph atlas was redundant because the geometry cache already caches a flattened string.
That reasoning only accounts for the cost of *building* the outline. It ignores the cost
of *evaluating* it, and `__canvas_edgeDistance` is `O(edges)` **per pixel**:

    Andale Mono, size 120           edges   floats
      "Sog@"          4 chars         688     3462
      "Sogsog"        6 chars         949     4767
      "Sogsogsog"     9 chars        1424     7142
      "Sogsogsogsog" 12 chars        1899     9517

About 160 edges per curved glyph. The last row's bounding box is roughly 800x150, so one
frame is ~228 million segment-distance evaluations — and one headless render of it takes
**8.1 seconds** measured on this host. That is not a tuning problem, it is the wrong
shape: a string's cost grows with (its length) x (its area), where a real text renderer's
grows with its length alone.

It also puts text out of reach of the GPU on Metal. `MAX_EDGES` is 256 because a
`setFragmentBytes:` payload is 4 KB, so at ~160 edges a glyph the cap is exceeded by the
**second** curved character and `__canvas_metalRenderable` declines. Vulkan's storage
buffer holds 16384 edges a frame, so it does not hit this — which is itself the argument
for giving Metal a buffer rather than a payload.

So **Phase 2 is the letter's load-bearing phase, not its tidying phase**, and its row
stands as written: rasterise each `(font, size, glyphId)` once into a coverage bitmap,
cache it, and draw a glyph as a blit rather than as a distance field over its own area.
Twelve characters becomes twelve blits instead of 228 million evaluations, and the
per-polygon edge cap stops mattering because a glyph stops being a polygon at draw time.
Phase 1's outline reader is not wasted — it is what fills the cache.

Recorded as its own correction rather than by editing Correction 11, because the
sequence is the point: the re-scope was a plausible inference from a real fact, and the
thing that refuted it was a stopwatch.

**Correction 13 (Phase 2) — the cache had to move to the build side of the seam.** The
obvious place to blit a cached glyph is the draw path, where the pen position is. It is
the wrong place: `__canvas_drawGeometry` owns a live 2.3 MB surface local, and
`collections::set` is in-place only while nothing else allocates underneath it, so a draw
arm that *rasterises* pays the whole-surface copy per write (the 290x trap in
`.ai/collections.md`). Rasterisation therefore happens in `__canvas_textGlyphRun`, at
geometry-build time, and the geometry entry carries **cache entry indices** rather than
glyph ids. The draw arm reads `__CANVAS_GLYPH_META`/`COV` and writes pixels; it allocates
nothing and needs no font.

**Correction 14 (Phase 2) — a glyph run is a kind no shader can draw, so both GPU
predicates must decline it.** The first version of the cache left `__canvas_metalRenderable`
and `__canvas_vulkanRenderable` alone. Metal then accepted a scene whose kind its shader
does not know and returned a frame with the text simply missing — **4,536 pixels wrong,
reported as success**. Both predicates now return `FALSE` on `__CANVAS_GEO_TEXT`, and
`a_scene_containing_text_is_declined_by_the_gpu_and_falls_back_completely` asserts the
fallback is *complete* with `compare_exact` rather than a tolerance: a frame that nearly
matched the oracle would mean the GPU had drawn part of it. Restoring GPU text needs an
atlas texture and a sampler, which is now a Phase 2 row of its own rather than an
unrecorded regression.

**Correction 15 (Phase 2) — `__canvas_hashItem` gave every text item the same hash.**
Text defers its header (a glyph run's bounds are a property of the flattened outlines, so
building one per probe would re-read `glyf` for every character on screen), and a deferred
kind therefore probes the geometry cache **on the hash alone**. But `__canvas_hashItem`
hashed `__canvas_headerFor(item)`, which for `Text` returns `__canvas_emptyHeader()` — the
same empty header for every text item in existence. Every string in a scene collapsed onto
one cache entry and drew as the first string, in the first string's position. A sixty-item
scene drew **one** glyph. Fixed with `__canvas_textHash`, which hashes by hand exactly what
the deferred header would have carried — font, size, position, paint and every codepoint.
This is the cost of the hash-only probe, and it is worth writing down as such: the
optimisation is sound, but it moves an obligation from the header builder to the hasher,
and nothing in the type system says so.

**Correction 16 (Phase 2) — two eviction bugs, both silent, both found by the test that
was written to find them.**

*The stale insert index.* `__canvas_glyphEntry` captured `LET known = len(KEYS)` for its
miss scan and returned `known` as the new entry's index. An eviction pass runs between the
two and renumbers everything that survives, so `known` was stale by exactly the number of
entries that pass dropped. The run then carried an index to an entry that does not exist,
which the blit reads as a zero-sized bitmap and draws as nothing. Six of the 300-item
scene's glyphs vanished, with the cache reporting a healthy hit rate throughout. The index
is now read immediately before the appends that use it.

*The in-flight run.* A run being built is not yet in the geometry cache, so the pin scan
could not see it: the eleventh glyph of a string could evict the first ten — glyphs the
very item under construction was about to draw. `__CANVAS_GLYPH_PINS` publishes the run so
`__canvas_glyphEvict` can pin *and* renumber it. Writing that list exposed a third defect
worth its own line: `PINS = collections::append(PINS, __canvas_glyphEntry(...))` is a
**use-after-free**, because the append resolves `PINS` before the call and the call's
eviction pass reassigns it. The symptom was not a wrong pixel but a dead graphics thread —
a 0%-CPU hang with three threads in `sample` where there should be four. Recorded in
`.ai/collections.md`, because it is a property of the language and not of this cache.

**Correction 17 (Phase 2) — an eviction pass is entitled to free nothing, so it must not
be re-run per insert.** Pinning is absolute, and a scene of `__CANVAS_GEO_CAPACITY` items
can pin the entire cache. Re-running a full compaction on every subsequent insert is then
quadratic in the cache size for a scene we are *required* to keep whole. A pass now defers
the next one until the cache has grown by another half budget. Memory stays bounded because
the pins are: the geometry cache is capped, and a glyph unpins as soon as the item
referencing it leaves that cache — which the test measures, at `glyphs=261` of 300 under
pressure.

**Correction 18 (Phase 2) — the test needed two affordances, and building them was the
work, not a detour.** The glyph cache lives on the **graphics** thread, so a program asking
for its size from `main` asks the worker, whose copies of those globals are its own and
always empty. The counters therefore go on the `MFB_CANVAS_STATS` line, which the graphics
thread writes: `glyphs=`, `glyphBytes=`, `glyphEvictions=`. And a megabyte of the fixture
font is a scene far larger than one that can also be checked pixel by pixel, so
`MFB_CANVAS_GLYPH_BUDGET` shrinks the budget. Without both, the first version of the test
passed **without ever evicting anything** — it was measuring nothing, and said so only
because the stats line let it check.

## Summary

G ships minimum-viable text on a hand-rolled TrueType path across all backends, with a
shaper-independent
`measureText` so a future HarfBuzz/MSDF upgrade is non-breaking, plus glyph-atlas LRU eviction and
optional damage-rect present. The real risks are software-text-golden determinism and never
evicting a live glyph; the named blocker was the vendored-dependency policy, resolved
before code (Correction 1).
With G landed, canvas mode is feature-complete for general 2D (images, shapes, text) on software +
Metal + Vulkan; complex text shaping remains a deliberately-scoped future plan.
