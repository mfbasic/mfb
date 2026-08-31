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

The ring is **process-global storage** (`CANVAS_SCENE_SYMBOL`, a writable data
symbol), not arena state. That is what makes it visible to a thread with its own
`x19`, and it is the only canvas state shared between threads.

It is **three pointers, not three buffers.** A fixed slot array presumes a slot is a
reusable buffer the producer refills; an MFBASIC collection is a *value*, so every
`present` deep-copies into a block sized for that scene and a slot can only ever hold
a pointer. The three that exist are:

```
items / hashes / layers          the published scene   (what the renderer reads)
retiredItems / retiredHashes /
retiredLayers + retiredFrame     the block just displaced
                                 (plus a fresh block being built inside present)
```

### Ordering

**Worker, in `canvas::present`:**

1. Deep-copy the caller's scene into a fresh block.
2. Compare against the published block; if the content is identical, stop — the
   frame skip (§3.1).
3. **Reclaim**: if a previous retirement exists and `frames > retiredFrame`, free it.
4. **Retire**: move the currently-published pointers into the retired slots and stamp
   `retiredFrame = frames`.
5. Publish the new pointers, then the revision **last**.
6. Signal the redraw condition.

**Graphics, at frame start:** read the published pointers and copy what it needs.

### Why retirement rather than an immediate free

The block a publish replaces may be the one the renderer is copying *right now*.
Freeing it there is a use-after-free. Waiting until the frame counter has passed
`retiredFrame` means a frame has **completed** since the retirement, so no render can
still hold it — the same drain gate §7 specifies for textures, and deliberately so.

### Who frees

Only the **worker**, and only blocks the worker allocated. An arena is per-thread, so
a cross-thread free would corrupt the worker's free list. The graphics thread never
returns memory.

### 3.1 The frame skip compares CONTENT, not bytes

An identical re-present must publish nothing. The comparison is `count`, then
`dataLength`, then the data region — **not** a whole-block `memcmp`. A collection
block is not byte-comparable even between two shrink-to-fit copies: a lookup entry is
40 bytes of which a *list* writes only some, so `keyOffset`/`keyLength` hold whatever
the arena handed out. The whole-block form never once reported "unchanged".

### Two presents before one frame

The intermediate scene is **skipped**, and that is correct — it was never on screen
and nothing observed it. The redraw signal is a flag, not a counter, so two presents
between frames produce one frame. A test that needs one frame per present must set
`MFB_CANVAS_SYNC` (§10).

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

1. **Main**, in the platform's resize callback (macOS: `MFBCanvasView setFrameSize:`),
   publishes the new width and height into the graphics state and signals a redraw.
2. **Graphics**, at frame start, reads them (`canvas::surfaceWidth` /
   `surfaceHeight`) and allocates the frame buffer at that size.

There is **no `resizePending` flag**. The renderer reads the size at the start of
every frame anyway, so the size *is* the flag; a separate one would be a second thing
to keep in sync with what it describes.

Main never touches the pixel buffer, and graphics never touches the window. The worker
is not involved at all — the guarantee `term::` does not give: **a canvas resizes and
repaints correctly while the program is blocked in `io::input`.**

A frame already in flight when the resize lands finishes at the old size and is
presented; the next frame is at the new size. Tearing the frame mid-render would mean
drawing part of the picture at each size.

**The failure this must rule out is doing nothing.** `CALayer`'s default
`contentsGravity` stretches the old frame to fill the resized layer, so a resize that
never reached the renderer still *looks* plausible. The test therefore measures a
fixed-size shape as a fraction of the window (`test-macapp.sh` Case 3g).

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
* **A closed image cannot be named again.** `canvas::imageRef(image)` is a read of
  the *resource* and raises `ErrResourceClosed` (plan-98-B), so a program cannot mint
  a fresh handle to a closed image and no future scene can resurrect one whose free is
  pending. Note the guard is at `imageRef`, **not** at `present`: a `Picture` carries
  an `ImageRef`, which is a plain value, so presenting a stale one draws nothing rather
  than raising.

