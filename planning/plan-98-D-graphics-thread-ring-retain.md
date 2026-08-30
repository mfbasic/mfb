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
| plan-98-C complete (software render + goldens) | `ls planning/completed/plan-98-C-*` → hit | NOT MET |
| B's `Image`/`Font` RES backend closes + marks textures pending-free | plan-98-B Phase 4 acceptance met | NOT MET |
| Working tree builds | `cargo build` → pass | UNVERIFIED (run before starting) |

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

- [ ] Author `.ai/canvas-threading.md`: the three-thread model; the scene ring state
      machine (who writes/reads `building`/`pending`/`live`, the CAS/atomic order); the
      resize handshake; the deferred texture free (`lastUsedFrame`/`lastCompletedFrame`
      + closed flag); and the **dirty-texture upload** ordering (`setBytes` writes the CPU
      shadow + dirty flag on the worker; the graphics thread uploads at frame start;
      uploading to a texture a still-in-flight frame is sampling needs a per-texture ring
      or barrier — deferred to E/F, trivial for software). Spell each out as an ordered
      sequence. State plainly there is no refcount.
- [ ] Enumerate the race orderings that must hold (publish vs close vs draw vs
      frame-completion vs setBytes-upload), each with the invariant that protects it. This
      list becomes Phase 4's test matrix.
- [ ] Cross-link from `AGENTS.md` "Read before that kind of work" and MEMORY-worthy
      lessons (durable invariants only, no status).

Acceptance: the doc exists, enumerates every race the design names plus any found while
writing, and states the normative ordering for each — reviewed before any threading code.
Commit: —

### Phase 2 — Graphics thread + software render loop (no ring yet)

- [ ] Spawn the graphics thread from each platform's surface-ready UI callback (after A's
      surface exists); it renders the single live scene (B's slot) via C's software path
      and blits. No time-based spin — it waits on a redraw condition.
- [ ] Wire redraw triggers 1 (present) and 3 (OS damage) to signal the graphics thread.
- [ ] Dirty-texture upload + trigger 5: at frame start, upload any texture marked dirty by
      `canvas::setBytes` (coalesced); `setBytes` signals a redraw only when its id is in the
      current live scene's id set (else no repaint). Software backend: the "upload" is a
      staging copy into the render buffer's source.
- [ ] Tests: a present signals exactly one repaint; a synthetic OS-damage signal repaints
      with no worker involvement; an idle static scene triggers zero repaints (no spin);
      `setBytes` on an in-scene image repaints once (content updated next frame) and on an
      off-scene image repaints zero times; N `setBytes` before a frame coalesce to one
      upload — assert via repaint/upload counters over a fixed wall-clock window.

Acceptance: the graphics thread renders C's golden scene off the worker thread; static
scene = zero repaints; damage repaints without the worker; `setBytes` on an in-scene image
shows updated pixels next frame and coalesces, off-scene image triggers no repaint. Golden
still exact-match.
Commit: —

### Phase 3 — Triple-buffer scene ring + resize handshake

- [ ] Replace B's single live slot with `SceneRing{slots[3], atomic live/pending,
      building}`; `present()` builds into `building`, CAS-publishes `pending`; graphics
      thread swaps `live ← pending` at frame start and recycles the old `live`.
- [ ] Implement the resize handshake: main publishes size + `resizePending`; graphics
      thread reallocates the software buffer at frame start, clears the flag, renders.
      macOS `drawableSize`-from-main path.
- [ ] Tests: two presents before a frame → the intermediate scene is skipped (assert the
      rendered scene is the latest); `present()` never blocks on a stalled graphics thread
      and vice versa (drive with an injected render stall); resize mid-render recreates the
      buffer and repaints at the new size with no worker involvement.

Acceptance: the ring is lock-free (neither side blocks the other under injected stalls);
scene skipping is correct; resize repaints correctly with zero worker activity; golden
still exact-match at each size.
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

<Further corrections filled in during execution — especially any race discovered while
writing the paper doc that the design didn't name.>

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
