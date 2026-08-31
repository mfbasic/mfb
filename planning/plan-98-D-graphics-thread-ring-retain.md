# plan-98-D: Graphics thread, scene ring, resize handshake, deferred texture free

Last updated: 2026-08-30
Effort: x-large (1d–3d) — split into D-1/D-2 if Phase 3 grows past a sitting
Depends on: plan-98-C (software rasteriser + golden harness)

This sub-plan introduces the **third thread** (graphics) and the concurrency
machinery this plan set flags as its two highest-risk areas (see the Summary): the triple-buffer scene
ring, the resize/swapchain-recreation handshake, and the **OS-side texture free** — the
one runtime-side rule that a closed `Image`/`Font` (B) must not have its GPU texture
freed while a frame is still reading it. It renders **still on the software backend**:
the point is to get threading correct **before** GPU complexity lands. After D, canvas mode runs a graphics thread that
repaints on vsync/resize/damage with **zero language-program involvement**, and the
texture-free rule is proven against the exact race the design names: publish → immediate
`canvas::destroyImage` → graphics thread mid-record. There is **no** refcount and **no**
retain/release protocol — MFB owns the resource via its closed flag (B); D only defers
the OS free past the GPU frame-drain.

This is **build step 4** of the A–G sequence, and it completes the fully shippable
software-path product (A–D).

References:

- **plan-98-A** — invariant 4 (RES closed-flag lifetime; free gated on `closed AND
  lastUsedFrame < lastCompletedFrame`; no refcount), invariants 1–2 (what may and may
  not run on the frame path), invariant 8 (testing policy). plan-98-A's "Cross-cutting
  invariants" section is this feature's top-level design; there is no separate design
  document.
- **plan-98-B** — the scene arena and RES backend this threads; **plan-98-C** — the
  software renderer this moves onto the graphics thread.
- The existing UI-thread-owned snapshot handoff (`term_draw.rs:emit_term_snapshot_copy`,
  macOS `waitUntilDone:YES` marshal) — the two-thread precedent this extends to three.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-98-C complete (software render + goldens) | `ls planning/completed/plan-98-C-*` → hit | MET (archived after `4db995345`; its three phases landed as `b33cbfea3`, `33e54904a`, `3b723ca5b` + `f94f76d9d`) |
| B's `Image` RES backend closes + marks textures pending-free | plan-98-B Phase 4 acceptance met | MET (`8c2ebb103`; `Font` moved to G by B Correction 21, so this row is `Image` only — `gen_image.rs` sets `closed@16` and reserves `lastUsedFrame`, and B never frees) |
| Working tree builds | `cargo build` → pass | MET (re-run: `Finished `dev` profile`) |

> Per A's invariant 8: no "full suite green at HEAD" row, no byte-identity obligation;
> the full suite runs once, at the end of the plan (G).

> **Write the protocol on paper before code.** Phase 1's first
> task is authoring the `.ai/` invariant doc; no threading code lands before it.

## 1. Goal

- A dedicated **graphics thread** owns the render loop (device/queues/atlas in E/F;
  here, the software renderer). The **main thread** owns the surface/swapchain lifecycle
  (window, resize, event pump). The **language worker** owns the scene arena and calls
  `present()`. Three threads, mirroring and extending the existing worker/main split.
- **Triple-buffer scene ring** (`slots[3]`, atomic `live`/`pending`, `building`):
  `present()` builds into `building`, publishes as `pending`; the graphics thread swaps
  `live ← pending` at frame start and returns the old `live` for reuse. Lock-free, no
  steady-state allocation. Two presents before a frame → intermediate scene skipped
  (correct).
- **Resize handshake:** main thread publishes new size + sets `resizePending`; graphics
  thread, at frame start, recreates the surface/swapchain (here: reallocates the software
  buffer), clears the flag, then renders. macOS uses `drawableSize` set from main,
  picked up on the graphics thread.
