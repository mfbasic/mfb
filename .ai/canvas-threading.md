# Canvas threading: the three-thread model

Normative ordering rules for `Mode.Canvas` rendering. Written before the threading
code (plan-98-D Phase 1) and binding on plan-98-E/F, which swap the *renderer* behind
this boundary without changing any rule here.

Read this before touching the graphics thread, the scene ring, the resize handshake,
or how an image reaches a frame.

## 1. The three threads, and what each owns

| Thread | Owns | Never touches |
|---|---|---|
| **main** (UI) | the window, the surface/layer, the event pump, resize notifications | the scene, the geometry cache, the pixel buffer |
| **worker** (language) | the program, the scene arena, `canvas::present`, `canvas::setBytes`, `canvas::destroyImage` | the surface, the pixel buffer, the GPU frame buffers |
| **graphics** | the render loop, the geometry cache, the pixel buffer, copying a frame's picture texels into its GPU buffer | the window, the scene *slots it does not hold* |

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

**Corollary for anything that crosses threads (bug-498): never allocate from another
thread's arena.** `_mfb_arena_alloc` pops a quick-bin free list with a plain
load/store; a sender that repointed `x19` at the *receiver's* arena to deep-copy a
message there raced the receiver's own allocations and both threads faulted in the
pop. A boundary copy is made in the SENDER's arena and the block handed across. The
hand-over is sound because a *free* touches only the freeing thread's arena state
(`arena_free` pushes onto its own bins; it never asks which arena carved the block)
and every arena's chunks stay mapped for the life of the process (only the main arena
is destroyed, at `_mfb_shutdown`). So "a block allocated by one thread is freed by
another" is fine **as adoption** — the allocating thread must simply hold no further
reference — while "one thread allocates *into* another's arena" never is.

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
retiredHead                      -> node { next, items, hashes, layers, frame }
                                    one node per publish still waiting for a frame
                                 (plus a fresh block being built inside present)
