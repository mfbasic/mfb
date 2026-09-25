# bug-688: The Vulkan canvas backend is still at pre-bug-686 Metal: ordinary scenes fall to software, large polygons are O(edges × pixels), pictures copy per occurrence, every frame is read back

Last updated: 2026-09-24
Effort: huge (>3d)
Severity: HIGH
Class: Correctness / Performance

Status: V1-V4 FIXED; V5/V6 BLOCKED on hardware (no reachable Vulkan GPU)
Regression Test: `scripts/test-canvas-gpu-rows.sh` (the bug-686 rows and the geo-verify rows, per target); `scripts/test-canvas-vulkan.sh --target`; the `vulkan.rs` unit tests `the_pipeline_specializes_every_region_base` and `the_shared_buffer_holds_every_region`; `both_predicates_count_each_picture_block_once`; `rt_canvas_metal` rows for the shared scenes

## STATUS: V1-V4 FIXED, V5/V6 BLOCKED

Landed on `worktree-B-688` and merged to `main`. What holds, with the commands behind it:

- **V1 (no ordinary scene falls to software).** Vulkan's caps are Metal's (65,536 quads,
  262,144 edges, 131,072 stops, 8M texels), generated into the predicate, and the shader's
  region bases are specialization constants (`937ddc149`).
  `scripts/test-canvas-gpu-rows.sh <mfb> --target linux-aarch64` (box 2226) and
  `--target windows-x86_64` (box 2230): every render row draws on Vulkan
  (`gpuFrames=1`) within `GPU_DEFAULT`: quads worst 0, 40k opaque edges worst 1 /
  1.09%, gradients 0, tilemap 0, 1080p background 0, large polygons 1 / 0.26%,
  64k polygon 1 / 0.03%, layers 1 / 0.84%. At `main` the first six were declined
  (`gpuFrames=0`).
- **V2 (band index).** `05c5ffa81`. The GPU frame with bands against the full loop on
  the same device (2226), byte-compared: identical for large_polygons, huge_polygon,
  forty_thousand_opaque_edges and a worst-case probe ({4001/1999} star, 4,000 spikes, a
  stroked 3,000-edge ring under a shearing transform).
- **V3 (pictures once per block).** `937ddc149`: the per-frame picture table and the
  distinct-block predicate count; the tilemap row (2,000 tiles, two images) draws on
  Vulkan.
- **V4 (shared native paths on x86-64 and AArch64).** `MFB_CANVAS_GEO_VERIFY=1` on every
  row and the four `rt_canvas_geo_native` programs: `geoVerifyMismatches=0` on
  linux-aarch64 (2226) and windows-x86_64 (2230), including the publish race (2226: 15
  frames on the GPU; 2230: 3 frames, software — the GPU run is too slow there to race,
  see `df4f589a7`). **Not run on linux-x86_64** (2228/2227): this fix was limited to
  boxes 2226 and 2230. x86-64 code generation is covered by 2230 (same ISA, Win64 ABI).
- **A bug found and fixed on x86-64** (`069348f8b`): `emit_state_load`'s temporary vreg
  could be coloured onto a live hand-assigned `SCRATCH` register; the band index faulted
  on Windows (`0xC0000005`). The helper now uses no temporary.
- **V5 / V6 BLOCKED.** Every reachable Vulkan driver is Mesa's CPU lavapipe
  (`.ai/remote_systems.md`), so no frame-rate or present-cost number here would describe a
  GPU. Per Open Decisions ("Hardware", recommended), no swapchain was built on a guess.
  The instrument is ready: `tools/canvas-bench/bench.sh --target linux-… --box <port>`
  (`bd49be0d0`). What remains is Phase 4, on a machine with a Vulkan GPU.

Deviations:

- **The Vulkan 40k-edge row uses `forty_thousand_opaque_edges.mfb`.** The translucent
  bug-686 scene draws on Vulkan, but lavapipe's blending drifts one step from the oracle
  per 20+ stacked blends (11.8% of the frame, every overlapped pixel off by exactly 1,
  measured per overlap depth) — blend precision, not edges.
- **`large_polygons.mfb` gained a grey ground**: over black, lavapipe tripped bug-687's
  one-coverage-step flip (evidence recorded in bug-687).