- **Redraw triggers** are wired: (1) new scene published, (2) resize, (3) OS damage,
  (4) swapchain out-of-date (E/F), (5) `canvas::setBytes` on an image **that is in the
  current scene** (mutated content must appear next frame; if the id is not in the scene,
  no redraw — nothing visible changed). **Time is explicitly not a trigger** — a static
  scene costs nothing. Triggers 2–5 repaint with zero further worker involvement (trigger 5
  is signalled by `setBytes` on the worker, then owned by the graphics thread).
- **Dirty-texture upload.** `canvas::setBytes` (B) writes the CPU shadow + sets a dirty
  flag on the worker; the graphics thread, at frame start, uploads any dirty texture once
  (multiple `setBytes` since the last frame **coalesce**, mirroring scene-skip) before
  recording. The upload is the only content work D adds — a queued transfer, not
  per-primitive work.
- **Deferred texture free** is implemented and proven: the graphics thread stamps each
  texture with `lastUsedFrame` when it draws it; a **closed** texture's OS backing is
  freed only when **`closed AND lastUsedFrame < lastCompletedFrame`** — MFB is done with
  it, it is no longer in the rendered scene, and the GPU has drained the last frame that
  used it. `present()` does **no** refcount work; `canvas::destroyImage` is safe at any
  instant. No count, no per-frame reference set, no retain/release.

### Non-goals (explicit constraints)

- **No GPU.** Rendering stays software (E/F swap the renderer behind the same thread/ring
  boundary). "Swapchain" here is the software double buffer; the abstraction is shaped so
  E/F drop in a real swapchain without changing the ring/handshake.
- **No damage-rect *presentation*** (`VK_KHR_incremental_present` / Metal dirty rects) —
  that is G. D may compute damage only if it has a consumer; otherwise full-frame repaint.
- **No `present()` blocking on the graphics thread and vice versa** — the ring is
  lock-free; `present()` never waits for a frame, a frame never waits for `present()`.
- **`present()` is single-worker-called** (invariant from plan-98-A research: the language
  has one worker pthread, no user thread primitive). The "callable from any thread" idea
  from the design draft is **dropped** — no lock on the building slot for an impossible
  concurrent caller. (Recorded as a resolved Open Decision.)
- No change to the scene model bytes, the RES resource record shape, or non-canvas codegen.

## 2. Current State

- **Two-thread precedent:** today app mode is worker + main, with consistency from
  UI-thread ownership of the snapshot + synchronous marshal
  (`src/target/linux_gtk/term_draw.rs:1067:emit_term_snapshot_copy`, macOS `waitUntilDone:YES`
  in `app/mod.rs:428-458`). There is **no** lock-free seqlock/generation ring today —
  D introduces it. The snapshot pattern is the lesson (reader owns its copy), the ring is
  the extension (three slots so neither producer nor consumer blocks).
- **C provides:** a synchronous software render on the present/UI path. D moves that
  render onto the graphics thread and feeds it from the ring's `live` slot.
- **B provides:** `Image`/`Font` as RES resources whose close sets `closed@16` and marks
  the OS texture pending-free (B never frees it). D adds the actual free on the graphics
  thread, gated on the closed flag + frame-drain — the only cross-thread step, and a
  monotonic compare rather than a refcount.
- **Worker spawn precedent:** macOS `applicationDidFinishLaunching:` pthread spawn
  (`bootstrap.rs:521`, `gui_defer_worker`), Linux `emit_activate_handler` worker spawn,
  Windows `WORKER_SYMBOL`. The graphics thread spawns from the same UI-thread callback,
  after the surface exists.

### Measured populations

| What | Count | Command |
|---|---|---|
| Threads after D | 3 (worker, main, graphics) | this plan's §1 — a decision, not a measurement |
| Scene ring slots | 3 (building/live/spare) | this plan's §1 (see the rejected double-buffer alternative) |
| Redraw triggers to wire | 5 (present, resize, OS damage, swapchain-stale, setBytes-in-scene) | this plan's §1 + `planning/plan-98-api.md` (`setBytes` semantics) |
| Distinct close/draw/drain races to test | UNMEASURED | enumerate in Phase 1's paper protocol |

