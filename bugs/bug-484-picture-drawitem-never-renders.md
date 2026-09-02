# bug-484: `canvas::Picture` never renders — the documented image item draws nothing on every backend

Last updated: 2026-09-01
Effort: x-large (1d–3d)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/rt_canvas_picture.rs (to be added by Phase 1)

`canvas::Picture` is documented, exported surface: *"An image drawn into a
rectangle, scaled to fit it"* (`src/codegen/builtins/canvas/mod.rs:649`), it
appears in `canvas::present`'s own ecosystem of man examples
(`func_load_image.rs:38`, `func_create_image.rs:46`, `func_set_bytes.rs:42`,
`func_get_size.rs:45`), and `tests/cli_canvas_image_resource.rs:119` presents
one. **No renderer draws it.** `__canvas_headerFor`'s `Picture` arm returns
`__canvas_emptyHeader()` — geometry kind `NONE` — so `__canvas_drawGeometry`
returns immediately; neither GPU emitter has a picture arm; there is no texture
upload or blit path anywhere
(`grep -rn 'GEO_KIND_PICTURE\|drawImage\|drawPicture' src/codegen/builtins/canvas/
src/codegen/runtime/canvas/ src/target/macos_aarch64/app/` → 0 hits, 2026-09-01).
A program that loads an image and presents a `Picture` silently gets background
pixels. The bug is silent — no diagnostic, no error, a plausible blank — which is
what makes it dangerous.

**The single correct behavior a fix produces:** a `Picture` renders its image's
pixels scaled into its destination rectangle on the software, Metal and Vulkan
paths, composited per its `paint` exactly as the (by-then-landed) plan-116
semantics define for every other item — and all existing non-`Picture` output is
byte-identical.

References:

- `src/docs/spec/app/06_canvas.md` §"Images are named, not embedded" — the model
  the fix must respect.
- `.ai/canvas-threading.md` §6 (dirty-texture upload) and §7 (deferred free) —
  written for this feature and currently consumer-less; the fix is their first
  real consumer.
- `planning/plan-116-B/C` — blend/clip/transform semantics a picture must obey;
  both letters cite this bug as the reason `Picture` is scoped out.
- `planning/plan-116-I-canvas-res-handles.md` — changes `Picture.image`'s type;
  this fix should land AFTER it so the sampling path is written once, against the
  final field type.
- Found during the 2026-09-01 review of plan-116 (this bug is why the series'
  B/C letters could not say what blend/clip/transform mean for `Picture`).

## Failing Reproduction

```
IMPORT app
IMPORT canvas
IMPORT collections
IMPORT io

SUB main()
  app::setMode(app::Mode.Canvas)
  MUT px AS List OF Byte = []
  px = collections::append(px, toByte(0))
  px = collections::append(px, toByte(255))
  px = collections::append(px, toByte(0))
  px = collections::append(px, toByte(255))
  RES img AS canvas::Image = canvas::createImage(1, 1, px)
  LET tile AS canvas::DrawItem = canvas::Picture[x := 100.0, y := 100.0, w := 32.0, h := 32.0, image := canvas::imageRef(img), paint := canvas::fill(canvas::rgb(255, 255, 255))]
  canvas::present([tile])
  io::print("rendered")
END SUB
```

Build with `mfb build -app`, run headless with `MFB_MACAPP_HEADLESS=1
MFB_CANVAS_SYNC=1 MFB_CANVAS_DUMP=/tmp/f.rgba`, read pixel (116, 116).

- Observed: `(0, 0, 0, 255)` — background; the 32×32 destination rectangle is
  untouched.
- Expected: `(0, 255, 0, 255)` — the 1×1 green image scaled across the rectangle.

Contrast case that works: replace the `Picture` with
`canvas::Rectangle[x := 100.0, y := 100.0, w := 32.0, h := 32.0, paint := …]` —
it renders. Every other `DrawItem` variant renders; only `Picture` is dead. All
three backends are equally affected (the software oracle itself has no path), so
there is no environment matrix to record.

## Root Cause

plan-98 landed the `Image` **resource** machinery in full — the CPU pixel shadow,
the dirty flag, and a `lastUsedFrame` slot *reserved* for a draw stamp
(`gen_image.rs:32-64`) — and the scene **type** (`mod.rs:647`), but no rendering
phase ever followed. `__canvas_headerFor` (`helper_geometry.rs`, `CASE
Picture(pic)`) deliberately returns the empty `NONE` header, and every downstream
stage correctly draws nothing for `NONE`. `.ai/canvas-threading.md` §6/§7
document the upload/free protocol the renderer would use — the design exists;
the implementation was never scheduled. Not a regression: dead since the variant
was declared (no commit ever referenced a picture draw path — `git log
--oneline -S 'GEO_KIND_PICTURE'` → none).

## Goal

- The reproduction renders green at (116,116) on the software path, and within
  `Tolerance::GPU_DEFAULT` of the oracle on Metal and Vulkan with the GPU path
  proven taken (`MFB_CANVAS_STATS` ready flags).
- Scaling samples the shadow deterministically (nearest — the plan-116-C §4.5
  sampling rule, for the same oracle-reproducibility reason).
- `setBytes` on a presented image redraws per §6 (dirty coalescing), and
  destroy-while-installed draws nothing per §7 — both asserted.

