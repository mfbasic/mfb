# plan-98-A: Canvas presentation mode — mode plumbing & reconcile seam

Last updated: 2026-08-30
Overall Effort: huge (>3d)
Effort: x-large (1d–3d) — split A-1 (enum + gate + surface, Phases 1–3) / A-2 (window
keyboard input, Phase 4) if it exceeds one sitting
Depends on: nothing

This sub-plan adds `Mode.Canvas` (discriminant `2`) to the app-mode `Mode`
enum and wires the per-platform reconcile seam to **build and tear down an
empty layer-backed surface** on entry/exit — no drawing, no GPU. The single
checkable outcome: a program that calls `app::setMode(Mode.Canvas)` in an
`--app` build enters canvas mode, a blank canvas surface is created on the UI
thread (macOS `CAMetalLayer`-capable view / GTK window / Win32 HWND), and switching
back to `Console`/`None` tears it down cleanly.

This is **build step 1** of the A–G sequence. It validates the mode plumbing before any
scene/GPU machinery exists.

References:

> **plan-98-A is the north star for this feature.** There is no separate design
> document: `planning/plan-98-A` … `plan-98-G` plus `planning/plan-98-api.md` are the
> entire corpus. This file's "Cross-cutting invariants" section below is the top-level
> design; `plan-98-api.md` is the language-visible surface. Nothing else exists — do not
> go looking for one, and do not cite one.

- `planning/plan-98-api.md` — the full language-visible API (types, calls, error
  contract, worked example).
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

| Letter | Scope (build step) | Effort | Depends on |
|---|---|---|---|
| **A** | `Mode.Canvas` variant + reconcile-seam build/teardown of an empty surface + `io::` gate & window keyboard input (step 1) | x-large→split A-1/A-2 if it grows | — |
| **B** | `DrawItem` union, deep copy, scene arena, content hashing, `Image`/`Font` as RES resources (step 2) | large | A |
| **C** | Software rasteriser + golden-image harness w/ tolerance policy (step 3) | large | B |
| **D** | Graphics thread + triple-buffer scene ring + resize handshake + **deferred texture free** (closed-flag + frame-drain) (step 4) | x-large→split if it grows | C |
| **E** | Metal backend (step 5) | large | D |
| **F** | Vulkan backend — Linux + Windows (step 6) | x-large→split if it grows | E |
| **G** | Text (stb path), atlas eviction, `measureText`/`TextMetrics`, damage-rect present (step 7) **+ the plan's closeout: its one full-suite run** | large | F |

A–D deliver a **fully shippable software-path product** (canvas mode that renders
correctly headless with no GPU). E–G add GPU backends and real text.
**The full `cargo test --no-fail-fast` suite runs once, at the end of the whole
plan (G's closeout) — not per letter and not per phase** (invariant 8).
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

4. **Resources are RES values; the closed flag drives lifetime; there is NO
   refcounting.** MFB is not a refcounted/GC language — an `Image`/`Font` is a plain
   **RES resource** (the existing native-resource record `tag@0 / handle@8 / closed@16
   / STATE@24`, `handle@8` holding the OS-side texture id), owned by MFB scope exactly
   like a file. `close ≠ drop`: `destroy*` (or scope-drop of the owner) sets `closed@16`
   and the canvas close runtime helper marks the OS texture pending-free; the record
   memory is reclaimed by the existing scope-drop path. Using a closed resource is the
   existing `ERR_RESOURCE_CLOSED` no-op — that is the stale-id safety net (no separate
   generation table needed). The one OS-side rule, entirely invisible to MFB:
   - **Free is gated on closed AND drained, via a single monotonic compare — not a
     count.** The graphics thread stamps each texture with `lastUsedFrame` when it
     draws it (the same LRU marker the geometry cache uses with `lastUsedRev`). The OS
     texture is freed only when **`closed AND lastUsedFrame < lastCompletedFrame`** —
     i.e. MFB is done with it, it is no longer in the rendered scene (so `lastUsedFrame`
     stopped advancing), and the GPU has drained the last frame that used it. No atomic
     refcount, no per-frame reference set, no increment-then-recheck. Scenes do **not**
     retain resources; the closed flag alone ends a resource's life. Lands in B (RES
     backend + close helper) and D (the `lastUsedFrame`/`lastCompletedFrame` free gate).

5. **GPU goldens are tolerance-based; the software backend is the exact-match
   oracle.** Blending, AA, SDF `fwidth`, and sRGB rounding differ per driver/GPU, so
   Metal/Vulkan output will not match each other or the software rasteriser exactly.
   Decision: the **software** backend gets deterministic **exact-match reference
   images** (the CI oracle); GPU backends are compared to that software reference with
   a per-channel epsilon / SSIM tolerance. This is settled in C and binds E/F.
   These reference images are **new artifacts this plan creates** — they are an oracle
   for a new rasteriser, not instances of the repo's `tests/byte-identity/` codegen
   drift gate, which invariant 8 puts out of scope for the whole plan.

6. **The `DrawItem` variant set is closed up front.** Adding a variant to a
   user-visible `UNION` is a breaking change (a `SELECT CASE` over it becomes
   non-exhaustive). B freezes the full set — Image, Rectangle, Line, Polygon, Circle,
   Arc, Text, RoundedRect (8 variants; see plan-98-api.md) — rather than shipping a
   subset and appending later.

7. **Software backend is permanent and first-class**, not a fallback: it keeps the
   GPU backends honest via golden images, guarantees canvas mode can never fail for
   lack of a GPU, and lets the whole feature be tested headless.

