# plan-98-F: Canvas Vulkan backend (Linux + Windows)

Last updated: 2026-08-15
Effort: x-large (1d–3d) — split F-1 (Linux) / F-2 (Windows) if it exceeds one sitting
Depends on: plan-98-E (Metal backend — proves the renderer-swap seam on a real GPU)

This sub-plan adds the **Vulkan** renderer for Linux (GTK4 native surface) and Windows
(HWND), behind the same D thread/ring/retirement boundary, reusing the exact
single-pipeline shader design proven on Metal in E. After it lands, canvas programs render
via Vulkan on Linux and Windows, matching the C software golden within tolerance
(invariant 5), with the software backend still the byte-exact CI oracle.

This is design-doc **build step 6**. GPU-specifics depend on D's real vertex/fence contract
(confirmed in E) and are marked `UNVERIFIED`/`UNMEASURED` where not yet pinnable.

References:

- The design summary — "Platform Surfaces" (Linux GTK4→`VK_KHR_xcb/wayland_surface`,
  Windows HWND→`VK_KHR_win32_surface`), "Resize handshake" (`vkDeviceWaitIdle` + recreate
  swapchain), "Shaders", the `dlopen("libvulkan.so.1")` + `vkGetInstanceProcAddr` note.
- plan-98-A invariant 4 (Vulkan submit fence advances `lastCompletedFrame`, driving the
  closed-flag texture free; no refcount), invariant 5 (tolerance).
- plan-98-D resize handshake (the Vulkan swapchain-recreation path is the concrete case D
  abstracted), plan-98-E's proven shader pipeline + renderer seam.
- `.ai/arch-abi.md` (Win64 PE/console; Windows app path is GDI today — Vulkan is new code).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-98-E complete (Metal proves the renderer seam on GPU) | `ls planning/completed/plan-98-E-*` → hit | NOT MET |
| A retrieves the GTK native surface handle + Windows HWND | plan-98-A Phase 3 acceptance met | NOT MET |
| Full suite green at HEAD | `cargo test` → pass | UNVERIFIED |

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
  stays byte-exact.

### Non-goals (explicit constraints)

- **No shared thread/ring/retirement changes** — Vulkan drops in behind D's seam exactly as
  Metal did in E.
- **No X11/Wayland windowing backend** — GTK4 owns the window; F only creates a Vulkan surface
  from GTK's native handle (design "Key saving").
- **No text** (G).
- **Software backend stays first-class and byte-exact.**

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
| Vulkan surface backends | 3 (xcb, wayland, win32) | design "Platform Surfaces" |
| Platforms | 2 (Linux, Windows) | — |
| Shaders reused from E | same set | invariant "one pipeline" |
| Vulkan entry points to load via `vkGetInstanceProcAddr` | UNMEASURED | enumerate in Phase 1 |

### Verified properties

- **Vulkan needs no SDK** (runtime `dlopen` + `vkGetInstanceProcAddr`) — VERIFIED per the
  design note; satisfies the no-shared-libs constraint.
- **GTK4 hands over the native surface handle**, so no X11/Wayland windowing backend is needed —
  VERIFIED per design; A already retrieves it. Phase 1 confirms both handle getters at runtime.
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
  software backend and its byte-exact goldens.

## Phases

### Phase 1 — Vulkan loader + Linux surface (xcb/wayland) + one-quad tolerance match

- [ ] Runtime `dlopen` libvulkan + bootstrap instance/device function pointers; enumerate the
      needed entry points (resolves `UNMEASURED`).
- [ ] Detect X11 vs Wayland session; create `VK_KHR_xcb_surface`/`VK_KHR_wayland_surface` from
      GTK's native handle (confirm both getters at runtime).
- [ ] Swapchain + E's single pipeline (shaders → SPIR-V); render one tinted quad; assert
      tolerance-match to the C software golden headless where GTK-headless Vulkan is available.

Acceptance: one quad renders via Vulkan on Linux (both session types where testable) and matches
the software reference within tolerance; software backend still byte-exact.
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
Full `cargo test` green.
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
- Acceptance: full `cargo test`; Vulkan tolerance goldens pass on both platforms; software
  byte-exact goldens unchanged; non-canvas byte-identity corpus unchanged; fmt.

## Open Decisions

- **Split F into F-1 (Linux) / F-2 (Windows)** — recommended: yes if Phase 3 grows past a
  sitting; keep letter order (F-1 Linux before F-2 Windows). Decide after Phase 2. (§Effort)
- **X11 vs Wayland at runtime** — recommended: detect the GTK backend at surface-creation time
  and pick the extension; support both, don't hardcode. (§Phase 1)
- **Headless Vulkan in CI** — recommended: run Vulkan goldens on lanes with a GPU or software
  rasterizer (lavapipe) available; keep the software byte-exact goldens as the always-headless gate. (§Phase 1)

## Corrections

<Filled in during execution — especially the runtime session-detection result and the Windows
Vulkan+GDI coexistence findings.>

## Summary

F reuses E's proven pipeline and D's boundary to add Vulkan on Linux (from GTK's native handle,
no windowing backend needed) and Windows (new GPU path alongside GDI). Risk is swapchain
lifecycle and the Windows-new-code path; the gate is tolerance-match to the software oracle,
which stays the byte-exact CI truth throughout.
