# plan-98-F: Canvas Vulkan backend (Linux + Windows)

Last updated: 2026-08-30
Effort: x-large (1d–3d) — split F-1 (Linux) / F-2 (Windows) if it exceeds one sitting
Depends on: plan-98-E (Metal backend — proves the renderer-swap seam on a real GPU)

This sub-plan adds the **Vulkan** renderer for Linux (GTK4 native surface) and Windows
(HWND), behind the same D thread/ring/retirement boundary, reusing the exact
single-pipeline shader design proven on Metal in E. After it lands, canvas programs render
via Vulkan on Linux and Windows, matching the C software golden within tolerance
(invariant 5), with the software backend still the exact-match CI oracle.

This is **build step 6** of the A–G sequence. GPU-specifics depend on D's real vertex/fence contract
(confirmed in E) and are marked `UNVERIFIED`/`UNMEASURED` where not yet pinnable.

References:

- **plan-98-A** — invariant 4 (Vulkan submit fence advances `lastCompletedFrame`,
  driving the closed-flag texture free; no refcount), invariant 5 (tolerance),
  invariant 8 (testing policy). plan-98-A's "Cross-cutting invariants" section is this
  feature's top-level design; there is no separate design document.
- **plan-98-E** — the pipeline/shader shape and renderer-swap seam this reuses on a
  second API; **plan-98-C** — the software reference images to diff against.
- plan-98-D resize handshake (the Vulkan swapchain-recreation path is the concrete case D
  abstracted), plan-98-E's proven shader pipeline + renderer seam.
- `.ai/arch-abi.md` (Win64 PE/console; Windows app path is GDI today — Vulkan is new code).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-98-E complete (Metal proves the renderer seam on GPU) | `ls planning/completed/plan-98-E-*` → hit | MET (archived; phases landed as `74b4dc0a2`+`0c2130c6d`, `fd2bf37fe`, `7d3d4b4e4`) |
| A retrieves the GTK native surface handle + Windows HWND | plan-98-A Phase 3 acceptance met | MET. Linux: `rg -n gtk_native_get_surface src/target/linux_gtk/` → the `GdkSurface*` is read after `gtk_window_present` and stored at `ST_CANVAS_SURFACE` (`mod.rs:186,193`). Windows: `CANVAS_HWND_SYM` (`win_x86_64/app/mod.rs:150`). A's Correction 16 records why the portable `gtk_native_get_surface` replaced the two backend-specific getters the plan named — **and see Correction 1 below: with the offscreen renderer neither handle is needed.** |
| Working tree builds | `cargo build` → pass | MET (`Finished \`dev\` profile`) |
| A SPIR-V compiler is reachable (added — F cannot start without one) | `ssh -p 2228 test@127.0.0.1 'apt-get download glslang-tools && dpkg -x … && ./glslangValidator --version'` | MET. Not installed on the macOS host and not installable without root on the test boxes, but `apt-get download` + `dpkg -x` into a scratch dir works as a plain user — the same trick `.ai/remote_systems.md` documents for qemu-user. `scripts/regen-spirv.sh` automates it. |
| Vulkan is loadable on the proof surface (added) | `ssh -p 2228 … vulkaninfo --summary` | MET. Ubuntu x86_64 glibc (2228): loader 1.4.309, device `llvmpipe` (`PHYSICAL_DEVICE_TYPE_CPU`). Alpine x86_64 musl (2227): `/usr/lib/libvulkan.so.1` present. Win11 (2230): `vulkan-1.dll` present. The aarch64 GTK boxes (2225, 2226) refuse connections right now, so the Linux proof surface is x86_64. |

> Per A's invariant 8: no "full suite green at HEAD" row and no byte-identity
> obligation.

## 1. Goal

- Load Vulkan at runtime via `dlopen("libvulkan.so.1")` / `vkGetInstanceProcAddr` (Linux)
  and the Windows loader — **no SDK, no link dependency**.
