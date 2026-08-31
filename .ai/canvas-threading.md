# Canvas threading: the three-thread model

Normative ordering rules for `Mode.Canvas` rendering. Written before the threading
code (plan-98-D Phase 1) and binding on plan-98-E/F, which swap the *renderer* behind
this boundary without changing any rule here.

Read this before touching the graphics thread, the scene ring, the resize handshake,
or the texture free.

## 1. The three threads, and what each owns

| Thread | Owns | Never touches |
|---|---|---|
| **main** (UI) | the window, the surface/layer, the event pump, resize notifications | the scene, the geometry cache, the pixel buffer |
| **worker** (language) | the program, the scene arena, `canvas::present`, `canvas::setBytes`, `canvas::destroyImage` | the surface, the pixel buffer, any texture's OS backing |
| **graphics** | the render loop, the geometry cache, the pixel buffer, texture uploads and frees | the window, the scene *slots it does not hold* |

Main and worker already exist (`WORKER_SYMBOL` spawned from the UI-thread
surface-ready callback on all three platforms). Graphics is the third, spawned the
same way and from the same place, after the surface exists.

## 2. Arena state is PER-THREAD — the fact everything below is shaped by

Each thread pins `ARENA_STATE_REGISTER` (`x19`) to **its own** block:

* The entry (`src/codegen/engine/function/entry.rs`) reserves `ENTRY_STACK_SIZE` on
  its own frame and points `x19` at it. In `--app` builds the *worker* runs the entry
  (`MACAPP_PROGRAM_SYMBOL`), so the worker's arena state is on the worker's stack.
* A thread spawned by `thread::start` arena-allocates a child block of
  `ENTRY_GLOBALS_OFFSET + arena_global_slots * 8` and zeroes it
  (`lower_thread_start_helper`).

Everything addressed off `x19` is therefore **thread-local**: module-level globals,
the `term::` state, the presentation-mode word, **and the canvas scene region**
(`canvas_scene_offset`, `builder/mod.rs`).

**Consequence, and the central design constraint of plan-98-D:** a graphics thread
with its own arena state would read *its own* canvas scene region — which is empty and
always will be. `canvas::installedItems()` on the graphics thread cannot see what
`canvas::present` published on the worker. Any design that "just moves the render to
another thread" is wrong for this reason and would render blank frames forever.

There is one process-global escape hatch, `MAIN_ARENA_GLOBAL_SYMBOL`, which each
entry stores its own `x19` into — but in an `--app` build the worker's entry is the
last writer, so it names the worker. Reaching the scene through it would work and is
**still rejected**: it makes the graphics thread's view of the scene depend on which
thread last ran an entry, and it gives the graphics thread a pointer into the
*worker's stack frame*. The ring below is explicit instead.

## 3. The scene ring

The ring is **process-global storage** (a writable data symbol, like
`_mfb_winapp_canvas_frame`), not arena state. That is what makes it visible to a
thread that has its own `x19`, and it is the only canvas state shared between
threads.

```
SceneRing {
    slots[3]     // scene buffers, ALL owned and allocated by the worker
    building     // index the worker is filling; worker-private
    pending      // atomic: index the worker has published, or NONE
    live         // graphics-private: index the graphics thread is rendering
}
```

### Ordering

**Worker, in `canvas::present`:**

1. Fill `slots[building]` with the deep-copied scene (unchanged from plan-98-B).
2. Release-store `pending ← building`.
3. `building ← ` the index that was in `pending` before step 2, or the free third
   index if `pending` was NONE.
4. Signal the redraw condition.

**Graphics, at frame start:**

1. Acquire-swap `pending → NONE`, taking the index if there was one.
2. If an index was taken: the old `live` becomes free for the worker, and
   `live ← taken`.
3. Render `slots[live]`.

`present()` never waits for a frame and a frame never waits for `present()`. Three
slots is the minimum that gives that: with two, the worker's step 3 has no free index
whenever the graphics thread is mid-render, and it would have to block.

### Two presents before one frame

The intermediate scene is **skipped**, and that is correct — it was never on screen
and nothing observed it. Step 2's overwrite of `pending` is the skip.

### Who frees a slot

**Nobody, in steady state.** All three buffers are allocated by the worker from the
worker's arena and *reused*. This is not an optimisation: an arena is per-thread, so
a cross-thread free would corrupt the worker's free list. The graphics thread returns
an index; it never returns memory.

## 4. Redraw triggers

Exactly five, and **time is not among them** — a static scene costs zero frames.

| # | Trigger | Signalled by | Notes |
|---|---|---|---|
| 1 | a scene was published | worker, in `present` | |
| 2 | resize | main | see §5 |
| 3 | OS damage/expose | main | no worker involvement |
| 4 | swapchain out of date | graphics | plan-98-E/F only |
| 5 | `setBytes` on an image **in the live scene** | worker | §6 |

Trigger 5 is conditional on purpose: mutating an image no scene draws changes nothing
visible, and repainting for it would turn an off-screen buffer update into a frame.

## 5. Resize handshake

1. **Main** publishes the new size and sets `resizePending` (release).
2. **Graphics**, at frame start, reads and clears `resizePending` (acquire),
   reallocates the pixel buffer to the new size, then renders.