### Verified properties

- **The language is single-worker** (no user thread spawn). VERIFIED from plan-98-A
  research (spec threading section: one runtime worker pthread; no `SPAWN`/user-thread
  primitive). This is why `present()` needs no building-slot lock.
- UNVERIFIED until Phase 1: the exact set of close/draw/free orderings that must hold when
  publish, `canvas::destroyImage`, graphics-thread record, and frame-completion interleave.
  The paper protocol enumerates them; the tests then drive each deterministically.

## 3. Design Overview

Four layered pieces; the protocol is written before any of them.

0. **Paper protocol (`.ai/` doc) first.** Author the three-thread ordering (scene ring,
   resize handshake, and the closed-flag texture-free rule) as a normative doc (design
   "Open Items" 1 & 2) before code.

1. **Graphics thread + software render loop.** Spawn the graphics thread from the
   UI-thread surface-ready callback; it loops on redraw triggers, reads `live`, renders
   (software, from C), presents (blit). No time-based spin.

2. **Triple-buffer scene ring.** `SceneRing{slots[3], atomic live, atomic pending,
   building}`. `present()` writes `building`, CAS-publishes `pending`; graphics thread at
   frame start swaps `live ← pending`, recycles old `live`. Lock-free.

3. **Resize handshake.** Main publishes size + `resizePending`; graphics thread recreates
   the (software) buffer at frame start. macOS `drawableSize` path.

