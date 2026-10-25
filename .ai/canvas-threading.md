# Canvas threading: the three-thread model

Normative ordering rules for `Mode.Canvas` rendering. Written before the threading
code (plan-98-D Phase 1) and binding on plan-98-E/F, which swap the *renderer* behind
this boundary without changing any rule here.

Read this before touching the graphics thread, the scene ring, the resize handshake,
or how an image reaches a frame.

## 1. The three threads, and what each owns

| Thread | Owns | Never touches |
|---|---|---|
| **main** (UI) | the window, the surface/layer, the event pump, resize notifications, publishing the pointer (plan-172-J) | the scene, the geometry cache, the pixel buffer |
| **worker** (language) | the program, the scene arena, `canvas::present`, `canvas::setBytes`, `canvas::destroyImage` | the surface, the pixel buffer, the GPU frame buffers |
| **graphics** | the render loop, the geometry cache, the pixel buffer, copying a frame's picture texels into its GPU buffer | the window, the scene *slots it does not hold* |

Main and worker already exist (`WORKER_SYMBOL` spawned from the UI-thread
surface-ready callback on all three platforms). Graphics is the third, spawned the
same way and from the same place, after the surface exists.

**There is a fourth thread on Vulkan targets, and it is not part of the model**
(bug-996, §10.1): the pipeline builder. It lives for one build, owns no arena, reads
no scene and draws nothing. It shares exactly one word with the graphics thread.

## 2. Arena state is PER-THREAD — the fact everything below is shaped by

Each thread pins `ARENA_STATE_REGISTER` (`x19`) to **its own** block:

* The entry (`src/codegen/engine/function/entry/mod.rs`, `lower_program_entry`) reserves `ENTRY_STACK_SIZE` on
  its own frame and points `x19` at it. In `--app` builds the *worker* runs the entry
  (`MACAPP_PROGRAM_SYMBOL`), so the worker's arena state is on the worker's stack.
* A thread spawned by `thread::start` arena-allocates a child block of
  `ENTRY_GLOBALS_OFFSET + arena_global_slots * 8` and zeroes it
  (`lower_thread_start_helper`).

Everything addressed off `x19` is therefore **thread-local**: module-level globals,
the `term::` state and the presentation-mode word. The canvas scene region WAS there
too, which is the problem this section describes; it has since moved to
process-global storage (`CANVAS_SCENE_SYMBOL`, §3).

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

1. Compare the CALLER's scene against the published block
   (`canvas::sceneUnchanged`); if the content is identical, stop — the frame skip
   (§3.1). Nothing has been allocated, so nothing is freed (plan-175-D). It used to
   deep-copy first and compare the copy, which made every no-op re-present copy the
   whole scene and free it again (and before bug-683, leak it). The compare is by
   type, so the caller's headroom does not matter. The carried hashes (§14) are taken
   only past this point: they are wanted only when hashes will be published.