- Linux: create a `VK_KHR_xcb_surface` / `VK_KHR_wayland_surface` from the GTK4 native
  handle retrieved in A (GTK keeps owning window/events/DPI). Windows: `VK_KHR_win32_surface`
  from the A-built HWND.
- Create device/queues/swapchain and the **same single pipeline** as E (shared shader design);
  the graphics thread records/submits/presents; the Vulkan submit fence drives D's fence-gated
  retirement.
- Resize: main thread signals `resizePending`; graphics thread at frame start
  `vkDeviceWaitIdle` + recreates the swapchain, per D's handshake. Handle out-of-date/
  suboptimal swapchain (redraw trigger 4).
- Output matches the C software golden within tolerance on both platforms; software backend
  stays exact-match.

### Non-goals (explicit constraints)

- **No shared thread/ring/retirement changes** — Vulkan drops in behind D's seam exactly as
  Metal did in E.
- **No X11/Wayland windowing backend** — GTK4 owns the window; F only creates a Vulkan surface
  from GTK's native handle — the saving that makes this letter tractable.
- **No text** (G).
- **Software backend stays first-class and exact-match.**

## 2. Current State

- **A** retrieves the GTK native surface handle and ensures the Windows HWND exists for Vulkan
  surface creation. **E** proved the renderer-swap seam, the shader pipeline, and the
  tolerance-match gate on a real GPU (Metal). **D** owns the resize handshake whose concrete
  Vulkan case (`vkDeviceWaitIdle` + swapchain recreate) F implements.
- Windows app mode is **GDI today** (research §3) — there is no `term::`/Vulkan precedent on
  Windows; F's Windows swapchain path is new code.
- UNVERIFIED until Phase 1: whether the GTK4 build in the import surface exposes both
  `gdk_x11_surface_get_xid` and `gdk_wayland_surface_get_wl_surface` at runtime (X11 vs Wayland
  session detection). Phase task confirms and picks the surface extension per session.

### Measured populations

| What | Count | Command |
|---|---|---|
| Vulkan surface backends | 3 (xcb, wayland, win32) | this plan's §1 — one per platform surface A builds |
| Platforms | 2 (Linux, Windows) | — |
| Shaders reused from E | same set | invariant "one pipeline" |
| Vulkan entry points to load via `vkGetInstanceProcAddr` | UNMEASURED | enumerate in Phase 1 |

### Verified properties

- **Vulkan needs no SDK** (runtime `dlopen("libvulkan.so.1")` + `vkGetInstanceProcAddr`
  rather than link-time binding) — this is the approach this plan chooses to satisfy the
  no-shared-libs constraint. **UNVERIFIED against this repo's build**: Phase 1 must prove
  the loader path works on both Linux targets and Windows before any pipeline code lands.
- **GTK4 hands over the native surface handle**, so no X11/Wayland windowing backend is
  needed. **UNVERIFIED**: plan-98-A Phase 3 is the letter that makes the handle
  retrievable (`gdk_x11_surface_get_xid` / `gdk_wayland_surface_get_wl_surface`); if A
  landed, this is met by A's acceptance, otherwise Phase 1 confirms both getters at
  runtime. Do not treat it as established until one of those has run.
- UNVERIFIED: Windows Vulkan swapchain coexisting with the GDI message loop from A — Phase task
  proves the HWND drives `VK_KHR_win32_surface` while the WNDPROC still pumps.

## 3. Design Overview

- **Runtime loader.** `dlopen` libvulkan (Linux) / the Windows loader; bootstrap instance +
  device function pointers via `vkGetInstanceProcAddr`/`vkGetDeviceProcAddr`.
- **Surface creation from GTK/HWND.** xcb/wayland from GTK's native handle (session-detected);
  win32 from the HWND. GTK/Win32 keep owning the window.
- **Vulkan renderer behind D's seam.** Swapchain, single pipeline (E's shaders → SPIR-V),
  per-frame acquire/record/submit/present on the graphics thread; submit fence → D's retirement.
