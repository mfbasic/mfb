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
| plan-98-F's **backends** render the non-text scene | `scripts/test-canvas-vulkan.sh <exe> --box 2228 --libc glibc` and `--box 2227 --libc musl --icd auto`; `cargo test --test rt_canvas_metal` | **MET** — software, Metal and Linux Vulkan all render the full primitive set within tolerance. The row used to read "plan-98-F complete", which is a *different* claim: F Phase 3 (Windows Vulkan) is blocked on bug-477 and G does not touch it. Corrected in place rather than waived — see Correction 2. |
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
- [ ] Build the chosen font rasteriser (hand-rolled, Correction 1); implement Text rendering: shape
      (positioning) + per-`(font,size,codepoint)` glyph raster into the atlas + tinted quads.
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
- [ ] Tests: a text scene renders exact-match on the software oracle (or documented text
      tolerance); `measureText` returns expected metrics; Text renders within tolerance on
      Metal + Vulkan.

Acceptance: Text `DrawItem`s render on all backends (software exact-match/documented-tolerance, GPU
within tolerance); `measureText` works and its API is shaper-independent; a `Font`
loads, is named by a `FontRef` in a `Text` item, and is released by
`canvas::destroyFont` and by scope-drop; `canvas::loadImage` decodes a real file to
the same `Image` resource `canvas::createImage` produces.
Commit: —

### Phase 2 — Glyph atlas LRU eviction

- [ ] Evict glyphs by last-used revision under atlas pressure; pin glyphs referenced by a live
      scene (never evict an in-use glyph).
- [ ] Tests: forcing a small atlas evicts least-recently-used glyphs; a live-scene glyph is never
      evicted; re-rendering a scene after eviction re-rasters cleanly (golden unchanged).

Acceptance: atlas eviction is LRU and never evicts a live glyph; output is golden-stable across
eviction cycles.
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
(Windows Vulkan) is blocked on **bug-477** — a Windows `--app` program faults before
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

## Summary

G ships minimum-viable text on a hand-rolled TrueType path across all backends, with a
shaper-independent
`measureText` so a future HarfBuzz/MSDF upgrade is non-breaking, plus glyph-atlas LRU eviction and
optional damage-rect present. The real risks are software-text-golden determinism and never
evicting a live glyph; the named blocker was the vendored-dependency policy, resolved
before code (Correction 1).
With G landed, canvas mode is feature-complete for general 2D (images, shapes, text) on software +
Metal + Vulkan; complex text shaping remains a deliberately-scoped future plan.

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
