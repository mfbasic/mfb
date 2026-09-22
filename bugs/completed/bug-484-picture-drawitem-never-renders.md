# bug-484: `canvas::Picture` never renders — the documented image item draws nothing on every backend

Last updated: 2026-09-21
Effort: x-large (1d–3d)
Severity: MEDIUM
Class: Correctness

Status: Fixed (see STATUS at the end)
Regression Test: tests/canvas/rt_canvas_picture.rs; tests/canvas/rt_canvas_metal.rs
(`every_picture_path_matches_the_software_oracle`,
`a_space_in_a_text_run_does_not_shift_the_draws_after_it`);
tests/canvas/rt_canvas_graphics_thread.rs (R1, R2, R11); scripts/test-canvas-vulkan.sh

`canvas::Picture` is documented, exported surface: *"An image drawn into a
rectangle, scaled to fit it"* (the `name: "Picture"` record in
`src/codegen/builtins/canvas/mod.rs`). It appears in the man examples of
`func_load_image.rs`, `func_create_image.rs`, `func_set_bytes.rs` and
`func_get_size.rs` (`grep -n 'canvas::Picture\[' src/codegen/builtins/canvas/func_*.rs`),
and `tests/cli/cli_canvas_image_resource.rs` presents one. **No renderer draws it.**
`__canvas_headerFor`'s `Picture` arm returns `__canvas_emptyHeader()` — geometry kind
`NONE` — so `__canvas_drawGeometry` returns immediately; neither GPU emitter has a
picture arm (the shared fragment shader says so outright: *"The empty kind (5) —
`Picture`, which draws nothing until it has an atlas"*, `mfb_canvas.frag`); there is
no texture upload or blit path anywhere
(`grep -rn -i 'GEO_KIND_PICTURE\|drawImage\|drawPicture' src/codegen/builtins/canvas/
src/codegen/runtime/canvas/ src/target/macos_aarch64/app/` → 0 hits, re-run 2026-09-13).
A program that loads an image and presents a `Picture` silently gets background
pixels. The bug is silent — no diagnostic, no error, a plausible blank — which is
what makes it dangerous.

**The single correct behavior a fix produces:** a `Picture` renders its image's
pixels scaled into its destination rectangle on the software, Metal and Vulkan
paths, composited per its `paint` exactly as the landed plan-116-B/C semantics define
for every other item — and all existing non-`Picture` output is byte-identical.

References:

- `src/docs/spec/app/06_canvas.md` §"Images are named, not embedded" — the model
  the fix must respect.
- `.ai/canvas-threading.md` §6 (dirty-texture upload) and §7 (deferred free) —
  written for this feature and still consumer-less; the fix is their first real
  consumer. §7 was rewritten by plan-116-I (the closed-flag-before-handle read order
  through `canvas::imageHandle`).
- `planning/completed/plan-116-B-canvas-blend-and-clip.md` and
  `planning/completed/plan-116-C-canvas-transform.md` — landed; both scope `Picture`
  out citing this bug, so blend/clip/transform for a picture are this fix's design.
- `planning/completed/plan-116-I-canvas-res-handles.md` — landed; changed
  `Picture.image`'s type (see *Re-verification 2026-09-13*).
- `planning/completed/plan-116-J-canvas-group-resource-ownership.md` — landed; a group
  owns the images its items name.
- Found during the 2026-09-01 review of plan-116 (this bug is why the series'
  B/C letters could not say what blend/clip/transform mean for `Picture`).

**Number collision — not this bug.** Commit `781a82f07` ("bug-484: a bare type name
means the LOCAL one, everywhere") and the source comments it seeded reuse this number
for an unrelated, landed resolver fix with no bug document. Every `bug-484` hit in
`src/codegen/builtins/{audio,color,json,net,term,udp}/mod.rs`,
`src/codegen/registry/mod.rs` and `planning/completed/plan-122-*` is that resolver fix,
not `Picture` (`git grep -n 'bug-484' -- src planning`, 2026-09-13). Only the
plan-116-B/C/I hits refer to this document.

## Re-verification 2026-09-13 — `ImageRef` is gone; the bug is not

Reviewed after plan-116-I and plan-116-J landed. Release `mfb` rebuilt at `edb782afe`
(`cargo build --release --bin mfb`) before every run below.

**`ImageRef` / `imageRef` are removed — verified.** plan-116-I Phase 2 (`8a9a9f294`,
"Picture holds a RES canvas::Image, Text a RES canvas::Font") deleted the
`ImageRef`/`FontRef` records and the `imageRef`/`fontRef` functions:

- `Picture.image` is now `ParameterType::res(ParameterType::named("canvas.Image"))`
  (the `name: "Picture"` record in `canvas/mod.rs`), pinned by
  `the_resource_naming_variants_hold_the_resource_itself` in the same file, which
  also asserts none of the four removed names is registered.
- Compiler: `LET r AS canvas::ImageRef = canvas::ImageRef[id := 1]` built with
  `mfb build -app` → `error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER] … Built-in package
  canvas does not export canvas.ImageRef` (likewise `canvas.FontRef`); exit 1.
- Man pages: `mfb man canvas --all | grep -i 'imageref\|fontref'` → 0 hits (2291 lines).
- Remaining `git grep -n 'ImageRef\|imageRef' -- src tests .ai` hits are all history
  comments ("used to", "which these replace") plus that negative assertion; the
  `CGImageRef` hits under `src/target/macos_aarch64/` are CoreGraphics, unrelated.
- The replacement backend-id bridge is `canvas::imageHandle` (`func_handle_bridge.rs`),
  which answers `0` for a closed image — exactly the read the Fix Design needs.

**Consequence for this document:** the 2026-09-01 reproduction no longer compiles —
`mfb build -app --debug` reports `canvas does not export canvas.imageRef` *and*
`canvas does not export canvas.rgb` (`rgb` moved to the `color` package, plan-122).
The reproduction below is rewritten to today's surface. Its original build line also
omitted `--debug`, and `MFB_CANVAS_DUMP` is read only by a `--debug` build (plan-130-E,
`.ai/canvas-threading.md` §11), so as written it could never have produced a dump.

**The bug itself is still live — verified.** With the corrected reproduction, the
headless run exits 0, prints `rendered`, and the dump (2,304,000 bytes = 900×640 RGBA)
has **0** non-background pixels; (116,116), (100,100) and (131,131) all read
`(0, 0, 0, 255)`. The `Rectangle` contrast, built and run identically, reads
`(0, 255, 0, 255)` at (116,116). Source still agrees: `__canvas_headerFor`'s
`CASE Picture(pic)` returns `__canvas_emptyHeader()`, and the only runtime reader of
`p.image` is ownership bookkeeping (`__canvas_closeRetired`,
`__canvas_listNamesImage` in `helper_render.rs`, plan-116-J), not a draw path.

## Failing Reproduction

```
IMPORT app
IMPORT canvas
IMPORT collections
IMPORT color
IMPORT io

SUB main()
  app::setMode(app::Mode.Canvas)
  MUT px AS List OF Byte = []
  px = collections::append(px, toByte(0))
  px = collections::append(px, toByte(255))
  px = collections::append(px, toByte(0))
  px = collections::append(px, toByte(255))
  RES img AS canvas::Image = canvas::createImage(1, 1, px)
  LET tile AS canvas::DrawItem = canvas::Picture[x := 100.0, y := 100.0, w := 32.0, h := 32.0, image := img, paint := canvas::fill(color::rgb(255, 255, 255))]
  canvas::present([tile])
  io::print("rendered")
END SUB
```

Build with `mfb build -app --debug <project>`, run
`build/<name>.app/Contents/MacOS/<name>` headless with `MFB_MACAPP_HEADLESS=1
MFB_CANVAS_SYNC=1 MFB_CANVAS_DUMP=/tmp/f.rgba`, read pixel (116, 116) of the
900-wide RGBA dump (byte offset `(116*900+116)*4`).

- Observed: `(0, 0, 0, 255)` — background; the 32×32 destination rectangle is
  untouched (whole frame: 0 non-background pixels). Last observed 2026-09-13.
- Expected: `(0, 255, 0, 255)` — the 1×1 green image scaled across the rectangle.

Contrast case that works: replace the `Picture` with
`canvas::Rectangle[x := 100.0, y := 100.0, w := 32.0, h := 32.0, paint := canvas::fill(color::rgb(0, 255, 0))]`
— it renders `(0, 255, 0, 255)` at (116,116). Every other `DrawItem` variant renders;
only `Picture` is dead. All three backends are equally affected (the software oracle
itself has no path, and neither `__canvas_metalRenderable` nor
`__canvas_vulkanRenderable` declines the `NONE` kind — the GPU accepts the scene and
draws nothing too), so there is no environment matrix to record.

## Root Cause

plan-98 landed the `Image` **resource** machinery in full — the CPU pixel shadow,
the dirty flag, and a `lastUsedFrame` slot *reserved* for a draw stamp
(`IMAGE_LAST_USED_FRAME` in `gen_image.rs`; its only writer is the zero-initialising
store in `func_create_image.rs` — `git grep -n IMAGE_LAST_USED_FRAME -- src`) — and
the scene **type**, but no rendering phase ever followed. `__canvas_headerFor`
(`helper_geometry.rs`, `CASE Picture(pic)`) deliberately returns the empty `NONE`
header, and every downstream stage correctly draws nothing for `NONE`.
`.ai/canvas-threading.md` §6/§7 document the upload/free protocol the renderer would
use — the design exists; the implementation was never scheduled. plan-116-I changed
what the item *holds* and plan-116-J who *owns* it; neither added a draw. Not a
regression: dead since the variant was declared (`git log --oneline -S
'GEO_KIND_PICTURE'` → none, re-run 2026-09-13).

## Goal

- The reproduction renders green at (116,116) on the software path, and within
  `Tolerance::GPU_DEFAULT` of the oracle on Metal and Vulkan with the GPU path
  proven taken (`MFB_CANVAS_STATS` ready flags).
- Scaling samples the shadow deterministically (nearest — the plan-116-C §4.5
  sampling rule, for the same oracle-reproducibility reason).
- `setBytes` on a presented image redraws per §6 (dirty coalescing), and
  destroy-while-installed draws nothing per §7 — both asserted.

### Non-goals (must NOT change)

- No new `canvas::` surface, no `Picture` field changes (the field type is settled:
  `RES canvas::Image`, plan-116-I).
- No change to plan-116-J's ownership rules (`__canvas_closeRetired`).
- No change to any other variant's output — every existing golden byte-identical.
- The tempting wrong fix, forbidden: making `cli_canvas_image_resource.rs` assert
  around the blank (it currently only checks exit markers, which is HOW this
  stayed invisible) — the fix must add pixel assertions, not avoid them.

## Blast Radius

Re-audited 2026-09-13 (`grep -n 'CASE Picture' src/codegen/builtins/canvas/*.rs`):

- `helper_geometry.rs` — **seven** `CASE Picture(pic)` arms now (header, tail,
  tail-match, deferred-kind, and the plan-116-G group-era arms) — fixed by this bug:
  `Picture` gets a real kind, header (dest rect + handle) and cache participation (its
  hash must include the image id and the shadow's dirty generation, or a `setBytes`
  won't invalidate).
- `__canvas_drawGeometry` (`helper_items.rs`) — fixed: a blit arm like `TEXT`'s.
- Both GPU emitters + shaders — fixed: first real texture path (§6's consumer);
  the *large* half of the work. The `mfb_canvas.frag` "empty kind (5)" comment
  changes with it.
- `__canvas_metalRenderable` / `__canvas_vulkanRenderable` (`helper_render.rs`) —
  fixed: they currently accept pictures (as `NONE`); Phase 2 makes them decline.
- `IMAGE_LAST_USED_FRAME` (`gen_image.rs`) — fixed: the reserved stamp gains its
  writer, activating §7's free gate for real.
- `__canvas_closeRetired` / `__canvas_listNamesImage` (`helper_render.rs`,
  plan-116-J) — unaffected but adjacent: a group may close an image the new draw path
  samples, so the §7 "closed flag before handle" read order is load-bearing for the
  blit.
- plan-116-B/C picture semantics — unaffected until this lands (both completed
  letters scope `Picture` out, citing this bug).
- `cli_canvas_image_resource.rs` — fixed: gains pixel assertions.

## Fix Design

Software first (it is the oracle): new geometry kind `PICTURE`, header carrying
the dest rect and image handle (read with `canvas::imageHandle`, which yields `0` for
a closed image — draw nothing); the draw arm computes, per pixel, the source texel by
nearest sampling of the shadow (`IMAGE_PIXELS`) scaled `w/h` → image extent,
multiplied through the existing paint/coverage machinery so
`Paint.blend`/`clip`/`transform` (as landed by plan-116-B/C) apply uniformly. GPU
second: the §6 upload protocol (dirty → upload once per frame → clear), a sampled
texture bound per run — noting the plan-116-A instancing constraint: a texture bind
is per-draw state, so a `Picture` ends an instanced run exactly as `Text` does, OR
pictures batch into an atlas — decide by measurement in the fix, not here.
Correctness risk concentrates in the §6/§7 threading protocol (upload racing
`setBytes`, free racing an in-flight frame); schedule GPU last, behind the oracle
tests. Rejected: rendering pictures only in software and declining on GPU forever —
it makes `MFB_CANVAS_GPU=1` silently slower for any scene with an image, the exact
false-comfort `.ai/canvas-threading.md` §10 warns about; the predicates may
decline pictures only until the GPU phases land.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `tests/canvas/rt_canvas_picture.rs` with the reproduction as a failing pixel
      test (software path; `common::build_app_debug`, `MFB_CANVAS_SYNC=1`).
- [x] Re-verify the blast-radius audit above at fix time; write verdicts here.

Reproduced 2026-09-21 at `4e0c50a8b` exactly as documented (exit 0, `rendered`, 0
non-background pixels, (116,116) = `(0,0,0,255)`). RED at pristine HEAD in a detached
checkout: 8 of 9 fail reading background where the picture belongs; the destroyed-image
test passes vacuously and stays as a guard.

Blast-radius verdicts: the seven `CASE Picture` arms in `helper_geometry.rs` — header
(now `__canvas_pictureHeader`), tail (`[]`, correct: nothing rides a tail),
`tailMatches` (`TRUE`, correct: the header carries the pixel-block address),
`headerIsDeferred` (`FALSE`), `deferredHeader`/`deferredHash` (unreachable for a
non-deferred kind), `hashItem` (the header hash already folds the address). Draw arm,
both predicates and both emitters as below. `IMAGE_LAST_USED_FRAME`: removed, not
stamped (see Deviations). `__canvas_closeRetired`/`__canvas_listNamesImage`: unchanged.

Acceptance: the new test fails for the documented reason.
Commit: bbfacf3e6

### Phase 2 — the software blit (the oracle defines the semantics)

- [x] Kind, header, cache arms (incl. hash/tailMatches per the landed polygon
      pattern), draw arm, `setBytes` dirty-generation invalidation.
- [x] Both `*Renderable` predicates decline scenes with pictures (honesty gate)
      until Phase 3.

Acceptance: Phase 1's test passes; all existing goldens byte-identical; a
`setBytes`-then-present test shows the new pixels.
Commit: 6858eafce (tests for R1/R2/R9/R11 and the image-contract pixel check: 250ffa9af)

### Phase 3 — Metal, then Vulkan textures + full validation

- [x] §6 upload path, per-backend; run-break or atlas decision recorded with a
      measurement; predicates accept pictures again.
- [x] `IMAGE_LAST_USED_FRAME` stamped; a destroy-mid-frame race test in
      `tests/canvas/rt_canvas_graphics_thread.rs`. *(Deviation: removed rather than
      stamped — see Deviations. The race tests landed.)*
- [x] Full suite, `scripts/test-accept.sh`, `scripts/artifact-gate.sh all`,
      `scripts/test-canvas-vulkan.sh`; regenerate `.ncodesum`.

Acceptance: reproduction passes on all three paths with the GPU proven taken;
full suite green on both axes.
Commit: 769bd0785 (shared prep), aa4ce9458 (Vulkan), 4a1c39c8e (Metal),
6c6502967 (goldens)

### Phase 3b — a blank glyph shifted every later GPU draw (found during Phase 3)

- [x] RED on both backends: `a_space_in_a_text_run_does_not_shift_the_draws_after_it`
      (Metal, pre-fix build: 6960 px differ, worst 235 — the group's dot drawn at the
      origin) and a spaced text run added to `test-canvas-vulkan.sh`'s main scene
      (pre-fix: worst=255 differing=3.04%).
- [x] Fix both emitters: a glyph that draws nothing still publishes a zero-size block.

Not a Picture bug, and pre-existing: both `emit_glyph_publish`s skipped the block for an
evicted entry, an empty bitmap (a space) or one over the region cap, while
`__canvas_blockInstances` counts every glyph — so every draw entry after a text run
containing a space named its neighbour's block (wrong pipeline, wrong group offset, the
last reading a block never written). The Metal picture scene's label "Pictures 01" is
what exposed it; `TEXT_LINE` has spaces but nothing after its runs with a draw state of
its own, and the Vulkan gate's only label "AAAA" has none.
Commit: d49023713 (tests), 4a1c39c8e (Metal), 7ae9c31fa (Vulkan)

## Validation Plan

- Regression tests: `tests/canvas/rt_canvas_picture.rs`; pixel assertions added to
  `tests/cli/cli_canvas_image_resource.rs`.
- Runtime proof: the reproduction under `MFB_CANVAS_DUMP` (a `--debug` build) on a
  Metal host and a Vulkan box, diffed against the oracle.
- Doc sync: `mod.rs` `Picture` description gains nothing (it was always written
  as if this worked); `.ai/canvas-threading.md` §6/§7 gain "implemented by
  bug-484" notes; the `mfb_canvas.frag` empty-kind comment is updated;
  plan-116-B/C's `Picture` scope-outs get closure notes.
- Full suite: `cargo test --no-fail-fast` (both axes), `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all`.

## Open Decisions

- **Sequencing** — resolved 2026-09-13: plan-116-I (field type + `imageHandle`
  bridge) and plan-116-J (group ownership) have both landed, so the fix is written
  once against the final `RES canvas::Image` field; nothing blocks it.
- **Per-picture texture bind (run break) vs atlas** — measure in Phase 3;
  recommend run-break first (correct and simple), atlas only with a measured
  scene that needs it.

## Summary

The risk is not the blit — it is the first-ever exercise of the §6/§7 texture
threading protocol, which has been documentation without a consumer since
plan-98-D. The software arm is small and lands first as the oracle; the GPU
halves follow behind pixel tests. Untouched: every other variant, the `Image`
resource surface, plan-116-I's field type, and plan-116-J's ownership rules.

## STATUS: FIXED (6c6502967, merged to main from worktree-B-484)

A `Picture` renders on the software, Metal and Vulkan paths: its geometry is a
rectangle's re-kinded to 9, so coverage, stroke, clip, blend, transform and group offset
are the rectangle's; the fill colour is the image sampled nearest and tinted by
`Paint.fill` (white = unchanged, the fill's alpha = opacity; a gradient is ignored).

### Deviations from the Fix Design

- **No texture object, so no §6/§7 protocol.** The "risk" the Summary names never
  materialised because the design avoided it. The header carries the image's pixel-block
  address (split in two 24-bit halves — the header hash would overflow a whole address);
  the software path samples that block through the allocation-free
  `canvas::shadowTexel`, and both GPU emitters copy its texels, one packed word each,
  into the frame buffer's **glyph region** with the block naming its slice. The
  run-break vs atlas question dissolved: a picture is one quad inside the instanced run,
  and the region is the atlas. Safe because `setBytes` swaps a fresh block in and no
  block or image record is ever freed; the address doubles as the content generation
  (cache, damage diff and GPU copy all see a `setBytes`).
- **`IMAGE_LAST_USED_FRAME` removed, not stamped** — and `IMAGE_DIRTY` with it. With no
  GPU object there is nothing to free and nothing to upload lazily, so both words would
  have been written for no reader.
- **`setBytes` gained redraw trigger 5** (`.ai/canvas-threading.md` §4) — an
  unlisted sub-issue: nothing signalled a frame after `setBytes`, and re-presenting the
  unchanged scene was frame-skipped, so the documented "appears on the next rendered
  frame" never happened. `setBytes` is now an MFBASIC wrapper over the native
  `canvas::setBytesRaw` that signals (and waits under `MFB_CANVAS_SYNC`) only when the
  installed items, layers or a group name the image (R10 holds).
- **Phase 3b** (above): the pre-existing blank-glyph block skip on both emitters.
- **The Vulkan gate never checked `gpuFrames` for its main scene**, so a declined frame
  compared software with itself; it does now.
- **`.ai/testing-gates.md` was wrong** that no gate fixture covers canvas:
  `tests/syntax/app/app-mouse-surface` does, and its seven goldens were regenerated
  after the IR delta was localized to exactly this change.

### Validation (2026-09-21)

- `cargo test --no-fail-fast`: 215 test binaries ok; the one failure,
  `artifact_gate_all`, was the app-mouse-surface goldens above — after regeneration
  `scripts/artifact-gate.sh all`: 2092 goldens, 0 diffs.
- `scripts/test-accept.sh`: 1505 of 1506 passed; the one mismatch, `acceptance`, was the harness timeout under load average ~100 (log: `timeout`, exit 99; the project imports neither canvas nor app) and passed on its own re-run (3m08s).
- `cargo test --test rt_canvas_picture` 10/10, `rt_canvas_graphics_thread` 11 passed (2 pre-existing ignores) incl. R1/R2/R11,
  `rt_canvas_metal` 10/10 (picture scene band: 0 differing px), `cli_canvas_image_resource`
  4/4.
- `scripts/test-canvas-vulkan.sh` all ok on box 2228 (glibc) and 2227 (musl, `--icd
  auto`): worst=2 differing=0.8177%, gpuFrames=1.
- The reproduction: (116,116) = `(0, 255, 0, 255)`, exactly the 32×32 destination lit.