- **Resize/out-of-date.** `resizePending` → `vkDeviceWaitIdle` + recreate; handle
  `VK_ERROR_OUT_OF_DATE_KHR`/suboptimal as redraw trigger 4.

**Where correctness risk concentrates:** swapchain lifecycle (acquire stalls, recreate on
resize/out-of-date) and the Windows-new-code path (no precedent). Land Linux first (GTK precedent
+ E's proven pipeline), Windows second; land the fence→retirement wiring last on each.

**Gate:** tolerance-match to the C golden on each platform (invariant 5). A beyond-tolerance
diff is a Vulkan blend/sRGB/coordinate/swapchain bug against the software oracle — root-cause,
never re-baseline the oracle.

**Rejected alternatives:** a hand-written X11/Wayland/Win32 windowing layer — rejected (design):
GTK4/Win32 already own windowing; F only needs a Vulkan surface from their handles.

## Compatibility / Format Impact

- **Changes:** a Vulkan render path on Linux + Windows; runtime Vulkan loader; new Windows GPU
  path alongside the existing GDI app path.
- **Unchanged:** API, scene model, thread/ring/retirement code, the shared shaders (from E), the
  software backend and its exact-match goldens.

## Phases

### Phase 1 — Vulkan loader + Linux surface (xcb/wayland) + one-quad tolerance match

- [x] Runtime `dlopen` libvulkan + entry points, **measured on box 2228**:
      `canvas::vulkanAvailable()` reports TRUE after `dlopen("libvulkan.so.1")`,
      `dlsym` of the entry points, `vkCreateInstance` from hand-written
      `VkApplicationInfo`/`VkInstanceCreateInfo`, `vkEnumeratePhysicalDevices`
      finding the llvmpipe device, and `vkDestroyInstance`. Nothing is linked — the
      loader arrives through `dlopen`/`dlsym`, never a DT_NEEDED, so a canvas binary
      still execs where no Vulkan is installed (`audio`'s rule, plan-33-C §3.1).
      `UNMEASURED` resolved: the probe needs four entry points; the renderer's set
      grows with it. Entry points are resolved by `dlsym` rather than through
      `vkGetInstanceProcAddr` — Correction 2.
- [x] ~~Detect X11 vs Wayland; create `VK_KHR_xcb_surface`/`VK_KHR_wayland_surface`
      from GTK's native handle~~ — moot: **there is no surface and no swapchain.**
      Correction 1 — the renderer draws offscreen and the frame leaves through
      `canvas::blitSurface`, exactly as plan-98-E Correction 5 established for Metal.
      That removes both surface extensions, the session detection, and the swapchain
      from this letter, and it is what makes the whole path testable at all: the only
      reachable Linux boxes have no display server (2225/2226 refuse connections,
      2228 is `XDG_SESSION_TYPE=tty`).
- [x] Shaders → SPIR-V: `mfb_canvas.vert`/`.frag` and their compiled blobs are
      checked in together, reproducible via `scripts/regen-spirv.sh` (a full
      regeneration leaves the `.spv` byte-identical). glslang's reflection confirms
      the push-constant block is byte-identical to the Metal item block — shape 16,
      fill 32, stroke 48, misc 64, arc 80, size 112 — so one CPU-side emitter feeds
      both backends. 112 bytes fits Vulkan's guaranteed 128-byte push-constant range,
      so the render path needs no descriptor sets.
- [x] Device, pipeline, offscreen render and readback, **measured on two boxes**.
      `scripts/test-canvas-vulkan.sh` renders the same program twice — once with
      `MFB_CANVAS_GPU=1`, once without — and diffs the frames against
      `Tolerance::GPU_DEFAULT`:

      * box 2228 (Ubuntu glibc, llvmpipe): worst channel delta **1**, 0.0306% of
        pixels differing;
      * box 2227 (Alpine musl): worst delta **0** — byte-identical.

      Both are inside the comparator's *per-pixel* bound, not merely its population
      budget. And the scene is far past the box's "one tinted quad": two circles, a
      swept arc, a rounded rect with both fill and stroke, a thick line and a
      translucent rect, so every arm of the fragment shader's distance dispatch is
      exercised. `Polygon` is the one kind declined (Correction 6).

Acceptance: MET, and exceeded — the full primitive set (less `Polygon`) matches the
software oracle within tolerance on two libcs, and the software backend is untouched.
Commit: —

### Phase 2 — Linux full scene + resize/out-of-date + texture free

- [ ] Full primitive set (incl. Circle/Arc SDF) from the `live` vertex buffer; atlas upload.
- [ ] Dynamic-texture upload for `canvas::setBytes` (D's dirty flag): staging-buffer upload
      at frame start via a per-texture ring or a transfer barrier, so uploading never races
      an in-flight frame sampling the texture. Coalesce multiple `setBytes` to one upload.
- [ ] Resize handshake: `resizePending` → `vkDeviceWaitIdle` + recreate swapchain; handle
      out-of-date/suboptimal (trigger 4).
- [ ] Vulkan submit fence advances D's `lastCompletedFrame` (drives the closed-flag texture free).
- [ ] Tests: the multi-primitive golden (incl. the smiley scene) matches within tolerance;
      a `setBytes` on an in-scene image updates next frame without tearing; resize repaints
      worker-free; the D race matrix passes on the Vulkan fence.

Acceptance: full scene tolerance-match on Linux; resize/out-of-date correct and worker-free;
retirement driven by the Vulkan fence; race matrix green.
Commit: —

### Phase 3 — Windows (win32 surface) — new GPU path (largest blast radius last)

- [ ] `VK_KHR_win32_surface` from the A-built HWND, coexisting with the GDI WNDPROC/message loop.
- [ ] Swapchain + the same pipeline; render + resize + out-of-date; fence → retirement.
- [ ] Tests: tolerance-match goldens on Windows (headless where `MFB_WINAPP_HEADLESS` allows a
      Vulkan surface; else a window-station CI lane); resize worker-free; race matrix green on
      Windows.

Acceptance: canvas renders via Vulkan on Windows matching the software reference within
tolerance; resize + retirement correct; the D race matrix green on the Windows Vulkan path.
Run only the new Vulkan tolerance-golden tests plus C's software goldens (the reference
they are compared against).
Commit: —

## Validation Plan

- Tests: per-platform tolerance goldens (per primitive + full scene), resize/out-of-date,
  session-type surface creation (Linux), and the retirement race matrix on each Vulkan fence.
- Coverage check: loader + surface + swapchain-recreate + retirement-hook logic in the
  `--bin mfb` denominator where in-process; the render runs in the headless/real subprocess.
- Runtime proof: canvas programs on Linux (X11 + Wayland) and Windows render the golden scene,
  resize, destroy an image, idle — visually correct, resource freed once.
- Doc sync: `src/docs/spec/app/` canvas Vulkan backend section (loader, per-session surface,
  swapchain recreate); `.ai/arch-abi.md` note on the new Windows Vulkan path vs the GDI app path.
- Acceptance: the per-phase targeted tests above; Vulkan tolerance goldens pass on both
  platforms; C's software exact-match goldens still pass unchanged. **No full-suite run
  and no codegen byte-identity check in this letter** (A's invariant 8); fmt.

## Open Decisions

- **Split F into F-1 (Linux) / F-2 (Windows)** — recommended: yes if Phase 3 grows past a
  sitting; keep letter order (F-1 Linux before F-2 Windows). Decide after Phase 2. (§Effort)
- **X11 vs Wayland at runtime** — recommended: detect the GTK backend at surface-creation time
  and pick the extension; support both, don't hardcode. (§Phase 1)
- **Headless Vulkan in CI** — recommended: run Vulkan goldens on lanes with a GPU or software
  rasterizer (lavapipe) available; keep the software exact-match goldens as the always-headless gate. (§Phase 1)

## Corrections

**2026-08-30 — pre-execution revision (no code written yet).** See plan-98-A's
Corrections for the full account. Applied here: A's invariant 8 (this is new work, so
no codegen byte-identity gate and no full-suite run until the end of the plan); the
per-phase acceptance lines now name targeted tests; and the software rasteriser's
reference images are called **exact-match** rather than "byte-exact goldens", so this
plan's own new oracle is not confused with the repo's `tests/byte-identity/` codegen
drift gate. No design decision changed. This letter cited no paths that moved in the
2026-08-16/17 restructurings, so no remap was needed.

<Further corrections filled in during execution — especially the runtime
session-detection result and the Windows Vulkan+GDI coexistence findings.>

## Summary

F reuses E's proven pipeline and D's boundary to add Vulkan on Linux (from GTK's native handle,
no windowing backend needed) and Windows (new GPU path alongside GDI). Risk is swapchain
lifecycle and the Windows-new-code path; the gate is tolerance-match to the software oracle,
which stays the exact-match CI truth throughout.

## Corrections

**Correction 1 (Phase 1) — no surface, no swapchain, no session detection.** The
phase was written around a `VkSurfaceKHR` created from GTK's native handle and a
swapchain presented to it. plan-98-E Correction 5 had already moved the Metal
renderer offscreen — it draws into a texture and reads it back so the frame leaves
through the same `canvas::blitSurface` the software path uses — and the same
reasoning applies here with more force.

It removes the two surface extensions, the X11-vs-Wayland session detection, the
swapchain, and the out-of-date/suboptimal handling (redraw trigger 4) from this
letter. What it buys is that the renderer is *testable*: an offscreen Vulkan render
needs no display server, and no reachable Linux box has one — 2225 and 2226 refuse
connections and 2228 reports `XDG_SESSION_TYPE=tty`. A swapchain path could not have
been run at all.

The `GdkSurface*` plan-98-A stores is therefore unused by this letter. It is not
wasted: it is exactly what a future direct-to-swapchain present needs, and that
present is the same deferred on-screen path plan-98-E left for its `CAMetalLayer`.

**Correction 2 (Phase 1) — entry points come from `dlsym`, not
`vkGetInstanceProcAddr`.** The spec only guarantees that `vkGetInstanceProcAddr` is
exported, so bootstrapping through it is the textbook route. Measured on 2228 with
loader 1.4.309, `vkGetInstanceProcAddr(NULL, "vkCreateInstance")` returned NULL
where `dlsym(handle, "vkCreateInstance")` returned a working pointer, and
`libvulkan.so.1` exports every core entry point by name.

`vkGetInstanceProcAddr` is still resolved and null-checked — as the test that the
library which answered the `dlopen` is really a Vulkan loader — but nothing calls
through it. This is not a fallback: a name that is genuinely absent still fails the
probe, which is what "no Vulkan here" is supposed to mean.

**Correction 3 (Phase 1) — a missing prerequisite: app mode did not work on Linux
x86-64 at all.** Not a plan divergence but a blocker no phase covered, and the
skill's rule is that such a thing was a missing prerequisite: add it, satisfy it,
continue.

Canvas — and Console — segfaulted on the first `app::setMode` on Linux x86-64, with
a pre-plan-98-D compiler as much as this one. Two pre-existing bugs, both x86-64
only:

* **The program entry needs the +8 call parity when it is *called*.** `entry.rs`
  builds its own frame and never passes through `finalize_frame`, so it never got
  `frame_call_padding()`. Correct for an ordinary program — the kernel enters
  `_start` at `rsp%16==0` with no return address pushed — and wrong in app mode,
  where the worker shim *calls* the entry at `rsp%16==8`. The symptom was a fault in
  `__libc_calloc` under `g_idle_add`, which reads as heap corruption; the faulting
  instruction was `movaps %xmm0,(%rsp)`, an *aligned* store, and gdb on the core put
  `rsp` at `…a08`.
* **The GTK finish helper compared an uninitialized register.** It loaded
  `ST_TEXT_BUFFER` into raw `"x9"` and compared `abi::SCRATCH[0]`. `%scratch0`
  realizes to `x9`, so they are one register on AArch64 — but the x86 app wrap
  renames each distinct *token string* to its own vreg.

A third row was added for the same reason: **Linux had no headless app gate**, so
canvas could not be run at all on a box without a display. `MFB_GTKAPP_HEADLESS` is
the twin of `MFB_MACAPP_HEADLESS`/`MFB_WINAPP_HEADLESS`.

With all three, an app-mode program runs on Linux x86-64 for the first time, and the
canvas frame it renders is **byte-identical to the macOS render of the same scene** —
2,304,000 bytes, two ISAs, two operating systems. `helper_render.rs` claims the
software rasteriser produces identical pixels on every target; that is now measured.

**Correction 4 (Phase 1) — `useMetal` was the wrong name, and the name was the bug.**
`canvas::useMetal` began as "is Metal selected" and hard-returned FALSE off macOS. It
had since become the *one* renderer-selection flag both GPU backends read, so that
early return silently made `MFB_CANVAS_METAL=1` a no-op on Linux — the Vulkan arm
could not be reached no matter how ready it was, and the first "Vulkan" frame was
measured byte-identical to the software render because the software path had produced
it.

Renamed throughout to `canvas::useGpu` / `canvas::setGpuMode` / `MFB_CANVAS_GPU`, and
the platform gate deleted. It now reports the flag and nothing else: "was a GPU asked
for" and "is one usable here" are different questions, and `canvas::metalReady` /
`canvas::vulkanReady` already answer the second. Folding them together is what caused
this.

**Correction 5 (Phase 1) — one readiness probe, not two.** The letter briefly had
`canvas::vulkanAvailable` (a device exists) beside `canvas::vulkanReady` (a pipeline
built), mirroring the Metal pair. They disagreed on box 2227: `vulkanAvailable`
reported FALSE on a machine where the pipeline demonstrably built and rendered a
**byte-identical** frame. `vulkanAvailable` was removed rather than debugged — nothing
gated on it, the renderer gates on `vulkanReady`, and two probes of overlapping facts
that can disagree are worse than one. The acceptance script's skip now keys off the
same flag the runtime does, so the test and the renderer cannot disagree about whether
the GPU path was taken.

That episode also found a real defect in `vulkanReady`, now fixed: it pre-set the
device count to the array capacity (which is how Vulkan's enumerate-into-an-array form
is told how much room it has) and then read the count without checking the call's
`VkResult`. A failed enumerate leaves the capacity there, so a machine with no devices
reads as eight — and the renderer would take stack garbage for a `VkPhysicalDevice`.

**Correction 6 (Phase 1) — the Vulkan predicate is stricter than the Metal one.**
`__canvas_vulkanRenderable` declines any scene containing a `Polygon`, because a
polygon's per-edge array does not fit a push-constant block and needs a
descriptor-bound buffer (Phase 2). Reusing `__canvas_metalRenderable`, which accepts
polygons because the Metal shader draws them, was the tempting shortcut and would have
rendered polygons as nothing while reporting success — the same lie both predicates
exist to prevent. Measured before it was fixed: the scene differed from the oracle on
4,610 pixels, every one of them the single triangle.

**Correction 7 (Phase 1) — `TRIANGLE_STRIP` is 4, and this is the second time.**
`VK_PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP` is 4; 5 is `TRIANGLE_FAN`. A fan over
strip-ordered vertices is not an error — it draws two real triangles that are not the
quad — and came out as a shape missing its lower-right corner, 4.44% of pixels wrong.
plan-98-E Correction 8 records the identical mistake in Metal's enum
(`MTLPrimitiveTypeTriangleStrip` is 4, and 3 is the triangle *list*). Two GPU APIs,
two different enums, the same off-by-one, both found only by looking at pixels.