2. Deep-copy the caller's scene into a fresh block (`canvas::publishScene(items,
   FALSE)`: FALSE because step 1 already compared). `publishLayers` still compares
   inside the publisher, now also before its copy.
3. **Reclaim**: if the retirement list is non-empty and `frames > head.frame`, free
   every node on it. Testing the head alone suffices because the head carries the
   largest stamp, so that one comparison proves a frame has completed since every node
   behind it.
4. **Retire**: allocate a node, move the currently-published pointers into it, **mark
   the publish pending** (below), stamp `frame = frames`, and link it at the head. A
   publish that displaces nothing — the first of a program, and the first of each
   shape — allocates no node.
5. Mark pending (again, for the no-node case), publish the new pointers, then the
   revision **last**. `canvas::publishHashes` follows with the scene's hashes and the
   revision they belong to.
6. Signal the redraw condition.

**Graphics, at frame start:** take ONE snapshot of the published pointers
(`canvas::sceneSnapshot`) and use only it for the whole frame.

### The scene is read under a sequence lock (bug-686)

A frame used to read the items, the layers and the hashes at several points, and the
hashes are published by a second call after the scene. So a frame could pair one
publish's items with another's items or hashes, and since a cache hit is trusted on its
hash alone (§14) it drew the other scene's geometry — measured as 12,000 wrong records
in 13 frames of two alternating 6,000-item scenes (`rt_canvas_geo_native`,
`a_frame_never_draws_one_scenes_geometry_for_anothers_items`).

* `pending` (`CANVAS_SCENE_PENDING_OFFSET`) is `revision + 1` while a publish is
  writing the region and equal to the revision otherwise. `publishScene` closes it by
  bumping the revision; `publishHashes` (which starts no new scene) by setting it back.
* The reader reads the revision, `pending`, the pointers, and `pending` again, and
  retries until both reads of `pending` equal the revision. The hashes count only if the
  revision they were published for (`CANVAS_SCENE_HASHES_REVISION_OFFSET`, read before
  and after the pointer) is the snapshot's; otherwise the frame hashes its own items.
* On AArch64 the marks are store-releases, the reader's loads load-acquires, and
  `scene_base::emit_scene_barrier` (`stlr` then `ldar` of `pending`) keeps the plain
  pointer stores between the mark and the revision. x86-64 keeps stores in order and
  loads in order by itself. There is no fence op in the backend: that barrier is the
  cheapest store-store fence the existing ops make.
* **The mark comes before the retirement stamp.** A frame whose snapshot names a block
  being displaced read `pending` before the mark, so it had not completed when the
  frame counter is read for the stamp (a load-acquire on AArch64) — and the block is
  freed only after the counter passes the stamp. The snapshot's raw pointers therefore
  stay valid for the whole frame. They live in `Integer` globals (`__CANVAS_SNAP_*`),
  never in a list MFBASIC owns; MFBASIC gets copies (`canvas::snapshotItems` and
  friends, `__canvas_frameItems`).

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

An identical re-present must publish nothing. The comparison walks the two scenes by
their TYPE (`emit_values_content_equal_branch`): a list's `count`, then each element
through its entry; a record field by field; a union by tag, then as that variant's
record. Nothing about how a block was built is read — not `capacity`, `dataLength`,
`dataCapacity`, an inlined field's offset, or a union's `size`.

Bytes cannot stand in for content at ANY level, and each shortcut has been shipped:

* A whole-block `memcmp` never once reported "unchanged". A list entry is 40 bytes of
  which a list writes `flags` (one byte), `valueOffset` and `valueLength`; the seven
  bytes after `flags` hold whatever the arena handed out. (`keyOffset`/`keyLength` are
  written, as 0.)
* `count`, `dataLength` and the outer data region byte for byte reported "changed" for
  every scene whose items hold a list built another way (bug-968). The top-level copy
  is shrink-to-fit, but shrink-to-fit is not recursive: a list embedded in an item (a
  polygon's contours, a paint's clip, a layer's items) keeps the capacity it was built
  with, so its spare entries, its entries' reserved bytes and its unused data bytes
  are all inside the outer data region.

An element's bytes are still tried FIRST — equal bytes are equal content — and only an
element whose bytes differ is walked. That keeps an unchanged scene of flat items at
one pass over its bytes, a word at a time.

The walk is emitted from the type model, one routine per type, and names no canvas
item: a new item, field or variant is compared without touching it. A type it has no
arm for (a `Map`, a `Set`, a function value) is a build error in `canvas::present`,
not a wrong answer.

### Two presents before one frame

The intermediate scene is **skipped**, and that is correct — it was never on screen
and nothing observed it. The redraw signal is a flag, not a counter, so two presents
between frames produce one frame. A test that needs one frame per present must set
`MFB_CANVAS_SYNC` (§11).

## 4. Redraw triggers

Exactly seven, and a static scene costs zero frames. **Time is among them in one
bounded case and no other**: trigger 7, which exists only while a `canvas::AutoPaint` is
inside its enter or exit window (`options.canvasAutoPaintMS`, 100 ms by default) and ends with it. Nothing else ever wakes the
render loop on a clock.

| # | Trigger | Signalled by | Notes |
|---|---|---|---|
| 1 | a scene was published | worker, in `present` | |
| 2 | resize | main | see §5 |
| 3 | OS damage/expose | main | no worker involvement |
| 4 | swapchain out of date | graphics | plan-98-E/F only |
| 5 | `setBytes` on an image **in the live scene** | worker | §6 |
| 6 | the pointer changed **and the live scene holds an `AutoPaint`** | main | §16; a WAKE, not a redraw that is owed |
| 7 | an `AutoPaint` enter or exit window ended | graphics (its own timed wait) | §16; likewise |

**Triggers 6 and 7 owe nothing, and that is a different kind of wake.** 1–5 set `PENDING`
and the frame is drawn whatever it finds. 6 and 7 make `canvas::waitForRedraw` answer 2
instead of 1, and the frame is drawn only if some `AutoPaint` now shows a different
paint — asked before the scene is walked (`__canvas_autoLight`). They are not folded
into `PENDING` because **damage is OFF unless `MFB_CANVAS_DAMAGE` is set**: an ordinary
frame is a full redraw and is never skipped for having nothing to do, so a pointer wake
that went down the ordinary path would draw the whole surface for every mouse move. The
plan that added them assumed an unchanged frame was skipped by the damage diff; it is
not, by default.

**A `canvas::ParticleSystem` is advanced by trigger 1 and is NOT an eighth trigger**
(plan-150-A). It holds no state and reads no clock: every particle is a closed form in
its slot and the system's `time`, a field the program sets. A program animates one by
presenting it again with a new `time`, which changes the item's hash and so the scene.
A renderer-side timer that repainted a system would be an eighth trigger and a second
clock; trigger 7 is the only one, and it is `AutoPaint`'s. The whole expansion — the
model, the template flattened into an ordinary shape per live particle — runs on the
graphics thread inside `__canvas_appendDraw`, so it must never raise; the one raise a
system has (`ErrCanvasParticleLifetime`) is the worker's, in the group-signature walk.

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

**The display rate is published the same way, and triggers no redraw** (plan-155-A).
The main thread writes the window's monitor's refresh rate, in millihertz, into
`GRAPHICS_OFFSET_DISPLAY_MHZ` with one plain store: macOS on every canvas entry and on
`NSWindowDidChangeScreenNotification` (`app/display.rs`, the screen's *maximum* rate,
guarded by `respondsToSelector:` because the binary's minimum is 11.0 and the property
is 12+); GTK where the canvas reconcile reads the window's `GdkSurface`, and on that
surface's `enter-monitor` (connected once per surface); Windows when the canvas window
is shown, on `WM_DISPLAYCHANGE`, and on a `WM_MOVE` that lands on a different monitor.
`0` means unknown — and on GTK it is what the first frame usually reports, because an
unmapped Wayland surface has no monitor until the compositor sends `enter`. A rate changes no pixel, so nothing is signalled — §4's triggers
stay as they are.

A frame already in flight when the resize lands finishes at the old size and is
presented; the next frame is at the new size. Tearing the frame mid-render would mean
drawing part of the picture at each size.

**The failure this must rule out is doing nothing.** `CALayer`'s default
`contentsGravity` stretches the old frame to fill the resized layer, so a resize that
never reached the renderer still *looks* plausible. The test therefore measures a
fixed-size shape as a fraction of the window (`test-macapp.sh` Case 3g).

### 5.1 The pointer is published the same way, but under a sequence word (plan-172-J)

The main thread also publishes the pointer — position, held buttons, inside or not —
into the graphics state, for the graphics thread to read (`GRAPHICS_OFFSET_POINTER_*`,
`emit_publish_pointer`, `canvas::pointerSnapshot`). Like the size: plain stores, no lock,
because a pointer move must not wait behind a frame. **Unlike the size, a torn read is
not harmless** — an `x` from one move with an `inside` from another is a position the
pointer never had — so the four words sit behind a sequence word: odd while a publish
is writing, even after; the reader retries until it reads the same even value on both
sides. Every access is a store-release or a load-acquire on AArch64, which is what keeps
a data word from being seen ahead of its mark.

What to remember when touching a handler:

* **Publish BEFORE the mouse-mode gate, never after.** Where the pointer is does not
  depend on whether the program asked for mouse events. On macOS that meant moving the
  gate, which every IMP had as its first act.
* **A canvas program has pointer handlers even with no mouse member.** They used to be
  compiled in only for a program that used one (`uses_mouse`), and `_mfb_rt_mouse_mode`
  still is — so a publish-only handler must not name it, or every plain canvas program
  fails to link. `mouse_view.rs`'s tests hold both halves.
* **Do not drop a point off the surface.** Publish it as outside. During a drag the
  platform sends no leave event; the bounds test is the only place the pointer is seen
  to go.
* **Take the buttons from the event, not only from "what is held now".** A quick click
  is released before its press is dispatched.
* **It wakes the render loop only while an `AutoPaint` is live** (§4 trigger 6, §16):
  after the publish, read `GRAPHICS_OFFSET_AUTOPAINT_LIVE` and, if set, call
  `canvas::signalRedraw` with **1** in the first argument register
  (`emit_skip_unless_autopaint_live`). The publish itself still makes no call — it sits
  in frameless leaf functions on Windows — so the signal is emitted at the handler, after
  it. **1 is "the pointer changed"; anything else is a redraw that is owed.** A resize
  passes 0. Pass 0 from a pointer handler and every mouse move draws a full frame; pass
  1 from a resize and the new size is drawn only if an `AutoPaint` happens to change.
  `runtime::canvas::signal_redraw_arguments` reads the argument off the emitted
  instructions, and each platform's handler has a test that uses it.
* **On GTK, do not read WHICH button is held from the event state.** Bits 9 and 10 are
  right-then-middle on Wayland and middle-then-right on X11. Keep the mask from the
  button number a press or release carries; ask the state only "is any button held".
* **Prove a handler in a real window before trusting it** (`tools/pointer-probe`). The
  headless variables call the callback, not the window system: on GTK they could not
  have shown that the reconcile-built window had no mouse controllers at all
  (bug-1043), nor that right and middle clicks were never delivered (bug-1044), nor the
  state-bit order above. All three were found in the first real window.

## 6. Image upload — implemented by bug-484 without a texture

This section was written (plan-98-D) for a texture protocol — a per-image GPU texture,
a `dirty` flag, upload-once-per-frame — that never got a consumer. bug-484 drew
`canvas::Picture` with **no texture object at all**, and that is what the rules below
now describe:

1. **Worker**, in `canvas::setBytes`: copy the new pixels into a **fresh** block and
   swap the record's pointer (`IMAGE_PIXELS`) to it. The old block is not freed and
   not written. Then signal a redraw if the live scene names the image (§4 trigger 5).
2. **Graphics**, while building a frame: a picture's geometry header captures the
   block address current at that moment (`__canvas_pictureStamp`). The software
   renderer samples that block per pixel (`canvas::shadowTexel`); both GPU emitters
   copy its texels, one packed word each, into the frame buffer's glyph region. Each
   emitter copies a distinct block ONCE per pass (a frame is one pass unless it does
   not fit the buffer, §10) and points every later picture of it at those texels
   (`emit_picture_lookup` — Metal's since bug-686, Vulkan's since bug-688, the same
   table layout), and the pass walk counts each distinct block once a pass.

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
  damage diff sees a changed item (`__canvas_noteEntry` folds the address into the
  draw hash, because the worker's published hash predates the swap).

The cost is the per-frame copy of every drawn picture's texels on the GPU paths, and
that those texels share the glyph region — a frame whose pictures and glyphs together
overflow it is drawn in more than one pass (§10, bug-765), each pass copying the blocks
it draws, and only an image larger than the region by itself is declined to software. A texture
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

  **"Directly" is load-bearing.** A resource parameter is an *alias*, so a close that
  happens behind one consumes nothing at the caller: `closeIt(img AS canvas::Image)`
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
| R3b | close behind a resource parameter → name it again | **Compiles**, and raises `ErrResourceClosed` at run time. A resource parameter is an alias and consumes nothing, so R3's compile-time refusal does not reach here — this is the row that covers every close performed inside the runtime. | §7 closed-read guard; `closedRefuses` in `tests/cli/cli_canvas_image_resource.rs` |
| R4 | two presents, no frame between | the second scene renders; the first is skipped, not rendered late | §3: one set of published pointers, replaced by each publish |
| R5 | present while graphics is mid-render | `present` does not block; the new scene renders next frame | §3: the frame reads one snapshot; the displaced block is retired, not freed |
| R6 | graphics stalled indefinitely, worker presents repeatedly | `present` still never blocks; every displaced block waits on the retirement list, which grows until a frame completes (bug-683) | §3: the retirement list, drained by the first publish after a frame |
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
| R22 | `waitFrame` in progress while the graphics thread stops | `waitFrame` returns; shutdown completes | §15: the stop path broadcasts `FRAME_COND` beside its `COND` signal, and `awaitPickup` returns on `STOPPING` |
| R23 | `waitFrame` before the first present | returns after the display floor; the program exits normally | §15: `waitFrame` calls `ensureGraphics` itself, so the mutex and both condition variables exist; nothing is owed |
| R24 | `waitFrame` after a present, with the previous frame held mid-render | returns only once the render loop has *taken* the redraw — one frame in flight | §15: `waitForRedraw` broadcasts `FRAME_COND` as it clears `PENDING` |
| R25 | a resize lands while the worker is in `waitFrame` | the loop finishes; the next frame is at the new size | §15: `waitFrame` waits on `FRAME_COND` only, so the resize's `COND` signal can only wake the render loop |
| R26 | the pointer changes while a frame is in flight | the frame completes with the state it resolved; the next frame has the new one | §16: a frame reads the pointer once, at its resolve; the change sets `AUTO_WAKE` under the mutex, so the next wait returns at once |
| R27 | a `present` lands during an enter window | the window is not restarted: it ends when it would have | §16: an instance is its place in the scene, and its deadline is part of its state |
| R28 | the graphics thread stops while a deadline is pending | shutdown completes at once | §16: the stop signals the condition variable the timed wait is on, and the wait checks `STOPPING` before its deadline |
| R29 | `waitFrame` is in progress when a deadline expires | `waitFrame` returns when it would have | §15/§16: an auto-only wake takes `AUTO_WAKE` and nothing else — not `PENDING`, not `WANTED`, no `FRAME_COND` broadcast |
| R30 | the program ends, or the window is closed, while the Vulkan pipeline is being built | the process ends at once; nothing waits for the builder and nothing it uses is freed | §10.1: shutdown joins only the graphics thread, which is never inside the build |
| R31 | the build fails, or its thread cannot be started | every frame is software, for the life of the process | §10.1: `READY = 2`, which `canvas::vulkanReady` answers FALSE for |
| R32 | the program leaves canvas mode while the pipeline is being built, and comes back | the build finishes meanwhile; the frames after the return are Vulkan frames | §10.1: the build depends on no mode, window or surface, and nothing stops the graphics thread at a mode change |

**R30–R32 are bug-996's.** R30 and R32 are proven in a real window on box 206 (the bug
document has the runs), because no headless run starts the builder thread (§10.1); R31
by `tests/codegen/codegen_canvas_vulkan_off_frame_path.rs`, which runs the emitted
build against a loader with no Vulkan and a thread library that refuses.

**R26–R29 are plan-172-K's**, pinned by the four `auto_…` tests at the end of
`tests/canvas/rt_canvas_graphics_thread.rs`. R26 is the row a headless test can only
half reach: the `--debug` pointer script is applied by the graphics thread at the START
of a frame, so "changes while a frame is in flight" is an entry that falls due during a
held frame, and what the test pins is that the held frame keeps the snapshot it took.
The cross-thread half — a main-thread publish landing mid-frame — is the sequence word
of §5.1 plus `AUTO_WAKE` being set under the mutex, and was watched in a real window.

**R17–R21 are plan-116-J's**, and they share one rule: **a group closes a resource only if
nothing live names it** — where "live" is the scene about to be published *plus every
group's live items*. Closing on the narrower "what the retired buffer named" is wrong in
three of these five rows, and wrong **silently**: `imageHandle`/`fontHandle` answer `0` for
a closed resource, `0` is "no such object", so the item simply stops drawing and nothing is
raised. All five are pinned by `tests/canvas/rt_canvas_group_ownership.rs`.

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
* **No time-based repaint but one.** §4 trigger 7: the render loop's wait is timed only
  while an `AutoPaint` is inside an enter or exit window, to that window's end.
  `canvas::waitFrame` (plan-155-C) waits on time and never repaints: it signals
  nothing, so a static scene under a `present` + `waitFrame` loop still costs one
  frame.
* **No cross-thread arena free.** §3.

## 10. The renderer branch, and what the Metal backend inherits

`__canvas_renderLoop` calls `__canvas_renderFrame`, which is the **only** place a
renderer is chosen. The choice is a runtime branch, not a build-time one, because
every input to it is a runtime fact:

```
IF canvas::useGpu() AND __canvas_metalReady() THEN
  IF __canvas_renderMetal() THEN RETURN
