# plan-98-A: Canvas presentation mode — mode plumbing & reconcile seam

Last updated: 2026-08-15
Overall Effort: huge (>3d)
Effort: large (3h–1d)
Depends on: nothing

This sub-plan adds `Mode.Canvas` (discriminant `2`) to the app-mode `Mode`
enum and wires the per-platform reconcile seam to **build and tear down an
empty layer-backed surface** on entry/exit — no drawing, no GPU. The single
checkable outcome: a program that calls `app::setMode(Mode.Canvas)` in an
`--app` build enters canvas mode, a blank canvas surface is created on the UI
thread (macOS `CAMetalLayer`-capable view / GTK window / Win32 HWND), switching
back to `Console`/`None` tears it down cleanly, and every existing app-mode and
console test stays byte-identical because the new slot value and gate are inert
for programs that never select `Canvas`.

This is design-doc **build step 1**. It validates the mode plumbing against the
existing test suite before any scene/GPU machinery exists.

References:

- The design summary (this feature's north star) — the "Integration With Existing
  App Mode", "Mode-gated I/O", "Teardown on mode switch", and "Platform Surfaces"
  sections.
- `src/docs/spec/app/05_presentation-mode.md` — the mode model, "universal I/O
  degrades, specialized I/O hard-fails" asymmetry, the presentation-mode slot.
- `src/docs/spec/app/04_term-backend.md` — the content-view swap (`term::on`/`off`)
  and the UI-thread-owned snapshot handoff this feature mirrors.
- `src/docs/spec/app/01_macos-runtime.md`, `02_linux-runtime.md` — the worker/main
  bootstrap this seam hooks.
- `.ai/arch-abi.md`, `.ai/resources-packages.md`, `.ai/testing-gates.md` — read
  before touching per-arch app code, package registration, and the gate suites.

---

## The plan-98 split (this section lives in A only)

`plan-98` is the consolidated 2D graphics / canvas mode. It is **huge**, so it is
split by effort into lettered sub-plans; **letter order is implementation order**
(A lands before B before C …). Each is a complete plan file.

| Letter | Scope (design-doc build step) | Effort | Depends on |
|---|---|---|---|
| **A** | `Mode.Canvas` variant + reconcile-seam build/teardown of an empty surface (step 1) | large | — |
| **B** | `DrawItem` union, deep copy, scene arena, content hashing, resource handle+refcount table (step 2) | large | A |
| **C** | Software rasteriser + golden-image harness w/ tolerance policy (step 3) | large | B |
| **D** | Graphics thread + triple-buffer scene ring + resize handshake + **fence-gated retain/release protocol** (step 4) | x-large→split if it grows | C |
| **E** | Metal backend (step 5) | large | D |
| **F** | Vulkan backend — Linux + Windows (step 6) | x-large→split if it grows | E |
| **G** | Text (stb path), atlas eviction, `measureText`/`TextMetrics`, damage-rect present (step 7) | large | F |

A–D deliver the design doc's "fully shippable software-path product" (canvas mode
that renders correctly headless with no GPU). E–G add GPU backends and real text.
E/F/G phase detail is written against the scene format that C/D make real; where a
GPU specific is not yet pinnable it is marked `UNVERIFIED`/`UNMEASURED` in that
sub-plan, to be resolved when D lands — never guessed.

## Cross-cutting invariants (decided once, binding on every letter)

These are design decisions that span sub-plans. They are recorded here so no later
letter re-litigates them.

1. **`present()` does all content-dependent work; the graphics thread does only
   swapchain-dependent work.** Deep copy, bounds, hashing, tessellation, stroke
   expansion, text shaping/raster, vertex-buffer build → `present()`. Command-buffer
   record/submit/present, swapchain acquire/recreate → graphics thread. This makes
   per-frame cost constant. It is the single most important invariant; no letter may
   move content work onto the frame path.

2. **`present()`'s cost is charged to the language worker's frame budget; the
   graphics thread's cost is not.** Animated content (a clock, a game, a scroll)
   calls `present()` every frame, so for those the deep copy + O(n) hash + any
   cache-miss geometry work runs synchronously inside the program's ~16 ms budget.
   The runtime-side geometry cache (keyed on content hash) is therefore **load-bearing
   for animation**, not an optional optimization: re-`present()` of an unchanged item
   must be free.