### Non-goals (must NOT change)

- No new `canvas::` surface, no `Picture` field changes (plan-116-I owns the
  field type).
- No change to any other variant's output — every existing golden byte-identical.
- The tempting wrong fix, forbidden: making `cli_canvas_image_resource.rs` assert
  around the blank (it currently only checks exit markers, which is HOW this
  stayed invisible) — the fix must add pixel assertions, not avoid them.

## Blast Radius

- `__canvas_headerFor` / `__canvas_tailFor` / hash arms (`helper_geometry.rs`) —
  fixed by this bug: `Picture` gets a real kind, header (dest rect + handle) and
  cache participation (its hash must include the image id and the shadow's dirty
  generation, or a `setBytes` won't invalidate).
- `__canvas_drawGeometry` (`helper_items.rs`) — fixed: a blit arm like `TEXT`'s.
- Both GPU emitters + shaders — fixed: first real texture path (§6's consumer);
  the *large* half of the work.
- `IMAGE_LAST_USED_FRAME` (`gen_image.rs:60`) — fixed: the reserved stamp gains
  its writer, activating §7's free gate for real.
- plan-116-B/C picture semantics — unaffected until this lands (both letters
  scope `Picture` out, citing this bug).
- `cli_canvas_image_resource.rs` — fixed: gains pixel assertions.

## Fix Design

Software first (it is the oracle): new geometry kind `PICTURE`, header carrying
the dest rect and image handle; the draw arm computes, per pixel, the source
texel by nearest sampling of the shadow (`IMAGE_PIXELS`, read via the plan-116-I
bridge once that lands) scaled `w/h` → image extent, multiplied through the
existing paint/coverage machinery so `Paint.blend`/`clip`/`transform` (as landed
by plan-116-B/C) apply uniformly. GPU second: the §6 upload protocol (dirty →
upload once per frame → clear), a sampled texture bound per run — noting the
plan-116-A instancing constraint: a texture bind is per-draw state, so a
`Picture` ends an instanced run exactly as `Text` does, OR pictures batch into an
atlas — decide by measurement in the fix, not here. Correctness risk
concentrates in the §6/§7 threading protocol (upload racing `setBytes`,
free racing an in-flight frame); schedule GPU last, behind the oracle tests.
Rejected: rendering pictures only in software and declining on GPU forever — it
makes `MFB_CANVAS_GPU=1` silently slower for any scene with an image, the exact
false-comfort `.ai/canvas-threading.md` §10 warns about; the predicates may
decline pictures only until the GPU phases land.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/rt_canvas_picture.rs` with the reproduction as a failing pixel
      test (software path).
- [ ] Re-verify the blast-radius audit above at fix time (plan-116 letters will
      have moved these files); write verdicts here.

Acceptance: the new test fails for the documented reason.
Commit: —

### Phase 2 — the software blit (the oracle defines the semantics)

- [ ] Kind, header, cache arms (incl. hash/tailMatches per the landed polygon
      pattern), draw arm, `setBytes` dirty-generation invalidation.
- [ ] Both `*Renderable` predicates decline scenes with pictures (honesty gate)
      until Phase 3.

Acceptance: Phase 1's test passes; all existing goldens byte-identical; a
`setBytes`-then-present test shows the new pixels.
Commit: —

### Phase 3 — Metal, then Vulkan textures + full validation

- [ ] §6 upload path, per-backend; run-break or atlas decision recorded with a
      measurement; predicates accept pictures again.
- [ ] `IMAGE_LAST_USED_FRAME` stamped; a destroy-mid-frame race test in
      `tests/rt_canvas_graphics_thread.rs`.
- [ ] Full suite, `scripts/test-accept.sh`, `scripts/artifact-gate.sh all`,
      `scripts/test-canvas-vulkan.sh`; regenerate `.ncodesum`.

Acceptance: reproduction passes on all three paths with the GPU proven taken;
full suite green on both axes.
Commit: —

## Validation Plan

- Regression tests: `tests/rt_canvas_picture.rs`; pixel assertions added to
  `tests/cli_canvas_image_resource.rs`.
- Runtime proof: the reproduction under `MFB_CANVAS_DUMP` on a Metal host and a
  Vulkan box, diffed against the oracle.
- Doc sync: `mod.rs` `Picture` description gains nothing (it was always written
  as if this worked); `.ai/canvas-threading.md` §6/§7 gain "implemented by
  bug-484" notes; plan-116-B/C's `Picture` scope-outs get closure notes.
- Full suite: `cargo test --no-fail-fast` (both axes), `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all`.

## Open Decisions

- **Sequencing** — recommended: land after plan-116-I (field type settles; the
  bridge exists) and before plan-116-J (groups owning images deserve images that
  draw). Alternative: fix immediately on today's `ImageRef` and rework the read
  after I — double work for no user-visible gain.
- **Per-picture texture bind (run break) vs atlas** — measure in Phase 3;
  recommend run-break first (correct and simple), atlas only with a measured
  scene that needs it.

## Summary

The risk is not the blit — it is the first-ever exercise of the §6/§7 texture
threading protocol, which has been documentation without a consumer since
plan-98-D. The software arm is small and lands first as the oracle; the GPU
halves follow behind pixel tests. Untouched: every other variant, the `Image`
resource surface, and plan-116-I's field migration.