## 8. The race matrix

Every ordering below must hold. This list is plan-98-D Phase 4's test matrix; each
row names the rule from above that protects it.

| # | Interleaving | Required outcome | Protected by |
|---|---|---|---|
| R1 | present → `destroyImage` → graphics mid-record | the in-flight frame keeps sampling the texture and completes normally | §7 "close never frees" |
| R2 | present → `destroyImage` → frame completes → next frame | the next frame skips the texture; the free fires exactly once | §7 skip-in-new-frames + the gate |
| R3 | `destroyImage` → try to name it again | `ErrResourceClosed` at `imageRef`, so no new scene can carry it | plan-98-B closed-read guard |
| R4 | two presents, no frame between | the second scene renders; the first is skipped, not rendered late | §3 step 2 overwrite |
| R5 | present while graphics is mid-render | `present` does not block; the new scene renders next frame | §3 three slots |
| R6 | graphics stalled indefinitely, worker presents repeatedly | `present` still never blocks; slots are reused, no unbounded allocation | §3 "nobody frees a slot" |
| R7 | resize while graphics is mid-render | the in-flight frame completes at the old size; the next is at the new size | §5 clear-at-frame-start |
| R8 | resize with the worker blocked in `io::input` | repaint happens with zero worker involvement | §5 main↔graphics only |
| R9 | N `setBytes` between two frames | one upload, last value wins | §6 coalescing |
| R10 | `setBytes` on an image not in the live scene | no repaint at all | §4 trigger 5 |
| R11 | `setBytes` → `destroyImage` → frame | no upload into a closed texture; free still gated | §7 skip-in-new-frames |
| R12 | program exits while a frame is in flight | no use-after-free of the scene slots or the pixel buffer | shutdown must join graphics before the worker's frame unwinds |

**Rows R1, R2, R9, R10 and R11 are not yet reachable.** They are the texture and
dirty-upload rows, and there is no texture: `Picture` draws nothing until plan-98-G
brings the sampler, and `canvas::createImage` allocates nothing outside MFB's own
resource record. They become testable in plan-98-E, which is where the deferred free
lands (plan-98-D Correction 13). Every other row is test-proven today.

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

## 10. The renderer branch, and what the Metal backend inherits

`__canvas_renderLoop` calls `__canvas_renderFrame`, which is the **only** place a
renderer is chosen. The choice is a runtime branch, not a build-time one, because
every input to it is a runtime fact:

```
IF canvas::useGpu() AND canvas::metalReady() THEN
  IF __canvas_renderMetal() THEN RETURN
END IF
__canvas_renderScene()
```

* `canvas::useGpu` — did the program ask? (`MFB_CANVAS_GPU=1`, read once at
  spawn.) **Software is the default and must stay so**: it is the oracle the GPU
  backends are measured against, so it cannot become the thing being measured.
* `canvas::metalReady` — did a pipeline build? It runs `_mfb_macapp_metal_init` on
  first call and remembers the answer in `GRAPHICS_OFFSET_MTL_READY` as a tri-state
  (untried / built / failed), so a host with no Metal device pays the device probe and
  the MSL compile once, not per frame.
* `__canvas_renderMetal` returning FALSE — is this *scene* one the GPU shader
  reproduces? It declines rather than draws wrongly. A backend that rendered a circle
  as its bounding box would still report success, and that is the failure mode this
  third condition exists to prevent.

Three consequences for anything added here later:

* **The Metal objects live in the graphics-state block**, not in the app module's own
  storage, for the reason in §2: the graphics thread creates them and is the only
  thread that may touch them, and the arena is per-thread. `GRAPHICS_OFFSET_MTL_*`.
* **The frame is rendered offscreen and read back**, then leaves through the same
  `canvas::blitSurface` the software path uses. That is what makes the two backends
  comparable — the tolerance comparator diffs an RGBA8 buffer, and a frame that only
  ever existed in a drawable is not one.
* **The graphics thread has no autorelease pool**, so the frame renderer pushes and
  pops its own. This is not a leak-avoidance nicety: an unpooled autorelease on this
  thread aborts it in libmalloc at thread exit, with none of your frames in the trace.