3. **A published scene may point at nothing caller-owned.** `present()` deep-copies
   transitively (params, polygon point arrays, text strings, referenced resources)
   into runtime-owned scene arena storage, because the graphics/render thread reads
   the scene at arbitrary times after `present()` returns with no further program
   involvement.

4. **Resource lifetime is refcount + generation, retirement is fence-gated.** `Image`/
   `Font` handles are `{u32 index; u32 generation;}` into a runtime table with an
   atomic refcount. Two hazards are normative and land in B (table/handles) and D
   (retire/release):
   - **Copy-vs-free race:** the generation check and the refcount increment in
     `present()` must be a single atomic step. The normative sequence is *increment
     refcount unconditionally, then re-read generation; if it changed, decrement and
     treat the handle as dead.* Never check-then-increment (a slot can be freed and
     resurrected in the window between).
   - **Retirement is fence-gated, not swap-gated.** A scene decrements its resource
     refs only when the **frame fence that referenced it signals**, not when it stops
     being `live` — a command buffer recorded from a retired scene may still execute.
     Each in-flight frame carries the set of scenes/resources it touched.

5. **GPU goldens are tolerance-based; the software backend is the byte-identity
   oracle.** Blending, AA, SDF `fwidth`, and sRGB rounding differ per driver/GPU, so
   Metal/Vulkan output will not be byte-identical to each other or to the software
   rasteriser. Decision: the **software** backend gets deterministic byte-identity
   goldens (the CI oracle, per this repo's `tests/byte-identity/` discipline); GPU
   backends are compared to the software reference with a per-channel epsilon / SSIM
   tolerance. This is settled in C and binds E/F.

6. **The `DrawItem` variant set is closed up front.** Adding a variant to a
   user-visible `UNION` is a breaking change (a `SELECT CASE` over it becomes
   non-exhaustive). B freezes the full set — Image, Rectangle, Line, Polygon, Text,
   and a rounded/SDF primitive — rather than shipping a subset and appending later.

7. **Software backend is permanent and first-class**, not a fallback: it keeps the
   GPU backends honest via golden images, guarantees canvas mode can never fail for
   lack of a GPU, and lets the full suite run headless.

## Prerequisites

This is sub-plan A; it has no plan dependency. Its preconditions are environmental.

| Must be true | Command | Status |
|---|---|---|
| App-mode enum & reconcile seam exist as described | `rg -n "EXPORT ENUM Mode" src/builtins/app_package.mfb` → hit | MET |
| Reconcile hook is a platform trait method | `rg -n "fn emit_app_mode_reconcile" src/target/shared/code/types.rs` → hit | MET |
| Presentation-mode slot machinery is live | `rg -n "presentation_mode_offset" src/target/shared/code` → hits | MET |
| Full suite green at HEAD | `cargo test` → pass | UNVERIFIED (run before starting) |

> The Status column is a snapshot; the Command column is the truth. Re-run and
> re-confirm before starting and before stopping.

## 1. Goal

- In an `--app` build, `app::setMode(Mode.Canvas)` transitions the presentation
  mode to `Canvas` (discriminant `2`), and the per-platform reconcile seam **builds
  an empty canvas surface** on the UI thread: macOS a layer-backed `NSView` whose
  layer is (or can host) a `CAMetalLayer`; Linux a GTK window whose native surface
  handle is retrievable; Windows an HWND. Switching to `Console` or `None`
  **tears the surface down** (mirror of the implicit `term::off`).
- `app::getMode` returns `Mode.Canvas` while in canvas mode.
- A canvas-default program (one whose static default mode is `None`, since it
  necessarily calls `setMode`) starts windowless — no transcript flash — exactly
  as a `None`-default program does today.
- No drawing occurs; there is no scene, no GPU device, no swapchain. This phase is
  pure surface lifecycle.

### Non-goals (explicit constraints)

- **No change to `Console = 0` / `None = 1` discriminants or their stored slot
  values.** `Canvas = 2` is appended; discriminants are declaration order and are
  the stored value with no remap (`app_package.mfb` enum comment).
- **No new `Result`-typed surface in `app::`.** Mode entry failure surfaces via the
  existing trappable-error path, matching `term::` (which has no `Result` returns).
- **Byte-identical output for every program that does not select `Canvas`.** The new
  slot value, the appended enum variant, and any `Canvas`-arm gate must be inert for
  console and existing app-mode programs — same discipline as `prepend_wrong_mode_gate`
  being a no-op when `presentation_mode_offset` is `None`.
- No `canvas::` drawing package yet (that is B onward). This sub-plan may register
  the package *shell* only if needed to host `setMode` gating; drawing calls are not
  added here.

## 2. Current State

App mode already solved the hard structural problems this feature extends. Cited:

- **`Mode` enum, source of truth:** `src/builtins/app_package.mfb` — `EXPORT ENUM
  Mode` with `Console = 0`, `None = 1`; the comment states discriminants are exactly
  the stored values and that a future `Canvas` mode is "a new variant appended here".
  Type surfaced to the compiler in `src/builtins/app.rs:APP` via `APP_TYPES`
  (`Mode` as `TypeKind::Enum`); callables `app.getMode`/`app.setMode`
  (`GET_MODE`/`SET_MODE`, both `Lowering::Inline`).
- **Mode storage = arena slot:** a per-arena presentation-mode word at
  `presentation_mode_offset` (`src/target/shared/code/types.rs:1135`; seeded by
  `src/target/shared/code/entry.rs:23:seed_presentation_mode_offset`), reserved one
  slot past the `term::` state region, app builds only. Non-app builds reserve no
  slot (`Option<usize>` is `None`), which is what keeps console output byte-identical.
- **setMode codegen:** `src/target/shared/code/app.rs:emit_set_mode` **stores the
  discriminant to the slot, then calls the reconcile seam** — store-before-reconcile
  so the seam re-reads the authoritative mode from memory, not a clobbered register
  (its doc comment says so explicitly). `emit_get_mode` loads the word.
- **Reconcile seam (the hook this sub-plan implements a `Canvas` arm of):**
  `src/target/shared/code/types.rs:1087:CodegenPlatform::emit_app_mode_reconcile`
  (default no-op → `None`), called from `emit_set_mode`.
  - macOS: `src/target/macos_aarch64/code.rs:212` → `src/target/macos_aarch64/app/mod.rs:emit_reconcile_seam`
    (mod.rs:716); worker marshals `mfbReconcile:` to the main thread via
    `performSelectorOnMainThread:waitUntilDone:YES`. Symbols `RECONCILE_SYMBOL`
    (`_mfb_macapp_reconcile`), `RECONCILE_MARSHAL_SYMBOL` (worker side, **no-op when
    headless** — no run loop to drain), `RECONCILE_BUILD_SYMBOL` (builds a transcript
    window on first `None`→`Console`).
  - Linux: `src/target/linux_gtk/bootstrap.rs:emit_reconcile_seam` (318) marshals via
    `g_idle_add`; callback `emit_reconcile_idle_helper` (348). A `None`-start program
    takes `g_application_hold` (289) which the reconcile balances.
  - Windows: GDI path in `src/target/win_x86_64/app/mod.rs` (no `term::`-style reconcile
    build helper today; the new mode's teardown/build hook is new code here).
- **Worker/main split the surface build hooks into:**
  - macOS `src/target/macos_aarch64/app/bootstrap.rs:emit_main_bootstrap` — `_main`
    allocs `NSWindow`, synthesizes `TermView : NSView` (bootstrap.rs:212); worker is a
    pthread spawned from `applicationDidFinishLaunching:` via `gui_defer_worker`
    (bootstrap.rs:521).
  - Linux `src/target/linux_gtk/bootstrap.rs:emit_activate_handler` (121) builds the
    `gtk_application_window_new` window and spawns the worker; a `None`-default program
    skips the surface and holds the app.
  - Windows `src/target/win_x86_64/app/mod.rs` — `_main` `RegisterClassExW`/
    `CreateWindowExW`, worker `WORKER_SYMBOL`, `WNDPROC_SYMBOL` handles `WM_DESTROY`.
- **Mode-gated I/O asymmetry (the model the `Canvas` gate extends):**
  `src/target/shared/code/app.rs:prepend_wrong_mode_gate` loads the presentation slot,
  branches on `== 0` (Console) else raises trappable `ErrWrongMode`; applied at
  `src/target/shared/code/mod.rs:1977` (every app-mode `term::` helper) and `mod.rs:2529`
  (blocking `io::` reads). `io::print` **degrades to stdout** outside `Console`
  (`src/docs/spec/app/05_presentation-mode.md:88-91`, "universal I/O degrades,
  specialized I/O hard-fails").
- **Package registration seams (for the `canvas::` shell, used from B on):**
  `src/builtins/descriptor.rs:643:REGISTRY`, test mirror
  `src/builtins/mod.rs:1074:ALL_BUILTIN_PACKAGES`, per-backend advertised calls
  `BackendCapabilities.runtime_calls` (`src/target.rs:106`, e.g.
  `src/target/macos_aarch64/mod.rs:33`), enforced by
  `src/target/shared/validate/capabilities.rs:validate_capabilities`.
- **Headless harness:** `MFB_MACAPP_HEADLESS` (`src/target/macos_aarch64/app/mod.rs:75:STR_HEADLESS_ENV`,
  read in bootstrap.rs:71-79 — skips window + `[NSApp run]` but runs full AppKit
  construction + worker), `MFB_WINAPP_HEADLESS`, and the GTK equivalent.
  `scripts/test-macapp.sh` builds+launches a bundle headless. Byte-identity codegen
  corpus at `tests/byte-identity/term/golden/`.

### Measured populations

| What | Count | Command |
|---|---|---|
| `Mode` enum variants today | 2 (Console, None) | `rg -n "Console = 0\|None = 1" src/builtins/app_package.mfb` |
| Platform reconcile impls to add a `Canvas` arm to | 3 (macos, linux_gtk, win) | `rg -rn "emit_app_mode_reconcile\|emit_reconcile_seam" src/target` |
| Backends advertising `app.setMode` in `runtime_calls` | UNMEASURED | `rg -n '"app.setMode"' src/target/*/mod.rs src/target/*/*.rs` (run in Phase 1) |
| Sites reading the presentation slot / gating on mode | 2 gate sites | `rg -n "prepend_wrong_mode_gate" src/target/shared/code/mod.rs` |

### Verified properties

- **Appending `Canvas = 2` is slot-safe.** VERIFIED from `app_package.mfb`'s own
  comment that discriminants are declaration order and are the stored value with no
  remap; existing stored values `0`/`1` are undisturbed.
- **The gate no-ops for non-canvas programs.** VERIFIED by the existing
  `prepend_wrong_mode_gate` pattern being a no-op when `presentation_mode_offset` is
  `None`; a `Canvas` arm added to the same branch inherits that.
- **Headless construction exercises the surface path without a window server on
  macOS.** VERIFIED from bootstrap.rs:71-79 (headless still runs AppKit construction +
  worker). UNVERIFIED for the GTK/Windows equivalents doing full surface construction
  headless — Phase task confirms before relying on it.

## 3. Design Overview

Three independent pieces, layered:

1. **Enum + validation (worker-visible, no platform code).** Append `Canvas = 2` to
   `app_package.mfb`; confirm `app.setMode` accepts it and `app.getMode` returns it.
   Lowest risk — pure declaration. Land first.

2. **The `Canvas` reconcile arm per platform (surface build/teardown).** Extend each
   `emit_*_reconcile` to branch on the new discriminant: on entry build an empty
   layer-backed surface; on exit (to `Console`/`None`) tear it down. This mirrors the
   existing `term::on`/`off` content-view swap but produces a bare
   `CAMetalLayer`-capable view (macOS) / native-handle-bearing window (GTK) / HWND
   (Windows) with **no renderer attached**. Highest structural risk (three platforms,
   UI-thread marshaling), so it lands behind the enum and behind a teardown test.

3. **The mode gate arm.** `canvas::` calls (none yet) will require `Canvas`; `term::`
   calls already hard-fail outside `Console` and continue to. This sub-plan only needs
   the gate to keep treating non-`Console` as wrong-mode for `term::`/blocking-`io::`
   — no code change if the existing `!= 0` branch already covers `Canvas`. Confirm,
   don't assume.

**Where correctness risk concentrates:** the per-platform teardown ordering (build a
surface, switch away, ensure no leaked view/window/HWND and no dangling UI-thread
references). This is the mode-switch mirror the design doc calls out. It lands last,
behind an explicit teardown test target (which cannot assert pixels yet, but can
assert clean enter→exit→re-enter cycles and no crash under headless).

**Byte-identity is the right gate for the inertness claim only** (non-canvas programs
must be byte-identical) — verified via the `tests/byte-identity/` corpus. The new
behavior (surface build) is **not** a byte-identity claim; it is verified by runtime
enter/teardown tests. A byte-identity diff on a non-canvas fixture is a bug to
root-cause (objdump one fixture), never a signal the design is dead.

**Rejected alternatives:**
- *A separate `Subsystem` rather than a new `Mode`.* Rejected in the design doc:
  graphics is a presentation mode; app mode already owns worker/main split, retained
  double-buffered surface, and marshaling. Reusing `Mode` inherits all of it.
- *Start canvas programs in `Console` then switch.* Rejected: a canvas program
  necessarily calls `setMode`, so starting `None` avoids a transcript flash — same
  reasoning as existing `None`-default programs.

## Compatibility / Format Impact

- **Changes:** one appended enum variant `Mode.Canvas = 2` (additive; no existing
  discriminant moves). New per-platform reconcile arms. New presentation-slot value
  `2` is now reachable.
- **Unchanged:** `Console`/`None` discriminants and stored values; the presentation
  slot layout and offset; `io::`/`term::` gate semantics; all console (non-app)
  codegen (byte-identical); the `app::` `Result`-free error contract.

## Phases

> Keep checkboxes current in the same commit as the work. An unticked box means NOT
> DONE.

### Phase 1 — Append `Mode.Canvas` and confirm setMode/getMode round-trip

Pure declaration; safe to land alone because no platform arm consumes `2` yet
(reconcile default-no-ops on an unknown discriminant, leaving mode a stored word).

- [ ] Add `Canvas = 2` to `EXPORT ENUM Mode` in `src/builtins/app_package.mfb`,
      following the existing comment's "appended variant" note.
- [ ] Measure and, if needed, advertise: run `rg -n '"app.setMode"' src/target` to
      confirm every `--app` backend's `runtime_calls` still covers `app.setMode`
      (no new call name is introduced, so this should already pass —
      record the result).
- [ ] Confirm `emit_set_mode` stores `2` and `emit_get_mode` reads it back with no
      arm-specific change (`src/target/shared/code/app.rs`).
- [ ] Tests: add an app-mode integration case (alongside `tests/cli_macos_app_io_input_imports.rs`
      / the linux app-mode tests) asserting `app::setMode(Mode.Canvas)` then
      `app::getMode() = Mode.Canvas` under `MFB_MACAPP_HEADLESS=1` / GTK-headless,
      with the reconcile still default-no-op (mode stored, no surface yet).

Acceptance: a headless app program sets and reads back `Mode.Canvas`; the full
`cargo test` suite passes; the `tests/byte-identity/` corpus is unchanged for all
non-canvas fixtures.
Commit: —

### Phase 2 — Presentation gate covers `Canvas` for `term::`/blocking-`io::`

Confirm (and only if wrong, fix) that specialized I/O hard-fails in `Canvas` exactly
as in `None`.

- [ ] Read `src/target/shared/code/app.rs:prepend_wrong_mode_gate`: verify its
      "not Console" branch already raises `ErrWrongMode` for slot value `2`
      (it branches on `== 0`, so `2` already falls to the raise). Record the read.
- [ ] Confirm `io::print`/`io::write` still **degrade to stdout** in `Canvas`
      (they are not gated by `prepend_wrong_mode_gate`; verify no `Console`-only
      assumption elsewhere keys off the exact value `1`).
- [ ] Tests: add cases asserting `term::sync` in `Mode.Canvas` traps `ErrWrongMode`
      (trappable), and `io::print` in `Mode.Canvas` writes to stdout.

Acceptance: `term::` in canvas mode raises trappable `ErrWrongMode`; `io::print` in
canvas mode reaches stdout; both proven by tests. No codegen change unless the read
finds a value-`1`-specific assumption, in which case fix it and cite the diff.
Commit: —

### Phase 3 — Per-platform reconcile `Canvas` arm: build/teardown an empty surface (largest blast radius last)

Three platforms; each builds a bare surface on entry and tears it down on exit. No
renderer, no GPU, no scene.

- [ ] macOS: extend `src/target/macos_aarch64/app/mod.rs:emit_reconcile_seam`
      (and its `RECONCILE_BUILD_SYMBOL` helper) with a `Canvas` arm that swaps the
      content view for a layer-backed `NSView` (`wantsLayer = YES`) sized to the
      window, and a teardown arm on exit that restores/removes it. Reuse the existing
      `NSWindow` + content-view swap machinery (the design doc's "substitute for
      TermView"). Marshal on the main thread (`waitUntilDone:YES`), no-op headless
      marshal as today.
- [ ] Linux: extend `src/target/linux_gtk/bootstrap.rs:emit_reconcile_seam` /
      `emit_reconcile_idle_helper` with a `Canvas` arm that ensures the GTK window
      exists and its native surface handle is retrievable
      (`gdk_x11_surface_get_xid`/`gdk_wayland_surface_get_wl_surface` — retrieval only,
      no Vulkan yet), balancing `g_application_hold` on exit.
- [ ] Windows: extend `src/target/win_x86_64/app/mod.rs` reconcile/mode path with a
      `Canvas` arm that ensures the HWND exists for later `VK_KHR_win32_surface` use
      and tears it down on exit (new code — no `term::` reconcile precedent here).
- [ ] **Teardown test target** (the design doc's highest-crash-risk mirror): a
      headless test that enters `Canvas`, exits to `None`, re-enters `Canvas`, and
      exits — asserting no crash, no leaked window/view/HWND, clean worker/main
      marshaling. It cannot assert pixels yet; it asserts lifecycle only. Wire it for
      all three headless env vars.

Acceptance: under each platform's headless mode, enter→exit→re-enter→exit of
`Mode.Canvas` completes without crash or leak; the surface's native handle is
retrievable while in canvas mode and released after exit; full `cargo test` green;
non-canvas byte-identity corpus unchanged.
Commit: —

## Validation Plan

- Tests: app-mode integration cases for setMode/getMode round-trip (Phase 1), the
  `ErrWrongMode`/`io::print`-degrade gate (Phase 2), and the enter/teardown lifecycle
  (Phase 3), under `MFB_MACAPP_HEADLESS` / GTK-headless / `MFB_WINAPP_HEADLESS`.
- Coverage check: confirm the new reconcile arms are in the `--bin mfb` denominator
  (per `.ai/build-tooling.md` coverage mechanics — src/ coverage is in-process bin
  unit tests; the headless subprocess is integration and uncaptured). Add `.ncode`
  or in-process unit tests for the arm-selection logic so the changed codegen is
  actually measured, not just exercised by the uncaptured subprocess.
- Runtime proof: a small `--app` program `app::setMode(Mode.Canvas)` then a blank
  loop, launched headless on each platform, exits 0 with the surface built and torn
  down (observable via the lifecycle test's assertions / logs).
- Doc sync: update `src/docs/spec/app/05_presentation-mode.md` (add `Canvas` to the
  mode table and the I/O degrade/hard-fail matrix) and `app_package.mfb`'s enum doc.
  Per `.ai/specifications.md`, keep the spec current with the compiler change.
- Acceptance: full `cargo test`; `tests/byte-identity/` corpus unchanged for
  non-canvas fixtures; `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup
  run 1.96.0 cargo fmt)` at session end.

## Open Decisions

- **Mode name: `Canvas` vs `Graphics`.** The design summary wrote `Mode.Graphics`;
  the existing enum comment anticipates a `Canvas` mode and the package is `canvas::`.
  Recommended: **`Canvas`**, for name-parity with the `canvas::` package. (§1)
- **Does the `canvas::` package shell register in this sub-plan or in B?** Recommended:
  **B** — this sub-plan needs no drawing call, so registering an empty package here
  would add a `runtime_calls` surface with nothing behind it. Register the package
  when its first call (`present`) exists. (§Non-goals)
- **Windows surface in Phase 3 without a renderer.** An HWND with no WM_PAINT content
  is fine, but confirm the GDI message loop doesn't assume a term memDC exists in
  canvas mode. Recommended: gate the term memDC paint on `mode == Console`. (§Phase 3)

## Corrections

<Filled in during execution.>

## Summary

The real risk in A is the per-platform teardown ordering (Phase 3): three surface
lifecycles, UI-thread marshaling, and the requirement that switching modes leaves no
leaked native object — the mirror of the implicit `term::off`. Everything before it
(enum append, gate confirmation) is low-risk and independently valuable. Untouched by
A: all scene/geometry/GPU machinery (B onward), the resource refcount table (B/D), and
any drawing whatsoever.