Main never touches the pixel buffer, and graphics never touches the window. The
worker is not involved at all — which is the guarantee `term::` does not give, and is
worth stating in the spec: **a canvas resizes and repaints correctly while the program
is blocked in `io::input`.**

A frame already in flight when the resize lands finishes at the old size and is
presented; the next frame is at the new size. Tearing the frame to the new size
mid-render would mean drawing part of the picture with each.

## 6. Dirty-texture upload

1. **Worker**, in `canvas::setBytes`: write the CPU shadow, then set the texture's
   `dirty` flag (release). Signal a redraw only if the id is in the live scene.
2. **Graphics**, at frame start: for each dirty texture, upload once and clear the
   flag (acquire).

Multiple `setBytes` between two frames **coalesce** to one upload, mirroring the
scene skip and for the same reason: only the last value was ever going to be seen.

Uploading into a texture a still-in-flight frame is sampling needs a per-texture ring
or a barrier. That is **plan-98-E/F's problem, not this document's**: the software
backend has no in-flight frame — the blit completes before the next frame starts —
so the upload is unconditionally safe here. E/F must revisit this section.

## 7. Deferred texture free — the closed flag, not a refcount

**There is no refcount.** MFB owns an `Image` through the RES model: scope-drop or
`canvas::destroyImage` sets `closed@16` (plan-98-B), and that is the whole ownership
story. What plan-98-D adds is *when the OS-side backing may be released*.

Two counters, both graphics-private:

* `lastUsedFrame` — stamped on a texture each time a frame **draws** it.
* `lastCompletedFrame` — advanced once per frame, when that frame's present
  completes. (Software: when the blit returns. E/F: from the GPU fence or completion
  handler — the same counter, so the free code is unchanged.)

**The rule:**

> A closed texture's OS backing is freed when
> **`closed AND lastUsedFrame < lastCompletedFrame`**.

Read as: MFB is done with it (`closed`), *and* no frame that drew it is still
outstanding (`lastUsedFrame < lastCompletedFrame`).

Supporting rules, each of which the gate depends on:

* **Close never frees.** `canvas::destroyImage` and scope-drop set the flag and
  nothing else. This is what makes them safe at any instant, from the worker, with no
  knowledge of what the graphics thread is doing.
* **A closed texture is skipped in new frames.** So `lastUsedFrame` stops advancing
  the moment it closes, and the gate is guaranteed to open.
* **A closed id in a NEW scene raises `ErrResourceClosed`** (plan-98-B). So no future
  frame can resurrect a texture whose free is pending.

## 8. The race matrix

Every ordering below must hold. This list is plan-98-D Phase 4's test matrix; each
row names the rule from above that protects it.

| # | Interleaving | Required outcome | Protected by |
|---|---|---|---|
| R1 | present → `destroyImage` → graphics mid-record | the in-flight frame keeps sampling the texture and completes normally | §7 "close never frees" |
| R2 | present → `destroyImage` → frame completes → next frame | the next frame skips the texture; the free fires exactly once | §7 skip-in-new-frames + the gate |
| R3 | `destroyImage` → present naming the same id | `ErrResourceClosed` at `present`, no frame ever sees it | plan-98-B closed-read guard |
| R4 | two presents, no frame between | the second scene renders; the first is skipped, not rendered late | §3 step 2 overwrite |
| R5 | present while graphics is mid-render | `present` does not block; the new scene renders next frame | §3 three slots |
| R6 | graphics stalled indefinitely, worker presents repeatedly | `present` still never blocks; slots are reused, no unbounded allocation | §3 "nobody frees a slot" |
| R7 | resize while graphics is mid-render | the in-flight frame completes at the old size; the next is at the new size | §5 clear-at-frame-start |
| R8 | resize with the worker blocked in `io::input` | repaint happens with zero worker involvement | §5 main↔graphics only |
| R9 | N `setBytes` between two frames | one upload, last value wins | §6 coalescing |
| R10 | `setBytes` on an image not in the live scene | no repaint at all | §4 trigger 5 |
| R11 | `setBytes` → `destroyImage` → frame | no upload into a closed texture; free still gated | §7 skip-in-new-frames |
| R12 | program exits while a frame is in flight | no use-after-free of the scene slots or the pixel buffer | shutdown must join graphics before the worker's frame unwinds |

R12 is **not** named by plan-98-D's design; it was found writing this document. The
scene slots live in the worker's arena and the worker's arena state lives on the
worker's *stack frame*, so a graphics thread still rendering after the worker's entry
returns would be reading freed stack. The graphics thread must be joined (or proven
stopped) before the worker's entry unwinds.

## 9. What is deliberately absent

* **No refcount, no retain/release, no per-frame reference set.** §7.
* **No lock on the building slot.** The language is single-worker (spec: one runtime
  worker pthread, no user thread primitive), so a second concurrent `present` is not
  a thing that can happen. A lock there would guard an impossible caller.
* **No time-based repaint.** §4.
* **No cross-thread arena free.** §3.

## See also

* `planning/completed/plan-98-A-*` — cross-cutting invariants 1, 2, 4, 5, 7, 8.
* `.ai/collections.md` — why the rasteriser's pixel writes must stay inside the
  function owning the buffer local (they are 290x slower otherwise).
* `.ai/testing-gates.md` — the canvas reference-image gate the ring must not disturb.