END IF
__canvas_renderScene()
```

* `canvas::useGpu` — did the program ask? Settled once, before the graphics thread
  is spawned (`__canvas_wantGpu`): a real window asks, a headless run does not, and
  `MFB_CANVAS_GPU` overrides both ways. **Software is the default for a headless run
  and must stay so**: it is the oracle the GPU backends are measured against, so it
  cannot become the thing being measured. What is settled once is the ASKING; which
  renderer draws a given frame is decided per frame by the other two conditions.
* `__canvas_metalReady` — is a pipeline built yet? Its first call starts the build
  (`canvas::metalBuild`: on a thread of its own, or on this one when `MFB_CANVAS_GPU`
  is set); every call answers `canvas::metalReady`, one load of
  `GRAPHICS_OFFSET_MTL_READY` (untried / built / failed / building, bug-1087 — §10.1's
  rule), so a host with no Metal device pays the device probe and the MSL compile once,
  off the frame path, and draws software meanwhile.
* `__canvas_renderMetal` returning FALSE — is this *scene* one the GPU shader
  reproduces? It declines rather than draws wrongly. A backend that rendered a circle
  as its bounding box would still report success, and that is the failure mode this
  third condition exists to prevent.

  It is also FALSE for a frame the GPU did not FINISH (bug-1024). `waitUntilCompleted`
  returning is not the buffer having succeeded: the renderer reads the command
  buffer's `status` after the wait, and anything but Completed leaves before the
  readback and before the present, with the submit's answer still 0 — the same answer
  and the same slot (`OFF_PRESENTED`) as "no texture" and `direct_failed`. Both
  `canvas::metalPresentScene` and `canvas::metalDrawScene` return that answer; do not
  make either ignore it again (`metalDrawScene` was `AS Nothing` until this bug, so a
  failed readback frame was shown and counted as a GPU frame).

Three consequences for anything added here later:

* **The Metal objects live in the graphics-state block**, not in the app module's own
  storage, for the reason in §2: the arena is per-thread, and a process-global word is
  the only place the builder thread can leave them and the graphics thread find them.
  After the ready word says built, only the graphics thread touches them.
  `GRAPHICS_OFFSET_MTL_*`.
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
two probes of overlapping facts can disagree and one of them did. **Unlike `metalReady` it
builds nothing** (§10.1). It has the same offscreen
shape: it renders into an image and reads it back so the frame leaves through
`canvas::blitSurface` like every other. That path needs no `VkSurfaceKHR` and no
swapchain, which is what lets it be tested headless, and on Linux it is the only path, in
a window too. **Windows has the twin of Metal's direct present** (bug-688 V6,
`runtime/canvas/vulkan/present/`, `canvas::vulkanPresentScene`; record-and-present in `vulkan/present/submit.rs`): the frame is recorded
into a swapchain image of the canvas's child window and presented, and any frame it
declines is read back instead. Box 2226 now has a GNOME Wayland session, but its Vulkan
driver, like every reachable Linux one, is Mesa's CPU lavapipe.

### 10.1 The Vulkan pipeline is never on a frame's critical path (bug-996)

`vkCreateGraphicsPipelines` compiles the fragment shader, and a real driver that has not
seen the executable NAME before takes tens of seconds over it (38 s on box 206's Intel
UHD 630; its cache is per application). `canvas::vulkanReady` used to BE that build, asked
inside the first frame, so a new program showed a blank window for the whole compile, a
`--debug` run wrote no stats line (bug-1045), and a program that ended meanwhile hung in
shutdown's join for the rest of it (45 s measured).

The rule now, and where each half lives:

* **`canvas::vulkanReady()` is one load.** It starts nothing and waits for nothing, so
  the render branch asks it every frame and the stats line asks it too.
* **`canvas::vulkanBuild(place)` is the build.** `__canvas_vulkanReady` (the render
  loop's wrapper) calls it once, the first time a frame wants the GPU: `0`, on a thread
  of its own, unless `MFB_CANVAS_GPU` is set, then `1`, on the graphics thread, to its
  end. The first is what a window gets; the second keeps a test's first frame a GPU
  frame, and is why every GPU test and box script still sees `vulkanReady=TRUE` on its
  first stats line.
* **Frames are software until the word says built**, then Vulkan. Software to GPU and
  back was already an ordinary per-frame event (a declined scene), so the window code
  needed nothing: a blit hides the swapchain's child, a direct present shows it.

**The hand-off is `GRAPHICS_OFFSET_VULKAN_READY`, and the order is the whole proof**
(`runtime/canvas/vulkan_handoff.rs`): untried 0, built 1, failed 2, building 3.

* Only the graphics thread takes it off 0, and it does so BEFORE the builder thread
  exists. One builder at most, with no compare-and-swap.
* The builder fills its object words with plain stores and writes the ready word LAST
  (a store-release on AArch64). Then it returns.
* The graphics thread reads the ready word FIRST (a load-acquire on AArch64) and an
  object word only after reading 1. It writes none of the builder's words.

Who owns which word:

| Builder thread (before `READY = 1`), graphics thread reads after | Graphics thread only, made on a GPU frame |
|---|---|
| `LIB`, `INSTANCE`, `PHYSICAL`, `DEVICE`, `QUEUE`, `QUEUE_FAMILY`, `WSI` | `IMAGE`, `IMAGE_MEMORY`, `IMAGE_VIEW`, `FRAMEBUFFER`, `READ_BUFFER`, `READ_MEMORY`, `MAPPED`, `TEX_WIDTH/HEIGHT` |
| `SET_LAYOUT`, `DESC_POOL`, `DESC_SET`, `EDGE_*`, `ITEM_*` | `COMMAND_POOL`, `COMMAND_BUFFER` |
| `PIPELINE_LAYOUT`, `RENDER_PASS`, `PIPELINE`, `PIPELINE_MODES` | `SURFACE`, `SURFACE_HWND`, `SWAPCHAIN`, `SC_*`, `SEM_*`, `PRESENT_PASS`, `LOAD_PASS`, `PRESENT_LOAD_PASS` |

No main-thread code touches any `GRAPHICS_OFFSET_VULKAN_*` word
(`grep -rn GRAPHICS_OFFSET_VULKAN src/target` finds nothing).

What to remember when touching it:

* **The builder has no arena, on purpose** (§2's corollary). Its entry pins nothing into
  the arena-state register; the build uses its own stack, the process-global state and
  the loader. Anything added to `emit_vulkan_build` that allocates from the arena
  allocates from whatever the thread library left in that register.
* **Its entry owes the thread library every register MFB code may change**
  (`emit_thread_entry`, shared with the graphics entry). The build is MFB-convention
  code: on Win64 it writes `rdi`/`rsi`/`rbx`/`r12` freely.
* **Nobody joins it and nobody stops it.** It is detached at birth
  (`pthread_detach`, `CloseHandle`). Shutdown joins the graphics thread, which is not
  inside the build (with `MFB_CANVAS_GPU` set it is, and an exit then waits as it always
  did), and the process then ends with the builder wherever it is: nothing
  it uses is freed (static state, its own stack, the driver's memory), and no Vulkan
  object is destroyed at exit, built or not. Do not add a device teardown to shutdown
  without first deciding what it does while the word is 3.
* **A new reader of a builder word must be behind the ready word.** Reading `DEVICE`
  "to see if there is one" is reading a half-built pipeline.
* **The swapchain is not part of the build and stays on the graphics thread.** It needs
  the window and the client size current at that frame, it is torn down and rebuilt on
  every resize, and it compiles nothing: on box 206 the first GPU frame of a window,
  which creates the render target, the surface and the swapchain, took 24 to 29 ms
  against 2 to 3 ms for the frames around it.
* **Metal has the same shape, and since bug-1087 the same rule.** macOS keeps the
  compiled pipeline per application: 10 to 14 s on the first launch of an app name never
  run before on this Mac, 24 to 83 ms on the second (`tools/canvas-bench/bench.sh <mfb>
  coldstart`). `canvas::metalReady()` is one load-acquire of `GRAPHICS_OFFSET_MTL_READY`
  (the same four states); `canvas::metalBuild(place)` is `_mfb_macapp_metal_init`, which
  claims the word and starts `_mfb_macapp_metal_builder` (detached, the graphics
  thread's 8 MiB attribute object, its own autorelease pool, the whole callee-saved bank
  saved) or, with `MFB_CANVAS_GPU` set, builds on the graphics thread. The builder
  writes device, queue, pipelines and the frame buffer with plain stores and the word
  last with a store-release; `_mfb_macapp_metal_layer_apply` reads `MTL_DEVICE` only
  from a GPU frame, so only after built. The ncode test is
  `tests/codegen/codegen_canvas_metal_off_frame_path.rs`; the window row of `coldstart`
  shows the software-then-Metal switch (a headless run cannot reach it, as below).
* **A headless run cannot reach the on-a-thread build.** Headless is software unless
  `MFB_CANVAS_GPU` is set, and a set variable builds on the graphics thread. The
  software-then-GPU switch is a real window's to prove (box 206, `schtasks /it`); the
  cargo suites cover the seam by running the emitted members in the `-ncode` interpreter
  (`tests/codegen/codegen_canvas_vulkan_off_frame_path.rs`).

### A group is one instanced draw per node, with an offset bound to BOTH stages

Since plan-116-H a `canvas::Group` is not flattened away before the GPU sees it. The
worker builds a **draw list** (`__canvas_sceneDraws`) alongside the block list, one entry
per contiguous run of blocks at a given offset:

```
(itemBase, itemCount, dx, dy, blendMode, dxLo, dyLo,        sixteen 64-bit words
 hasMatrix, ia, ib, ic, id, pass, 0, 0, 0)