```

The retirement side is a **list**, not a slot (bug-683). It was one slot, and a second
publish inside one rendered frame overwrote the pointer already there and lost a whole
scene copy — 16 KB a present on the program that found it. One slot cannot be enough:
`present` is not rate-limited to one call per frame, every block displaced since the
last frame tick may be the one a render in flight is reading, and freeing the older one
to make room is the exact use-after-free retirement exists to prevent. So the number of
blocks held is whatever the schedule produced. Nodes are newest-first and carry the
frame they were retired at; the list is bounded in practice by presents per rendered
frame, and the first publish after a frame completes drains it to empty.

### Ordering

**Worker, in `canvas::present`:**

1. Deep-copy the caller's scene into a fresh block.
2. Compare against the published block; if the content is identical, **free the fresh
   copy** and stop — the frame skip (§3.1). The free is step 2's own, and it is the
   only one ahead of the drain gate: the block was allocated in this call and never
   published, so no renderer can have seen it. Omitting it leaked one whole scene per
   no-op re-present (bug-683), which is the case `mfb man canvas` calls free.
3. **Reclaim**: if the retirement list is non-empty and `frames > head.frame`, free
   every node on it. Testing the head alone suffices because the head carries the
   largest stamp, so that one comparison proves a frame has completed since every node
   behind it.
4. **Retire**: allocate a node, move the currently-published pointers into it, stamp
   `frame = frames`, and link it at the head. A publish that displaces nothing — the
   first of a program, and the first of each shape — allocates no node.
5. Publish the new pointers, then the revision **last**.
6. Signal the redraw condition.

**Graphics, at frame start:** read the published pointers and copy what it needs.

### Why retirement rather than an immediate free

The block a publish replaces may be the one the renderer is copying *right now*.
Freeing it there is a use-after-free. Waiting until the frame counter has passed a
node's stamp means a frame has **completed** since the retirement, so no render can
still hold it — the same drain gate plan-98-D once specified for textures in §7
(which bug-484 found no texture to apply to) and §13 applies to group buffers.

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
It is implemented by bug-484: the public `canvas::setBytes` is an MFBASIC wrapper
(`func_set_bytes.rs`) over the native swap `canvas::setBytesRaw`, and it signals — and
under `MFB_CANVAS_SYNC` waits — only when the installed items, layers or any group name
the image. Before that nothing signalled at all, and re-presenting the unchanged scene
could not stand in for it: the frame skip refuses an identical scene.

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

## 6. Image upload — implemented by bug-484 without a texture

This section was written (plan-98-D) for a texture protocol — a per-image GPU texture,
a `dirty` flag, upload-once-per-frame — that never got a consumer. bug-484 drew
`canvas::Picture` with **no texture object at all**, and that is what the rules below
now describe:

1. **Worker**, in `canvas::setBytes`: copy the new pixels into a **fresh** block and
   swap the record's pointer (`IMAGE_PIXELS`) to it. The old block is not freed and
   not written. Then signal a redraw if the live scene names the image (§4 trigger 5).
2. **Graphics**, while building a frame: a picture's geometry header captures the
   block address current at that moment (`__canvas_pictureHeader`). The software
   renderer samples that block per pixel (`canvas::shadowTexel`); both GPU emitters
   copy its texels, one packed word each, into the frame buffer's glyph region. Metal copies each
   distinct block ONCE per frame and points every later picture of it at those texels
   (`emit_picture_lookup`, bug-686), and its predicate counts each distinct block once;
   Vulkan still copies and counts per item.

Why this holds up where the texture design needed care:

* **Coalescing is inherent.** Each frame reads whatever block is current when it is
  built, so N `setBytes` between two frames cost N swaps and one read — the last value
  wins, exactly as the scene skip.
* **Upload racing a draw cannot happen.** The copy is part of building the frame's
  own buffer on the graphics thread, from a block nobody writes after publishing it.
  There is no GPU object that an in-flight frame and a new upload could share, so no
  per-texture ring or barrier.
* **The block address is the content generation.** It is in the header, so a
  `setBytes` changes the header: the geometry cache's confirmation misses, and the
  damage diff sees a changed item (`__canvas_appendDraw` folds the address into the
  draw hash, because the worker's published hash predates the swap).

The cost is the per-frame copy of every drawn picture's texels on the GPU paths, and
that those texels share the glyph region's frame cap — a frame whose pictures and
glyphs together overflow it is declined to software by both predicates. A texture
cache would trade that for exactly the protocol above; do it only with a measured scene
that needs it. The superseded `dirty` and last-drawn-frame record words were removed.

## 7. Closing an image — the closed flag, not a refcount

**There is no refcount.** MFB owns an `Image` through the RES model: scope-drop or
`canvas::destroyImage` sets `closed@16` (plan-98-B), and that is the whole ownership
story.

plan-98-D wrote this section for a deferred free of a per-image GPU texture, gated on
`closed AND lastUsedFrame < lastCompletedFrame`. bug-484 drew pictures with no such
texture (§6), so there is **nothing to free** and no gate: the image's record and its
pixel blocks are never freed by anyone, and the frame counter comparison has no
subject. The rules the gate depended on are the ones that remain, and they are now
the whole mechanism:

* **Close never frees.** `canvas::destroyImage` and scope-drop set the flag and
  nothing else. This is what makes them safe at any instant, from the worker, with no
  knowledge of what the graphics thread is doing — a frame that already captured the
  pixel block's address keeps drawing from valid memory (R1,
  `destroying_an_image_mid_frame_lets_the_frame_finish`).
* **A closed image is skipped in new frames.** A picture's header is built through
  `canvas::imageShadow`, which answers `0` for a closed image, and `0` builds the empty
  header (R2).
* **A directly closed image cannot be named again — and since plan-116-I the compiler
  is what says so.** `canvas::destroyImage`'s parameter is a plain `canvas::Image`, not
  a `RES` one, so passing a resource to it is a **move**: a program that calls
  `canvas::destroyImage(img)` and then builds a `Picture` from `img` is refused with
  `2-203-0055 TYPE_USE_AFTER_MOVE`, *"Binding `img` was moved and cannot be used
  again"* — a compile error rather than a runtime raise. The old guard
  (`canvas::imageRef` raising `ErrResourceClosed`) is gone with the member.

  **"Directly" is load-bearing.** A `RES` parameter is an *alias*, so a close that
  happens behind one consumes nothing at the caller: `closeIt(RES img AS canvas::Image)`
  leaves the caller's `img` usable, and `canvas::getSize(img)` afterwards compiles and
  raises `ErrResourceClosed` at run time — pinned by `closedRefuses` in
  `tests/cli/cli_canvas_image_resource.rs`. Any close performed *inside* the runtime is in
  that second category by construction. The compile-time refusal is therefore a
  convenience for the direct case, **not** the invariant the rest of this section rests
  on; the runtime guarantees below are.

  What survives is the case the guard actually protected, and it is now a *render-time*
  rule rather than a mint-time one: **a scene may still hold an item whose resource has
  since been closed**, because the item was built while it was live. The renderer reads
  the backend id through `canvas::imageHandle`/`fontHandle`, which answer **0** for a
  closed resource instead of raising, and 0 is already "no such object" — so that item
  draws nothing and the frame around it renders normally.

  The property to preserve if this is ever touched: **the closed flag is read before the
  handle**, not after. Reading the handle first and testing closed afterwards races a
  concurrent destroy in exactly the window that makes the answer stale.

## 8. The race matrix

Every ordering below must hold. This list is plan-98-D Phase 4's test matrix; each
row names the rule from above that protects it.

| # | Interleaving | Required outcome | Protected by |
|---|---|---|---|
| R1 | present → `destroyImage` → graphics mid-record | the in-flight frame keeps sampling the image and completes normally | §7 "close never frees" |
| R2 | present → `destroyImage` → frame completes → next frame | the next frame skips the picture (there is no free — bug-484, §7) | §7 skip-in-new-frames |
| R3 | `destroyImage(img)` → try to name `img` again | **Refused at compile time** — a direct `destroyImage` moves the binding, so there is no "name it again". A scene built *before* the destroy still draws, as nothing. | plan-116-I; was plan-98-B's closed-read guard |
| R3b | close behind a `RES` parameter → name it again | **Compiles**, and raises `ErrResourceClosed` at run time. A `RES` parameter is an alias and consumes nothing, so R3's compile-time refusal does not reach here — this is the row that covers every close performed inside the runtime. | §7 closed-read guard; `closedRefuses` in `tests/cli/cli_canvas_image_resource.rs` |
| R4 | two presents, no frame between | the second scene renders; the first is skipped, not rendered late | §3 step 2 overwrite |
| R5 | present while graphics is mid-render | `present` does not block; the new scene renders next frame | §3 three slots |
| R6 | graphics stalled indefinitely, worker presents repeatedly | `present` still never blocks; slots are reused, no unbounded allocation | §3 "nobody frees a slot" |
| R7 | resize while graphics is mid-render | the in-flight frame completes at the old size; the next is at the new size | §5 clear-at-frame-start |
| R8 | resize with the worker blocked in `io::input` | repaint happens with zero worker involvement | §5 main↔graphics only |
| | *(proven on the Vulkan path too: `MFB_CANVAS_RESIZE_W`/`_H` resize while the worker sits in `os::sleep`, and both renderers repaint at the new size)* | | |
| R9 | N `setBytes` between two frames | one read, last value wins | §6 coalescing |
| R10 | `setBytes` on an image not in the live scene | no repaint at all | §4 trigger 5 |
| R11 | `setBytes` → `destroyImage` → frame | the closed image draws nothing; nothing reads past the close | §7 skip-in-new-frames |
| R12 | program exits while a frame is in flight | no use-after-free of the scene slots or the pixel buffer | shutdown must join graphics before the worker's frame unwinds |
| R13 | present → `removeGroup` → graphics mid-frame | the in-flight frame completes and still draws the group it had already resolved | §13 retire-then-drain |
| R14 | the same, then a completed frame, then a present **of an unchanged scene** | the buffer is freed exactly once and `groupBytes=` drops | §13 gate at the top of `present`, not on the publish path |
| R15 | `setGroup(A, …)` replacing a live A | the displaced buffer is freed only after a frame completes; the new one is not | §13 one retired buffer per slot |
| R16 | `removeGroup(A)` while a parent group still names A | the parent's node becomes a silent no-op, and the parent's other items still draw | §13 a parent holds a NAME, not a pointer |

| R17 | a group **owning** an image → `removeGroup` → graphics mid-frame | the in-flight frame keeps the image **open**; it is closed only after a frame completes past the retirement | §13 retire-then-drain + plan-116-J's close on the free path |
| R17b | the same with a **`Font`** | identical outcome, and it is a separate row because `destroyFont` unregisters the table slot **immediately**, behind no frame gate — so only this flavour can catch an early unregister | §13; the drain gate is what covers the asymmetry |
| R18 | `setGroup` replacing a group whose new items name the **same** resource | **nothing is closed.** This is the ordinary shape — one long-lived font, a group rebuilt each frame — and closing here makes its text vanish one frame later, silently | plan-116-J: close only what nothing live names |
| R19 | a resource named by a group **and** by the live scene, group then drops it | **nothing is closed.** `present` does not take ownership, so a `Picture` built before the `setGroup` reaches the scene with nothing for the move checker to object to | plan-116-J: the live set includes the incoming scene |
| R20 | a resource named by **two groups**, one of them reclaimed | **nothing is closed.** Refused at compile time when the compiler can see it (`2-203-0055`, `setGroup` consumes its `items`), and caught at run time when it cannot — a loop body is analysed once, so a rebuild across iterations is a deliberate false negative | plan-116-J: the live set is every group's items |
| R21 | 200 × install/remove of a group owning a resource | `groupBytes=` returns to baseline | plan-116-J + §13's gate |

**R17–R21 are plan-116-J's**, and they share one rule: **a group closes a resource only if
nothing live names it** — where "live" is the scene about to be published *plus every
group's live items*. Closing on the narrower "what the retired buffer named" is wrong in
three of these five rows, and wrong **silently**: `imageHandle`/`fontHandle` answer `0` for
a closed resource, `0` is "no such object", so the item simply stops drawing and nothing is
raised. All five are pinned by `tests/rt_canvas_group_ownership.rs`.

**R13–R16 are plan-116-G's, and R13 is the first mid-frame row that is deterministic
rather than probabilistic** — see `MFB_CANVAS_FRAME_HOLD_MS` in §11. R14's *"of an
unchanged scene"* is load-bearing and not incidental: a free placed where the scene
ring's `emit_reclaim_retired` sits would never run for it, because that code is after
the publish label, and a test that changed the scene would pass against that wrong
placement.

**Rows R1, R2, R9, R10 and R11 became reachable with bug-484**, which gave `Picture`
its first renderer. They were written as the texture and dirty-upload rows; with no
texture (§6) they are image-read rows, and they are pinned by
`destroying_an_image_mid_frame_lets_the_frame_finish` (R1),
`the_frame_after_a_destroy_skips_the_picture` (R2) and
`set_bytes_then_destroy_then_a_frame_draws_nothing` (R11) in
`tests/canvas/rt_canvas_graphics_thread.rs`, and
`set_bytes_twice_between_frames_shows_the_last` (R9) and
`set_bytes_on_an_undrawn_image_does_not_repaint` (R10) in
`tests/canvas/rt_canvas_picture.rs`. Every row is now test-proven.

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
* **The frame is rendered offscreen.** Where it goes next depends on who needs its
  pixels (bug-686 Phase 4). In a real window Metal GPU-copies it into the canvas view's
  `CAMetalLayer` sublayer's next drawable and presents it (`canvas::metalPresentScene`):
  no CPU surface, no readback, no swizzle, no `CGImage` blit — which at a full-screen
  surface had been ~10 ms of fixed cost per megapixel. It is read back and leaves through
  the same `canvas::blitSurface` the software path uses whenever something needs the
  pixels on the CPU: a headless run (no window layer — so **every test is on the readback
  path**, which is what keeps the oracle comparisons meaningful), damage mode (keeps the
  previous frame), a `--debug` build writing `MFB_CANVAS_DUMP`, or a frame with no
  drawable. All layer property changes (show/hide, drawable size, colour space) run on
  the MAIN thread through `mfbMetalLayer:`, and only when the state changes; a software
  frame's blit hides the Metal layer in the same main-thread step that sets its image.
  The stats line's `directFrames=` counts the frames that took the direct path, which is
  how `scripts/test-macapp.sh` Case 3h tells the two apart in a real window.
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

### A group is one instanced draw per node, with an offset bound to BOTH stages

Since plan-116-H a `canvas::Group` is not flattened away before the GPU sees it. The
worker builds a **draw list** (`__canvas_sceneDraws`) alongside the block list, one entry
per contiguous run of blocks at a given offset:

```
(itemBase, itemCount, dx, dy, blendMode, 0, 0, 0)     eight 64-bit words
```

Each backend walks it once, after every block is published, and issues one instanced
draw per entry: `vkCmdDraw` with `firstInstance = itemBase`, or
`drawPrimitives:…instanceCount:baseInstance:`. A group's blocks are recorded **once** and
referenced by base, so a diamond — two parents naming one child — reports
`entries=1 blocks=1` and two draw entries that share base 0 with different offsets.

**The offset goes to the fragment stage as well as the vertex stage, and that is the fact
most likely to be got wrong.** The obvious reading is that a translation is a vertex-stage
concern: move the quad, done. It is not, because this renderer is **signed-distance
based** and an SDF is evaluated at an *absolute* point. The fragment shader asks "how far
is this pixel from the shape", and the shape's coordinates are its own, not the group's.
So the split is:

* the **vertex** stage moves the quad by `+offset`, and
* the **fragment** stage moves the query point by `-offset` and evaluates everything —
  distance, gradient ramp, glyph coverage — at that shape-space point.

Bind it to the vertex stage alone and the shape lands in the right place with the *wrong
contents*: a gradient's ramp is sampled at the un-shifted pixel, so it slides across the
shape, and a glyph reads the wrong texel of its bitmap. A scene of flat-filled rectangles
cannot see any of it — which is why the acceptance scenes carry a gradient-filled item, a
`Text` item and a clipped item inside translated groups.

**The clip is the exception and stays in surface space.** `Paint.clip` is a window on the
surface, not on the shape, so it is evaluated at the *un-shifted* point on both backends.
Moving it with the group is the mistake that looks like a fix.

Where the offset is bound differs per backend and per stage, so it is three numbers, not
one: Vulkan uses a single push-constant range both stages declare; Metal uses
`setVertexBytes:` at buffer index **1** and `setFragmentBytes:` at index **3**, because
its fragment stage already has items, edges and the glyph bitmap at 0..2.

**A predicate cannot decline a group scene by looking for a `Group` item.** By the time
`__canvas_vulkanRenderable` or `__canvas_metalRenderable` sees the offsets list, the walk
has expanded every group away — a search finds nothing, the frame is accepted, and every
group's child draws at the origin. plan-116-G solved that with a flag the *walk* set; with
both backends taught the flag has no reader and was deleted, so a future backend that
needs to decline has to set one again rather than search.

**Count published records, not items, in the frame caps.** `__canvas_blockInstances` is
the one answer to "how many instances does this block become", and both the draw list and
the caps ask it. An item that both strokes and fills under a non-Normal blend mode
publishes **two** records; counting it as one under-fills the cap, which is the direction
that lets a scene write past the mapping. The emitters' own split test reads `strokeHalf`
**signed** for the same reason — a paint that does not stroke reports `-1.0`, and a
zero-extending load makes that a very large positive number.

### The per-item parameter block travels in a buffer, on both backends

Since plan-116-A the item block is **not** a per-draw value. It lives in a per-frame
buffer of `ITEM_BLOCK_SIZE`-byte records — `…_VULKAN_ITEM_BUFFER` on one side,
`…_MTL_ITEM_BUFFER` on the other — written once per item at a cursor and read back by
the shaders through the instance index. A run of consecutive non-text items is then one
instanced draw rather than N draws.

Two properties of the old transport had to go, and only one of them is the obvious one.
A push constant (or a `setVertexBytes:`) is per-*draw*, so it could describe exactly one
item — which forced one draw call per item — and it pinned the block under Vulkan's
*guaranteed* 128-byte push-constant range, which is the only guaranteed value and so the
only one a portable design may assume. The block was at 112 of those 128 bytes, and it is
what every later letter of plan-116 widens.

**The instance index includes the base on both languages.** Vulkan's `gl_InstanceIndex`
includes `firstInstance` and MSL's `[[instance_id]]` includes `baseInstance`, so each
shader indexes the buffer with that one value and adds nothing. Do not "fix" this by
adding a separate `[[base_instance]]` on the Metal side: that double-counts, and the
symptom is not a compile error but a scene in which `baseInstance = 0` draws perfectly
and every non-zero base draws *nothing* (2×base indexes past the published blocks into
zeroed buffer, giving a degenerate quad). A scene with no text is one run starting at 0,
so every GPU test that predates plan-116-A passes straight through that bug.

`gl_InstanceIndex`/`[[instance_id]]` reaches the fragment stage as a **flat** varying,
because neither builtin exists there. Flat, not interpolated: the value is an index, and
interpolating an index across a quad yields a plausible picture drawn from the wrong
blocks rather than a failure.

**Glyph runs are still N draws, not N instances.** A text item was never one draw
(`GEO_KIND_TEXT`), and folding it into the instancing scheme is a change of shape rather
than of transport. Each glyph still publishes its own block and is drawn at its own
index, and the per-glyph coverage bitmap is the one per-draw payload left anywhere on
either backend — it never has to survive an instanced run, so it did not have to move.

A consequence worth knowing before adding anything per-item: **an instanced run cannot
rebind a per-item side payload between its instances.** Anything that varies per item
must therefore be a region of a frame buffer reached by an index carried in the block,
not a payload. That is what forced Metal's polygon edges to move (below), and it is the
shape any future per-item payload has to take.

**The two predicates are still not the same predicate, but they differ less than they
did.** Both now decline a *frame* whose polygons sum past their edge cap
(`VULKAN_MAX_FRAME_EDGES` / `METAL_MAX_FRAME_EDGES`, both 16384) and a frame with more
drawn *quads* than `CANVAS_MAX_FRAME_ITEMS`. What still differs:

* Metal has **no per-polygon edge cap** since bug-686. The old one (256, kept "by
  policy" after plan-116-A removed the `setFragmentBytes:` payload that justified it) had
  a twin in `emit_edge_buffer` that drew an oversized polygon as NOTHING; both are gone.
  A large polygon is indexed by horizontal band instead (`emit_band_index`): the shader's
  `edgeDistance` walks only the edges within reach of the pixel's band, which is
  bit-identical to walking them all and 36-108× faster on realistic outlines.
* Metal's frame caps are its own and larger — 65,536 quads, 262,144 edges, 131,072
  gradient stops, 8M glyph/picture texels — while Vulkan keeps `CANVAS_MAX_FRAME_ITEMS`
  (4,096), `MAX_FRAME_GRADIENT_STOPS` (4,096) and its 1M texels.
* The glyph caps differ in *shape*: Metal's is per glyph (`METAL_MAX_GLYPH_SAMPLES`,
  its bitmap still rides `setFragmentBytes:`), Vulkan's is a frame total
  (`VULKAN_MAX_FRAME_GLYPH_SAMPLES`, its bitmaps ride the shared buffer).

A scene can therefore still be GPU-renderable on one backend and not the other, and that
is correct. The reason a frame buffer needs a per-item *index* at all is unchanged and
worth restating: a command buffer is recorded once and executed once, so rewriting — or
re-binding — one buffer per item would give every item the *last* one's data.

Metal scenes whose polygons sum past 16384 edges are the one class that plan-116-A newly
declines to software. Software is the oracle, so the picture is at least as correct;
truncating instead would draw a *different shape*.

## 11. Test affordances

Three environment variables, all off by default and none on the production path:

* `MFB_CANVAS_RESIZE_W` / `MFB_CANVAS_RESIZE_H` — in a headless run, wait for the
  first completed frame and then resize the surface to these dimensions, by calling
  the same handler the platform's resize signal calls. A resize is a *window* event
  and no reachable Linux box has a display server, so without this the handshake
  could be implemented and never executed. Waiting for a frame first is the whole
  point: resizing before one exists builds the render target once at the new size and
  proves nothing, where resizing after one forces the tear-down-and-rebuild.

  Two variables rather than one `WxH` string because parsing a separator in
  hand-written assembly buys nothing over `atoi`. Note that the program under test has
  to still be alive — with `MFB_CANVAS_SYNC=1` the worker returns from `main` the
  moment its frame lands and the finish helper `_exit`s the process, so a scene that
  ends at `present` loses the race every time.

* `MFB_CANVAS_DUMP` — write each rendered frame's raw RGBA to a file. How a headless
  run is observed at all, and what the golden harness reads. **A `--debug` build only**
  (plan-130-E): a normal build contains no reader, so the variable does nothing there —
  a suite builds with `common::build_app_debug`, a box script with `--debug`.

  **Always pair it with `MFB_CANVAS_SYNC=1`.** Without the wait, `present` returns at
  once and `main` returns behind it, and the process tears down while the graphics
  thread is still reading the scene. The geometry survives that — the ring holds a
  published copy — but a `canvas::Font`'s outlines do not, because they live in the
  worker's own arena, which is per-thread (§1). The dump then lands with **every shape
  and no text**.

  What makes this dangerous is that it is not a race. Measured on plan-116-C's
  transform scene: five consecutive runs without `SYNC` produced 0 text pixels *every
  time*, so the truncated frame is perfectly reproducible and `compare_exact` reports
  it as a match. `tests/canvas/rt_canvas_golden.rs` was regenerated from one and the suite was
  green. The third measurement is what names the mechanism: no `SYNC` but an
  `os::sleep(1500)` after `present` gives the full 840 text pixels, so it is the
  teardown and not the font path.

  A scene with no font shows nothing — `smiley.png` and `blendmodes.png` are
  byte-identical with and without the flag, which is why the gap survived two letters.
* `MFB_CANVAS_STATS` — **a `--debug` build only**, like `MFB_CANVAS_DUMP` (plan-130-E).
  **Append** one line per rendered frame with the geometry-cache
  and glyph-cache counters (`entries=`, `floats=`, `glyphs=`, `glyphBytes=`,
  `glyphEvictions=`), `generations=` (geometry builds — one per CHANGED item per frame
  since bug-686), `directFrames=` (Metal frames presented straight to the window), and
  the cumulative phase timers `phaseOffsetsMs=` / `phaseDrawsMs=` / `phaseDamageMs=` /
  `phaseRenderMs=` (bug-686 Phase 0; `__canvas_phaseMark` is an empty SUB in a normal
  build). `tools/canvas-bench/` reads them. **A `--debug` build is ~35% slower on this
  path** (116 vs 157 frames/5 s on 1,000 moving polygons), and much more on
  allocation-heavy code, so judge frame rates on a release build. Appends rather than overwrites because the interesting quantity is
  the delta between frames. It is also the **only** window onto either cache: both live
  in globals owned by the graphics thread, so a program asking from `main` asks the
  worker, whose copies are its own and always empty (§1).
* `MFB_CANVAS_GLYPH_BUDGET` — shrink the glyph coverage cache's byte budget (default
  1 MiB), so a test can force eviction with a scene small enough to also check pixel by
  pixel. Resolved once and cached, so the ordinary path is a compare against a global
  rather than a `getenv` per glyph.
### The surface is opaque black on every backend, at four layers

An unpainted canvas pixel is opaque black, and that takes agreement in four places
because each could independently be transparent:

* the software surface — `canvas::newSurface` fills opaque black;
* the Vulkan render pass — its clear value is opaque black;
* the Metal render pass — `setClearColor:` is set **explicitly**, not left to
  `MTLRenderPassAttachmentDescriptor`'s documented default, so the three backends agree
  by construction rather than by three defaults happening to match;
* the macOS canvas `CALayer` — its `backgroundColor` is an opaque black `CGColor`, built
  once when the view is made layer-backed. This one is what a program sees *before* the
  first frame, and anywhere a frame does not reach: a layer-backed `NSView` is
  transparent by default, so without it the canvas is the window showing through and no
  amount of clearing above would fix it.

### `canvas::didResize` is a counter pair, not a flag

The platform's resize path bumps `GRAPHICS_OFFSET_RESIZES` — but only when the size
actually **changed**, because AppKit re-publishes the size it already has and the
headless scripted resize does too. `canvas::didResize` compares it against
`GRAPHICS_OFFSET_RESIZES_SEEN`, which the worker owns, and records the new value when it
answers TRUE.

Two words with one writer each, so no lock: the main thread only ever writes the
counter, the worker only ever writes the acknowledgement. A single read-and-clear flag
would need one, because a resize landing between a reader's load and its store would be
lost — on the one path whose entire job is to report edges.

* `MFB_CANVAS_DAMAGE` — repaint only what changed: keep the previous frame's pixels,
  clear the union of the changed items' bounds, and redraw only the items that meet it.
  Off by default. It changes no pixels — that is what `tests/rt_canvas_damage.rs`
  asserts, byte for byte — but it does change *when* the renderer runs at all, and a
  frame counter that silently stops advancing is the kind of thing a stale test reads as
  a pass.

  Two notes for anyone testing it. An unchanged scene never reaches the renderer in the
  first place: `canvas::publishScene` refuses it and `present` does not signal a redraw
  (§2's invariant), so the *empty* damage union only fires on a platform wake — a resize
  or an OS damage repaint. And the GPU backends always render full-frame: they draw into
  their own texture and read it back, so there is no kept surface for them to preserve.
* `MFB_CANVAS_SYNC` — make `present` wait for the frame it asked for. Frames coalesce
  by design (§3), so frame counts are otherwise a scheduling detail — the same
  three-present program was observed producing one, two and three frames. Any
  frame-level assertion needs this. **Any non-empty value enables it** —
  `MFB_CANVAS_SYNC=0` is ON (`__canvas_ensureGraphics` tests the length), which is how a
  "pipelined" measurement in bug-686 silently ran serialized. Leave it unset to pipeline.

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

### `MFB_CANVAS_FRAME_HOLD_MS` — hold the graphics thread inside a frame

Milliseconds to sleep in `__canvas_renderFrame`, immediately after
`__canvas_sceneOffsets` has built the draw list. Unset or `0` is off, and it is off the
production path like the four above.

**It exists because nothing else here produces "graphics mid-frame."** The two proven
mid-render rows (R5, R7) reach it through `MFB_CANVAS_RESIZE_W`/`_H` firing while the
worker sits in `os::sleep`, which is specific to resize — so a row needing any *other*
worker action mid-frame was either untestable (R1 sat marked "not yet reachable" for
three letters) or tested by luck, and a green run tested by luck reads exactly like one
tested by construction.

The hold point is after the draw list is built, deliberately: by then every group name
is resolved and every group's items copied, so a `removeGroup` arriving during the hold
lands while a frame is demonstrably still working from that block. That is the window
the drain gate exists for.

**The worker has to be made to lose the race, too.** `present` returns as soon as it has
signalled, so a worker that calls `removeGroup` immediately usually gets there before the
graphics thread has resolved anything — the group is then removed before it is used and
the frame correctly draws nothing, which exercises the absent-name path instead. The
worker needs its own short sleep so its action lands *inside* the hold. R13's test uses
a 600 ms hold and a 120 ms worker sleep against a frame measured in single-digit ms.

## 12. Why the font rasteriser is hand-rolled

Canvas rasterises glyphs with a TrueType reader and a contour rasteriser written in
**MFBASIC**, in the same helpers as every other primitive — not a vendored library.
That was plan-98-G's named open question, decided 2026-08-31, and it is recorded here
because the reasoning is not obvious from the code and the question will be asked again.

**The oracle is the reason.** The software renderer is not one renderer among three; it
is the reference every GPU backend is measured against (`Tolerance::GPU_DEFAULT`), and
plan-98-F Phase 1 measured it **byte-identical across macOS/Linux and
aarch64/x86-64** — 2,304,000 bytes, two ISAs, two operating systems. A vendored font
library would have to ship per platform and architecture, so the same string would
rasterise differently on each target, and the text goldens would need a tolerance
instead of exact match. That trades away the gate the whole feature set rests on, to
save writing a `glyf` parser.

**Compiling a font library into `mfb` does not work**, and is worth stating so it is not
re-proposed: rasterisation happens at *program run time* for arbitrary strings, and an
emitted program has no C toolchain and no CRT. The compiler can only bake glyphs it
already knows, which text rendering is not.

**It also fits what is here.** `__canvas_edgeDistance` already walks a polygon's edges
for a signed distance and `__canvas_geoDistance` dispatches the kinds; a glyph is a set
of quadratic contours, and coverage-from-a-signed-distance is that same machinery. The
font-specific code is a `cmap`/`loca`/`glyf` reader plus contour flattening — fill,
antialiasing and blending are shared with rectangles and circles, which is also what
keeps a glyph's edge pixels consistent with everything else on the surface.

`canvas::loadImage` rides the same decision: an inflate and a PNG unfilter beside the
font reader, rather than a second vendored library.

The residual risk moved rather than vanished. It is no longer "is the third-party
rasteriser deterministic" but "does the contour rasteriser use anything width- or
order-dependent" — a thing to not do, caught by the same cross-target byte-identity
comparison.


## 13. The named-group table (plan-116-G)

A third process-global block, `_mfb_rt_canvas_groups`: 256 fixed slots plus a one-word
header. Process-global for the reason the scene region and the font table are — §2 —
`canvas::setGroup` runs on the worker and the renderer that draws a group runs on the
graphics thread. **Fixed, not growable**: the graphics thread scans it without a lock,
and a reallocating array would move under a reader.

**A slot is published name-LAST and dropped name-FIRST.** `name` is the discriminator: a
scanning graphics thread treats a non-zero name as "this slot is real, follow its
pointers". So a slot must never be visible under a name before its `items` and `count`
are written, and dropping one must hide it before anything else changes. This is the same
publish-then-flag rule §3 gives for the scene revision.

### There is no refcount, and there is nothing to count

`canvas::groupItems` returns a **copy**, so a published scene never holds a pointer into
a group's buffer and a parent group never holds one into its child's — a `canvas::Group`
node carries a *name*, and the renderer resolves it per frame. The only window in which
anything reads the block is that copy, on the graphics thread, inside one frame.

So the **buffer's** lifetime rule is the drain gate alone, and it is the one §3 and §7
already use *(the resources the buffer's items name are a separate question, and
plan-116-J answers it below — still without a refcount)*:
`removeGroup` and a replacing `setGroup` **retire** the displaced buffer and stamp the
frame; the buffer is freed once a frame has completed since. A reference count would have
been a second mechanism guarding a lifetime this already bounds.

The cost is a copy per group per frame *that renders*, against a copy of the whole
sub-picture per `present` — and presents outnumber rendered frames by design, because
the frame skip is what this feature's reuse goal rests on.

### The gate runs at the top of `present`, not where the scene ring reclaims

`emit_reclaim_retired` is emitted *after* the publish label in `gen_present.rs`, so it
runs only on a present that actually changes the scene. A group free placed beside it
inherits that: `removeGroup("panel")` followed by presents of an unchanged scene would
never free anything — the frame skip working exactly as designed, and the memory held
anyway. **A memory bound that depends on the scene changing is not a bound.**

`canvas::nextReclaimableGroup()` therefore runs first and unconditionally in
`__canvas_present`. It is a scan of 256 slots with no allocation, which is what makes
unconditional affordable. *(It was `canvas::groupReclaim()`, which both found and freed;
plan-116-J split the two so the resources a retired buffer owns can be closed in
between — see below. `groupReclaim(slot)` is now the freer and takes the slot the finder
named.)*

### A group OWNS the images and fonts its items name (plan-116-J)

`canvas::setGroup`'s `items` parameter **consumes** the resources reachable from its
argument. A group outlives the `present` that draws it, so it has to keep them alive — and
it cannot while the caller still owns them, because scope-drop closes a `RES` its binding
still owns and the group holds only an **alias** into the same record. So the caller's
binding is moved: naming the image again is `2-203-0055 TYPE_USE_AFTER_MOVE`, and codegen
drops that scope's close obligation.

**The close hangs off the free path, and the rule is not "close what the retired buffer
named".** It is:

> A retired resource is closed only if **nothing live names it** — where *live* is the
> scene about to be published **plus every group's live items**.

Three ordinary programs break under the narrower rule, all of them silently, because
`imageHandle`/`fontHandle` answer `0` for a closed resource and `0` already means "no such
object" — the item simply stops drawing and nothing is raised:

* **a group rebuilt each frame from one long-lived font.** The retired buffer and the one
  replacing it name the same font. This is the shape every real canvas program has.
* **a resource named by a group and by the live scene.** `present` does not take
  ownership, so a `Picture` built *before* the `setGroup` reaches the scene with nothing
  for the move checker to object to.
* **a resource named by two groups.** Refused at compile time where the checker can see
  it, but a loop body is analysed once, so a rebuild across iterations is a deliberate
  false negative (that conservatism is what keeps the rule from rejecting valid programs).

Identity is compared through `canvas::imageHandle`/`canvas::fontHandle`, which return the
backend id as an `Integer` — two aliases of one resource are not comparable as `RES`
values. This is the second reason their read order matters: they test the closed flag
**before** loading the handle (§7), so a concurrent destroy cannot yield a stale non-zero
id that keeps alive a resource nothing names.

**A `Font`'s close is not symmetric with an `Image`'s.** `destroyImage` sets the closed
flag and nothing else; `destroyFont` runs `emit_unregister_font` **first**, which clears
the table slot immediately and is not deferred behind any frame gate — *"so that text
still naming a released font draws empty rather than reading a block the program has
finished with"*. The live-set check is what keeps that safe here: the close only fires
once a frame has completed since the retirement, and if the frame now in flight names the
font, the scan sees it. One narrow case survives — an in-flight frame drawing a previous
scene that names the font **directly**, while the incoming scene and every group do not —
and it costs that frame's glyphs, not a crash. It is what `destroyFont` mid-frame has
always done. **R17 is pinned in both flavours** — an `Image` and a `Font` — because the
image one cannot detect an early unregister, `destroyImage` having no unregister step to
be early about.

The walk is an MFBASIC `MATCH` (`__canvas_closeRetired`), not an open-coded step over the
`DrawItem` union's layout in codegen: a `MATCH` that a new variant must handle is a
compile error, and a hand-written tag offset that a new variant must not break is a hope.
The scan runs **only when a buffer is actually being reclaimed**, so a present with
nothing due costs what it always did — one call.

Rows **R17–R21** in §8.

### A group node is expanded before any consumer sees it

`__canvas_appendDraw` resolves and flattens the group tree where the draw list is built,
so the offsets list handed to the render walk, the damage diff and both GPU predicates
contains only leaf items, each carrying its accumulated `(dx, dy)` in parallel globals.
No consumer knows groups exist.

Two consequences worth stating because they are easy to get wrong in the other order:

* **The GPU predicates cannot look for a `Group`** — by the time they see the list there
  is none. They read `__CANVAS_DRAW_HAS_GROUP`, set by the walk. A predicate that
  searched would find nothing, accept the frame, and draw every group's children at the
  origin: §10's failure exactly.
* **Both sides of the damage diff need the offset.** The remembered bounds and the
  current bounds must both be translated, and each draw entry's recorded hash must fold
  in its offset — otherwise a moved group changes no geometry and reports "nothing
  changed".

## 14. The geometry cache and item hashes (bug-686)

The graphics thread's geometry cache (`helper_geometry.rs`) is keyed by the item's content
hash through `__CANVAS_GEO_INDEX` (a `Map` hash → slot), and **a hit is trusted on the hash
alone** — nothing is rebuilt to confirm it — except for a `Picture`, whose pixel block
`setBytes` swaps without the item changing (`__canvas_pictureIsCurrent`). That is only
sound because the hash is 62 bits (two independent 31-bit lanes, `__canvas_hashStep`) and
resolves a float to 2^-46 px (`__canvas_hashFloat`); the old single 31-bit lane collided
at ~0.3 per frame at 10,000 items.

Lifetime rules, and why:

* **Nothing is evicted inside a frame.** Slots are stamped with the frame that used them;
  `__canvas_geoBeginFrame`, at the one point no offset is live (the top of
  `__canvas_sceneOffsets`), drops what the previous frame did not use once the stale
  floats reach the live ones, moving kept floats a contiguous run at a time. So every
  offset a frame resolves stays valid until that frame is drawn — the old 256-slot LRU
  needed `__CANVAS_GEO_LIVE` to protect offsets it had evicted mid-frame, and that list
  is gone.
* **Glyph eviction pins only what the frame in progress uses** — slots stamped this
  frame. The cache can hold the previous scene's text too, and pinning it would pin
  everything; an unused TEXT slot is dropped from the index instead (its glyph indices
  are about to be renumbered), and rebuilt if it is named again.
* **One resolve per frame.** `__canvas_sceneOffsets` probes every item from the published
  hash list first and fetches the scene out of the ring only if something missed (or is a
  picture or a group); `__canvas_sceneDraws` lays the draw list out from those offsets —
  items AND layers (it used to walk items only, and a `presentLayers` scene drew nothing
  on Metal).

The worker computes the hashes in `canvas::present`: `canvas::carriedHashes` (native,
taken BEFORE `publishScene` replaces the installed scene) carries the installed hash of
every item whose payload bytes are unchanged, and only the rest are hashed. Byte equality
only ever says "different" too often — two identically BUILT items are not byte-equal
(list headroom, padding), so a rebuilt scene is hashed, never mis-carried. `Text` and
`Picture` are never carried: their hash folds in a resource id that answers 0 once
closed, with the item's bytes unchanged.

## See also

* `planning/completed/plan-98-A-*` — cross-cutting invariants 1, 2, 4, 5, 7, 8.
* `.ai/collections.md` — why the rasteriser's pixel writes must stay inside the
  function owning the buffer local (they are 290x slower otherwise).
* `.ai/testing-gates.md` — the canvas reference-image gate the ring must not disturb.

### Growing `ITEM_BLOCK_SIZE` moves six things, and three are silent

Widening the per-item block is the single most repeated change in plan-116 — A grew it
for the item buffer, C for the transform, D for the arc caps, E for the ellipse, and F
twice (the ellipse `ivec4`, then the gradient one, reaching 208) — and each time it
moved the same set. Three of them are unrelated by any type, and none of the three
fails loudly.

It was five until plan-116-F added a **third** buffer region; the sixth is item 5.

1. **`ITEM_BLOCK_SIZE`** itself, which must stay a multiple of 16 so std430's array
   stride equals the size.
2. **BOTH `ItemBlock` declarations.** The struct is written twice, in
   `mfb_canvas.vert` and `mfb_canvas.frag`, and **the vertex one is what sets the
   stride**. Widening only the fragment copy leaves the stages disagreeing, so every
   item after the first reads a block straddling two records — a plausible wrong
   picture, not a failure. Verify by reflection, not by reading:
   `glslangValidator -V -q mfb_canvas.vert | grep topLevelArrayStride` on box 2228
   (`scripts/regen-spirv.sh` shows how it gets there; the tool is not on the mac host).
3. **Metal's hand-assigned stack frame.** The block is built at `OFF_ITEM` in a frame
   whose slots are hand-numbered, so widening it runs the block into `OFF_TEXTURE` and
   every slot above must shift, with `DRAW_FRAME` growing to match. An overlap
   corrupts a pointer the `objc_msgSend` sequence reads and produces a **black GPU
   frame that reports success**.
4. **`METAL_EDGE_BASE`** and every later Metal region base (band, gradient, glyph,
   picture table). Since bug-686 these are no longer literals: `METAL_SHADER_SOURCE` is
   built at first use with the bases formatted in from the `METAL_*_BASE_WORDS`
   constants, so they move with `ITEM_BLOCK_SIZE` by construction. The region-chain
   test still pins that each region starts where the previous one ends.
5. **Vulkan's `GRADIENT_BASE`** (plan-116-F) IS still a literal, in the checked-in GLSL,
   mirroring `VULKAN_GRADIENT_BASE_WORDS`; fixing Metal's bases and not this one leaves
   one item's stops read as another's on Linux — a plausible wrong ramp, not a failure.
6. The `.spv` blobs, via `scripts/regen-spirv.sh`.

Items 3, 4 and 5 are caught by `the_draw_frame_slots_do_not_overlap`,
`the_metal_shader_region_bases_match_the_buffer_layout` (Metal — it guards **both**
bases and the region chain, despite plan-116-F having found it named for the edge one
alone) and `the_shaders_gradient_base_matches_the_buffer_layout` (Vulkan), which fired
on **every** one of those letters and were the only thing that noticed. Item 2 has no guard beyond the
reflection — plan-116-D shipped the fragment half alone and found it that way.

### A layout constant shared by MFBASIC source and the emitter has no compiler between them

Builtin MFBASIC lives in a Rust `&'static str`, so a record width or slot index it shares
with the native emitter is checked by nothing: the build is clean and tests are green.
Two real cases:

- `__canvas_pushOneDraw` appended four words per entry while `emit_draw_list_pass` stepped
  eight, so every other entry was misread. Under Vulkan that crashes inside the driver,
  with no MFB frame in the backtrace.
- The geometry record's declared length (written by `__canvas_paintHeader`) and its tail
  (appended by `__canvas_tailFor`) disagreed for one kind, so the reader took the next
  record's header as data: wrong colours, no crash.

Define the width once as a Rust constant, derive the emitter's literals from it, and pin
the MFBASIC side with a unit test that reads the source string. The render predicates' frame
caps go one step further since bug-686: each `LET __CANVAS_*MAX*` line carries an `@NAME@`
token that `RENDER_METAL` replaces with the Rust constant at first use (`RENDER_CAPS`), so
there is one definition, and `every_cap_token_in_the_render_source_is_generated` pins that
no token is left behind.