- **Band builder ported, not hoisted** (Open Decisions' alternative): hoisting would have
  re-emitted Metal's instructions, which the non-goals keep unchanged.
- **The `app-window-surface` linux-x86_64 sentinel was stale at `main`** and was corrected
  (`df722b970`).

Gates: `cargo test --release --no-fail-fast` — 227 suites, 6,032 passed, 0 failed;
`scripts/artifact-gate.sh <mfb> all` — at main 1 diff (the stale sentinel), on the branch
the 7 app-mouse-surface diffs, localized to `__canvas_vulkanRenderable` and the Vulkan
emitter and regenerated (`7668925a4`); `scripts/test-canvas-vulkan.sh --box 2226 --target
linux-aarch64` passed; `scripts/test-winapp.sh` (2230) passed; `rt_canvas_metal` 19
passed; `scripts/spec-census.sh --citations` 0 misses.

bug-686 made Metal draw ordinary scenes on the GPU and made the graphics thread fast.
The shared half landed for every backend. Vulkan (Linux and Windows, `has_vulkan_backend`,
`src/codegen/runtime/canvas/vulkan.rs`) got **none** of the Metal half:

- Its frame caps are still the ones bug-686 measured sending wind and 5,000-quad scenes to
  software.
- Its polygon shader still loops over every edge per pixel.
- It copies a picture's texels once per occurrence, not once per image.
- Its gradient region base is a hand-maintained literal in checked-in GLSL.
- Every frame is read back and blitted, even in a real window.

The shared half — the native geometry resolve, the native item hash, the carried hashes,
the scene sequence lock and the damage-only work — is built for Vulkan's targets but has
**never run on x86-64**. The bug-686 agents built it for `linux-x86_64`, `windows-x86_64`
and `linux-aarch64`, but ran it only on macOS.

**The single correct behavior a fix produces:**

- On a Linux or Windows machine with a Vulkan GPU, every scene bug-686 put on Metal draws
  on Vulkan, not software: `gpuFrames = frames`, within `Tolerance::GPU_DEFAULT` of the
  software oracle.
- The same frame-rate rows bug-686 measured on Metal hold on that hardware, within the
  hardware's own GPU budget.
- The shared bug-686 native paths are proven on x86-64 at run time: `MFB_CANVAS_GEO_VERIFY`
  reports 0 mismatches.

References:

- `bugs/completed/bug-686-metal-declines-ordinary-scenes-and-falls-back-to-software.md` —
  the Metal work this bug mirrors. Its sections B–H are the measurements and its Phases 1–4
  are the designs.
- `.ai/canvas-threading.md`: §6 (pictures), §10 (the GPU backends, caps, predicates,
  direct present), §11 (stats), §14 (geometry cache, native store, sequence lock), and the
  "Growing `ITEM_BLOCK_SIZE`" list, whose item 5 names Vulkan's `GRADIENT_BASE` literal.
- `src/docs/spec/app/06_canvas.md` — the spec's canvas topic. It describes the Metal caps,
  band index, picture dedupe and direct present, and gives Vulkan's old caps.
- `.ai/remote_systems.md` — the Linux boxes (2228 glibc, 2227 musl) are **QEMU-TCG-emulated
  x86_64 VMs**.
- bug-687 (open): the fp32 coverage flip. It applies to the GLSL shader as much as to MSL.
- plan-152: the program-side cost spike. It applies to every backend.

## Failing Reproduction

Correctness rows run on the Linux box through the existing oracle harness:

```
cargo build --release
scripts/test-canvas-vulkan.sh target/release/mfb --box 2228
```

To reproduce each gap, add bug-686's `rt_canvas_metal.rs` scenes to that harness. The
expectation for each row is "Vulkan draws it, within `GPU_DEFAULT` of software".

| scene (bug-686 section) | Metal at `main` | Vulkan at `main` (expected from the code; Phase 1 measures it) |
| --- | --- | --- |
| 5,000 quads (B) | Metal | software: `quads > __CANVAS_MAX_FRAME_ITEMS` (4,096) |
| wind, ~5,700 items (A) | Metal | software: same cap |
| 40,000 total polygon edges (B) | Metal | software: `total > __CANVAS_VULKAN_MAX_FRAME_EDGES` (16,384) |
| 2,000 tiles from two 32×32 images (H) | Metal | software past 1,024 tiles: per-occurrence texel count against 1M |
| 1920×1080 background (H) | Metal | software: 2.07M texels > 1M |
| 5,000 gradient stops in a frame | Metal | software: `MAX_FRAME_GRADIENT_STOPS` (4,096) |
| one 64,000-edge polygon (E/G) | Metal, band index | software (frame edge cap); even under the cap, the shader loops over every edge |

Contrast cases that work today and must keep working: every scene inside the old caps.
Those are what `test-canvas-vulkan.sh` asserts at `main`.

Environment matrix. **Phase 0 must fill it in; nothing here has been run yet:**

| Environment | GPU | Can check correctness | Can check frame rate |
| --- | --- | --- | --- |
| box 2226, Debian 12 aarch64 glibc (native VM, GNOME Wayland session) | virtio; Mesa lavapipe | **yes — run** | no: CPU driver |
| box 2230, Win11 x86_64 (emulated) | virtio; Mesa lavapipe (`C:\mfbvk`) | **yes — run** | no |
| box 2228, Ubuntu x86_64 glibc | QEMU-TCG VM, software Vulkan ICD | yes | no — not run by this fix |
| box 2227, Alpine x86_64 musl | QEMU-TCG VM, user-local ICD (`--icd auto`) | yes | no — not run by this fix |
| Windows x86_64 with a GPU | — (none reachable) | — | — |
| Linux x86_64 or aarch64 with a GPU | — (none reachable) | — | — |

## Root Cause

bug-686 split into a **shared** half (graphics thread, worker, scene ring) and a
**Metal-only** half (the emitter, shader and predicate in `metal.rs` / `helper_render.rs`).
"Metal only" was a deliberate scope decision in bug-686 ("Vulkan is out of scope"). So each
Metal-only change left a Vulkan twin behind:

1. **Frame caps.** `__canvas_vulkanRenderable` (`src/codegen/builtins/canvas/helper_render.rs`)
   still gates on:
   - `CANVAS_MAX_FRAME_ITEMS` = 4,096;
   - `VULKAN_MAX_FRAME_EDGES` = 16,384;
   - `MAX_FRAME_GRADIENT_STOPS` = 4,096;
   - `VULKAN_MAX_FRAME_GLYPH_SAMPLES` = 1 << 20.

   All four are in `src/codegen/runtime/canvas/mod.rs`. The frame buffer is sized from them
   (`VULKAN_BUFFER_BYTES`), so raising a cap moves every region after it.
2. **Region bases.** The fragment shader's `GRADIENT_BASE` is a literal (`const int
   GRADIENT_BASE = 1114112;`, `src/codegen/runtime/canvas/shaders/mfb_canvas.frag`),
   compiled to the checked-in `mfb_canvas.frag.spv` by `scripts/regen-spirv.sh` and pinned
   to `VULKAN_GRADIENT_BASE_WORDS` only by a unit test. Metal's bases are formatted into its
   MSL at first use (bug-686). Vulkan's SPIR-V is a binary compiled ahead of time, so the
   Metal technique does not transfer as-is (see Fix Design).
3. **Polygon fill.** `edgeDistance` in `mfb_canvas.frag` loops `for (int e = 0; e < count;
   ++e)` over the polygon's edges, per pixel. This is the O(edges × covered pixels) cost
   bug-686 section E measured at 252 ms for 64,000 edges on Metal before the band index.
4. **Pictures.** `emit_picture_upload` (`vulkan.rs`) copies an item's texels into the glyph
   region per item. The predicate adds `__canvas_pictureSamples(offset)` per item. Metal
   copies each distinct block once (`emit_picture_lookup`, a per-frame picture table) and
   counts distinct blocks.
5. **Present.** `canvas::vulkanDrawScene` (`emit_vulkan_draw_scene`) always renders
   offscreen, waits with `vkQueueWaitIdle`, copies image → buffer, and hands the frame to
   `canvas::blitSurface`. On Metal that path cost ~10 ms per megapixel of fixed work before
   bug-686 Phase 4 presented straight to a `CAMetalLayer` (root cause 10 there). Vulkan has
   no swapchain path. The doc comment gives the reason: "no reachable Linux box has a
   display server".
6. **Shared native paths unproven on x86-64.** These are emitted through arch-neutral
   `CodeBuilder`/`abi::` code, so they **build** for x86-64:
   - `canvas::geoBuild` (`func_geo_build.rs`);
   - `canvas::sceneResolve` / `sceneLayout` / `sceneDrawsFlat` / `geoBeginFrame` / the
     table calls (`func_geo_cache.rs`);
   - `canvas::itemHash` / `sceneHashes` (`func_item_hash.rs`);
   - `canvas::carriedHashes`;
   - the scene sequence lock (`scene_base.rs`: `emit_scene_store`, `emit_scene_load`,
     `emit_mark_pending`, `emit_scene_barrier`).

   **None has executed on x86-64.** The sequence lock has **no fences** off aarch64
   (`orders_scene_accesses`). That is correct on x86-64 only because x86-64 keeps stores in
   order, and the doc comment records one step that relies on the store buffer draining
   within a frame.

Hypotheses Phase 0 has to settle, each with how to settle it:

- **H1:** the item buffer, which `ITEM_BLOCK_SIZE` sizes against `CANVAS_MAX_FRAME_ITEMS`,
  is mapped HOST_VISIBLE and sized at pipeline creation. Raising the cap to 65,536 grows it
  16×. The pipeline must still build on a low-memory device. **Settle:**
  `vulkanReady=TRUE` on the boxes with the new caps, and the size in the stats line.
- **H2:** SPIR-V specialization constants (`VkSpecializationInfo`) can carry the region bases
  and band parameters into the checked-in shader. That needs no `glslangValidator` at build
  time, and the vendor Mesa / NVIDIA / AMD drivers honour them. **Settle:** a spike shader
  on the box's ICD.
- **H3:** `vkCmdCopyImageToBuffer` from an OPTIMAL-tiled image is a GPU copy, so bug-686's
  "linear render target" fix (a CPU decompress in `getBytes:`) has **no Vulkan twin**.
  **Settle:** a profile of the readback on real hardware.

## Goal

- **(V1)** No ordinary scene falls to software on Vulkan. Every bug-686 row above draws on
  Vulkan within `GPU_DEFAULT` of software, including 65,536 quads, 262,144 frame edges,
  131,072 gradient stops and 8M picture/glyph texels. Caps and region bases have one
  definition in Rust, and the MFBASIC predicates are generated from it (`RENDER_CAPS`).
- **(V2)** A polygon of any edge count draws on Vulkan through the band index, with the same
  pixels as the full loop.
- **(V3)** A picture's texels upload once per distinct image per frame. The predicate counts
  distinct images.
- **(V4)** On Linux and Windows x86-64 (and Linux aarch64), `MFB_CANVAS_GEO_VERIFY=1` reports
  0 mismatches on every bug-686 row, including the snapshot race test's alternating scenes.
  The sequence lock's x86-64 ordering argument is re-checked against those runs.
- **(V5)** On real GPU hardware, measure the bug-686 fps rows and wind on Vulkan (release,
  `MFB_CANVAS_SYNC=1` and pipelined). The renderer path must not be the limit at 10,000
  moving items. The Metal reference is ~13 ms per frame at 10k quads, where the program's
  own build is the limit.
- **(V6)** In a real window, measure present without readback (swapchain) against the
  readback and blit. If the readback threatens the budget at full-screen, present through
  a swapchain. The oracle tests keep a readback path, as on Metal.

### Non-goals (must NOT change)

- **Software stays the oracle.** Every Vulkan change is accepted on "agrees with software
  within `Tolerance::GPU_DEFAULT`". A golden's byte-identity is not the acceptance test.
- **The honesty gate.** A scene the backend truly cannot draw is declined, never clamped or
  truncated.
- **Metal.** Nothing in `metal.rs` or the Metal predicate changes. Any shared refactor
  (for example hoisting the band-index builder) must leave Metal bit-identical:
  `rt_canvas_metal.rs` and the app-mouse-surface sentinel must move only by the intended
  reorganisation.
- **The API.** No new public canvas surface.
- **The wrong fix:** raising the MFBASIC cap constants without growing the buffer and moving
  the shader bases. Past the old region end, items would read another region's words: a
  plausible wrong picture, the exact hazard `.ai/canvas-threading.md` warns of. Forbidden.
- **The other wrong fix:** calling a scene "on Vulkan" because it drew on the QEMU boxes'
  software ICD, and treating that as a performance result. Frame-rate acceptance needs
  real hardware.

## Blast Radius

Found with `grep -rn "VULKAN_\|vulkan" src/codegen src/target` and a read of each bug-686
commit's Metal-only files:

- `src/codegen/runtime/canvas/mod.rs`: `CANVAS_MAX_FRAME_ITEMS`, `VULKAN_MAX_FRAME_EDGES`,
  `VULKAN_EDGE_BYTES`, `MAX_FRAME_GRADIENT_STOPS`, `VULKAN_MAX_FRAME_GLYPH_SAMPLES`,
  `VULKAN_GLYPH_BASE_WORDS`, `VULKAN_GRADIENT_BASE_WORDS`, `VULKAN_BUFFER_BYTES`. **Fixed
  here.** Give Vulkan its own `VULKAN_MAX_FRAME_*` caps, as bug-686 gave Metal its own,
  rather than raising the shared `CANVAS_MAX_FRAME_ITEMS`, which other code may read. The
  audit decides.
- `src/codegen/builtins/canvas/helper_render.rs`: `__canvas_vulkanRenderable`,
  `__canvas_renderVulkan`, the `@VULKAN_…@` tokens in `RENDER_CAPS`. **Fixed here.** Count
  distinct pictures, and use the new caps.
- `src/codegen/runtime/canvas/shaders/mfb_canvas.frag` / `.vert`, and the `.spv` blobs via
  `scripts/regen-spirv.sh`. **Fixed here:** band loop, region bases, picture table.
- `src/codegen/runtime/canvas/vulkan.rs`: `emit_edge_upload`, `emit_picture_upload`,
  `emit_item_block`, `emit_vulkan_target` (the buffer and mapping size),
  `emit_vulkan_draw_scene`, `emit_vulkan_descriptors`. **Fixed here.**
- `src/target/macos_aarch64/app/metal.rs`: `emit_band_index`, `emit_picture_lookup`.
  **Reference, not changed.** They are written against the macOS `Asm` with aarch64 scratch
  registers, so they cannot be called from Vulkan's `CodeBuilder` code. Port the algorithm
  (bug-686 section G, bit-identical) to `CodeBuilder`, or hoist one `CodeBuilder` version
  that both backends call. The second leaves Metal's instructions changed, and is decided
  in Open Decisions.
- `src/codegen/builtins/canvas/func_geo_build.rs`, `func_geo_cache.rs`, `func_item_hash.rs`,
  `func_carried_hashes.rs`, `scene_base.rs`. **Verified here on x86-64 (V4)**, and fixed if
  the run finds anything.
- `src/target/win_x86_64/**`, `src/target/linux_common/**` (the window, the blit, the
  runtime-call lists). **Changed only by V6** (swapchain surface). Otherwise unaffected: the
  runtime-call lists already carry every bug-686 call.
- `tools/canvas-bench/`: **latent.** macOS-only (it drives headless `.app` bundles). V5
  needs a Linux runner; add one to the tool, not a new script.
- `scripts/test-canvas-vulkan.sh`: **extended here** with bug-686's rows and a
  `MFB_CANVAS_GEO_VERIFY=1` run.
- bug-687 (fp32 coverage flip): **latent, same hazard, out of scope.** Owned by bug-687. A
  Vulkan oracle row that trips it goes there, not here.

## Fix Design

Mirror bug-686 phase for phase, in the same order, with the same proofs. Only the mechanism
differs where Vulkan's toolchain does.

- **Region bases: specialization constants, not formatted source.** Metal compiles MSL
  source at run time, so bug-686 formats the bases in. Vulkan loads checked-in SPIR-V.
  Declare the bases (and later the band parameters) as `layout(constant_id = N) const
  int`, and feed them through `VkSpecializationInfo` at pipeline creation from the Rust
  constants.
  - This removes the hand-edited literal and the test that only mirrors it. `ITEM_BLOCK_SIZE`
    item 5 in `.ai/canvas-threading.md` then goes away.
  - **Rejected:** generating SPIR-V at build time (a `glslangValidator` build dependency the
    project avoids). **Rejected:** a uniform (a per-frame read for a per-build constant).
- **Band index: the same algorithm, emitted through `CodeBuilder`.** Use section G's
  structure: 2 px bands, reach `r = 0.5` fill / `half + 0.5` stroke plus a 1 px guard, scaled
  for transforms, and taller bands when the index budget forces it. It is built from the
  polygon's cached geometry into the edge region, and the item block carries base, band top,
  band height and band count. Keep it bit-identical to the full loop: a band count of 0
  means loop over every edge, as on Metal.
- **Picture table: a per-frame `(block, base)` table in a new last region**, looked up in
  `emit_picture_upload`. The predicate counts each distinct block once (bug-686 section H /
  `5c8bbe36d`).
- **Caps: Vulkan's own constants**, sized like bug-686 spike C: 65,536 items, 262,144 edges,
  131,072 stops, 8M texels, plus the band region. Pass them into the predicates through
  `RENDER_CAPS` tokens. Buffer sizing stays fixed (bug-686's Open Decision), unless H1 shows
  a device that cannot map it.
- **Present:** measure first (V6). If needed, add a `VK_KHR_swapchain` path per window
  system:
  - Win32: `VK_KHR_win32_surface` on the canvas HWND.
  - Linux: `VK_KHR_xlib_surface` / `VK_KHR_wayland_surface` under GTK.

  Keep the readback for headless runs, damage mode and dumps, like Metal's `directFrames`
  split, with a stats counter for it.
- **x86-64 proof before any of the above.** The shared native paths are the base every
  Vulkan frame stands on. A mismatch found there is a bug in shared code, fixed for every
  backend.

Expected output shifts:

- The `app-mouse-surface` sentinel's `.ncodesum` for `linux-*` / `windows-*` (Vulkan
  emitter) and the Linux/Windows helper source.
- No Metal golden changes, unless the band builder is hoisted.
- The `.spv` blobs.

## Phases

### Phase 0: environment and baseline (no behavior change)

- [ ] Find real Linux and Windows machines with a Vulkan GPU, and record them in
      `.ai/remote_systems.md`. **BLOCKED: none is reachable** — 2226 and 2230 run Mesa
      lavapipe, recorded in `.ai/remote_systems.md` (`948ef1044`). This blocks V5 and V6.
- [x] Fill in the environment matrix above, for the boxes this fix could use (2226, 2230).
- [x] Add a Linux runner to `tools/canvas-bench/` (headless GTK app,
      `MFB_GTKAPP_HEADLESS`), with the same subcommands as the macOS one.

Acceptance: `scripts/test-canvas-gpu-rows.sh <main mfb> --target linux-aarch64`
reproduced sections B and H at `main`, each row's renderer shown (`gpuFrames=0`).
Commit: `bd49be0d0`, `948ef1044`

### Phase 1: failing tests + the x86-64 proof of the shared paths

- [x] Add bug-686's `rt_canvas_metal.rs` scale, large-polygon, tilemap, background, layered
      and gradient rows — as `scripts/test-canvas-gpu-rows.sh` over shared
      `tests/canvas/scenes/*.mfb`, not inside `test-canvas-vulkan.sh`, so the Rust suites and
      the script read one copy. RED at `main` on 2226: quads, edges, gradients, tilemap,
      background and the 64k polygon declined (`gpuFrames=0`).
- [x] Run every row, plus the snapshot race scene (`rt_canvas_geo_native`'s alternating
      scenes), with `MFB_CANVAS_GEO_VERIFY=1` on `linux-x86_64` (2228), `linux-x86_64` musl
      (2227), `linux-aarch64` and `windows-x86_64`. Record `geoVerified`, `geoResolvedChecked`
      and mismatches. Any mismatch is a shared-code bug: fix it here, with its own test.
      Run on linux-aarch64 (2226) and windows-x86_64 (2230), at `main` and on the branch:
      0 mismatches everywhere. linux-x86_64 glibc/musl not run (boxes not available to
      this fix).

Acceptance: met on the two targets run. The one row failing at `main` for a non-cap
reason, `polygons`, failed on bug-687's flip (see Deviations).
Commit: `6ef055958`

### Phase 2: caps, derived bases, picture table

- [x] Vulkan's own frame caps and buffer size, and the specialization-constant region bases.
      Delete the `GRADIENT_BASE` literal. Replace `the_shaders_gradient_base_matches_the_buffer_layout`
      with a region-chain test like Metal's. Prove it wrong first (AGENTS.md): the new
      mechanism removes the literal the test mirrors.
- [x] Per-frame picture table and distinct-block predicate count. Regenerate the SPIR-V with
      `scripts/regen-spirv.sh` (`MFB_SPIRV_PORT=2226`).

Acceptance: met on 2226 and 2230.
Commit: `069348f8b`, `937ddc149`

### Phase 3: band index

- [x] Build the band table and edge index with the polygon's cached geometry, through
      `CodeBuilder`. Write them into the edge region, carry band fields in the item block,
      and loop over one band in `edgeDistance`.
- [x] Oracle rows at 300, 1,000, 4,000 and 64,000 edges, plus stroked, transformed and
      self-intersecting polygons (`large_polygons.mfb`, `huge_polygon.mfb`).

Acceptance: met. "Band count 0 forced" was measured without a production switch: the
previous commit's build IS the full loop, so the same scenes were rendered with both
builds on 2226 and byte-compared — identical.
Commit: `05c5ffa81`

### Phase 4: performance on real hardware, and present (needs Phase 0's machines)

- [ ] Release fps rows (1k/4k/10k quads and lines, `MFB_CANVAS_SYNC=1` and pipelined) and
      wind, on Linux and Windows GPUs. Split the frame with the debug phase timers.
- [ ] Measure readback + blit against the frame budget at full-screen. If it threatens the
      budget, add the swapchain present, with the readback kept for headless runs, damage
      and dumps, and a stats counter.

**BLOCKED: no reachable Vulkan GPU** (Phase 0). Nothing here was measured; the renderer
still reads back in a window, as before.

Acceptance: at 10k moving items the renderer path is not the limit, and the numbers are
recorded here with commands. A full-screen window holds the rate.
Commit: —

### Phase 5: regenerate outputs, docs, full validation

- [x] Regenerate the `app-mouse-surface` sentinel after the full suite, inspect the diff,
      and prove it is Vulkan-only. (The macOS dumps move too: the Vulkan predicate is
      shared MFBASIC compiled into every app; the normalized `.ir`/`.nir` delta is only
      `__canvas_vulkanRenderable`.)
- [x] Doc sync:
      - `.ai/canvas-threading.md` §10 caps and §6 pictures ("Vulkan still copies and
        counts per item" goes);
      - the `ITEM_BLOCK_SIZE` list item 5;
      - `mfb spec app canvas`'s Vulkan sentences, with `scripts/spec-census.sh
        --citations` showing 0 misses.
- [x] Full suite (`cargo test --release --no-fail-fast`), `scripts/test-canvas-vulkan.sh` on
      2226, `scripts/test-winapp.sh` on 2230, and `rt_canvas_metal` (19 passed).

Acceptance: met. `cargo test --release --no-fail-fast`: 227 suites, 6,032 passed, 0 failed (includes `artifact_gate_all`, after the sentinel regeneration).
Commit: `948ef1044`, `df722b970`, `7668925a4`, `df4f589a7`

## Validation Plan

- Regression: the `test-canvas-vulkan.sh` rows (RED → GREEN per phase), `GEO_VERIFY` on four
  targets, the `vulkan.rs` layout and region-chain unit tests.
- Runtime proof: the oracle rows on 2228 and 2227 (correctness); fps rows and wind on a
  real-GPU Linux and Windows machine (performance).
- Doc sync: `.ai/canvas-threading.md`, `mfb spec app canvas`, `tools/canvas-bench/README.md`.
- Full suite: `cargo test --release --no-fail-fast` (includes `artifact_gate_all`).

## Open Decisions

- **Band-index builder.**
  - *Recommended:* one `CodeBuilder` implementation both backends call. The algorithm then
    exists once, at the cost of re-emitting Metal's instructions: prove it with
    `rt_canvas_metal` unchanged and an inspected sentinel diff.
  - *Alternative:* a Vulkan-only port, with the algorithm kept twice.
- **Swapchain.**
  - *Recommended:* measure first, and build it only if the readback threatens the budget
    at full-screen, as bug-686 Phase 4 did.
  - *Alternative:* build it unconditionally for parity.
- **Hardware.**
  - *Recommended:* V1–V4 land on the QEMU boxes, and V5/V6 wait for a real GPU, stated
    as a blocker.
  - *Alternative:* hold the whole bug for hardware.

## Summary

The engineering risk is in two places:

- **The shader-side changes.** These are the band index and the specialization-constant
  bases. They are only checkable against the software oracle on a device, and the only
  reachable devices are emulated.
- **The first-ever x86-64 run of bug-686's shared native code.** Anything it finds is a bug
  in every backend.

The caps and the picture table are mechanical once the bases are derived.

Left untouched:

- Metal (unless the band builder is hoisted, and then bit-identical);
- the API;
- the software rasteriser;
- bug-687's tolerance question;
- the program-side per-item cost (plan-152).