8. **This is new work: no byte-identity gate, and no full-suite run until the end.**
   Two testing-policy decisions, binding on every letter:
   - **No codegen byte-identity obligation anywhere in plan-98.** Per AGENTS.md,
     `.ncode`/`.ncodesum` byte-identity is a *drift sentinel for pure code motion*;
     plan-98 adds new behavior, so there is nothing for its output to be identical to.
     No letter asserts "the `tests/byte-identity/` corpus is unchanged", runs
     `artifact-gate.sh`, or shapes a change to keep bytes stable. If an unrelated
     fixture's `.ncode` does drift, that is an ordinary regression to root-cause — not
     a gate this plan owns. (The one exception is *terminology*: C's software-raster
     **reference images** are exact-match by design — invariant 5 — because they are
     this plan's own new oracle.)
   - **Run only the tests the phase adds or touches; run the full suite once, at the
     end of the plan.** Each phase's acceptance is its own new tests plus whatever
     existing targets its change can actually reach (scope by blast radius). The
     single full `cargo test --no-fail-fast` run — plus the acceptance harness
     (`test-accept.sh`, which is *not* in `cargo test`) — is G's closeout. Use
     `--no-fail-fast`: plain `cargo test` stops at the first failing target and
     silently skips every `rt_*` runtime test that sorts after it.

## Prerequisites

This is sub-plan A; it has no plan dependency. Its preconditions are environmental.

| Must be true | Command | Status |
|---|---|---|
| The `Mode` enum is a registry descriptor (no `.mfb` companion) | `rg -n 'add_enum\(RegistryEnum' src/codegen/builtins/app/mod.rs` → hit | MET (re-run 2026-08-30, execution: hit at mod.rs:72) |
| Reconcile hook is a platform trait method | `rg -n "fn emit_app_mode_reconcile" src/codegen/engine/types/types.rs` → hit | MET (re-run: hit at types.rs:1157) |
| Presentation-mode slot machinery is live | `rg -n "presentation_mode_offset" src/codegen src/target` → hits | MET (re-run: 20 hits across codegen + all 3 targets) |
| `app.getMode`/`app.setMode` are `Body::abi_function` members | `rg -n "Body::abi_function" src/codegen/builtins/app/` → 2 hits | MET (re-run: func_get_mode.rs:87, func_set_mode.rs:111; 2 doc-comment mentions besides) |
| Working tree builds | `cargo build` → pass | MET (re-run: `Finished `dev` profile` in 31.66s) |

> Per invariant 8 there is deliberately **no** "full suite green at HEAD" row: the
> full suite runs once at the end of the plan, not before each letter.

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
- **`io::` works in canvas mode:** outputs degrade to stdout/stderr; blocking reads
  (`io::readByte`/`readChar`/`readLine`) are permitted and read the canvas window's key
  events (the same input source `Console` uses). `term::` still traps in `Canvas`.
- No drawing occurs; there is no scene, no GPU device, no swapchain. This sub-plan is
  surface lifecycle + mode-gated I/O (including keyboard input).

### Non-goals (explicit constraints)

- **No change to `Console = 0` / `None = 1` discriminants or their stored slot
  values.** `Canvas = 2` is appended; discriminants are declaration order and are
  the stored value with no remap (`src/codegen/builtins/app/mod.rs:register` doc
  comment).
- **No new `Result`-typed surface in `app::`.** Mode entry failure surfaces via the
  existing trappable-error path, matching `term::` (which has no `Result` returns).
- **No behavior change for non-canvas programs.** The new slot value, the appended
  enum variant, and the `term::`/`canvas::` gate are inert when
  `presentation_mode_offset` is `None`; `Console` (`== 0`) and `None` (`== 1`) read
  behavior is unchanged. This is a *behavioral* claim proven by targeted tests, **not**
  a byte-identity claim — per invariant 8 this plan asserts nothing about
  `.ncode`/`.ncodesum` output. Phase 2 does change the emitted comparison in the
  console-read gate (from "trap unless `Console`" to "trap only in `None`"), so
  app-mode codegen for those three helpers legitimately differs; that is the plan
  working.
- No `canvas::` drawing package yet (that is B onward). This sub-plan may register
  the package *shell* only if needed to host `setMode` gating; drawing calls are not
  added here.

## 2. Current State

App mode already solved the hard structural problems this feature extends. Cited
**against the tree as of 2026-08-30** — every path below moved in the two
restructurings that landed right after this plan was first written (`4ed7d60de`
"app: migrate onto clean-room registry", 2026-08-16, deleted
`src/builtins/app_package.mfb`; `f32179ed4` "relocate target-generic codegen into a
tiered `src/codegen`", 2026-08-17, deleted `src/target/shared/code/`):

- **`Mode` enum, source of truth:** `src/codegen/builtins/app/mod.rs:register` —
  `pkg.add_enum(RegistryEnum { name: "Mode", variants: [Console, None] })`. There is
  **no `.mfb` companion source**: the registry renders the enum into the injected
  package source (`get_mfb`), exactly as `money`/`datetime` do. The `register` doc
  comment states variant declaration order fixes the discriminants and that those are
  the values `setMode`/`getMode` store and load. The two callables are
  `app.getMode`/`app.setMode`, each registering itself from its own
  `func_get_mode.rs`/`func_set_mode.rs`, both **`Body::abi_function`**
  (`func_get_mode.rs:87`, `func_set_mode.rs:111`). The old `BuiltinModule`/`APP_TYPES`/
  `TypeKind::Enum`/`Lowering::Inline` vocabulary is gone tree-wide.
- **Mode storage = arena slot:** a per-arena presentation-mode word at
  `presentation_mode_offset` (`src/codegen/engine/types/types.rs:1294`
  `seed_presentation_mode_offset`; seeded in
  `src/codegen/engine/builder/mod.rs:1101`), reserved one slot past the `term::` state
  region, app builds only. Non-app builds reserve no slot (`Option<usize>` is `None`),
  which is what makes the gate inert for console programs.
- **setMode codegen:** `src/codegen/builtins/app/func_set_mode.rs:lower_set_mode`
  **stores the discriminant to the slot, then calls the reconcile seam** —
  store-before-reconcile so the seam re-reads the authoritative mode from memory, not
  a clobbered register (its doc comment says so explicitly, and specifically
  anticipates that "in C/D the reconcile emits register-clobbering `bl`s").
  `func_get_mode.rs:lower_get_mode` loads the word.
- **Reconcile seam (the hook this sub-plan implements a `Canvas` arm of):**
  `src/codegen/engine/types/types.rs:1157:CodegenPlatform::emit_app_mode_reconcile`
  (default no-op → `None`), called from `lower_set_mode` (`func_set_mode.rs:41`).
  - macOS: `src/target/macos_aarch64/code.rs:249` → `src/target/macos_aarch64/app/mod.rs:emit_reconcile_seam`
    (mod.rs:721); worker marshals `mfbReconcile:` to the main thread via
    `performSelectorOnMainThread:waitUntilDone:YES`. Symbols `RECONCILE_SYMBOL`
    (`_mfb_macapp_reconcile`, mod.rs:353), `RECONCILE_MARSHAL_SYMBOL` (worker side,
    **no-op when headless** — no run loop to drain), `RECONCILE_BUILD_SYMBOL`
    (mod.rs:363 — builds a transcript window on first `None`→`Console`).
  - Linux: `src/target/linux_common/code.rs:520:emit_app_mode_reconcile` →
    `src/target/linux_gtk/bootstrap.rs:emit_reconcile_seam` (320) marshals via
    `g_idle_add`; callback `emit_reconcile_idle_helper` (348). A `None`-start program
    takes `g_application_hold` (291) which the reconcile balances.
  - Windows: GDI path in `src/target/win_x86_64/app/mod.rs` (no `term::`-style reconcile
    build helper today; the new mode's teardown/build hook is new code here).
- **Worker/main split the surface build hooks into:**
  - macOS `src/target/macos_aarch64/app/bootstrap.rs:emit_main_bootstrap` — `_main`
    allocs `NSWindow`, synthesizes `TermView : NSView`; worker is a pthread spawned
    from `applicationDidFinishLaunching:` via `gui_defer_worker`.
  - Linux `src/target/linux_gtk/bootstrap.rs:emit_activate_handler` (121) builds the
    `gtk_application_window_new` window and spawns the worker; a `None`-default program
    skips the surface and holds the app (`g_application_hold`, bootstrap.rs:291).
  - Windows `src/target/win_x86_64/app/mod.rs` — `_main` `RegisterClassExW`/
    `CreateWindowExW`, worker `WORKER_SYMBOL`, `WNDPROC_SYMBOL` handles `WM_DESTROY`.
    Its own presentation-mode guard is at `win_x86_64/app/mod.rs:1749`.
- **Mode-gated I/O asymmetry (the model the `Canvas` gate extends):**
  `src/codegen/app/hook/app.rs:33:prepend_wrong_mode_gate` (plan-62-E) loads the
  presentation slot, branches on `== 0` (Console) else raises trappable `ErrWrongMode`.
  It has exactly **two** call sites today:
  - `src/codegen/builtins/term/gen_shared.rs:95` — every app-mode `term::` helper.
  - `src/codegen/engine/builder/mod.rs:1988` — the console-reading `io::` helpers,
    gated on the literal predicate `matches!(spec.call, "io.input" | "io.readLine" |
    "io.readChar")`.

  **Correction to this plan's first draft:** `io.readByte` is **not** in that list and
  never has been — `git log -S` over the predicate finds a single author commit,
  `b7964aa43` (plan-62-E), which shipped the three-call form. So `io::readByte`
  already works outside `Console` today; Phase 2's gate relaxation covers three calls,
  not four. `io::print` **degrades to stdout** outside `Console`
  (`src/docs/spec/app/05_presentation-mode.md:86-93`, "universal I/O degrades,
  specialized I/O hard-fails"). This sub-plan **relaxes the read gate** so `Canvas` (not
  just `Console`) permits reads.
- **Window key events already feed the `io::` read path in app mode** (so Phase 4 is a
  reuse, not new machinery): the read helpers `io::readChar`/`readByte`/`readLine`/`input`
  read fd 0 and are **not** rewritten in app mode, while the window delivers keystrokes
  into that path — Linux `_mfb_gtkapp_key_pressed` (main thread) feeds
  `readChar`/`readByte` (`src/docs/spec/app/02_linux-runtime.md`,
  `src/docs/spec/app/03_console-io.md`; the term keyboard-input work,
  plan-69-term-keyboard-input). Canvas reuses this: its window's key handler feeds the same
  path the term view does.
- **Package registration seams (for the `canvas::` shell, used from B on):** every
  builtin package now registers itself on the clean-room registry —
  `src/codegen/registry/mod.rs:1550-1576` (28 `crate::codegen::builtins::<pkg>::register(&mut r)`
  calls), test mirror `src/codegen/builtins/mod.rs:921:ALL_BUILTIN_PACKAGES` (26
  user-visible names; `general` and `testing` are internal), per-backend advertised
  calls `BackendCapabilities.runtime_calls` (`src/target.rs:106`, e.g.
  `src/target/macos_aarch64/mod.rs:33`), enforced by
  `src/target/shared/validate/capabilities.rs:7:validate_capabilities`. The registry
  already exposes `add_record` / `add_union` / `add_enum` / `add_resource`
  (`src/codegen/registry/mod.rs:1100-1145`), so B's `DrawItem` union and `Image`/`Font`
  resources are declarable as registry data with no new machinery. The authoring
  procedure is `planning/migrate.md`.
- **Headless harness:** `MFB_MACAPP_HEADLESS` (`src/target/macos_aarch64/app/mod.rs:80:STR_HEADLESS_ENV`,
  read in bootstrap.rs:87 — skips window + `[NSApp run]` but runs full AppKit
  construction + worker), `MFB_WINAPP_HEADLESS`, and the GTK equivalent.
  `scripts/test-macapp.sh` builds+launches a bundle headless.

### Measured populations

> **Correction (execution, 2026-08-30): three of these four commands used `rg -rn` /
> `rg -rln`.** `-r` is ripgrep's **replace** flag, so those commands print substituted
> text rather than the counts claimed. Every row below is re-measured with the
> corrected command; the reconcile-impl row's number changed as a result.

| What | Count | Command |
|---|---|---|
| `Mode` enum variants today | 3 (Console, None, Canvas) — was 2 before Phase 1 | `rg -n '"Console"\|"None"\|"Canvas"' src/codegen/builtins/app/mod.rs` |
| Platform `emit_app_mode_reconcile` impls to add a `Canvas` arm to | **2, not 3** (`macos_aarch64/code.rs:249`, `linux_common/code.rs:520`; the third hit is the default trait method at `engine/types/types.rs:1157`). Windows has **no** override — see Corrections 8. | `rg -n "fn emit_app_mode_reconcile" src/` |
| Backends advertising `app.setMode` in `runtime_calls` | 2 files (`macos_aarch64/mod.rs:37`, `linux_common/mod.rs:52`) — **Windows advertises neither `app.` call** | `rg -l '"app\.setMode"' src/target/` (re-measured 2026-08-30) |
| Sites reading the presentation slot / gating on mode | 2 gate call sites (`term/gen_shared.rs`, `engine/builder/mod.rs`) | `rg -n "prepend_wrong_mode_gate\(" src/ \| grep -v hook/app.rs` |

### Verified properties

- **Appending `Canvas = 2` is slot-safe.** VERIFIED from the `register` doc comment in
  `src/codegen/builtins/app/mod.rs`: variant declaration order fixes the discriminants,
  and those are the values `setMode`/`getMode` store and load, with no remap. Existing
  stored values `0`/`1` are undisturbed by appending a third `EnumVariant`.
- **The gate no-ops for non-canvas programs.** VERIFIED by the existing
  `prepend_wrong_mode_gate` pattern (`src/codegen/app/hook/app.rs:39` — early-returns
  when `presentation_mode_offset` is `None`); a `Canvas` arm added to the same branch
  inherits that.
- ~~**Headless construction exercises the surface path without a window server on
  macOS.**~~ **CORRECTED during Phase 3 (Correction 15).** True only of the
  *bootstrap's* AppKit construction, which runs before the headless branch at
  `src/target/macos_aarch64/app/bootstrap.rs:87`. It does **not** extend to the
  reconcile: headless installs no app delegate, and
  `_mfb_macapp_reconcile_marshal` skips when `[NSApp delegate]` is nil — it must,
  since headless parks the main thread in `pause()` with no run loop to drain a
  `waitUntilDone:YES` perform. Windows is the same shape (`headless_spawn` builds no
  window and runs no message pump). So **no headless run on any platform can observe
  the canvas surface**; Phase 3's acceptance is a real GUI run plus per-platform
  codegen inspection.

## 3. Design Overview

Three independent pieces, layered:

1. **Enum + validation (worker-visible, no platform code).** Append a third
   `EnumVariant { name: "Canvas", … }` to the `RegistryEnum` in
   `src/codegen/builtins/app/mod.rs:register`; confirm `app.setMode` accepts it and
   `app.getMode` returns it. Lowest risk — pure registry data. Land first.

2. **The `Canvas` reconcile arm per platform (surface build/teardown).** Extend each
   `emit_*_reconcile` to branch on the new discriminant: on entry build an empty
   layer-backed surface; on exit (to `Console`/`None`) tear it down. This mirrors the
   existing `term::on`/`off` content-view swap but produces a bare
   `CAMetalLayer`-capable view (macOS) / native-handle-bearing window (GTK) / HWND
   (Windows) with **no renderer attached**. Highest structural risk (three platforms,
   UI-thread marshaling), so it lands behind the enum and behind a teardown test.

3. **The mode gate arm + `io::` input from the window.** `term::` calls hard-fail outside
   `Console` and continue to (they trap in `Canvas`). But **`io::` works fully in `Canvas`**:
   outputs degrade to stdout/stderr as today, and the gated console reads
   (`io::input`/`io::readLine`/`io::readChar` — the three calls actually in the
   predicate) must **not** trap in `Canvas`; they read from the window's key events,
   the same input source `Console` uses. This is a change from today's
   `!= 0 → ErrWrongMode` gate: the read gate becomes "trap only in `None`" (`Console`
   and `Canvas` both have an input source; `None` has no window). `io::readByte` is
   already ungated and needs no gate change — only the Phase 4 window wiring. It
   requires wiring the canvas window's key events into the `io::` input path —
   mirroring term keyboard input (plan-69-term-keyboard-input). `term::` stays
   trap-in-`Canvas`.

**Where correctness risk concentrates:** the per-platform teardown ordering (build a
surface, switch away, ensure no leaked view/window/HWND and no dangling UI-thread
references). This is the mirror of the implicit `term::off` teardown. It lands last,
behind an explicit teardown test target (which cannot assert pixels yet, but can
assert clean enter→exit→re-enter cycles and no crash under headless).

**How the inertness claim is proven (invariant 8).** "Non-canvas programs are
unaffected" is a *behavioral* claim, proven by targeted tests: a `Console`-mode and a
`None`-mode program still gate exactly as they do today, and a non-app build still
reserves no presentation slot. It is **not** a byte-identity claim — this plan adds new
behavior, so `.ncode`/`.ncodesum` stability is neither asserted nor gated anywhere in
plan-98, and no phase runs `artifact-gate.sh`.

**Rejected alternatives:**
- *A separate `Subsystem` rather than a new `Mode`.* Rejected: graphics **is** a
  presentation mode, and app mode already owns the worker/main split, the retained
  double-buffered surface, and the UI-thread marshaling. Reusing `Mode` inherits all
  of it; a parallel `Subsystem` axis would duplicate every one of those.
- *Start canvas programs in `Console` then switch.* Rejected: a canvas program
  necessarily calls `setMode`, so starting `None` avoids a transcript flash — same
  reasoning as existing `None`-default programs.

## Compatibility / Format Impact

- **Changes:** one appended enum variant `Mode.Canvas = 2` (additive; no existing
  discriminant moves). New per-platform reconcile arms. New presentation-slot value
  `2` is now reachable. The console-read gate predicate changes to "trap only in
  `None`" so `Canvas` permits reads; canvas-window key events feed the `io::` input path.
- **Unchanged:** `Console`/`None` discriminants and stored values; the presentation
  slot layout and offset; `term::` gate semantics (traps outside `Console`); `io::`
  output degradation; `Console`/`None` `io::`-read behavior; `io::readByte` (already
  ungated); all console (non-app) behavior; the `app::` `Result`-free error contract.

## Phases

> Keep checkboxes current in the same commit as the work. An unticked box means NOT
> DONE.
>
> **Test scope for every phase (invariant 8):** run the tests this phase adds, plus the
> existing targets its change can actually reach — not the full suite. Name the exact
> command in the Commit line. The full `cargo test --no-fail-fast` run is G's closeout.

### Phase 1 — Append `Mode.Canvas` and confirm setMode/getMode round-trip

Pure registry data; safe to land alone because no platform arm consumes `2` yet
(reconcile default-no-ops on an unknown discriminant, leaving mode a stored word).

- [x] Add a third `EnumVariant { name: "Canvas", description: …, advisory: None }` to
      the `RegistryEnum` in `src/codegen/builtins/app/mod.rs:register`, after `None`
      so the discriminant is `2`. There is no `.mfb` file to edit — the registry
      renders the enum into the injected package source via `get_mfb`.
- [x] Update the in-file tests in `src/codegen/builtins/app/mod.rs` that assert the
      rendered source contains the variants, and the `MODULE_DESC` prose that today
      says the mode "is one of `Console` … or `None`". Also updated: the module doc
      comment ("two `Mode` enum members" → three) and the `register` doc comment
      (discriminant list + why appending is slot-safe but reordering is not). Added
      `mode_variant_order_pins_the_discriminants`, which asserts the variant order
      is exactly `[Console, None, Canvas]` — the check that would catch a reorder.
- [x] Measure and, if needed, advertise: `rg -l '"app\.setMode"' src/target/` → 2 files
      (`src/target/linux_common/mod.rs:52`, `src/target/macos_aarch64/mod.rs:37`),
      matching the plan's count. No new call name is introduced, so this passes
      unchanged. **Correction: the plan's command was `rg -rln`, and `-r` is ripgrep's
      *replace* flag** — it prints substituted text, not a file list. Corrected to
      `rg -l` here and in Measured populations. Windows advertises neither `app.` call;
      see Corrections 8.
- [x] Confirm `lower_set_mode` stores `2` and `lower_get_mode` reads it back with no
      arm-specific change (`src/codegen/builtins/app/func_set_mode.rs`,
      `func_get_mode.rs`). VERIFIED by reading `lower_set_mode`: it moves `c_arg(0)`
      into a vreg and `store_u64`s it to `ARENA_STATE_REGISTER + offset` with no
      per-variant branch at all, so any discriminant round-trips.
- [x] Tests: the `src/codegen/builtins/app/mod.rs` unit tests above, plus an app-mode
      integration case (alongside `tests/cli_macos_app_io_input_imports.rs` / the linux
      app-mode tests) asserting `app::setMode(Mode.Canvas)` then
      `app::getMode() = Mode.Canvas` under `MFB_MACAPP_HEADLESS=1` / GTK-headless,
      with the reconcile still default-no-op (mode stored, no surface yet).
      → new `tests/cli_app_canvas_mode.rs`: host-target build, macOS headless
      `Canvas → None → Canvas` round-trip, macOS `Canvas ≠ Console ≠ None`, and a
      cross-compiled `linux-aarch64` build (build-only; the host cannot run a Linux
      GTK aarch64 binary).

Acceptance: a headless app program sets and reads back `Mode.Canvas`. Run only
`cargo test --bin mfb codegen::builtins::app` and the new app-mode integration test.
→ MET: `cargo test --bin mfb codegen::builtins::app` = 6 passed (incl. the new
order test); `cargo test --test cli_app_canvas_mode` = 4 passed.
Commit: 12a706ea6

### Phase 2 — Mode gate: `term::` traps in `Canvas`, `io::` reads allowed in `Canvas`

`term::` hard-fails in `Canvas` (like `None`). But **the console `io::` reads must be
allowed in `Canvas`** (input from the window), a change from the current "reads trap
outside `Console`" gate — so the read gate becomes "trap only in `None`". `io::` outputs
still degrade to stdout/stderr.

- [x] Read `src/codegen/app/hook/app.rs:33:prepend_wrong_mode_gate`: it raises
      `ErrWrongMode` for any non-`Console` slot value. For `term::` (applied at
      `src/codegen/builtins/term/gen_shared.rs:95`) this is already correct for
      `Canvas` (value `2`) with no change. CONFIRMED — `term::` now passes
      `ModeRequirement::Console` explicitly, which emits the same `cmp 0 / b.eq`
      predicate it had before.
- [x] For the console-read gate (`src/codegen/engine/builder/mod.rs:1988`, predicate
      `matches!(spec.call, "io.input" | "io.readLine" | "io.readChar")`), change the
      comparison from "trap unless `Console` (`== 0`)" to "trap only in `None`
      (`== 1`)" so `Console` **and** `Canvas` both permit reads. This is a change to
      `prepend_wrong_mode_gate`'s emitted comparison, so it needs a mode parameter (or
      a second entry point) — `term::` must keep the "`Console` only" form. Keep it a
      no-op when `presentation_mode_offset` is `None`.
      → Done as a `ModeRequirement` parameter (`Console` | `WindowedMode`) rather than
      a second entry point, so the two predicates cannot drift apart. `WindowedMode`
      emits `cmp 1 / b.ne`; the `presentation_mode_offset == None` early return is
      unchanged and is now asserted for **both** requirements.
      Deliberate shape: "not `None`" is one compare, not `Console`-or-`Canvas`; a
      future windowed mode inherits the input source rather than falling off the end
      of a two-arm test and trapping where a window exists.
- [x] `io.readByte` is **not** in the gate predicate and needs no change here (see
      §2's correction); it is covered by Phase 4's window wiring only. CONFIRMED
      unchanged — the predicate still lists exactly three calls.
- [x] Confirm `io::print`/`io::write` (and error variants) still **degrade to stdout/stderr**
      in `Canvas`. Proven at runtime by
      `macos_io_writes_degrade_to_stdout_in_canvas`, which asserts the exact bytes
      `"CANVAS_LINE\nCANVAS_NONL"` on the headless bundle's stdout.
- [x] Tests: `term::sync` in `Mode.Canvas` traps `ErrWrongMode`; `io::print` in `Mode.Canvas`
      reaches stdout; a blocking `io::readLine` in `Mode.Canvas` does **not** trap (fed by
      the test harness / window-input stub — full window-key wiring is Phase 4); the same
      read in `Mode.None` still traps; a `Console`-mode read is unchanged.
      → 4 new headless runtime cases in `tests/cli_app_canvas_mode.rs`
      (`macos_term_traps_wrong_mode_in_canvas` — uses `term::moveTo`, which the
      existing Case 3c also uses, rather than `term::sync`;
      `macos_io_reads_are_permitted_in_canvas_and_still_trap_in_none`;
      `macos_console_reads_are_unchanged_by_the_relaxation`;
      `macos_io_writes_degrade_to_stdout_in_canvas`), plus 3 new unit tests in
      `src/codegen/app/hook/app.rs` asserting the *emitted predicate* (immediate +
      branch condition) per requirement and that the two differ.
- [x] **RED-check** (added task — a relaxation test that passes both before and after
      proves nothing): reverted the builder call site to
      `ModeRequirement::Console`, rebuilt release, and re-ran
      `macos_io_reads_are_permitted_in_canvas_and_still_trap_in_none` → **FAILED with
      exit 50** ("wrongly trapped in Canvas"). Restored; green again.
- [x] Doc sync for this phase's behavior change (moved up from the Validation Plan so
      it lands with the code): the `Mode-gated I/O` section of
      `src/docs/spec/app/05_presentation-mode.md` is now a per-mode × per-call-family
      matrix with the reasoning for each family's requirement; the `Mode` enum section
      lists `Canvas` and states the append-not-reorder rule; the `ErrWrongMode` row in
      `src/docs/spec/diagnostics/02_error-codes.md` and the matching `errorCode`
      registry descriptor prose (`src/codegen/builtins/errorcode/mod.rs:118`) both
      split the `term::` and `io::`-read requirements. Verified by rendering:
      `mfb man app`, `mfb man app types`, `mfb spec app presentation-mode`.

Acceptance: in canvas mode `term::` raises trappable `ErrWrongMode`, `io::` outputs reach
stdout/stderr, and the console `io::` reads are permitted (do not trap); `None` still
traps them — all proven by the tests above. Run only the `src/codegen/app/hook/app.rs`
unit tests, the term wrong-mode gate test, and the new cases.
→ MET. `cargo test --bin mfb codegen::app::hook::app` = 6 passed;
`cargo test --test cli_app_canvas_mode` = 8 passed;
`cargo test -p mfb --bins citations_resolve` = 1 passed (the new
`[[…/hook/app.rs:ModeRequirement]]` citation resolves).
**Correction to this line:** the plan pointed at "the term wrong-mode gate test
(`b2485eb45` added one — find it with `rg -rn "wrong_mode" tests/`)". Two defects —
`-r` is ripgrep's replace flag again, and with the corrected `rg -ln "wrong_mode" tests/`
there is **no such test under `tests/`**. The term wrong-mode gate's only behavioral
coverage is `scripts/test-macapp.sh` Case 3c, which is not in `cargo test`. That gap
is why `macos_term_traps_wrong_mode_in_canvas` asserts the `Console` half too: it is
now the first in-suite coverage of the `term::` gate.
Commit: —

### Phase 3 — Per-platform reconcile `Canvas` arm: build/teardown an empty surface (largest blast radius last)

Three platforms; each builds a bare surface on entry and tears it down on exit. No
renderer, no GPU, no scene.

- [x] macOS: extend `src/target/macos_aarch64/app/mod.rs:emit_reconcile_seam`
      (and its `RECONCILE_BUILD_SYMBOL` helper) with a `Canvas` arm that swaps the
      content view for a layer-backed `NSView` (`wantsLayer = YES`) sized to the
      window, and a teardown arm on exit that restores/removes it. Reuse the existing
      `NSWindow` + content-view swap machinery — the canvas view is a substitute for
      `TermView`, not a second window. Marshal on the main thread (`waitUntilDone:YES`), no-op headless
      marshal as today.
      → `emit_reconcile_canvas_helper` (`RECONCILE_CANVAS_SYMBOL`) + `emit_canvas_teardown`,
      both in `app/bootstrap.rs`; the reconcile IMP became a three-way dispatch
      (`cmp 2 / b.eq` **before** the old `cmp 0`, since with a third variant "not
      `Console`" no longer implies `None`). Sends `setWantsLayer:YES` then `layer` —
      the second forces the backing layer to exist now rather than at first display,
      so the handle is retrievable the moment `setMode` returns. `ASSOC_KEY` is
      cleared on entry, so `io::` writes degrade to the fd sink in canvas mode.
      **Lifetime**: the view is `alloc`'d and deliberately never released, mirroring
      the transcript view — the window's retain plus the un-released `alloc` means
      `setContentView:` swapping it away on exit drops the count to 1, not 0, so the
      `OBJC_ASSOCIATION_ASSIGN` stash stays valid and re-entry reuses the one view.
      Releasing would invert both properties; a test pins that.
- [x] Linux: extend `src/target/linux_gtk/bootstrap.rs:emit_reconcile_seam` (320) /
      `emit_reconcile_idle_helper` (348) with a `Canvas` arm that ensures the GTK window
      exists and its native surface handle is retrievable
      (`gdk_x11_surface_get_xid`/`gdk_wayland_surface_get_wl_surface` — retrieval only,
      no Vulkan yet), balancing `g_application_hold` on exit.
      → Same three-way dispatch. The canvas surface is a `GtkDrawingArea`
      `g_object_ref_sink`ed like the transcript's scrolled window, installed with
      `gtk_window_set_child`; teardown restores the scrolled window, which unparents
      rather than destroys it. New `ST_CANVAS_AREA` / `ST_CANVAS_SURFACE` state slots.
      The window build was **extracted** into `RECONCILE_BUILD_SYMBOL` because a
      canvas-first program has never presented a surface, so `ST_WINDOW` is null when
      it enters canvas mode and both arms must be able to create one — a test asserts
      the build is shared, not inlined into the `Console` arm.
      **Correction to the plan's named API:** it named
      `gdk_x11_surface_get_xid`/`gdk_wayland_surface_get_wl_surface`, but those are
      *backend-specific* symbols — importing either fails to bind on the other
      display server. The portable handle at this layer is `gtk_native_get_surface`
      (a `GdkSurface*`), which is exactly what plan-98-F then feeds to whichever of
      those two applies. Read **after** `gtk_window_present`: an unrealized window has
      no `GdkSurface`, so reading earlier would store null. A test pins that ordering.
- [x] Windows: extend `src/target/win_x86_64/app/mod.rs` reconcile/mode path with a
      `Canvas` arm that ensures the HWND exists for later `VK_KHR_win32_surface` use
      and tears it down on exit (new code — no `term::` reconcile precedent here).
      → Substantially larger than "extend": Windows had **no presentation-mode path
      at all** (Corrections 8). Delivered, in one phase rather than deferred:
      (a) `app.getMode`/`app.setMode` added to `win_x86_64` `RUNTIME_CALLS` —
      RED-checked, see Correction 14; (b) a `win_x86_64` `emit_app_mode_reconcile`
      override + `app::emit_reconcile_seam`, which `SendMessageW`s a new
      `WM_APP_RECONCILE` (`WM_APP + 1`, `wParam` = the mode) to the main window —
      *Send*, not *Post*, because a cross-thread `SendMessageW` blocks until the UI
      thread's pump dispatches it, which is the Win32 equivalent of macOS's
      `waitUntilDone:YES`; (c) the wndproc's three-way reconcile arm; (d) `_main`
      now honours `spec.initial_mode`, hiding the window and clearing the io routing
      global for a `None`-default program so a canvas program does not flash a
      transcript window. On Windows the HWND *is* the native surface handle, so the
      "surface build" is baring the client area (hide the EDIT) and publishing the
      HWND in `CANVAS_HWND_SYM`; teardown clears it. `EDIT_HWND_SAVED_SYM` keeps the
      transcript control reachable while its routing global is zeroed.
- [x] **Teardown test target** (this phase's highest-crash-risk surface): a
      headless test that enters `Canvas`, exits to `None`, re-enters `Canvas`, and
      exits — asserting no crash, no leaked window/view/HWND, clean worker/main
      marshaling. It cannot assert pixels yet; it asserts lifecycle only. Wire it for
      all three headless env vars.
      → **Acceptance strengthened, not weakened — see Correction 15: headless cannot
      reach the reconcile at all**, on any platform, by construction. Delivered
      instead, and it is strictly more than the plan asked for:
      1. `scripts/test-macapp.sh` **Case 3e (GUI)** — the real enter → exit →
         re-enter cycle on a real window, asserting
         `CANVAS_ON|CANVAS_OFF|CANVAS_AGAIN` on stdout with `CANVAS_HIDDEN`
         *absent*. That absence is the proof the reconcile ran at all (Console routes
         io to the transcript); `CANVAS_AGAIN` is the proof re-entry did not message
         a freed view. **Run and green** on this host via
         `MFB_MACAPP_GUI=1 scripts/test-macapp.sh` (all 16 cases pass).
      2. 18 codegen-inspection unit tests — 6 macOS, 6 GTK, 7 Windows minus overlap —
         asserting the emitted arm's structure per platform (dispatch order, the
         layer-backing sends, the no-release lifetime, the shared window build, the
         post-present surface read, the synchronous marshal, the null-window guard
         ordering, and that every referenced global/selector is emitted).
      3. The headless lifecycle case in `tests/cli_app_canvas_mode.rs`, which proves
         the worker-side seam and mode slot survive the cycle.

Acceptance (**strengthened, Correction 15** — the original rested on headless
observing the surface, which it cannot): enter→exit→re-enter→exit of `Mode.Canvas`
completes without crash or leak **on a real window**, and the native surface handle
is retrievable while in canvas mode and released after exit — the first proven by
the GUI Case 3e run, the second by the per-platform publish/clear inspection tests
(macOS `[view layer]`, GTK `ST_CANVAS_SURFACE`, Windows `CANVAS_HWND_SYM`).
→ MET. `cargo test --bin mfb target::` = 153 passed;
`cargo test --test cli_app_canvas_mode --test cli_linux_app_mode --test
cli_macos_app_io_input_imports` = 16 passed;
`MFB_MACAPP_GUI=1 bash scripts/test-macapp.sh ./target/release/mfb` = all 16 ok.
Cross-builds green for `linux-aarch64`, `linux-x86_64`, `windows-x86_64`.
Commit: —

### Phase 4 — Window key events → `io::` input path (canvas keyboard input)

Deliver the canvas window's key events into the worker's `io::` input source, so
`io::readByte`/`readChar`/`readLine`/`input` in `Mode.Canvas` read real keystrokes.
Mirrors term keyboard input (plan-69-term-keyboard-input); reuse its
input-queue/marshaling machinery.

- [ ] macOS: the layer-backed `NSView`'s `keyDown:` marshals bytes into the same input
      channel the worker's `io::` reads drain (reuse the term keyboard-input path).
- [ ] Linux: GTK key-event controller on the canvas window feeds the input pipe the worker
      reads.
- [ ] Windows: `WM_CHAR`/`WM_KEYDOWN` in the wndproc feeds the worker's input channel.
- [ ] Tests: a headless test injects key events and asserts `io::readByte` in `Mode.Canvas`
      returns them in order; EOF/close behaves like console input.

Acceptance: with the canvas window focused, `io::readByte`/`readChar`/`readLine` return the
window's keystrokes on all three platforms (injected-key headless test green); no busy-spin
while waiting. Run only the new injected-key tests plus the existing app-mode `io::` input
tests (`tests/cli_macos_app_io_input_imports.rs` and its linux/windows peers).
Commit: —

## Validation Plan

- Tests: app-mode integration cases for setMode/getMode round-trip (Phase 1); the mode
  gate (Phase 2 — `term::` traps in `Canvas`, `io::` output degrades, `io::` reads
  permitted in `Canvas` but still trap in `None`); the enter/teardown lifecycle (Phase 3);
  and window-key → `io::` input (Phase 4), under `MFB_MACAPP_HEADLESS` / GTK-headless /
  `MFB_WINAPP_HEADLESS`.
- Coverage check: confirm the new reconcile arms are in the `--bin mfb` denominator
  (per `.ai/build-tooling.md` coverage mechanics — src/ coverage is in-process bin
  unit tests; the headless subprocess is integration and uncaptured). Add in-process
  unit tests for the arm-selection logic so the changed codegen is actually measured,
  not just exercised by the uncaptured subprocess. (A `.ncode` *inspection* test —
  asserting the emitted arm is present — is fine and is not a byte-identity golden;
  see `.ai/testing-gates.md` and the "register/slot/import bugs need
  codegen-inspection" lesson.)
- Runtime proof: a small `--app` program `app::setMode(Mode.Canvas)` then a blank
  loop, launched headless on each platform, exits 0 with the surface built and torn
  down (observable via the lifecycle test's assertions / logs).
- Doc sync: update `src/docs/spec/app/05_presentation-mode.md` (add `Canvas` to the
  mode table and the I/O matrix — note `io::` reads work in `Canvas` from the window,
  `term::` traps), the `MODULE_DESC` / `Mode` variant descriptions in
  `src/codegen/builtins/app/mod.rs`, and the `ErrWrongMode` row in
  `src/docs/spec/diagnostics/02_error-codes.md:139` (it names the three gated `io::`
  calls and the `Console` requirement). Per `.ai/specifications.md`, keep the spec
  current with the compiler change. Per AGENTS.md, built-in man content is **rendered
  from the registry descriptors** — there is no Markdown page to hand-edit and no
  template to follow. For `app` that means the `MODULE_DESC` and the `Mode` variant
  `description`s in `src/codegen/builtins/app/mod.rs`; verify with `mfb man app` and
  `mfb man app setMode`. (The `.ai/man*_template.md` files are the retired
  `src/docs/man/**` workflow — do not use them.)
- Acceptance: the per-phase targeted tests above — **no full-suite run and no
  byte-identity check in this letter** (invariant 8); `rustup run 1.96.0 cargo fmt --all
  && (cd repository && rustup run 1.96.0 cargo fmt)` at session end.

## Open Decisions

- **Mode name: `Canvas` vs `Graphics`.** Both spellings appear in early drafts of this
  plan set. `src/codegen/builtins/app/mod.rs:MODULE_DESC` anticipates the extension but names it
  only generically ("A future graphical mode is a new `Mode` variant entered through
  `app::setMode`, with no change to this surface") — it does not pick a name, so
  neither choice contradicts shipped docs. Recommended: **`Canvas`**, for name-parity
  with the `canvas::` package. (§1)
- **Does the `canvas::` package shell register in this sub-plan or in B?** Recommended:
  **B** — this sub-plan needs no drawing call, so registering an empty package here
  would add a `runtime_calls` surface with nothing behind it. Register the package
  when its first call (`present`) exists. (§Non-goals)
- **Windows surface in Phase 3 without a renderer.** An HWND with no WM_PAINT content
  is fine, but confirm the GDI message loop doesn't assume a term memDC exists in
  canvas mode. Recommended: gate the term memDC paint on `mode == Console`. (§Phase 3)

## Corrections

**2026-08-30 — pre-execution revision (no code written yet).** This plan was authored
2026-08-15; two restructurings landed within 48 hours and invalidated its mechanics
while leaving its design intact. Applied:

1. **Citation remap.** 18 mentions of `src/builtins/*` and `src/target/shared/code/*`
   repointed. Commits responsible: `4ed7d60de` (2026-08-16, app onto the clean-room
   registry — deleted `src/builtins/app_package.mfb`) and `f32179ed4` (2026-08-17,
   codegen relocated into a tiered `src/codegen` — deleted `src/target/shared/code/`).
   Verified by extracting every `src/…` path from this file and testing it with
   `[ -e ]`. Consequences beyond the paths: there is **no `.mfb` companion for `app`**
   (the `Mode` enum is `RegistryEnum` data), `BuiltinModule`/`APP_TYPES`/`TypeKind`
   are gone, and `Lowering::Inline` no longer exists — the sanctioned body kinds are
   `Body::abi_inline` and `Body::abi_function` (`app.getMode`/`app.setMode` are both
   `abi_function`).
2. **`io.readByte` correction.** The plan asserted the console-read gate covers
   `readByte`/`readChar`/`readLine`. It never has: the predicate at
   `src/codegen/engine/builder/mod.rs:1988` is `"io.input" | "io.readLine" |
   "io.readChar"`, and `git log -S` over it finds one author commit (`b7964aa43`,
   plan-62-E) that shipped the three-call form. Phase 2's scope shrinks by one call;
   `io::readByte` needs Phase 4's window wiring only.
3. **Byte-identity removed (invariant 8, first bullet).** Per the user's direction and
   AGENTS.md — byte-identity is a drift sentinel for pure code motion, and plan-98 is
   new work. Every "the `tests/byte-identity/` corpus is unchanged" acceptance line and
   the "re-baseline the gate golden" expected-diff machinery are deleted from all eight
   files. **Judgment call, flag if wrong:** C's software-rasteriser *reference images*
   are kept as exact-match (invariant 5) — they are this plan's own new oracle for a new
   rasteriser and the reference E/F compare against with tolerance, not an instance of
   the repo's codegen byte-identity gate. Their description was reworded from
   "byte-identity golden" to "exact-match reference image" throughout so the two ideas
   are not confused.
4. **Test scoping (invariant 8, second bullet).** Every "full `cargo test` green"
   phase-acceptance line is replaced with the targeted tests that phase can actually
   reach. The single full-suite run — `cargo test --no-fail-fast`, plus the acceptance
   harness, which is not in `cargo test` — moved to G's closeout.
5. **Prerequisites re-verified against the tree** on 2026-08-30; the "full suite green
   at HEAD" row was dropped per (4).

6. **All 47 "design summary" / "design doc" citations deleted.** There is no separate
   design document and there never was — `planning/plan-98-A` … `plan-98-G` plus
   `planning/plan-98-api.md` are the entire corpus. Every letter's References section
   used to open with "The design summary — …", and facts throughout were sourced to
   section names in it ("Rendering Notes", "Threading Model", "Platform Surfaces",
   "Diff / damage", …). All of them are now either owned by the plan that states them
   or pointed at plan-98-A's "Cross-cutting invariants" (the real top-level design) and
   plan-98-api.md (the real API surface). Each References section now says outright that
   no other document exists, so the citation cannot regrow.
7. **Four false `VERIFIED` claims downgraded**, all of which were "verified" only
   against the phantom document. Per AGENTS.md a claim is measured or it is a guess:
   - B: "a `List OF DrawItem` is a plain language array" and "deep copy is mandatory"
     → restated as design decisions of this plan, with the phase that actually proves
     each one named.
   - F: "Vulkan needs no SDK" and "GTK4 hands over the native surface handle"
     → **UNVERIFIED**, with the Phase 1 task that must prove each before pipeline code
     lands.
   - G: "stb_truetype is single-header, public-domain, vendorable" → **UNVERIFIED
     here**; Phase 1 must read the actual header's licence before vendoring. And
     "`measureText` from day one makes the shaper swappable" → a design decision,
     conditional on `TextMetrics` staying shaper-independent.
   Several `Measured populations` rows also cited `design "X"` in the Command column —
   those are decisions, not measurements, and now say so.

**2026-08-30 — during execution.**

8. **Windows cannot run an `app::` program at all today, so Phase 3's Windows arm is
   larger than "extend the mode path".** Two measurements:
   - `rg -n "fn emit_app_mode_reconcile" src/` → **2** platform overrides
     (`macos_aarch64/code.rs:249`, `linux_common/code.rs:520`) plus the default trait
     method (`engine/types/types.rs:1157`). `win_x86_64` has **no** override, so its
     reconcile is the default no-op. The Measured-populations row claiming "3 (macos,
     linux_common→gtk, win)" is corrected to 2.
   - `rg -n 'app\.' src/target/win_x86_64/mod.rs` → **no `app.getMode`/`app.setMode`
     in `RUNTIME_CALLS`** (`src/target/win_x86_64/mod.rs:28`), even though
     `supports_app_mode()` returns `true` (mod.rs:271). `validate_capabilities`
     (`src/target/shared/validate/capabilities.rs:19`) errors with "native backend
     does not support runtime call 'app.setMode'" for any call not in that list, so a
     Windows `--app` program that touches `app::` is rejected before codegen.

   Consequence, folded into Phase 3 rather than deferred: the Windows arm must first
   **advertise `app.getMode`/`app.setMode`** and add a `win_x86_64`
   `emit_app_mode_reconcile` override, then hang the `Canvas` build/teardown off it.
   That is a prerequisite Phase 3 owns, not a new letter — it is the same task the
   plan already listed, correctly scoped.

9. **Three Measured-populations commands used `rg -rn`/`rg -rln`.** `-r` is ripgrep's
   *replace* flag: `rg -rn "fn emit_app_mode_reconcile" src/` prints the literal
   replacement text `n(` per match, not a numbered list, which is how the wrong
   reconcile-impl count (3) survived authoring. All rows re-measured with `rg -n` /
   `rg -l`; the table now carries the corrected commands.

10. **`Mode.Canvas` needed no `func_set_mode`/`func_get_mode` change** (Phase 1). Read
    `lower_set_mode`: it moves `c_arg(0)` to a vreg and `store_u64`s it to
    `ARENA_STATE_REGISTER + presentation_mode_offset` with no per-variant branch, so
    the appended discriminant round-trips by construction. Proven at runtime, not
    inferred: `tests/cli_app_canvas_mode.rs` runs a headless bundle doing
    `Canvas → None → Canvas` and asserting `Canvas ≠ Console ≠ None`.

11. **The plan's named "term wrong-mode gate test" does not exist.** Phase 2's
    acceptance line said to run "the term wrong-mode gate test (`b2485eb45` added one
    — find it with `rg -rn "wrong_mode" tests/`)". Corrected:
    `rg -ln "wrong_mode|WrongMode" tests/` returns **nothing**. The `term::` gate's
    only behavioral coverage is `scripts/test-macapp.sh` Case 3c, a shell harness that
    is not part of `cargo test`. Rather than leave the gate uncovered while relaxing
    the code it shares, `macos_term_traps_wrong_mode_in_canvas` asserts **both** halves
    (`Canvas` traps, `Console` does not), so it is also the first in-`cargo test`
    coverage of the `term::` gate.

12. **The `io::`-read relaxation was RED-checked, not assumed.** A test that passes
    both with and without the change would have made Phase 2 look done while shipping
    nothing. Reverting the builder call site to `ModeRequirement::Console` and
    re-running made `macos_io_reads_are_permitted_in_canvas_and_still_trap_in_none`
    fail with exit 50 — the program's own "wrongly trapped in Canvas" code — which is
    the proof that the relaxation is what the test observes.

13. **Phase 2's doc sync was pulled forward from the Validation Plan.** The spec's
    `Mode-gated I/O` section asserted "`term::*` and the console-reading side of `io::`
    require the `Console` surface", which this phase makes false for the `io::` half.
    Leaving that correction to a later step would have left the spec actively wrong in
    every intermediate commit, so it lands with the code that changes the behavior.

14. **Windows advertising was RED-checked.** Commenting the two new `RUNTIME_CALLS`
    entries out and rebuilding made `mfb build -app -target windows-x86_64` fail with
    `error: native backend does not support runtime call 'app.setMode'` on a program
    that now builds. So the advertising is load-bearing, not decorative, and
    `Mode.None`/`Mode.Canvas` really were unreachable on Windows before this phase.

15. **The Phase 3 acceptance as written was unmeetable: headless never reaches the
    reconcile — on any platform.** This is a criterion defect, not a design failure,
    and per the skill it is *strengthened*, not weakened.
    - macOS: `emit_main_bootstrap` installs the app delegate only on the non-headless
      path, and `_mfb_macapp_reconcile_marshal` returns early when `[NSApp delegate]`
      is nil. It must: headless parks the main thread in `pause()` with no run loop,
      so `performSelectorOnMainThread:waitUntilDone:YES` would deadlock.
    - Windows: `_main`'s `headless_spawn` path builds no window and runs no message
      pump, so the new seam's null-`MAIN_HWND_SYM` guard skips for the same reason —
      a `SendMessageW` with no pump to dispatch it blocks the worker forever.
    - The plan's Verified-properties row ("headless construction exercises the surface
      path… VERIFIED from bootstrap.rs:87") is true of the **bootstrap's** AppKit
      construction, which runs before the headless test — it does not extend to the
      reconcile, which is what this phase adds. That row is corrected in §2.

    Replacement acceptance, strictly stronger than "headless did not crash": a **real
    GUI** enter → exit → re-enter → exit cycle (`scripts/test-macapp.sh` Case 3e, run
    green on this host), plus 18 codegen-inspection tests pinning the emitted arm per
    platform, plus the headless lifecycle case for the worker-side seam. Case 3e's
    observable is io routing — `CANVAS_HIDDEN` must be *absent* from stdout (Console
    routed it to the transcript, so the reconcile ran) while `CANVAS_ON` /
    `CANVAS_OFF` / `CANVAS_AGAIN` are present.

16. **The GTK native-surface API the plan named is backend-specific.** Phase 3 said to
    retrieve the handle with `gdk_x11_surface_get_xid` /
    `gdk_wayland_surface_get_wl_surface`. Importing either binds only under that
    display server, so an X11-linked build would fail to start under Wayland and vice
    versa. The portable handle at this layer is `gtk_native_get_surface` →
    `GdkSurface*`, which is precisely the value those two functions *take*; plan-98-F
    picks the right one at surface-creation time. Also pinned by test: it must be read
    **after** `gtk_window_present`, because an unrealized window has no `GdkSurface`
    and an earlier read would store null.

17. **The GTK window build had to be extracted, not copied.** The `Console` arm built
    the window inline. The `Canvas` arm needs the same window — a canvas-first program
    has never presented a surface, so `ST_WINDOW` is null when it enters canvas mode.
    Duplicating the build would have left two constructions to keep in sync, so it
    moved into `RECONCILE_BUILD_SYMBOL` (matching the macOS shape, which already had
    such a helper). A test asserts `gtk_application_window_new` appears **zero** times
    in the idle helper, so a future inline rebuild fails rather than silently
    diverging.

<Further corrections filled in during execution.>

## Summary

The real risk in A is the per-platform teardown ordering (Phase 3): three surface
lifecycles, UI-thread marshaling, and the requirement that switching modes leaves no
leaked native object — the mirror of the implicit `term::off`. Everything before it
(enum append, gate confirmation) is low-risk and independently valuable. Untouched by
A: all scene/geometry/GPU machinery (B onward), the `Image`/`Font` RES resources and
their deferred texture free (B/D), and any drawing whatsoever.
