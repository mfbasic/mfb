# plan-98-G: Canvas text, atlas eviction, measureText, damage-rect present

Last updated: 2026-08-30
Effort: large (3h–1d) — the stb path only; a real shaper (HarfBuzz) is a separate future plan
Depends on: plan-98-F (Vulkan backend — all three GPU backends + software render text)

This sub-plan adds **text rendering** (stb_truetype path), glyph-atlas LRU eviction,
`measureText`/`TextMetrics` from day one, and optional damage-rect presentation. After it
lands, Text `DrawItem`s render on all backends (software exact-match golden, GPU within
tolerance), glyphs are packed into the shared atlas on demand and evicted under pressure,
and `canvas::measureText` returns metrics whose API is shaper-independent so a future
HarfBuzz/FreeType upgrade doesn't change the surface.

This is **build step 7** of the A–G sequence — text, the hard part. It ships the *minimum viable*
stb path, not full shaping.

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
| plan-98-F complete (all GPU backends render the non-text scene) | `ls planning/completed/plan-98-F-*` → hit | NOT MET |
| The shared atlas exists (white pixel + images) | plan-98-C/E/F atlas in place | NOT MET |
| The vendored-dependency policy is decided | see Open Decisions / this feature's blocker | UNVERIFIED (resolve first) |
| Working tree builds | `cargo build` → pass | UNVERIFIED (run before starting) |

> Per A's invariant 8: no "full suite green at HEAD" row and no byte-identity
> obligation.

> The **vendored-single-header policy** is a real gate: it decides stb_truetype vs a
> hand-rolled rasteriser vs MSDF. Resolve it before Phase 1 (it is this feature's named open
> question). Treated as a precondition, not scope.

## 1. Goal

- Render Text `DrawItem`s: shape (stb: positioning only, no complex shaping) and rasterise
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

- **No complex text shaping.** stb path only: no kerning, ligatures, bidi, Arabic/Indic shaping,
  emoji, or subpixel positioning — all explicitly out of scope for the stb path. A real
  shaper (HarfBuzz + FreeType) or MSDF is a separate future plan; the `measureText`/`TextMetrics`
  API is designed so that upgrade is non-breaking.
- **No API change** to the frozen `DrawItem` set — Text is already a variant (B).
- **Damage-rect present is optional** and must degrade to full-frame where the platform lacks it;
  it never changes visible output, only present efficiency.
- Software backend stays exact-match for text goldens (deterministic stb raster).

## 2. Current State

- **B** froze Text as a `DrawItem` variant and reserved the `measureText`/`TextMetrics` and
  `canvas::loadFont` signatures; **C/E/F** built the shared atlas (white pixel + images) and left
  Text rendering stubbed. **D** computes positional-diff damage but has no consumer (deferred here).
- **Font is a RES resource** (from B): a `Font` id with closed-flag lifetime, freed by the same
  closed + frame-drain rule as Image — no refcount. Same lifetime machinery as Image.