```

It was eight words until plan-172-G, and a translate-only entry is still those eight
followed by zeros. `hasMatrix` is 1 when the entry's groups compose to more than a
translation, and `ia`..`id` are then the INVERSE linear part of that composition, as
float32 bit patterns. `pass` is 0 in every entry the frame's layout writes; only a
pass's own list sets it, in its first entry (bug-765, below).

**The per-draw payload is 32 bytes, bound to both stages** (plan-172-H): the four offset
words, then `ia`..`id` — `MfbDraw` in the MSL, `uniform Draw { ivec4 offset; ivec4
inverse; }` in the GLSL. `hasMatrix` itself is not sent: an inverse is never all zero, so
all-zero bits mean "no matrix" and the shaders take the branch they always took, which is
what keeps a translate-only frame byte-identical on each backend. With a matrix the
vertex stage carries the item's box through the FORWARD linear part (it inverts the 2×2
itself), and the fragment stage puts the query point through the inverse before anything
else reads it.

**The box a group's matrix carries is not the header's as it stands (bug-1083).** The
header's box (slots 16-19) ends in a rim sized for the item drawn with no group, and the
record says how much: slots 67 and 68 (`__CANVAS_GEO_RIM`, `HEADER_RIM`), the x sides'
and the y sides', zero for a box that is not an extent and a rim (a glyph run, a text
box, a picture, an arc). Three sites carry a box through a group's matrix,
`helper_items/geo_read.rs:__canvas_drawBound`, `mfb_canvas.vert` and Metal's `mfbVertex`, and all
three replace the rim by itself times the Frobenius norm `N` of the groups' inverse:
lower `(bound + rim) - rim * N`, upper `(bound - rim) + rim * N`. The derivation is in
`__canvas_drawBound`'s comment. Keep the three in step, and note what they do NOT share:
the software site pads the mapped HULL by one pixel, and the vertex stages grow the box
by `N` before the map, because their quad is the parallelogram and has no hull to pad.
With rim 0 each site is exactly what it was before the bug, which is what keeps text,
pictures and arcs inside a group byte-identical on each device.

The rim reaches the vertex stage in two words of the item block that nothing else
wrote, the fourth words of `xform1` and `gradInv1` (`ITEM_RIM_WORDS`), float32 bits. That is
why this did not grow `ITEM_BLOCK_SIZE` and the fragment stage's SPIR-V did not change:
do not hand those two words to anything else.

`dx`/`dy` are float32 bit patterns (`__canvas_float32Bits`), entry words 2-3; words 5-6
held their 16.16 residuals until the shaders went to fp32 and are now reserved zeros. Both
backends still bind four words as the stage offset (`int4`/`ivec4`, 16 bytes), the last
two zero, so the push-constant range and `MfbDraw` did not change size.

The matrix is bound to the fragment stage as well as the vertex stage for the reason
the offset is, below: bound to the vertex stage alone, a turned group has the right
outline and the wrong contents.

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

**The clip is no exception (since plan-163-C).** `Paint.clip` is in the item's own
coordinates, so it is evaluated at the same shape-space point as the distance, on every
backend: it comes out of `shapeCoverage` (`.w`) beside the shape's coverage. Until
plan-163-C it was a surface window evaluated at the un-shifted point, and
`the_metal_fragment_clips_and_samples_in_shape_space` now pins that no consumer but `p`'s
own definition reads `in.pos.xy` — evaluating the clip there again leaves it behind when
the group moves.

Where the payload is bound differs per backend and per stage, so it is three numbers,
not one: Vulkan uses a single 32-byte push-constant range both stages declare
(`PUSH_OFFSET_BYTES`, which the pipeline layout's range, the `vkCmdPushConstants` size
and the staging slot all read); Metal uses `setVertexBytes:` at buffer index **1** and
`setFragmentBytes:` at index **3**, 32 bytes each, because its fragment stage already has
items, edges and the glyph bitmap at 0..2.

**A gate cannot decline a group scene by looking for a `Group` item.** By the time
`__canvas_vulkanRenderable` or `__canvas_metalRenderable` sees the block list, the walk
has expanded every group away — a search finds nothing, the frame is accepted, and every
group's child draws at the origin. plan-116-G solved that with a flag the *walk* set; with
both backends taught the flag has no reader and was deleted, so a future backend that
needs to decline has to set one again rather than search.

**Count published records, not items, against the item region.** `__canvas_blockInstances`
is the one answer to "how many instances does this block become", and both the draw list
and a block's cost (`__canvas_blockCost`) ask it. An item that both strokes and fills
under a non-Normal blend mode publishes **two** records; counting it as one under-fills
the region, which is the direction that lets a pass write past the mapping. The emitters' own split test reads `strokeHalf`
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

**A glyph run is one instanced draw, N instances.** Each glyph publishes its own block,
so a `Text` item is N contiguous blocks and one draw-list entry draws them all
(`emit_draw_list_pass` on both backends). Its coverage bitmap is not a per-draw payload:
both emitters copy it into the frame buffer's glyph region and the glyph's block names
its slice (`emit_glyph_publish`, bug-670 on Metal). It was a per-glyph
`setFragmentBytes:` while each glyph was its own draw; under one instanced draw every
glyph read whichever bitmap was bound last.

A consequence worth knowing before adding anything per-item: **an instanced run cannot
rebind a per-item side payload between its instances.** Anything that varies per item
must therefore be a region of a frame buffer reached by an index carried in the block,
not a payload. That is what forced Metal's polygon edges to move (below), and it is the
shape any future per-item payload has to take.

**The region sizes are one PASS's, not a frame's (bug-765).** Each backend has its own
constants — `METAL_MAX_FRAME_*` and `VULKAN_MAX_FRAME_*` — generated into the MFBASIC
source (`RENDER_CAPS`), and since bug-688 they are the same numbers: 65,536 quads,
262,144 polygon edges, 131,072 gradient stops, 1,048,576 dash words (plan-163-B) and 8M
glyph/picture texels, each a SUM over the blocks of one pass, with each distinct picture
block counted once a pass. A stroke's join records ride the EDGE region, two edge slots
each, so they count twice against it rather than having a region of their own; so do a
curve shape's PIECES (below). Neither backend has a per-polygon edge
cap: a polygon's edges are indexed by horizontal band (`emit_band_index`, bug-686 on
Metal, ported to Vulkan's `CodeBuilder` by bug-688 — the algorithm exists twice so that
Metal's instructions did not change), and the shader's `edgeDistance` walks only the
edges within reach of the pixel's band, which is bit-identical to walking them all.
bug-688 measured that on Vulkan too: the same scenes rendered with and without the index
gave byte-identical GPU frames on lavapipe, stars and spikes included.

Until bug-765 those sums were FRAME caps and a frame past one was declined to software:
100,000 dots were a software frame for good. The owner ruled that no scene size is a
limit. What is left to decline is a block that does not fit a region BY ITSELF — one
polygon, one gradient, one image, one `Text` larger than the region. Software is the
oracle, so a declined frame is at least as correct, and truncating instead would draw a
*different shape*.

### A frame that does not fit the frame buffer is drawn in passes (bug-765)

`helper_render/gpu_passes.rs`. `__canvas_metalRenderable` / `__canvas_vulkanRenderable` answer 0
(a block fits no pass: software), 1 (the frame fits one buffer) or 2 (it does not).
For 1 the renderer is handed `__CANVAS_DRAW_BLOCKS` and `__CANVAS_DRAWS` exactly as
before. For 2 `__canvas_passNext` lays out one pass at a time into
`__CANVAS_PASS_BLOCKS` / `__CANVAS_PASS_DRAWS`, the same two layouts, and the same
member is called once per pass. What to keep true:

* **One definition of a block's cost.** `__canvas_blockCost` is what the frame sum
  (`__canvas_framePasses`) and the pass walk both add up. They were two predicates'
  own loops; a count that differs between the sum and an emitter writes past a region.
* **A pass is a run of DRAW ENTRIES, cut between two BLOCKS.** The entries are the
  frame in paint order, so a pass must be a prefix of what is left of them; the block
  list is in first-recorded order, and a group drawn again later names blocks recorded
  long before. Walking the block list instead draws a group's second use under the
  items that should be beneath it. The cut is between blocks and never inside one, so
  the two records of a split blended item and the glyphs of a `Text` stay together
  without the walk asking what kind a block is.
* **Nothing else has to stay together.** An entry is one instanced draw at one offset
  under one pipeline; cut in two it is two entries with the same words 2 to 11, and N
  instances composite as their two halves do, in order, into the same target. That is
  why translucent items composite across a pass boundary exactly as inside a pass.
* **Every table an emitter keeps while filling the buffer is a pass's**, with no code:
  each pass is one call of the renderer, which zeroes its cursors, its band region and
  its picture-table count at entry. So the walk's own bookkeeping is per pass too: a
  picture block is charged once a pass, and an entry whose blocks this pass already
  holds (the same group again) is drawn from them, while one whose blocks an EARLIER
  pass held is laid out again.
* **The pass word, `CANVAS_DRAW_PASS_WORD`** (word 12 of the list's first entry), is all
  an emitter knows of passes: `CONTINUES` (load the target, do not clear it; on a
  swapchain, take the image the first pass acquired, `GRAPHICS_OFFSET_VULKAN_SC_HELD`)
  and `MORE` (finish the pass, but neither present nor read back). It rides the list
  because the seam's argument bank is full, and it is 0 in every list the frame layout
  writes, so a frame of one pass runs the instructions it ran before.
* **Each pass is submitted and waited for before the next is laid out.** The frame
  buffer is single-buffered and a command buffer reads it when it EXECUTES, so two
  passes recorded into one command buffer would both draw the last one's blocks.
* **Nothing but the last pass touches what the window shows.** Metal: every pass
  renders into the offscreen target; the drawable is taken, copied into and presented
  by the last (`emit_branch_if_more`). Vulkan in a window: the first pass acquires the
  image and every pass records into it, but it reaches the screen only through the
  last pass's `vkQueuePresentKHR`; the first pass alone waits on the acquire semaphore
  and the last alone signals the render one.
* **A failed pass fails the frame** (bug-1024's rule, per pass). FALSE from a direct
  present at any pass sends the WHOLE frame down the readback path from its first
  pass; FALSE from a Metal readback pass makes it a software frame. Nothing is read
  back or presented from a target holding some of a frame's passes. On Vulkan a
  failing pass that holds the swapchain image gives it up (`emit_present_abandon`
  clears `SC_HELD`, and the swapchain is rebuilt before the next frame).
* **The later passes' render passes are made lazily on the graphics thread**
  (`LOAD_PASS`, `PRESENT_LOAD_PASS`: `loadOp = LOAD`, entered and left in the layout the
  first pass leaves the image in). They differ from the pass the pipelines were built
  against in load operation and layouts only, so no pipeline is rebuilt (bug-996).
* **The stats line describes the frame.** `blocks=` and `draws=` are the frame's lists
  and `gpuFrames` counts the frame once, however many passes drew it.

The reason a frame buffer needs a per-item *index* at all is unchanged and worth
restating: a command buffer is recorded once and executed once, so rewriting — or
re-binding — one buffer per item would give every item the *last* one's data.

**A curve shape's pieces ride the edge region; there is no curve region (plan-164-C).**
A `PolyCurve` (kind 11) or `CurvePolygon` (kind 12) record's tail is y-monotone
rational-quadratic pieces, seven floats each. Both emitters (`emit_edge_buffer`,
`emit_edge_upload`) lay them down as TWO runs of `count` slots at the item's edge base:
every piece's chord `x0, y0, x2, y2` first, then every piece's `x1, y1, bits(w1), 0`.
The split is what made the feature cheap, and it is the thing to keep:

- A chord is an edge to `emit_band_index` — a y-monotone piece's ends are its whole
  y-range — so the band index, the item block and every region base are unchanged. Do
  not interleave the two slots of a piece: the band index would then read control
  points as edges.
- A block's cost counts `2 * pieces` against the edge region. A count of one a piece
  puts more in a pass than the emitter can fit; `rt_canvas_metal.rs` pins the factor
  with a 512-ring frame that is one pass and a 513-ring frame that is two, both of which
  must match the oracle.
- The shader text is ONE template, `shaders/curves.tpl`, expanded to MSL at run time and
  into `mfb_canvas.frag` by `tools/canvas-curve-shader/generate.py`. Edit the template,
  run the tool, then `scripts/regen-spirv.sh`; a unit test fails on a stale GLSL copy,
  but nothing fails on a stale `.spv` except the Vulkan lanes.
- A piece is evaluated in fp32 on coordinates taken relative to the pixel first
  (`curvePiece`), and judged under `GPU_DEFAULT` like every other row. The 30,000x
  `curve_residuals_on_black` row is what failed (139 levels) while positions travelled
  as 16.16 and the shaders ignored the residuals: a 16.16 step is half a pixel there.
  That is why positions are float32, not fixed point.

**Two edges that share a vertex must READ one value for it.** The double-float shaders'
`dfCrossing` used to rebuild an edge's end as `ay + vy`, and double-float addition does
not always give back `by`: with a vertex exactly on a pixel-centre row the two edges disagreed about which
side of the row it was on, both counted, and that whole row of the polygon came out
inverted. It predates the curve shapes (it reproduces with plain polygons on the commit
before them) and was invisible until a picture was clipped to a flattened
curve, whose points are dyadic (`t = 3/4` of a cubic with whole-number control points
lands on a half-pixel). `DfEdge` now carries `by`, read from the buffer; anything new
that tests a row against an edge's end uses it. `scenes/vertex_on_a_pixel_row.mfb`.

## 11. Test affordances

A set of environment variables. A normal build reads only `MFB_HEADLESS` and
`MFB_CANVAS_GPU`; every other variable below is read by a `--debug` build only —
the name and the reader are both absent from a normal build's binary, so a test that
sets one builds with `--debug`. The canonical list is `mfb spec tooling debug-report`.
The canvas ones are `RegistryHelper::debug_split` pairs (`TEST_SWITCHES` /
`TEST_SWITCHES_DEBUG` in `helper_render/render_loop.rs`, `DAMAGE_SWITCH` / `DAMAGE_SWITCH_DEBUG` in
`helper_damage.rs`, `GLYPH_BUDGET` / `GLYPH_BUDGET_DEBUG` in `helper_glyph_cache.rs`).

* `MFB_CANVAS_AUTO_WINDOW_MS` — **a `--debug` build only**: the length of an
  `AutoPaint`'s enter and exit windows, in place of the project's
  `options.canvasAutoPaintMS` (1 to 3,600,000). A 100 ms
  flash is over before a `--debug` software frame can be compared, so a test that has
  to photograph one stretches it; a test that proves the window ENDS uses the real one
  and reads the sequence off the stats lines' `auto=` field — the paint each instance
  of the frame shows, one digit each: 0 own, 1 enter, 2 exit, 3 hover, 4 pressed.

* `MFB_CANVAS_POINTER_X` / `MFB_CANVAS_POINTER_Y` — **a `--debug` build only**; GTK and Windows, headless: after the
  first completed frame, call the platform's own pointer callback once with that
  position. It runs AFTER the scripted resize, and that order matters: "inside" is
  asked of the published surface size, and a headless run has none (0 by 0, everything
  outside) until `MFB_CANVAS_RESIZE_W`/`_H` publish one. A value of 0 or less is "unset"
  on Windows (`emit_parse_wide_env`). macOS has no headless path.
  `scripts/test-canvas-pointer.sh` drives it on the boxes.

* `MFB_CANVAS_POINTER` — **a `--debug` build only**: `ms,x,y,buttons,inside;…`, a
  script for the pointer words (§5.1). At the start of each frame the graphics thread
  publishes every entry whose time has come, counted from the FIRST frame, through
  the writer the platforms' handlers call. It is what puts the pointer somewhere on a
  headless run, on every platform. Two traps: the clock starts at the first frame, not
  at launch; and a `--debug` software frame is slow, so leave seconds, not hundreds of
  milliseconds, between an entry and the frame that must not see it yet
  (`rt_canvas_pointer`'s first cut used 100 ms margins and saw an entry a frame
  early). It proves the words, the writer and the reader — NOT that a platform's real
  handler calls the writer; that is a real window's to prove
  (`scripts/test-macapp.sh` case 6c, `tools/pointer-probe`).

  **Never time anything from the FIRST frame.** With a dozen headless programs starting
  at once its render phase has been seen to take 7 s (`phaseRenderMs=7008` for one
  rectangle, in `rt_canvas_graphics_thread`), and 6 s beside one `cargo build`; alone it
  is tens of milliseconds. An `AutoPaint` window that starts on that frame is over
  before the first `present` returns. Put the pointer's arrival at script time **1**,
  not 0: it then lands on the frame after the first, which is when a synchronous
  `present` returns, and the program's clock and the window's start together
  (plan-172-K's R27 and R29, whose first cuts both timed from frame one and failed).
  And with an `AutoPaint` live the render loop wakes for each script entry by itself
  (`__canvas_pointerNextDue`), so a script needs no `present` to be applied.

* `MFB_CANVAS_RESIZE_W` / `MFB_CANVAS_RESIZE_H` — **a `--debug` build only**; in a headless run, wait for the
  first completed frame and then resize the surface to these dimensions, by calling
  the same handler the platform's resize signal calls. A resize is a *window* event,
  and until box 2226's GNOME Wayland session no reachable Linux box had a display
  server, so without this the handshake could be implemented and never executed; it is
  still the only scripted resize a test can drive. Waiting for a frame first is the whole
  point: resizing before one exists builds the render target once at the new size and
  proves nothing, where resizing after one forces the tear-down-and-rebuild.

  Two variables rather than one `WxH` string because parsing a separator in
  hand-written assembly buys nothing over `atoi`.

  `term::` has the Windows twin, `MFB_WINAPP_TERM_RESIZE_W`/`_H` (bug-540, also
  `--debug` only): client
  pixels handed to the `WM_SIZE` handler once `term::on` has built the surface —
  see `mfb spec app term-backend`, "Windows: GDI memDC". Note that the program under test has
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
  worker's own arena, which is per-thread (§2). The dump then lands with **every shape
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
  `phaseRenderMs=` (bug-686 Phase 0; `__canvas_phaseMark` is an empty `FUNC … AS Nothing` in a normal
  build). After them come the Metal renderer's own split of `phaseRenderMs`, cumulative
  too: `profAcquireUs=` / `profFillUs=` / `profEncodeUs=` / `profWaitUs=` / `profGpuUs=` /
  `profReadbackUs=` / `profPresentUs=` in microseconds, and the counters `profPasses=` /
  `profBytes=` / `profTexels=` / `profDraws=` (`mfb spec tooling debug-report`, "The canvas
  frame profile"). They are `emit_metal_draw`'s, so they are 0 for a software frame and on
  every Vulkan target. A rendered frame's line is written BEFORE its `phaseRenderMs` is
  added, so a line's `prof…` fields include its own frame and its `phaseRenderMs` does not.
  `tools/canvas-bench/` reads the phase timers. **A `--debug` build is ~35% slower on this
  path** (116 vs 157 frames/5 s on 1,000 moving polygons), and much more on
  allocation-heavy code, so judge frame rates on a release build. Appends rather than overwrites because the interesting quantity is
  the delta between frames. It is also the **only** window onto either cache: both live
  in globals owned by the graphics thread, so a program asking from `main` asks the
  worker, whose copies are its own and always empty (§1).
* `MFB_CANVAS_GLYPH_BUDGET` — **a `--debug` build only**; a normal build's budget is
  always 1 MiB. Shrink the glyph coverage cache's byte budget (default
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

**"Changed" is against the size the program SEES, not against the stored words.** The
two words start at 0 and a zero word reads as the startup size (`SurfaceDimension`,
900 by 640 — what `canvas::getSize` answers before any publish). The publisher reads the
old size through that same reading, so a window that opens at 900 by 640 publishes no
change, and one that opens at another size (Windows' first client area) publishes one.
Comparing against the raw words made every real window answer `didResize` TRUE once at
startup (bug-970); a headless run never showed it, because nothing publishes there
unless a resize is scripted. Do not add a second reading of a size word, and do not
special-case "the first publish".

Two words with one writer each, so no lock: the main thread only ever writes the
counter, the worker only ever writes the acknowledgement. A single read-and-clear flag
would need one, because a resize landing between a reader's load and its store would be
lost — on the one path whose entire job is to report edges.

* `MFB_CANVAS_DAMAGE` — **a `--debug` build only**; repaint only what changed: keep the previous frame's pixels,
  clear the union of the changed items' bounds, and redraw only the items that meet it,
  each through that rectangle as its scissor (`__CANVAS_SCISSOR_*`, bug-1089): an item
  drawn over its whole box blends its edge twice wherever the box leaves the cleared area.
  Off by default, and always off in a normal build. It changes no pixels — that is what `tests/canvas/rt_canvas_damage.rs`
  asserts, byte for byte — but it does change *when* the renderer runs at all, and a
  frame counter that silently stops advancing is the kind of thing a stale test reads as
  a pass.

  Two notes for anyone testing it. An unchanged scene never reaches the renderer in the
  first place: `canvas::publishScene` refuses it and `present` does not signal a redraw
  (§3.1's frame skip), so the *empty* damage union only fires on a platform wake — a resize
  or an OS damage repaint. And the GPU backends always render full-frame: they draw into
  their own texture and read it back, so there is no kept surface for them to preserve.
* `MFB_CANVAS_SYNC` — **read by every build** (so is `MFB_MOUSE_INJECT`; `tools/canvas-bench`
  measures release builds with both); make `present` wait for the frame it asked for. Frames coalesce
  by design (§3), so frame counts are otherwise a scheduling detail — the same
  three-present program was observed producing one, two and three frames. Any
  frame-level assertion needs this. **Any non-empty value enables it** —
  `MFB_CANVAS_SYNC=0` is ON (`__canvas_ensureGraphics` tests the length), which is how a
  "pipelined" measurement in bug-686 silently ran serialized. Leave it unset to pipeline.

* `MFB_CANVAS_DISPLAY_HZ` — **a `--debug` build only**; pin the display refresh rate the runtime reports
  (plan-155-A), in whole hertz. Stored ×1000 into `GRAPHICS_OFFSET_DISPLAY_OVERRIDE`
  by `__canvas_ensureGraphics`, and it wins over the rate the main thread publishes,
  which it never overwrites (a separate word). It exists because a headless run has no
  window and so no monitor to ask; the stats line's `displayHz=` shows what the runtime
  would use.

* `MFB_CANVAS_VSYNC` — **a `--debug` build only**, macOS windowed runs: `0` makes the
  main-thread layer apply send `setDisplaySyncEnabled:NO` when it first configures the
  `CAMetalLayer`, and set the profile's `profVsyncOff=` word (`emit_vsync_switch`). It
  does NOT remove the refresh from a windowed frame: the compositor still paces the
  drawables, and the wait moves out of `profAcquireUs=` into `profGpuUs=`/`profWaitUs=`
  (acquire 7.4 → 0.05 ms, GPU 0.23 → 8.0 ms, frame 8 → 16 ms on one rectangle at
  120 Hz). With sync ON the refresh wait is all in acquire and the other `prof…` times
  are clean, which is why `benchmark/canvas` runs with it on by default.

A fourth selects the renderer rather than observing it:

* `MFB_CANVAS_GPU` — ask for the GPU backend, or (`0`) refuse it (§10). The stats
  line reports the branch's discriminants — `metal=`, `gpuSelected=`, `metalReady=`
  and `vulkanReady=` — which is how a test tells "the GPU agreed with the oracle" from
  "there was no GPU and both runs were the oracle".

  **`vulkanReady=` is "built by now", not "can be built"** (bug-996). The stats line
  starts no build and waits for none, so the field is FALSE on every line of a software
  run, FALSE while a window's pipeline is still being built, and TRUE from the first
  line of a run with `MFB_CANVAS_GPU` set (which builds before its first frame). Until
  bug-996 the stats line's own question built the device, so a software `--debug` run
  on a new executable name paid the whole compile; it no longer touches Vulkan at all.
  `metalReady=` still builds on being asked.

And a fifth is not about the renderer at all, but is what makes any of this
observable on Linux:

* `MFB_HEADLESS` — run the app without a window; read in every build, by the macOS,
  GTK and Windows bootstraps alike. On Linux it means running without GTK, and it is
  structurally different from the macOS path: macOS builds its window and merely skips
  showing it, keeping the AppKit run loop, whereas `gtk_init` fails outright with "Failed to open display", so
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
`__canvas_sceneOffsets` has built the draw list. Unset or `0` is off, and it is a
`--debug` build only like the others.

**It exists because nothing else here produces "graphics mid-frame."** The two proven
mid-render rows (R5, R7) reach it through `MFB_CANVAS_RESIZE_W`/`_H` firing while the
worker sits in `os::sleep`, which is specific to resize — so a row needing any *other*
worker action mid-frame was either untestable (R1 sat marked "not yet reachable" for
three letters) or tested by luck, and a green run tested by luck reads exactly like one
tested by construction.

The hold point is after the draw list is built, deliberately: by then every group name
is resolved and every group's items block read, so a `removeGroup` arriving during the hold
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

Growing is still possible without a lock, if nothing moves. The font table
(`gen_font_table.rs`) is a **chain of fixed chunks** (bug-756): the worker appends an
arena chunk only when every slot is taken, fills it, and links it in with one
store-release of the previous chunk's `next` word, and a reader follows `next` with a
load-acquire. A chunk is never freed, and arena memory stays mapped (§2), so a chunk
address a reader holds never goes stale. It was sixteen fixed slots, and a seventeenth
live font silently drew nothing — a cap that fails **silently** is the case to chain
rather than to size.

**A slot is published name-LAST and dropped name-FIRST.** `name` is the discriminator: a
scanning graphics thread treats a non-zero name as "this slot is real, follow its
pointers". So a slot must never be visible under a name before its `items` and `count`
are written, and dropping one must hide it before anything else changes. This is the same
publish-then-flag rule §3 gives for the scene revision.

### There is no refcount, and there is nothing to count

`canvas::groupItems` returns a **copy**, so a published scene never holds a pointer into
a group's buffer and a parent group never holds one into its child's — a `canvas::Group`
node carries a *name*, and the renderer resolves it per frame. The only window in which
anything reads the block is on the graphics thread, inside one frame. Since plan-175-A
Phase 5 the frame's walk reads the block **in place** rather than copying it per node
(`canvas::groupItemsBlock` once per node, after `canvas::groupRevision`, then
`canvas::groupBlockItem` per child, `func_group_block.rs`): it holds the block's address
for the frame, which the drain gate below makes as safe as holding the copy. Taking the
address once per node, not re-reading the slot per child, is what keeps a `setGroup`
landing mid-walk from giving one node the old list's head and the new list's tail. The
walk's per-slot table of the children's identities (`__canvas_groupHashes`) is keyed by
the revision AND the block, and needs no clearing when a buffer is freed: the revision
only grows.

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
contains only leaf items, each carrying its accumulated `(dx, dy)` — and, since
plan-172-G, a matrix number (`__CANVAS_DRAW_MAT`) — in parallel globals.
No consumer knows groups exist.

Two consequences worth stating because they are easy to get wrong in the other order:

* **The GPU predicates cannot look for a `Group`** — by the time they see the list there
  is none. A backend that must decline is told by the walk: `__CANVAS_DRAW_HAS_GROUP`
  did that until both backends drew groups, and `__CANVAS_DRAW_HAS_MATRIX` did it for a
  group with a transform between plan-172-G and plan-172-H. Neither exists now; a future
  backend that needs to decline has to set a flag again rather than search. A predicate
  that searched would find nothing, accept the frame, and draw every group's children
  untransformed: §10's failure exactly.
* **Both sides of the damage diff need the offset.** The remembered bounds and the
  current bounds must both be translated, and each draw entry's recorded hash must fold
  in its offset — otherwise a moved group changes no geometry and reports "nothing
  changed".

## 14. The geometry cache and item hashes (bug-686)

The graphics thread's geometry cache (`helper_geometry/`, kept by the inline native
members of `func_geo_cache/`) is keyed by the item's GEOMETRY identity `G` (plan-175-A,
below) through `__CANVAS_GEO_TABLE` (an open-addressing table of `(hash, slot)` over
`__CANVAS_GEO_SLOTS`), and each slot also remembers the damage identity `D` its record was
last written for (`__CANVAS_GEO_DRAWS`, one word a slot beside the four-word slots).
Resolving an item: `G` found holding its `D` is a hit; `G` found holding another `D`, and
no entry of this frame using the slot, is a REFRESH — the record's appearance slots
rewritten in place (`emit_refresh_record`; the MFBASIC walk rebuilds the record into the
same floats, `canvas::geoRefresh`), no allocation, no insert, `refreshes=` + 1; `G` found
holding another `D` with the slot already used this frame (two equal shapes, two paints)
is resolved under `D` instead; `G` not found is built and inserted under `G`. **A hit is
trusted on the key alone** — nothing is rebuilt to confirm it — except for a `Picture`, whose pixel block
`setBytes` swaps without the item changing (`__canvas_pictureCachedOf`). That is only
sound because the hash is 62 bits (two independent 31-bit lanes, `__canvas_hashStep`) and
resolves a float to 2^-46 px (`__canvas_hashFloat`); the old single 31-bit lane collided
at ~0.3 per frame at 10,000 items.

Lifetime rules, and why:

* **Nothing is evicted inside a frame.** Slots are stamped with the frame that used them;
  `canvas::geoBeginFrame`, at the one point no offset is live (the top of
  `__canvas_sceneOffsets`), drops what the previous frame did not use once the stale
  floats reach the live ones, moving kept floats down in place and keeping the arena's
  capacity. So every
  offset a frame resolves stays valid until that frame is drawn — the old 256-slot LRU
  needed `__CANVAS_GEO_LIVE` to protect offsets it had evicted mid-frame, and that list
  is gone.
* **Glyph eviction pins only what the frame in progress uses** — slots stamped this
  frame. The cache can hold the previous scene's text too, and pinning it would pin
  everything; an unused TEXT slot is forgotten instead (`canvas::geoForget`: out of the
  index for good, its glyph indices being about to be renumbered), and rebuilt if it is
  named again.
* **One resolve per frame, over one snapshot.** `canvas::sceneResolve` resolves every
  index of the frame's snapshot (§3) by its frame hash and builds the natively built
  kinds' misses in place; MFBASIC sees only what it leaves (pictures, text, groups,
  declined kinds), and `canvas::sceneLayout` / `canvas::sceneDrawsFlat` lay the draw list
  out from the offsets — items AND layers (the draw list used to walk items only, and a
  `presentLayers` scene drew nothing on Metal).
* **A picture is never resolved on its hash alone** — `sceneResolve` leaves every picture
  hit to the MFBASIC walk, whose `__canvas_pictureCachedOf` confirms the record against
  the image's current pixel block (§17).

* **An item has two identities, and a damage identity mixed from them** (plan-175-A).
  `G`, the GEOMETRY identity, folds what decides the shape of the item's record: its
  own fields, its transform, and the parts of its paint that are geometry — the stroke
  width, cap, join, miter limit, dashes, clip, and three predicates of appearance
  fields (whether the stroke's alpha is zero, the gradient's stop count, whether the
  ramp paints the stroke). `A`, the APPEARANCE identity, folds the rest: colours,
  blend, the ramp, the transparency. `D = mix(G, A)` changes exactly when either does
  and is what the damage diff and every draw hash compare. No identity folds an `id`.
  The classification is one table (`func_item_lanes.rs`), read by the one native walk
  (`emit_item_hash`, both lanes in one pass) and mirrored by the MFBASIC walk for the
  kinds it hashes (`__canvas_hashPaintGeometry` / `__canvas_hashPaintAppearance`). The
  worker publishes `2n` words for `n` items — every `D`, then every `G` — under the one
  hashes revision, so the sequence lock covers both, and every reader of "the item's
  hash" at index `i` reads `D`.
* **Collisions, for both walks and both lanes.** Each identity is 62 bits: the native
  walk's multiply-xorshift keeps 62 bits of a 64-bit state, and the MFBASIC walk's two
  31-bit lanes (`__canvas_hashStep`) are 62. A false hit on a key needs two different
  items with one key among the k keys alive in the cache: about k² / 2^63, 1.1e-9 at
  k = 100,000. The cache key is trusted alone; keyed by `G` (plan-175-A Phase 2), a
  paint change mints no new key, so the exposure falls to geometry changes. `A` is never trusted alone — it
  is only ever compared with the `A` of the record `G` already chose, so a wrong
  "equal" needs one item's old and new appearance to collide (2^-62 a changed item), and
  costs that item its last paint. `D` decides damage only: a wrong "unchanged" is 2^-62
  a changed entry and shows only under `MFB_CANVAS_DAMAGE`.
* **A record may be in the arena without a slot** (plan-150-B). A particle that is
  neither turned nor scaled is built by `canvas::geoBuild` and appended straight to
  `__CANVAS_GEO_DATA` — no lookup, no slot, no index entry
  (`helper_particles/expand.rs:__canvas_particleRecord`) — because a particle's hash is its own
  and new every frame, so the cache only ever missed on it. Nothing new protects that
  offset: it is the first rule above. `canvas::geoBeginFrame` computes stale as "every
  float no slot used", so an unslotted record is stale at the next boundary and is
  dropped by the pass that keeps the used slots. Two things follow for whoever touches
  the arena next: **a float is not necessarily owned by a slot**, so nothing may walk
  the arena by slots and assume it saw every float; and `entries=` on the stats line
  undercounts a particle scene's records while `floats=` does not.

The worker computes the hashes in `canvas::present`: `canvas::carriedHashes` (native,
taken BEFORE `publishScene` replaces the installed scene) carries the installed hash of
every item whose payload bytes are unchanged, and only the rest are hashed. Byte equality
only ever says "different" too often — two identically BUILT items are not byte-equal
(list headroom, padding), so a rebuilt scene is hashed, never mis-carried. `Text` and
`Picture` are never carried: their hash folds in a resource id that answers 0 once
closed, with the item's bytes unchanged.

### The worker has a geometry cache too, once a program polls the mouse (plan-172-I)

`canvas::pollMouse` answers `MouseEvent.overID` **on the worker**, from the worker's own
copy of the scene (`canvas::installedItems()` / `installedLayers()`), into the worker's
own thread-local geometry cache — the same `__CANVAS_GEO_*` globals, a different thread's
instance of them (§2). `helper_hit.rs:__canvas_hitBuild` is that cache's only writer and
calls `canvas::geoBeginFrame` at its own "no offset is live" point. Nothing crosses
threads and nothing waits on a frame.

Three things follow, and each was a design choice rather than an accident:

* **The worker's records are not the renderer's.** A `Text` is built as its layout box
  with no glyph (`__canvas_textBoxHeader`), because the renderer's version rasterises
  every glyph — work for a thread that draws nothing. A `Picture` is not built on the
  worker at all: it is hit as its shape (`__canvas_pictureHitItem`, §17). So a `TEXT` record on the worker has zero glyphs and a
  box in the gradient point slots; **do not hand a worker offset to anything that
  draws.**
* **It is its own walk** (`__canvas_hitAppend`), not a parameter on
  `__canvas_appendDraw`: the graphics thread's entries are re-laid natively by
  `canvas::sceneLayout`, which knows no list of strings, so an id path cannot ride
  there. Both walks call `__canvas_composeGroup`, which is what keeps them agreeing
  about a nested group.
* **All of it is gated on `pollMouse` being called** (`HelperGate::WhenUsed`). The
  native half is `canvas::pollMouseRaw`, and mouse state is reserved by that symbol's
  suffix (`module_uses_mouse`) — so an always-compiled wrapper would have made every
  canvas program reserve a mouse ring. `tests/codegen/codegen_mouse_arena_region.rs`
  holds the line.

## 15. Frame statistics (plan-155-B)

`canvas::getFrameStats` compares two things two different threads do — the worker's
presents and the graphics thread's drawn frames — so everything it reads is in the
**graphics state**, not in MFBASIC globals. `__CANVAS_FRAMES`, `__CANVAS_SKIPPED` and
`__CANVAS_GPU_FRAMES` are graphics-thread globals (§2): the worker's copies are its own
and always zero, which is why the stats line, written on the graphics thread, is the
only place they are visible.

Two event logs, each a total, a ring head and a 64-entry ring of `canvas::frameNanos`
timestamps (`GRAPHICS_OFFSET_PRESENTS…LAST_RENDERER`):

* **presents** — written by `canvas::notePresent` from both present wrappers,
  unconditionally, identical re-presents included;
* **drawn** — written by `canvas::frameDone` when its `drew` argument is TRUE, inside
  the critical section it already held for `FRAMES` and the broadcast. `FRAMES` itself
  still counts every loop iteration, skipped ones too — a `syncFrame` waiter is released
  by a skipped frame, so the two counters must stay separate.

**One lock, one snapshot.** `canvas::frameCounters` copies both logs, the last render
time and renderer, and the display rate into one list inside one critical section, so
`pps - fps` never compares two samples taken at different moments. It allocates the list
before locking; the critical section is loads and stores.

**The mutex must exist before either writer locks it.** It is initialized by
`startGraphics`, so both wrappers now call `__canvas_ensureGraphics()` ahead of their
publish test rather than inside it — the first present of any scene, even one identical
to the empty initial scene, starts the thread. `notePresent` still checks `STARTED`
(an out-of-memory spawn leaves it 0), and `frameCounters` reads without the lock while it
is 0, when nothing has written a word.

**`canvas::waitFrame` has its own condition variable** (plan-155-C,
`GRAPHICS_OFFSET_FRAME_COND`). `signalRedraw` and the stop path wake `COND` with
`pthread_cond_signal`, which wakes one waiter; a worker waiting on `COND` could take the
wake-up a resize meant for the render loop, leaving the redraw owed and the worker
waiting for a pickup that never comes. `syncFrame` has that hazard, but only under the
`MFB_CANVAS_SYNC` test switch. `waitFrame` waits on `FRAME_COND` alone, broadcast where
its answer can change: in `waitForRedraw` as `PENDING` is cleared, and on stop. Reusing
`COND` with broadcasts everywhere would also fix the lost wake-up, at the price of waking
the render loop for every `waitFrame` event and the reverse.

**Why rings and not a moving average.** An average updated per event never decays: a
static scene would report its last rate forever. A rate over the trailing second counts
ring entries newer than `now - 1 s`; when all 64 are, the ring spans under a second and
the rate is taken from its span (`__canvas_rate`), so a fast loop is measured, not
capped at 64.

## 16. `AutoPaint`: state on the graphics thread (plan-172-K)

A `canvas::AutoPaint` is a shape and four alternative paints; which is drawn follows
the pointer. **The graphics thread decides, alone.** The worker never resolves a state
and `present` never waits on one — hover goes on working while the program is blocked
in `io::input` — so everything below is graphics-thread globals (§2) in
`helper_autopaint.rs`, and the only words it ADDS to what is shared with another
thread are `AUTOPAINT_LIVE` and `AUTO_WAKE` in the graphics state (it reads the pointer
words too, which are §5.1's).

### An instance is a PLACE, and the table is keyed by it

State is kept per instance across frames (a phase, a deadline, whether it owns the
press), and an instance is identified by where the walk found it: the index of its
top-level item, and inside a group the chain of (node index, resolved slot) down to its
own index (`__canvas_autoPath`). Three keys that look more natural are each wrong:

* **The item's hash** changes whenever the shape moves or is restyled, so anything
  animated would restart its state every frame and never leave `enter`.
* **The item's `id`** need not be unique — it is what `overID` reports — and a named
  item inside a group drawn twice has ONE `id` and must have TWO states. The owner's
  ruling: "ID is more for mouse overId, not for AutoPaint logic."
* **The draw-entry index** moves whenever an earlier group changes how many items it
  expands to.

What the structural key costs is stated in the spec: insert or remove an item ahead of
an instance in its list and it gets a new key and starts again from its own paint.

**A row is found through an index, never by scanning the table.** Every instance looks
its row up, and every group use asks for its variant, once a frame; a scan of the table
in either made a frame of n instances cost n * n — 277 ms a frame at 10,000 in
`benchmark/canvas`. `__canvas_autoIndexTable` rebuilds `__CANVAS_AUTO_T_ROW` (key to
FIRST row) and the parent chains (`__CANVAS_AUTO_T_FIRST`, `__CANVAS_AUTO_T_NEXT`)
wherever `__CANVAS_AUTO_T_KEY` is written. A chain and not a list per parent: every
top-level instance has parent 0, and appending to a list held in a map copies it, which
is the same n * n again. The variant still reads `__CANVAS_AUTO_T_SHOWN` live, because
`__canvas_autoLight` rewrites it between resolves.

### The walk notes, the resolve swaps

`__canvas_appendDraw`'s `AutoPaint` arm appends the shape's entry built from the shape's
OWN paint and notes the instance. After `canvas::sceneLayout`, `__canvas_autoResolve`
hit-tests the pointer against those entries, steps every instance, and only then swaps
the entry of an instance showing a state paint for that paint's record (a new geometry
record under a stepped hash, and a stepped draw hash). **Hit test first, swap second**
is the rule that a state paint never changes where a shape is hit; a test in
`helper_autopaint.rs` holds the order.

Three things that are easy to get wrong here:

* **An `AutoPaint`'s geometry is never cached under the item's own hash.** The native
  first pass (`canvas::sceneResolve`) resolves any item whose published hash is in the
  cache without the MFBASIC walk seeing it — and an instance the walk does not see has
  no state. So the shape's record is cached under the hash stepped by 16
  (`__canvas_autoGeoHash`), the item's own hash is always a miss, and an `AutoPaint` is
  always walked, as a `Picture` is.
* **The GPU draw list is a second walk and must be told.** It lays out from
  `__CANVAS_TOP_OFFSETS` (so the resolve writes the swapped offset there too), and a
  group's run is memoised ONCE per frame and shared by every node that draws the group
  — so a node under which an `AutoPaint` shows a state paint needs a run of its own.
  The memo is keyed by slot AND `__canvas_autoVariant`. A hover that reaches software
  and not the GPU is a wrong frame on one backend only;
  `a_hovered_autopaint_matches_the_software_oracle_on_metal` is where that shows.
* **A text's draw bounds are not its hit box.** The hit test's first step rejects on a
  record's bounds, which are what it DRAWS; a stroked run's are its outlines', tighter
  than the layout box a `Text` is hit on. A text box skips the reject. The worker never
  saw this, because its text record's bounds ARE the box.

### A wake that owes nothing walks nothing

Triggers 6 and 7 are by far the commonest thing an `AutoPaint` scene sees — every
pointer event — and nearly all of them change no paint. `__canvas_renderFrame` asks
`__canvas_autoLight` before it walks: the last walked frame's entries
(`__CANVAS_AUTO_BASE`), group transforms and geometry are untouched since nothing was
owed, each table row holds its instance's entry and flags, so one hit test and a step
of each row say whether any paint changed. If none did the frame ends there. Walking
first cost 8 ms a pointer event at 10,000 items (the scene is copied out of the ring
whenever anything in it must be walked); the light step is under a millisecond at any
size measured.

When it answers TRUE the full resolve steps the same rows again a moment later. **That
second step must be idempotent**, and it is because the first one's results were stored:
a window it started is running, so it is not restarted; a press it took is recorded in
`__CANVAS_AUTO_WAS_BUTTONS`, so it is not a new press.

### The live word can be read one frame stale

`AUTOPAINT_LIVE` is a plain word and a hint. Stale as 1 costs a wake that changes
nothing. Stale as 0 is the one that could lose something: a pointer change landing
between a frame's pointer snapshot and that frame's first store of the word finds 0 and
signals nobody. So the resolve that first sets the word reads the pointer again
afterwards, and if it moved, has the loop wait 1 ns (`__CANVAS_AUTO_AGAIN`).

### The pointer is sampled

A frame reads the pointer once. A press and release inside one frame are not seen; a
button that goes down off a shape and reaches it before the next frame is a press that
began on it. Every publish wakes the loop while an `AutoPaint` is live, so the window is
one frame — but it is a property, and a test that scripts two changes into one frame
will see one.

### Under `MFB_CANVAS_SYNC`, a frame nobody asked for can release a `present`

`syncFrame` waits for the frame counter to reach `WANTED`, and `frameDone` counts every
frame the loop finishes — a skipped one and an auto-only one included. So a `present`
that signals while an auto-only frame is in flight is released by THAT frame, one
frame before its own scene is drawn. The scene is still drawn (its `PENDING` is set,
and shutdown drains it), so a test that reads the last frame is unaffected; a test that
assumes "the n-th stats line is the n-th present" is not, in a scene with an
`AutoPaint` and a pointer. Read `auto=` and `frames=` off the lines instead of counting
them. A resize in flight has always done the same.

### The code is gated on the program constructing one

`helper_autopaint.rs` is two chunks that define the same fifteen names:
`HelperGate::WhenUsedOrConstructed(&[], &["AutoPaint"])` for the real ones and
`WhenNotConstructed` for `ABSENT`, the bodies that are right when no `AutoPaint` can
exist. The unconditional source calls the names. Adding a hook means adding it to BOTH,
with the same signature — `canvas_auto_hooks_are_declared_alike_in_both_chunks` fails
otherwise, and without it the build breaks for one kind of program only.

**Measure what an always-compiled arm costs before leaving it there.** The item unions
have the `AutoPaint` variant in every canvas program, so every `MATCH` over them needs
its arm, and an arm that copies a record is thousands of instructions: the converter
`__canvas_autoItem` (eight arms, each a whole shape record) and the hash arm were
24,744 bytes of code in every canvas program until they moved behind the gate. The
arms that remain call a hook.

**A record whose every field has a default needs the NAMED gate, not the constructed
one.** `helper_curves/` (plan-164-B) is gated with
`HelperGate::WhenCalledOrNamed(&[], &["PolyCurve", "CurvePolygon"])` and its exact
complement `WhenNeitherCalledNorNamed`: the chunk is in when the program's source names
either type anywhere — a constructor or any annotation. A constructor gate would be
wrong for them: `MUT c AS canvas::PolyCurve` then `WITH c { … }` holds a curve no
constructor expression made, and that program would get the stand-ins and draw nothing.
(`AutoPaint` and `Picture` have a union field, so no default, so constructing is the
only way to hold one.) Ungated, the curve chunk was 288,896 bytes of `__text` in every
canvas program; gated, a program with no curve pays 21,652 for the arms and stand-ins.

## 17. A `Picture` is one or two draw entries (plan-172-L)

`helper_picture/`. A `canvas::Picture` holds a shape and draws an image inside it, with
no geometry kind, header slot or shader of its own: a **plain** picture (an image
stretched over a square-cornered rectangle) is one picture record, and a **shaped** one
is a picture record with the shape's outline for its clip, followed by the shape itself
with no fill. The rules that keep that sound:

- **An item that is not exactly ONE entry is negative in `__CANVAS_TOP_OFFSETS`.** That
  list is one offset per scene index; `canvas::sceneDrawsFlat` lays the frame out from it
  natively and declines on any negative, and `__canvas_sceneDrawsWalk` must then handle
  the item. A group node (−1) was the only such item before; a shaped picture (two
  entries, or none) is −2. Writing the first entry's offset there instead draws half the
  picture on the GPU and all of it in software — the two-renderer failure, with nothing
  raised.
- **The draw walk takes a −2 item's blocks from the frame's ENTRIES, not from the item.**
  Asking the item again re-reads its image, which the worker may have refilled or closed
  since the entries were made: the first cut did, and a run that ended mid-frame
  reported one block for a picture whose two entries the same frame had drawn. A frame
  draws from one reading of each image (section 7).
- **Nothing a multi-entry item draws from is cached under the item's own hash.** The
  native first pass (`canvas::sceneResolve`) resolves an index from its hash alone, to
  one record, and the walk never sees the item. A shaped picture's records are under the
  hash stepped (`__canvas_pictureKey`); only a plain picture's is under the hash, and a
  test reads the source to keep it so
  (`canvas_picture_only_the_plain_record_is_cached_under_the_item_hash`).
- **Inside a group the two walks each ask.** `__canvas_appendDraw` (the entries) and
  `__canvas_memoGroup` (the group's GPU run, shared by its uses) both get a picture's
  records from `__canvas_pictureQuick` / `__canvas_pictureBlocksOf`, which decide "is
  this entry there" from the cached record's kind, not from the item — the same answer
  unless the image changed in between, as it always could for a picture in a group.
- **A record's coverage is its shape's TIMES its clip's.** Where a picture rectangle's
  edge lies along its clip's, a half-covered pixel comes out a quarter covered. The
  image record therefore lets the clip alone draw the edge: its radius slot (6) is 1
  with its box (slots 2–5) whole, which moves the rectangle's edge a pixel out and the
  image not at all — all three renderers take a rectangle's distance as box less radius
  and sample a picture from the box alone. Check all three before leaning on a slot
  like that again.
- **Fit the image to the shape's exact box, never a flattened outline's.** A circle's
  flattened ring is its box to ~4e-5 px (the package's own sine). A destination that far
  off a whole number moves a texel boundary off a pixel centre, and float32 (the GPU) and
  float64 (the oracle) then pick different texels along that whole row — 44 pixels, max
  delta 220, found only by the Metal comparison.
- **The chunk is gated on constructing a `Picture`, and so is everything it needs from
  another gated chunk.** `__canvas_itemId` lives in the `pollMouse`-gated hit chunk; a
  picture chunk that called it built only for programs that poll the mouse. A gated
  chunk may call the unconditional source and another chunk's hooks (the names its
  `ABSENT` twin also defines) and nothing else.
- **A picture in a `CurvePolygon` clips its image to the outline FLATTENED** (plan-164-B):
  `__canvas_curveRings` turns each contour's pieces into a ring of straight edges within
  the tolerance they were reduced at, because the clip machinery — the software
  intersection, both shaders' path clip — knows only straight edges. The OUTLINE entry
  is the `CurvePolygon` itself and is the true curve. So the one item costs clip edges
  (one slot each; 128 for a circle of radius 50, 504 at 400) AND pieces (two each), both
  against the edge cap. The image is fitted to the curve's true extent
  (`__canvas_curveExtent`), not the flattened ring's box and not the control points' —
  the previous rule, for the same reason.
- **An `AutoPaint` for a shape** makes the picture an instance at the picture's place.
  It is then always two entries, either of which may be the empty record, so the
  frame's resolve can swap both in place; the instance's entry is the OUTLINE's, which
  is the shape and so is what the pointer is tested against. The host picture is kept
  by the picture chunk (`__CANVAS_PICTURE_HOSTS`), not the AutoPaint chunk — a
  `List OF DrawItem` there cost every `AutoPaint` program 19 KB of code for a picture it
  may not have.

## See also

* `planning/completed/plan-98-A-*` — cross-cutting invariants 1, 2, 4, 5, 7, 8.
* `.ai/collections.md` — why the rasteriser's pixel writes must stay inside the
  function owning the buffer local (they are 290x slower otherwise).
* `.ai/testing-gates.md` — the canvas reference-image gate the ring must not disturb.

### Growing `ITEM_BLOCK_SIZE` moves six things, and three are silent

Widening the per-item block is the single most repeated change in plan-116 — A grew it
for the item buffer, C for the transform, D for the arc caps, E for the ellipse, and F
twice (the ellipse `ivec4`, then the gradient one, reaching 208) — bug-735 added four
residual `ivec4`s (272), plan-163-B the `strokeStyle` `ivec4` (288: first join slot,
join record count, first dash word, dash word count), and plan-163-C the `clipBand` and
`clipPath` `ivec4`s (320: a path clip's band header at 288, the same shape as `arcCaps`'s
polygon one; then first clip edge, clip edge count, mode | rule << 2, a picture's
transparency step T at 304), and plan-163-D the `gradInv0`/`gradInv1`/`gradFocal`
`ivec4`s (368: a radial gradient's own inverse as float32 bits and its flag at 320/336,
its focal offset and radius at 352), then the move to fp32 removed the four residual
`ivec4`s (304); each time it moved the same set. Three of them are unrelated by any type, and none of the three
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
   whose slots are hand-numbered. Until bug-735 widening it ran the block into
   `OFF_TEXTURE` and every slot above had to shift by hand; they are now written as
   `OFF_TEXTURE + n` with `OFF_TEXTURE = OFF_ITEM + ITEM_BLOCK_SIZE`, so they move by
   construction and `DRAW_FRAME` with them. An overlap corrupts a pointer the
   `objc_msgSend` sequence reads and produces a **black GPU frame that reports
   success**, so the overlap test still earns its keep: a slot that GROWS (bug-735's
   16-byte `OFF_DRAW_PAIR`) is the case construction does not cover.
4. **`METAL_EDGE_BASE`** and every later Metal region base (band, gradient, glyph,
   picture table). Since bug-686 these are no longer literals: `METAL_SHADER_SOURCE` is
   built at first use with the bases formatted in from the `METAL_*_BASE_WORDS`
   constants, so they move with `ITEM_BLOCK_SIZE` by construction. The region-chain
   test still pins that each region starts where the previous one ends.
5. **Vulkan's region bases** (glyph, gradient, band). Since bug-688 these are not
   literals either: the GLSL declares them `layout(constant_id = N) const int`, and the
   pipeline feeds `VULKAN_SPEC_CONSTANTS` through `VkSpecializationInfo`, so they move
   with every cap by construction and the checked-in SPIR-V never spells a layout number.
   Vulkan's item buffer is its own binding, so `ITEM_BLOCK_SIZE` does not move them; a
   cap does.
6. The `.spv` blobs, via `scripts/regen-spirv.sh`.

Items 3, 4 and 5 are caught by `the_draw_frame_slots_do_not_overlap`,
`the_metal_shader_region_bases_match_the_buffer_layout` (Metal — it guards **both**
bases and the region chain, despite plan-116-F having found it named for the edge one
alone), and on Vulkan `the_pipeline_specializes_every_region_base` (the GLSL's
name-to-id pairing, and that the blob was regenerated) with
`the_shared_buffer_holds_every_region` (the chain). Their literal-comparing
predecessors fired on **every** plan-116 letter and were the only thing that noticed.
Item 2 has no guard beyond the reflection — plan-116-D shipped the fragment half alone
and found it that way.

### A geometry record's tail is read back from its END

A record is the 69-float header, then its tail in a fixed order: **edges | join records |
dash tail | clip edges | gradient stops** (plan-163-B, plan-163-C). Only the edge count is
known from the front; every other part is found by counting back from slot 1 (the
record's length) — the stops are `slot1 - stops * 5`, a path clip's edges
`__CANVAS_GEO_CLIP_EDGE_COUNT * 5` (slot 52) before them, the dash tail
`__CANVAS_GEO_DASH_COUNT` (slot 49) floats before that, the join records
`__CANVAS_GEO_JOIN_COUNT * 8` (slot 47) before that. Every reader calls
`__canvas_gradientStopBase` / `__canvas_clipEdgeBase` / `__canvas_dashBase` /
`__canvas_joinBase` rather than recomputing (the 2026-09-01 polygon-cache lesson), and the
GPU emitters that copy a part out (`emit_join_buffer`/`emit_join_upload`,
`emit_dash_buffer`/`emit_dash_upload`, `emit_clip_buffer`/`emit_clip_upload`) subtract the
parts after it the same way.

A path clip's edges share the frame's EDGE region with the polygons' — copied after the
item's own edges and join records, float32 words like any edge, and counted
against the same edge region, one pass's, by `__canvas_blockCost` — and are banded by the same
`emit_band_index`, parameterised by a `BandSet` (which count, base and header words; a
clip's reach has no stroke, cap or join term). A clip that would pass the region is left at
edge count 0, which the shader reads as a clip enclosing nothing: hidden, never drawn
unclipped.

**`surface.w` is packed, and every reader must unpack it (plan-163-D).** It was the
gradient's kind alone, read as `surface.w == 1` (`gradientKind == 1` in the MSL); it is now
`kind | spread << 1 | apply << 3` (spread 0 Pad, 1 Reflect, 2 Repeat; apply 1 for
`FillAndStroke`), so the kind is `& 1`. A reader left comparing the whole word draws every
spread or stroke-painting radial gradient as linear — a plausible wrong picture.

**A split blended item's stroke record marks its fill OFF, not transparent
(plan-163-D).** `emit_split_or_publish` publishes a blended item that fills and strokes as
two records; record two's fill alpha is −1, which both shaders read as "no fill here".
Zero was wrong: a gradient's ramp and a picture's image do not come from the fill alpha,
so record two drew them a second time under the blend. "Fills" is likewise fill alpha > 0,
two or more stops, or a picture — in `emit_split_or_publish` (both backends),
`emit_block_instances` and `__canvas_blockInstances`, which must agree or every later draw
base shifts. A new tail part goes BEFORE
the stops and every emitter that locates an earlier part must subtract it too — a missed
one copies the wrong floats as corners or dashes, a plausible wrong picture.

The dash words are their own frame region (between the gradient stops and the glyph
texels, `METAL_DASH_BASE_WORDS` / `VULKAN_DASH_BASE_WORDS`, the Vulkan base a
specialization constant like the others), as float32 bit patterns: the phase (slot 50),
then the dash tail. The shaders evaluate them in fp32 and must not use the driver's
`atan`/`sin`/`cos`: Vulkan allows `atan` 4096 ULP and `sin`/`cos` 2^-11 absolute, which
box 206's Intel driver uses and lavapipe and Metal do not, so a curve's dash end drawn
with them passes on two backends and fails on real hardware (`dashAtan`, `dashSin`,
`dashCos` are the oracle's series).

**`/` is on that list too: never let `floor` of a quotient choose a discrete outcome
(bug-1079).** Vulkan allows `/` 2.5 ULP and the same driver takes it (it multiplies by a
reciprocal: `110 / 110` came out below 1), so a picture whose texel boundary was a pixel
centre drew the neighbouring texel along that whole row. `+ - *` are correctly rounded
everywhere, so the index is `floor(t / d)` corrected by at most one from the products
(`pictureTexel`, and the same three steps in the oracle's picture arm). The sites left
with the hazard are listed in bug-1079's sweep: a hard gradient stop and a `Repeat`
seam exactly on a pixel centre. A quotient that only feeds a distance or a ramp
position is continuous and stays inside the tolerance.

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