The Vulkan backend (plan-98-F) sits behind the same branch, gated on the single
`canvas::vulkanReady` — there is deliberately no second "is Vulkan present" probe, because
two probes of overlapping facts can disagree and one of them did. It has the same offscreen
shape: it renders into an image and reads it back so the frame leaves through
`canvas::blitSurface` like every other. It needs no `VkSurfaceKHR` and no swapchain, which
is what lets it be tested on a box with no display server — and no reachable Linux box has
one.

**The two predicates are not the same predicate.** `__canvas_metalRenderable` declines a
*polygon* past `MAX_EDGES`, because Metal's edges cross as a `setFragmentBytes:` payload,
which is per-item and small. `__canvas_vulkanRenderable` has no per-item limit at all — the
edges live in a descriptor-bound storage buffer — but it declines a *frame* whose polygons
sum past `VULKAN_MAX_FRAME_EDGES`, because one buffer serves the whole frame. That
asymmetry is forced by the APIs: a Vulkan command buffer is recorded once and executed
once, so rewriting or re-binding one buffer per item would give every polygon the *last*
one's edges. Each polygon instead takes a slice and carries its start index in the item
block (`ITEM_ARC_EDGE_BASE`), which is a push constant and so genuinely per-item. A scene
can therefore be GPU-renderable on one backend and not the other, and that is correct.

## 11. Test affordances

Three environment variables, all off by default and none on the production path:

* `MFB_CANVAS_DUMP` — write each rendered frame's raw RGBA to a file. How a headless
  run is observed at all, and what the golden harness reads.
* `MFB_CANVAS_STATS` — **append** one line per rendered frame with the geometry-cache
  counters. Appends rather than overwrites because the interesting quantity is the
  delta between frames.
* `MFB_CANVAS_SYNC` — make `present` wait for the frame it asked for. Frames coalesce
  by design (§3), so frame counts are otherwise a scheduling detail — the same
  three-present program was observed producing one, two and three frames. Any
  frame-level assertion needs this.

A fourth selects the renderer rather than observing it:

* `MFB_CANVAS_GPU` — ask for the Metal backend (§10). The stats line reports all
  of the branch's discriminants — `metal=`, `gpuSelected=`, `metalReady=` and, on
  Linux, `vulkan=` and `vulkanReady=` — which is how a test tells "the GPU agreed
  with the oracle" from "there was no GPU and both runs were the oracle". The
  Vulkan pair distinguishes a third case the Metal pair cannot: a machine with a
  *loader* but no ICD reports `vulkan=FALSE`, which is a real configuration (box
  2227) and not a failure.

And a fifth is not about the renderer at all, but is what makes any of this
observable on Linux:

* `MFB_GTKAPP_HEADLESS` — run the app without GTK. The twin of
  `MFB_MACAPP_HEADLESS` / `MFB_WINAPP_HEADLESS`, and structurally different from
  both: macOS builds its window and merely skips showing it, keeping the AppKit run
  loop, whereas `gtk_init` fails outright with "Failed to open display", so
  `activate` never fires and the worker — spawned from `activate` — never starts.
  The Linux gate therefore skips GTK entirely, spawns the worker from the
  bootstrap, and parks.

  Nothing downstream needs a flag to notice: the finish helper already exits when
  `ST_TEXT_BUFFER` is null, and the canvas blit gates on `ST_CANVAS_AREA`. Both are
  states headless naturally leaves behind rather than a mode to be told about.
  `MFB_CANVAS_DUMP` still sees every frame — the dump is written by
  `__canvas_presentSurface`, before and independently of the blit.

## See also

* `planning/completed/plan-98-A-*` — cross-cutting invariants 1, 2, 4, 5, 7, 8.
* `.ai/collections.md` — why the rasteriser's pixel writes must stay inside the
  function owning the buffer local (they are 290x slower otherwise).
* `.ai/testing-gates.md` — the canvas reference-image gate the ring must not disturb.