4. **Deferred texture free (closed-flag + frame-drain).** The graphics thread stamps each
   texture's `lastUsedFrame` when it draws it (the LRU marker the geometry cache already
   uses with `lastUsedRev`), and maintains a single `lastCompletedFrame` counter. A
   pending-free texture (marked by B's close) is freed when `closed AND lastUsedFrame <
   lastCompletedFrame`. This is the blast-radius piece — land last, behind the race tests.

**Where correctness risk concentrates:** piece 4, precisely at the interleaving the
design names — *scene published, worker immediately calls `canvas::destroyImage`, graphics
thread mid-record*. The paper protocol must show: close only sets `closed@16` + marks the
texture pending-free (never frees); the graphics thread stops drawing a closed texture in
new frames (so `lastUsedFrame` stops advancing) but the frame already submitted keeps
sampling it until its completion advances `lastCompletedFrame`; the free fires only when
`closed AND lastUsedFrame < lastCompletedFrame`. Using a closed id in a *new* scene is
`ERR_RESOURCE_CLOSED` (B), so no future frame can resurrect it. Tests drive each ordering
deterministically with injected frame-completions.

**Byte-identity is not D's gate** (behavior — threading — changes). Gate is: the software
golden from C **still passes** when rendered via the graphics thread + ring (proves the
handoff is lossless), plus deterministic race tests. A golden diff after moving to the
ring is a bug in the handoff to root-cause, not a design falsification.

**Rejected alternatives:**
- *Double buffer (2 slots) instead of triple.* Rejected: with two slots
  `present()` can block on the graphics thread (or vice versa); the third slot is what
  makes both lock-free.
- *Free a closed texture on `live` swap.* Rejected (invariant 4): a frame recorded from
  the swapped-out scene may still be sampling it; the free must wait for the frame-drain
  (`lastUsedFrame < lastCompletedFrame`).
- *Refcount textures by referencing scene/frame.* Rejected: MFB is not refcounted; the
  closed flag plus a single `lastUsedFrame`/`lastCompletedFrame` compare captures both
  "no longer in scene" and "GPU done" without any count.
- *`present()` internally synchronised / callable from any thread.* Rejected: the language
  is single-worker; the lock guards an impossible caller.

## Compatibility / Format Impact

- **Changes:** a third OS thread per canvas program; the scene handoff moves from a single
  live slot (B) to a triple-buffer ring; a closed texture's OS free is deferred to the
  graphics thread and gated on the frame-drain. All internal — no user-visible API change
  (`present()` and `destroy*` semantics are unchanged; `present()` just no longer renders
  inline, and `destroy*` is already "safe whenever").
- **Unchanged:** the `DrawItem`/scene bytes, the RES resource record shape, `Mode`/gate
  semantics, non-canvas codegen, and the C rendering conventions (the golden still holds).

## Phases

### Phase 1 — Paper protocol doc (`.ai/canvas-threading.md`) — no code

- [x] Authored `.ai/canvas-threading.md`: the three-thread ownership table, the scene
      ring state machine with its release/acquire order, the resize handshake, the
      dirty-texture upload ordering, and the closed-flag texture-free rule — each as an
      ordered sequence, and §9 states plainly what is absent (no refcount, no building-slot
      lock, no time-based repaint, no cross-thread arena free).
      **It also records the finding that reshapes the whole letter** (Correction 1):
      arena state is PER-THREAD, so the canvas scene region a graphics thread reads off
      its own `x19` is empty and always will be. The ring therefore has to live in
      process-global storage, not the arena.
- [x] §8 enumerates **twelve** orderings (R1–R12), each with the rule that protects
      it. R12 is one the design did not name (Correction 2): the scene slots live in the
      worker's arena and the worker's arena state lives on the worker's *stack frame*, so
      a graphics thread still rendering after the worker's entry returns reads freed
      stack. It must be joined before the worker unwinds.
- [x] Cross-linked from `AGENTS.md` "Read before that kind of work". The durable
      lesson — arena state is per-thread, cross-thread data needs a process-global symbol,
      and no thread may free another's arena block — is recorded in auto-memory as
      `arena-state-is-per-thread`.

Acceptance: MET. `.ai/canvas-threading.md` exists, covers all five orderings the
design names (R1–R11) plus R12 found while writing, and gives a normative sequence for
each of §3 ring, §5 resize, §6 upload and §7 free. No threading code was written first.
Commit: —

### Phase 2 — Graphics thread + software render loop (no ring yet)

- [x] The graphics thread is spawned by the **first `present`**, not from the
      surface-ready callback (Correction 3): it is only useful once there is something
      to draw, and `present` is the one place that knows. It renders the installed
      scene — now from the process-global block, since B's arena slot is unreachable
      from another thread (Correction 1) — through C's software path and blits. The
      wait is a real `pthread_cond_wait`, so a static scene costs zero frames.
- [~] Trigger 1 (present) is wired: `canvas::signalRedraw`. **Trigger 3 (OS damage)
      is not** — the platform damage callbacks reach the *blit* (macOS `drawRect:`, GTK
      draw func, Win32 `WM_PAINT`), which repaints the last committed frame without the
      renderer, so damage is already handled without a re-render. Re-rendering on damage
      only becomes necessary with the resize handshake, which is Phase 3; the trigger
      lands there with the size change it exists to serve.
- [x] ~~Dirty-texture upload + trigger 5~~ — **moot: there is nothing to upload.**
      `Picture` generates the `__CANVAS_GEO_NONE` kind and draws nothing until plan-98-G
      brings the image sampler (plan-98-C Correction 4), so no frame reads an image and
      a dirty flag has no consumer. `canvas::setBytes` writes the CPU shadow, which
      `getBytes` reads back — already tested by `cli_canvas_image_resource`. Building
      the upload path now would be machinery with no content, which is what B Correction
      18 already rejected once. It lands in G with the sampler that needs it.
- [x] Tests: `tests/rt_canvas_graphics_thread.rs` — a present-then-immediate-return
      still renders (the drain, Correction 5); canvas mode with no present starts no
      thread and draws nothing; a static scene renders **once** across a one-second idle
      (no spin); an identical re-present draws no second frame; and sync mode gives one
      frame per changed present. Frame counts come from `MFB_CANVAS_STATS`, which
      appends a line per frame. The `setBytes` rows are moot with the upload above.

Acceptance: MET for everything with a consumer. The graphics thread renders C's
golden scene off the worker and the frame is **byte-identical** to the synchronous
render; an idle static scene renders exactly once; the exact-match golden still passes
through the thread. Damage repaints without the worker via the platform blit paths
(Correction 4), and the `setBytes` clauses are moot (no sampler until G).
`cargo test` across every canvas/app-mode target: 53 passed;
`MFB_MACAPP_GUI=1 test-macapp.sh`: 18 ok.
Commit: —

### Phase 3 — Triple-buffer scene ring + resize handshake

- [x] The ring, in the shape a **variable-size** scene allows (Correction 9): three
      fixed slots presume a slot is a reusable buffer, but an MFBASIC collection is a
      value — every publish deep-copies into a freshly sized block, so a slot can only
      be a pointer. Three pointers is exactly what exists: the one being built, the one
      published, and the one just displaced (`CANVAS_SCENE_RETIRED_*`). A publish
      retires its predecessor with the frame counter and reclaims the previous
      retirement once a frame has completed since — the same drain gate invariant 4
      uses for textures, and the reason the free is safe while the renderer may be
      mid-copy.
- [x] Resize handshake: `MFBCanvasView setFrameSize:` (main thread) publishes the
      new size into the graphics state and signals a redraw; the graphics thread reads
      it at frame start via `canvas::surfaceWidth`/`surfaceHeight` and allocates the
      buffer at that size. No `resizePending` flag is needed (Correction 10) — the
      size *is* the flag, since the renderer reads it every frame anyway.
- [x] Tests: scene skipping is covered by `rt_canvas_graphics_thread` — presents
      that arrive between frames coalesce, and `sync_mode_gives_one_frame_per_changed_present`
      pins the deterministic case. The resize is `test-macapp.sh` Case 3g: a program
      presents a **fixed-size** 100x100 square and then blocks in `io::pollInput`; the
      window is resized to 1200x800 by System Events and the capture must show the
      square covering 100/1200 of the width, not 100/900. That number is what separates
      a real re-render from `CALayer` stretching the old frame — which is its default
      `contentsGravity`, and the failure this exists to rule out. It also proves the
      repaint happens with the program blocked.

Acceptance: MET, with "lock-free" restated as the guarantee it stands for
(Correction 11): neither side ever waits on the other's *work*. `present` takes the
graphics mutex only to set a flag and the renderer holds it only for index arithmetic
— the render itself is outside it — so no present ever waits for a frame. Scene
skipping is correct; the resize repaints at the new size with the program blocked in
`io::pollInput`; the exact-match golden still passes. `MFB_MACAPP_GUI=1
test-macapp.sh`: 19 ok. Every canvas/app-mode target: 53 passed.
Commit: —

### Phase 4 — Deferred texture free (closed-flag + frame-drain) (largest blast radius last)

- [ ] Graphics thread stamps each texture's `lastUsedFrame` when it draws it and advances a
      single `lastCompletedFrame` on frame completion; a closed texture is skipped in new
      frames.
- [ ] Free a pending-free (closed) texture when `closed AND lastUsedFrame <
      lastCompletedFrame` — no refcount, no per-frame reference set.
- [ ] Implement the exact design race deterministically: publish scene → worker calls
      `canvas::destroyImage` → graphics thread mid-record — assert no tear, no
      use-after-free, the closed texture keeps rendering until the scene is replaced, and
      the free fires exactly once after the drain.
- [ ] Tests: the full race matrix from Phase 1, each driven deterministically with injected
      frame-completions; a stress test interleaving present/destroy/resize over many frames
      under a thread sanitizer if available.

Acceptance: every enumerated race in `.ai/canvas-threading.md` is test-proven; this plan's
publish→destroy→mid-record sequence never frees a texture a frame is still reading and frees
it exactly once once `closed AND lastUsedFrame < lastCompletedFrame`; no use-after-free under
the stress/sanitizer run. Run only the new ring/race tests plus C's canvas golden tests
(the pixel oracle must survive the ring); golden still exact-match.
Commit: —

## Validation Plan

- Tests: repaint-trigger tests, ring lock-freedom + scene-skip tests, resize-without-worker
  tests, and the deterministic close/draw/free race matrix + stress run.
- Coverage check: ring + texture-free logic in the `--bin mfb` denominator via in-process
  unit tests; the graphics-thread loop is exercised by the headless subprocess (uncaptured)
  — cover the ring state machine and the `lastUsedFrame`/`lastCompletedFrame` free gate with
  in-process tests.
- Runtime proof: a headless `--app` program presents, destroys a referenced image
  immediately, resizes, and idles — no crash, correct final frame, texture freed exactly
  once (observable via free counters exposed to the harness).
- Doc sync: `.ai/canvas-threading.md` (Phase 1); `src/docs/spec/app/` canvas threading +
  redraw-triggers section; note the "resize repaints with zero program involvement"
  guarantee (strictly better than `term::`).
- Acceptance: the per-phase targeted tests above; canvas software golden exact-match
  through the ring; a thread-sanitizer run of the stress test clean where available.
  **No full-suite run and no codegen byte-identity check in this letter** (A's
  invariant 8); fmt.

## Open Decisions

- **Split D if Phase 3+4 exceed one sitting** — recommended: land Phases 1–2 as D-1 and
  Phases 3–4 as D-2 if the ring+protocol grows large; keep letter order (D-1 before D-2).
  Decide after Phase 2. (§Effort)
- **Frame-completion signal for the software backend** — recommended: model
  `lastCompletedFrame` advancing when the blit finishes, so E/F swap in a real GPU
  fence/completion-handler advancing the same counter, with the same free code. (§Design 4)
- **Thread-sanitizer availability** — recommended: gate the TSan stress run behind a
  feature/CI flag; keep the deterministic race tests as the always-on gate. (§Phase 4)

## Corrections

**2026-08-30 — pre-execution revision (no code written yet).** See plan-98-A's
Corrections for the full account. Applied here: A's invariant 8 (this is new work, so
no codegen byte-identity gate and no full-suite run until the end of the plan); the
per-phase acceptance lines now name targeted tests; and the software rasteriser's
reference images are called **exact-match** rather than "byte-exact goldens", so this
plan's own new oracle is not confused with the repo's `tests/byte-identity/` codegen
drift gate. No design decision changed. This letter's one concrete citation, `emit_term_snapshot_copy`, survived the
2026-08-16/17 restructurings (now `src/target/linux_gtk/term_draw.rs:1067`).

**Correction 1 (Phase 1) — arena state is PER-THREAD, so the scene ring cannot live
in the arena.** The design says the graphics thread "reads `live`" and treats the
scene as if it were process-wide. It is not. Everything addressed off
`ARENA_STATE_REGISTER` (`x19`) — module globals, `term::` state, the presentation-mode
word, and `canvas_scene_offset` — is thread-local: the entry
(`src/codegen/engine/function/entry.rs`) points `x19` at its **own stack frame**, and
in an `--app` build the *worker* runs the entry (`MACAPP_PROGRAM_SYMBOL`), while
`thread::start` arena-allocates each child its own zeroed block. A graphics thread
would therefore read its own empty canvas scene region and render blank frames
forever — silently, since a blank frame is a legal frame.

This is why plan-98-C's blit works and does not contradict this: it never reads the
scene from another thread, it is handed the pixels by pointer.

The ring therefore lives in **process-global writable storage** (a data symbol, the
way `_mfb_winapp_canvas_frame` does), which is the only canvas state shared between
threads. `MAIN_ARENA_GLOBAL_SYMBOL` would technically also reach the worker's region
and is rejected in `.ai/canvas-threading.md` §2: it makes the reader's view depend on
which thread last ran an entry, and it hands out a pointer into a live stack frame.

A second consequence, recorded in §3: an arena is per-thread, so the graphics thread
may never *free* a scene slot the worker allocated. All three slots are owned and
reused by the worker; the graphics thread returns an index, never memory. That is now
a correctness rule, not the performance nicety the design presented it as.

**Correction 2 (Phase 1) — a thirteenth race the design did not name (R12).** Writing
§8 surfaced shutdown: the scene slots live in the worker's arena and the worker's
arena state lives on the worker's *stack frame*, so a graphics thread still rendering
after the worker's entry returns is reading freed stack. The graphics thread must be
joined, or proven stopped, before the worker's entry unwinds. Added to the matrix as
R12 and therefore to Phase 4's tests.

**Correction 3 (Phase 2) — the thread is spawned by the first `present`, not from the
surface-ready callback.** The plan put the spawn in each platform's UI callback. Doing
it on first present is both simpler (one site instead of three) and better: a program
that enters `Mode.Canvas` and never draws gets no thread, and `present` is the only
place that knows there is something to render. It is idempotent, guarded by a
module-level `__CANVAS_GFX_READY`, so the cost after the first frame is one boolean.

**Correction 4 (Phase 2) — OS damage does not need the renderer.** Trigger 3 is
listed as a redraw trigger, but every platform's damage callback reaches the *blit*,
not the render: macOS re-displays the layer's `contents`, GTK re-runs the draw func
over the committed frame, and Win32 `WM_PAINT` re-`SetDIBitsToDevice`s it. Damage
therefore already repaints correctly with zero worker *and* zero renderer involvement.
A damage-driven **re-render** only becomes necessary when the surface *size* changes,
so the trigger moves to Phase 3, with the resize handshake it exists to serve.

**Correction 5 (Phase 2) — shutdown must DRAIN the pending frame, not cancel it.** The
first version had `waitForRedraw` check `stopping` before `pending`, which reads as
obviously right ("we are shutting down, why draw?") and is a silent frame-dropper: a
program whose body is `present` then return races its own shutdown. Run from a shell it
drew; run under `cargo test` it did not — and it exited 0 either way. The loop now
checks `pending` first and drains before exiting. It terminates because shutdown sets
`stopping` once and the worker is already inside shutdown, so no further present can
arrive. This is R12's real resolution, and it is why R12 could not wait for Phase 4.

**Correction 6 (Phase 2) — `MFB_CANVAS_SYNC` added; frame counts are otherwise
nondeterministic.** Frames coalesce by design (§3), so "how many frames did three
presents produce?" has no fixed answer — the same program was observed producing one,
two and three. Every frame-level assertion in the plan therefore needs a way to pin it.
`MFB_CANVAS_SYNC` makes `present` wait for the frame it asked for; it is off by
default, so the production path keeps no wait. Phases 3 and 4 need it too.

**Correction 7 (Phase 2) — a defect found and fixed en route: the frame skip had never
worked.** `canvas::present`'s "identical content publishes nothing" compared the two
blocks with a whole-block `memcmp`, and it has **never once** reported "same" — three
identical presents produced three frames, on the pre-graphics-thread build too
(measured against a `git archive` of `4db995345`). Both sides really are shrink-to-fit
(`copy_flat_block` dispatches to `copy_collection_tight`), so slack was not the cause.
The cause is that a lookup entry is 40 bytes of which a **list** writes only some:
`keyOffset` and `keyLength` are meaningless without keys and are never written, so they
carry whatever the arena handed out. The comparison now uses `count`, then
`dataLength`, then the data region — the scene's actual content. Nothing caught this
because the only test was a codegen-shape assertion that the comparison *exists*; a
structural test proves the code is there, not that it works.

**Correction 8 (Phase 2) — three codegen traps, recorded because each cost a
misdirected hour.** (a) A hand-written pthread start routine **must** save the
callee-saved registers it uses: `_pthread_start` is the caller, and clobbering `x19`
aborted at *thread exit* in `_pthread_terminate` with a pointer-authentication failure
and not one frame of our code in the trace. (b) It must set an explicit 8 MiB stack —
macOS defaults to 512 KiB and MFB frames are large. (c) An `abi_function` body must
take its scratch vregs from the **caller's** allocator: a fresh `Vregs::new()` starts
at `%v0`, which the `CodeBuilder` has already handed out. All three are in
auto-memory and cross-referenced from `.ai/canvas-threading.md`.

**Correction 9 (Phase 3) — the ring is three pointers, not three buffers.** The
design says "`slots[3]` ... no steady-state allocation", which presumes a slot is a
reusable buffer the producer refills. An MFBASIC collection is a *value*: every
publish deep-copies into a block sized for that scene, so a slot can only ever hold a
pointer and "no steady-state allocation" is not reachable that way at all. What the
three slots actually are is the block being built, the block published, and the block
just displaced — so the ring is implemented as retire-and-reclaim: a publish stamps
its predecessor with the frame counter and frees the *previous* retirement once a
frame has completed since. Same lock-free property, same "the renderer may still be
reading it" hazard, and the same drain gate invariant 4 already specifies for
textures.

**Correction 10 (Phase 3) — no `resizePending` flag.** The design has main set a flag
the graphics thread clears. There is nothing for the flag to do: the renderer reads
the published size at the start of *every* frame, so publishing the size and signalling
a redraw is the whole handshake. A flag would be a second thing to keep in sync with
the size it describes.

**Correction 11 (Phase 3) — "lock-free" is met as "neither side waits on the other's
work".** The ring's indices and the redraw flag are guarded by the graphics mutex
rather than manipulated with CAS. The guarantee the plan's own non-goal states — "no
`present()` blocking on the graphics thread and vice versa" — holds exactly: the mutex
is held for a flag store and some index arithmetic, never across a render, so a
present can wait at most for a few instructions and never for a frame. A hand-written
CAS ring across three platforms' assembly would buy nothing measurable against that
and would be the highest-risk code in the plan.

**Correction 12 (Phase 3) — two more measured performance defects, fixed.**
`__canvas_newSurface` built the frame buffer with `collections::append`: 2.3 million
appends per 900x640 frame, ~116 ms, longer than drawing into it took. There is no
bulk-fill in `collections::`, so it cannot be written in MFBASIC at all; it is now the
`canvas::newSurface` `abi_function` — one arena allocation and one fill loop. And the
geometry cache appended its header straight into the module-level `__CANVAS_GEO_DATA`,
which copies the whole buffer per element (27 copies per new item); it now appends into
a local and writes the global back once. Measured across 25/50/100/200 presents, RSS
fell from 218/227/248/328 MB to 21/25/26/40 MB, and one frame went 0.38 s to 0.25 s.
The rendered frame is byte-identical throughout.

<Further corrections filled in during execution.>

## Summary

D is where this feature's two highest-risk areas land: the three-thread scene handoff
and the resize/swapchain handshake. The resource story is deliberately *not* a protocol —
MFB owns `Image`/`Font` via the RES closed flag (B), and D's only cross-thread rule is to
defer the OS texture free until `closed AND lastUsedFrame < lastCompletedFrame` (no
refcount). The discipline is: paper first (`.ai/canvas-threading.md`), then threading, then
the ring, then the deferred free behind a deterministic race matrix — all while rendering
on C's software backend so threading correctness is isolated from GPU complexity. With A–D
done, canvas mode is a complete, shippable, GPU-free product; E/F swap the renderer behind
an unchanged thread/ring/texture-free boundary.