- **The no-dependency bar is unresolved** (see this letter's Open Decisions): "vendorable single-header
  public-domain C" (stb_truetype) vs "not one line of third-party code" (hand-rolled) vs a
  build-time MSDF step (msdfgen). This decides Phase 1's approach and must be settled first.

### Measured populations

| What | Count | Command |
|---|---|---|
| Glyph cache key | `(font, size, codepoint)` | this plan's §3 — a decision, not a measurement |
| Text features in scope (stb) | positioning + per-glyph raster only | this letter's Non-goals list above |
| Damage consumers to wire | 2 (Vulkan incremental present, Metal dirty rects) | this plan's Phase 3 — one per GPU backend (E, F) |

### Verified properties

- **stb_truetype is single-header, public-domain, vendorable** — this is a claim about
  the upstream library, **UNVERIFIED here**: Phase 1 must check the actual header's
  licence and self-containedness before vendoring anything. Whether it clears *this
  project's* no-dependency bar is a separate, unresolved policy question (Prerequisite).
- **`measureText` from day one makes the shaper swappable** — a design decision of this
  plan set, not a verified property. G ships the API even though stb underlies it, so a
  later HarfBuzz swap is non-breaking. It is only true if `TextMetrics` stays
  shaper-independent; Phase 1 must keep stb-specific fields out of it.
- UNVERIFIED: that stb glyph rasterisation is deterministic enough for exact-match software
  goldens across machines. Phase task pins it (fixed hinting/rounding) or documents a text-golden
  tolerance for the software path too.

## 3. Design Overview

- **Glyph pipeline (in `present()`, invariant 1).** stb shape (positioning) + rasterise each
  needed `(font,size,codepoint)` into the atlas on demand; cache with LRU; emit tinted quads.
  Shaping is per-string, per-`present()` — so a per-frame-changing string re-shapes each present
  (charged to the worker budget, invariant 2); the glyph *raster* cache absorbs the per-glyph
  cost.
- **`measureText`/`TextMetrics`.** Compute metrics via the same stb path; return the reserved
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
stb raster proves machine-variant) and within tolerance on GPU. Damage-rect present must produce
**identical visible output** to full-frame (verified by golden equality with damage on vs off).

**Rejected alternatives (for this sub-plan):**
- *HarfBuzz + FreeType now.* Deferred: months of work; the design ships stb first with a
  shaper-independent `measureText` API so the upgrade is non-breaking later.
- *MSDF now.* Deferred: kills the per-size cache explosion but needs a build-time msdfgen step;
  revisit when text scale demands it. The `measureText` API is unaffected by the choice.

## Compatibility / Format Impact

- **Changes:** Text rendering + glyph atlas + `measureText`; optional damage-rect present. New
  text-golden corpus. A vendored single-header font library (pending the policy decision).
- **Unchanged:** the frozen `DrawItem` set, the scene model, thread/ring/retirement, the GPU
  backends' pipeline, and visible output under damage-rect present (efficiency only).

## Phases

### Phase 1 — Resolve the dependency policy; stb text render + measureText

- [ ] Resolve the vendored-single-header policy (Prerequisite/Open Decision) and record it.
- [ ] Integrate the chosen font rasteriser (stb path); implement Text rendering: shape
      (positioning) + per-`(font,size,codepoint)` glyph raster into the atlas + tinted quads.
- [ ] Implement `canvas::loadFont` (Font RES resource via B's backend) and `canvas::measureText`
      → `TextMetrics` (reserved API).
- [ ] Tests: a text scene renders exact-match on the software oracle (or documented text
      tolerance); `measureText` returns expected metrics; Text renders within tolerance on
      Metal + Vulkan.

Acceptance: Text `DrawItem`s render on all backends (software exact-match/documented-tolerance, GPU
within tolerance); `measureText` works and its API is shaper-independent.
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
- Doc sync: `src/docs/spec/app/` canvas text section (stb scope + limits, `TextMetrics`, the
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

- **The no-dependency bar (this feature's named blocker)** — recommended: **allow vendored
  single-header public-domain C** (stb_truetype), documenting the distinction from "zero
  third-party code". This unblocks minimum-viable text without a package manager or shared lib.
  Resolve before Phase 1. (§Prerequisites)
- **stb software-golden determinism** — recommended: pin stb hinting/rounding for exact-match
  text goldens; if machine-variant, document a small text-golden tolerance for the software path.
  (§Phase 1)
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

<Further corrections filled in during execution — especially the resolved dependency
policy and any text-golden determinism findings.>

## Summary

G ships minimum-viable text on the stb path across all backends, with a shaper-independent
`measureText` so a future HarfBuzz/MSDF upgrade is non-breaking, plus glyph-atlas LRU eviction and
optional damage-rect present. The real risks are software-text-golden determinism and never
evicting a live glyph; the named blocker is the vendored-dependency policy, resolved before code.
With G landed, canvas mode is feature-complete for general 2D (images, shapes, text) on software +
Metal + Vulkan; complex text shaping remains a deliberately-scoped future plan.
