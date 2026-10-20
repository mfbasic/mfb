//! plan-176-V: the graphics-state block layout — `GRAPHICS_STATE_SYMBOL`, every
//! `GRAPHICS_OFFSET_*`, the frame ring and counter sizes, and the `WAKE_*` answers. The shared
//! imports and `GraphicsScratch` live in `runtime/canvas/mod.rs`.

/// The graphics thread's shared state, as one writable process-global block.
///
/// Process-global rather than arena state for the reason everything cross-thread in
/// canvas is: arena state is per-thread (`.ai/canvas-threading.md` §2).
pub(crate) const GRAPHICS_STATE_SYMBOL: &str = "_mfb_rt_canvas_graphics";

/// `started` — 0 until the thread has been spawned. Checked and set under the mutex,
/// so a program that presents from a loop spawns exactly one thread.
pub(crate) const GRAPHICS_OFFSET_STARTED: usize = 0;
/// `pending` — non-zero when a redraw is owed. A counter would be wrong: two presents
/// before a frame must produce **one** frame, not two (`.ai/canvas-threading.md` §3),
/// so this is a flag the renderer clears.
pub(crate) const GRAPHICS_OFFSET_PENDING: usize = 8;
/// The mutex guarding `started` and `pending`.
pub(crate) const GRAPHICS_OFFSET_MUTEX: usize = 16;
/// The condition the render loop waits on. `pthread_cond_t` is 48 bytes on both
/// macOS and glibc; a Win32 `CONDITION_VARIABLE` is 8 and fits inside it.
pub(crate) const GRAPHICS_OFFSET_COND: usize = 64;
/// The child's arena-state pointer, handed to the trampoline as its thread argument.
pub(crate) const GRAPHICS_OFFSET_ARENA: usize = 112;
/// `stopping` — set by shutdown. The loop returns `FALSE` from its wait and exits.
pub(crate) const GRAPHICS_OFFSET_STOPPING: usize = 128;
/// The OS thread id. Nothing joins the graphics thread — the process exit tears it
/// down — but `pthread_create` requires a writable slot to write it into, and it must
/// not be one of the fields above: aliasing it onto `started` would work by accident
/// (a tid is never 0) right up until it did not.
pub(crate) const GRAPHICS_OFFSET_TID: usize = 120;
/// Frames completed. Incremented by the render loop after each frame.
pub(crate) const GRAPHICS_OFFSET_FRAMES: usize = 200;
/// The frame number a `signalRedraw` is asking for (`frames + 1` at signal time).
/// Only read by `canvas::syncFrame`.
pub(crate) const GRAPHICS_OFFSET_WANTED: usize = 208;
/// Non-zero when `MFB_CANVAS_SYNC` was set at spawn: `canvas::present` then waits for
/// the frame it asked for before returning.
///
/// **A test affordance, and off by default.** Frames coalesce by design
/// (`.ai/canvas-threading.md` §3), so how many a run produces is a scheduling
/// detail — which makes any frame-level assertion a flake without this. It is read
/// once, at spawn, rather than per present.
pub(crate) const GRAPHICS_OFFSET_SYNC: usize = 216;
/// The surface's current pixel dimensions, published by the **main** thread on a
/// resize and read by the graphics thread at frame start.
///
/// Zero means "not yet published", which reads as the startup size — the three
/// platform surfaces are all created 900x640, so a program that never resizes sees
/// exactly what it saw before this existed.
pub(crate) const GRAPHICS_OFFSET_WIDTH: usize = 224;
pub(crate) const GRAPHICS_OFFSET_HEIGHT: usize = 232;
/// Non-zero when the Metal renderer is selected (plan-98-E).
///
/// **The default for a real window, never for a headless run.** The software path is
/// the *oracle* the GPU backends are measured against (plan-98-A invariant 7), and its
/// goldens are exact-match; a GPU frame only matches within a tolerance. Every test
/// that renders runs headless, and a headless run keeps software, so the oracle and
/// its goldens are untouched; a window, where frame rate is what the user sees, gets
/// the GPU. `MFB_CANVAS_GPU` overrides either way (`0` = software, anything else =
/// GPU); plan-98-E's own tests set it to `1`. Chosen by `__canvas_wantGpu`.
pub(crate) const GRAPHICS_OFFSET_GPU: usize = 296;
/// Scratch for the `pthread_attr_t` the spawn configures. 64 bytes covers macOS
/// (64) and musl/glibc (56). It lives here rather than on the spawner's stack
/// because there is exactly one spawn and the block is already process-global.
pub(crate) const GRAPHICS_OFFSET_ATTR: usize = 304;
/// The Metal device, command queue and render pipeline state (plan-98-E Phase 1),
/// and the `MTLBuffer`-backed offscreen texture the renderer draws into.
///
/// They live in the graphics-state block rather than in the macOS app module's own
/// storage for the reason §2 of `.ai/canvas-threading.md` gives for everything else
/// here: the graphics thread creates them, uses them, and is the only thread that
/// may, and the arena is per-thread — so a process-global word is the only place
/// they can be kept where the thread that made them will find them again.
///
/// `GRAPHICS_OFFSET_MTL_READY` is the "tried already" flag, distinct from
/// `PIPELINE` being non-zero: a machine with no Metal device must not re-run the
/// device probe and the shader compile on every frame just because they failed.
pub(crate) const GRAPHICS_OFFSET_MTL_READY: usize = 368;
pub(crate) const GRAPHICS_OFFSET_MTL_DEVICE: usize = 376;
pub(crate) const GRAPHICS_OFFSET_MTL_QUEUE: usize = 384;
pub(crate) const GRAPHICS_OFFSET_MTL_PIPELINE: usize = 392;
/// The offscreen render target, and the dimensions it was created for.
///
/// The renderer draws into a texture rather than straight to a drawable so that the
/// finished frame goes back through the *same* `canvas::blitSurface` the software
/// path uses. That is what makes the two comparable at all — the tolerance
/// comparator reads an RGBA8 buffer, and a frame that only ever existed inside a
/// `CAMetalLayer` is not one. plan-98-E Phase 2 adds the direct-to-drawable present
/// alongside it.
///
/// A resize creates a new texture and releases the old one, which is why the
/// dimensions are kept here rather than re-read from the texture: comparing two
/// words is cheaper than two message sends, every frame.
pub(crate) const GRAPHICS_OFFSET_MTL_TEXTURE: usize = 400;
pub(crate) const GRAPHICS_OFFSET_MTL_TEX_WIDTH: usize = 408;
pub(crate) const GRAPHICS_OFFSET_MTL_TEX_HEIGHT: usize = 416;
/// The Vulkan renderer's device layer (plan-98-F).
///
/// Same reasoning as the Metal slots above: the graphics thread creates these, uses
/// them, and is the only thread that may — and the arena is per-thread, so a
/// process-global word is the only place they survive.
///
/// `VULKAN_READY` is a state, not "is the device non-null", for the same reason
/// `MTL_READY` is: a machine with a loader but no ICD must pay the failed enumeration
/// once, not on every frame. Its four values, who writes each and the order that makes
/// it the build's hand-off are [`vulkan_handoff`]'s (bug-996).
pub(crate) const GRAPHICS_OFFSET_VULKAN_READY: usize = 424;
pub(crate) const GRAPHICS_OFFSET_VULKAN_LIB: usize = 432;
pub(crate) const GRAPHICS_OFFSET_VULKAN_INSTANCE: usize = 440;
pub(crate) const GRAPHICS_OFFSET_VULKAN_PHYSICAL: usize = 448;
pub(crate) const GRAPHICS_OFFSET_VULKAN_DEVICE: usize = 456;
pub(crate) const GRAPHICS_OFFSET_VULKAN_QUEUE: usize = 464;
pub(crate) const GRAPHICS_OFFSET_VULKAN_QUEUE_FAMILY: usize = 472;
/// The pipeline the renderer records against, built once with its layout and render
/// pass. Viewport and scissor are dynamic state, so a resize reuses all three.
pub(crate) const GRAPHICS_OFFSET_VULKAN_PIPELINE_LAYOUT: usize = 480;
pub(crate) const GRAPHICS_OFFSET_VULKAN_RENDER_PASS: usize = 488;
pub(crate) const GRAPHICS_OFFSET_VULKAN_PIPELINE: usize = 496;
/// The offscreen render target and the readback path, plus the dimensions they were
/// built for — a resize rebuilds them, as the Metal path rebuilds its texture.
///
/// `VULKAN_MAPPED` is the readback buffer's persistently-mapped pointer. Vulkan
/// allows a `HOST_VISIBLE` allocation to stay mapped for its lifetime, so mapping
/// once and keeping the pointer avoids a map/unmap pair per frame.
pub(crate) const GRAPHICS_OFFSET_VULKAN_IMAGE: usize = 504;
pub(crate) const GRAPHICS_OFFSET_VULKAN_IMAGE_MEMORY: usize = 512;
pub(crate) const GRAPHICS_OFFSET_VULKAN_IMAGE_VIEW: usize = 520;
pub(crate) const GRAPHICS_OFFSET_VULKAN_FRAMEBUFFER: usize = 528;
pub(crate) const GRAPHICS_OFFSET_VULKAN_READ_BUFFER: usize = 536;
pub(crate) const GRAPHICS_OFFSET_VULKAN_READ_MEMORY: usize = 544;
pub(crate) const GRAPHICS_OFFSET_VULKAN_MAPPED: usize = 552;
pub(crate) const GRAPHICS_OFFSET_VULKAN_TEX_WIDTH: usize = 560;
pub(crate) const GRAPHICS_OFFSET_VULKAN_TEX_HEIGHT: usize = 568;
/// The command pool and the single command buffer the frame is recorded into.
pub(crate) const GRAPHICS_OFFSET_VULKAN_COMMAND_POOL: usize = 576;
pub(crate) const GRAPHICS_OFFSET_VULKAN_COMMAND_BUFFER: usize = 584;
/// The polygon edge buffer and the descriptor machinery that binds it.
///
/// A polygon carries an unbounded number of edges and the guaranteed push-constant
/// range is 128 bytes, which the 112-byte item block already fills — so the edges
/// need a storage buffer, and a storage buffer needs a descriptor set. This is the
/// only reason the Vulkan pipeline has one; every other kind rides the push
/// constants alone.
///
/// The buffer is host-visible and stays mapped, exactly like the readback buffer:
/// it is rewritten every frame, so a map/unmap pair per frame would cost more than
/// the writes. It is created once with the device rather than with the target,
/// because its size does not depend on the surface.
pub(crate) const GRAPHICS_OFFSET_VULKAN_SET_LAYOUT: usize = 592;
pub(crate) const GRAPHICS_OFFSET_VULKAN_DESC_POOL: usize = 600;
pub(crate) const GRAPHICS_OFFSET_VULKAN_DESC_SET: usize = 608;
pub(crate) const GRAPHICS_OFFSET_VULKAN_EDGE_BUFFER: usize = 616;
pub(crate) const GRAPHICS_OFFSET_VULKAN_EDGE_MEMORY: usize = 624;
pub(crate) const GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED: usize = 632;
/// How many times the platform has published a **different** surface size.
///
/// A counter, not a flag, and that is what makes `canvas::didResize` lock-free. Only
/// the main thread writes this and only the worker writes `…_RESIZES_SEEN`, so the two
/// never race for the same word — where a single read-and-clear flag would lose a
/// resize that landed between a reader's load and its store, on a path whose whole
/// purpose is to report edges.
pub(crate) const GRAPHICS_OFFSET_RESIZES: usize = 640;
/// The value `canvas::didResize` last reported. The worker owns this word.
pub(crate) const GRAPHICS_OFFSET_RESIZES_SEEN: usize = 648;
/// The per-frame **item buffer**: one `ITEM_BLOCK_SIZE` record per drawn quad, indexed
/// by `gl_InstanceIndex`, plus the memory backing it and its persistent mapping.
///
/// This is the transport that replaced the push constants (plan-116-A). A push
/// constant is a per-*draw* value, so it could only ever describe one item, which
/// forced one draw call per item and — far more importantly — pinned the item block
/// under Vulkan's guaranteed 128-byte range. The buffer has neither property: the
/// whole frame's items are written once, a run of them is drawn with a single
/// instanced `vkCmdDraw`, and the block's size is bounded by
/// `VULKAN_ITEM_BUFFER_BYTES` rather than by a device limit.
///
/// Host-visible and mapped for its lifetime, created with the *device* and not with
/// the target, exactly like `…_VULKAN_EDGE_*` above — its size does not depend on the
/// surface, so a resize must not tear it down.
pub(crate) const GRAPHICS_OFFSET_VULKAN_ITEM_BUFFER: usize = 656;
pub(crate) const GRAPHICS_OFFSET_VULKAN_ITEM_MEMORY: usize = 664;
pub(crate) const GRAPHICS_OFFSET_VULKAN_ITEM_MAPPED: usize = 672;
/// Metal's frame buffer — the same transport, one `MTLBuffer` instead of a Vulkan
/// buffer plus a descriptor (plan-116-A).
///
/// It carries the item blocks from byte 0, then the polygon edges from
/// `METAL_EDGE_BASE_WORDS`, then the regions listed at `METAL_BUFFER_BYTES` (gradient
/// stops, glyph and picture texels, …). Before this, Metal's edges rode a per-item
/// `setFragmentBytes:` payload, which an instanced draw cannot rebind between
/// instances — so every polygon would have ended the instanced run, and letters F and
/// H would each have rediscovered the same conflict. Both backends now carry edges the
/// same way.
///
/// `…_CONTENTS` caches `[buffer contents]` so the frame path writes through a plain
/// pointer instead of sending a message per item, mirroring the Vulkan side's
/// persistent mapping. Created with the *device*, not the target: its size does not
/// depend on the surface, so a resize must not tear it down.
pub(crate) const GRAPHICS_OFFSET_MTL_ITEM_BUFFER: usize = 680;
pub(crate) const GRAPHICS_OFFSET_MTL_ITEM_CONTENTS: usize = 688;
/// The **three non-`Normal` pipelines**, one per remaining `BlendMode`, on each
/// backend (plan-116-B).
///
/// A blend mode is per-*pipeline* state on both APIs, not per-draw — it is baked into
/// `VkPipelineColorBlendAttachmentState` and into `MTLRenderPipelineDescriptor`'s
/// colour attachment — so "per-item blend" means four pipelines selected per draw, not
/// a shader branch. All four differ *only* in their blend factors; they share one
/// vertex function, one fragment function and one layout.
///
/// **Four contiguous slots each, indexed by the `BlendMode` tag directly** — entry 0
/// is `Normal`. Contiguous and 0-based so the frame path computes the handle's address
/// as `base + mode * 8` with no branch and no `mode - 1` correction; a four-way branch
/// per mode change is the kind of arithmetic that is right for three of the four cases.
///
/// `Normal`'s handle is *also* stored in `…_VULKAN_PIPELINE` / `…_MTL_PIPELINE`, which
/// stay exactly what they were. That is not redundancy for its own sake: those slots
/// are what the readiness checks test for non-zero, and what an ordinary scene binds
/// once at frame start, so leaving them untouched keeps "the pipeline built" meaning
/// what it has always meant.
pub(crate) const GRAPHICS_OFFSET_VULKAN_PIPELINE_MODES: usize = 696;
pub(crate) const GRAPHICS_OFFSET_MTL_PIPELINE_MODES: usize = 728;
/// How many blend modes there are, and therefore how many pipelines each backend
/// builds. Spelled again in MFBASIC as the `BlendMode` variant count; the emitters and
/// the header agree through `HEADER_BLEND`'s 0..3 range.
pub(crate) const BLEND_MODE_COUNT: usize = 4;
/// The macOS window's `CAMetalLayer`, and the state of the direct present (bug-686
/// Phase 4).
///
/// A GPU frame in a real window is copied into this layer's next drawable and
/// presented, instead of being read back into a CPU surface and blitted as a
/// `CGImage`. The layer is a hidden sublayer of the canvas view's backing layer,
/// created by the **main** thread when it builds the view — so a headless run, which
/// builds no view, leaves `…_MTL_LAYER` zero, and that zero is what sends every
/// headless frame down the readback path the oracle tests compare.
///
/// Ownership, one writer per word:
///
/// * `LAYER`, `LAYER_READY` and `LAYER_SHOWN` — **main**. `READY` is set once the
///   layer's device, pixel format and `framebufferOnly` are configured; `SHOWN`
///   tracks its `hidden` property, which the main thread flips on only when the
///   graphics thread asks and off when a software (or readback) frame lands.
/// * `DRAWABLE_W`/`_H` and `LAYER_SHOW` — **graphics**: the drawable size and the
///   visibility it asks the main thread to apply, read by the main-thread IMP during
///   a `waitUntilDone:YES` hop, so the graphics thread never touches a layer property.
pub(crate) const GRAPHICS_OFFSET_MTL_LAYER: usize = 760;
pub(crate) const GRAPHICS_OFFSET_MTL_LAYER_READY: usize = 768;
pub(crate) const GRAPHICS_OFFSET_MTL_LAYER_SHOWN: usize = 776;
pub(crate) const GRAPHICS_OFFSET_MTL_DRAWABLE_W: usize = 784;
pub(crate) const GRAPHICS_OFFSET_MTL_DRAWABLE_H: usize = 792;
pub(crate) const GRAPHICS_OFFSET_MTL_LAYER_SHOW: usize = 800;
/// The refresh rate of the monitor the window is on, in **millihertz**, published by
/// the **main** thread (plan-155-A) when the surface is ready and again whenever the
/// window lands on a monitor with a different rate.
///
/// `0` means unknown: headless, no window yet, or an OS that does not answer. Millihertz
/// because GTK answers in it and 59.94 Hz panels exist. One writer and one aligned
/// store, so no lock — the same reasoning as `GRAPHICS_OFFSET_WIDTH`. Publishing it
/// signals nothing: a rate is not a redraw trigger (`.ai/canvas-threading.md` §4).
pub(crate) const GRAPHICS_OFFSET_DISPLAY_MHZ: usize = 808;
/// `MFB_CANVAS_DISPLAY_HZ × 1000`, stored once by the worker; non-zero wins over
/// `…_DISPLAY_MHZ`. **A test affordance, and off by default.** A separate word rather
/// than a write into the published one, so a platform republish can never clobber it.
pub(crate) const GRAPHICS_OFFSET_DISPLAY_OVERRIDE: usize = 816;
/// Frame statistics (plan-155-B), read by `canvas::getFrameStats` through
/// `canvas::frameCounters`. Process-global for the reason everything cross-thread here
/// is (`.ai/canvas-threading.md` §2): the graphics thread's frame record would be
/// invisible to the worker as an MFBASIC global.
///
/// Two event logs, each a running total, a ring head and a ring of the last
/// [`FRAME_RING_SLOTS`] timestamps (`canvas::frameNanos`), written and read only under
/// `GRAPHICS_OFFSET_MUTEX`:
///
/// * **presents** — every `present` / `presentLayers`, identical re-presents included,
///   written by the worker (`canvas::notePresent`);
/// * **drawn** — every frame a backend actually drew, written by the graphics thread in
///   `canvas::frameDone` (a wake-up whose damage was empty is not one).
///
/// Then the render time of the last drawn frame, in nanoseconds, and which renderer drew
/// it (the `canvas::Renderer` ordinal: 0 Software, 1 Metal, 2 Vulkan).
pub(crate) const GRAPHICS_OFFSET_PRESENTS: usize = 824;
pub(crate) const GRAPHICS_OFFSET_PRESENT_HEAD: usize = 832;
pub(crate) const GRAPHICS_OFFSET_PRESENT_RING: usize = 840;
pub(crate) const GRAPHICS_OFFSET_DRAWN: usize = GRAPHICS_OFFSET_PRESENT_RING + FRAME_RING_SLOTS * 8;
pub(crate) const GRAPHICS_OFFSET_DRAWN_HEAD: usize = GRAPHICS_OFFSET_DRAWN + 8;
pub(crate) const GRAPHICS_OFFSET_DRAWN_RING: usize = GRAPHICS_OFFSET_DRAWN_HEAD + 8;
pub(crate) const GRAPHICS_OFFSET_LAST_RENDER_NANOS: usize =
    GRAPHICS_OFFSET_DRAWN_RING + FRAME_RING_SLOTS * 8;
pub(crate) const GRAPHICS_OFFSET_LAST_RENDERER: usize = GRAPHICS_OFFSET_LAST_RENDER_NANOS + 8;
/// Timestamps kept per event log. A rate over the trailing second counts ring entries
/// newer than a second; when all of them are, the ring spans less than a second and the
/// rate is measured from its span instead (`__canvas_rate`), so the bound costs no
/// accuracy. A power of two, so the head wraps with a mask.
pub(crate) const FRAME_RING_SLOTS: usize = 64;
/// The words `canvas::frameCounters` returns, in order: `now`, presents, present head,
/// drawn, drawn head, last render nanos, last renderer, display millihertz, then the
/// present ring and the drawn ring. `__canvas_getFrameStats` indexes by these.
pub(crate) const FRAME_COUNTER_HEADER_WORDS: usize = 8;
pub(crate) const FRAME_COUNTER_WORDS: usize = FRAME_COUNTER_HEADER_WORDS + 2 * FRAME_RING_SLOTS;
/// The condition `canvas::waitFrame` waits on (plan-155-C), 48 bytes like
/// `GRAPHICS_OFFSET_COND`.
///
/// **Its own condition variable, and that is the point.** `signalRedraw` and the stop
/// path wake `COND` with `pthread_cond_signal`, which wakes ONE waiter. A worker parked
/// in `waitFrame` on `COND` could take a wake-up meant for the render loop, leaving a
/// redraw owed and never taken while the worker waits for a pickup that never comes.
/// So `waitFrame` waits on this one only, and it is broadcast wherever a waiter's answer
/// can change: when the render loop takes a redraw (`PENDING` cleared) and on stop.
pub(crate) const GRAPHICS_OFFSET_FRAME_COND: usize = GRAPHICS_OFFSET_LAST_RENDERER + 8;
/// When `canvas::waitFrame` last returned, in the `CLOCK_REALTIME` nanoseconds its timed
/// wait is expressed in. Written and read only by `waitFrame`, under the mutex.
pub(crate) const GRAPHICS_OFFSET_LAST_WAIT: usize = GRAPHICS_OFFSET_FRAME_COND + 48;
/// bug-688 V6: the Vulkan swapchain present path (`vulkan_present.rs`), **graphics
/// thread only** — the one thread that renders, so no lock.
///
/// * `WSI` — 0 no window-system integration; 1 the instance took `VK_KHR_surface` +
///   `VK_KHR_win32_surface` and the device `VK_KHR_swapchain`. Set once, at build.
/// * `SURFACE` / `SURFACE_HWND` — the surface and the window it was made on; a new
///   window handle (canvas mode left and re-entered) rebuilds both.
/// * `SWAPCHAIN`, `SC_W`/`SC_H`, `SC_COUNT`, `SC_STALE` (recreate before the next
///   acquire: out of date, suboptimal, or a size change), `SC_FAILED` (the device or
///   surface can never present — every frame takes the readback path).
/// * `PRESENT_PASS` — the render pass whose final layout is `PRESENT_SRC_KHR`; the
///   offscreen pass's pipelines are compatible with it (same single attachment format).
/// * `SEM_ACQUIRE` / `SEM_RENDER` — acquire -> submit -> present ordering.
/// * `SC_IMAGES` / `SC_VIEWS` / `SC_FRAMEBUFFERS` — [`VULKAN_MAX_SWAPCHAIN_IMAGES`] each.
pub(crate) const GRAPHICS_OFFSET_VULKAN_WSI: usize = GRAPHICS_OFFSET_LAST_WAIT + 8;
pub(crate) const GRAPHICS_OFFSET_VULKAN_SURFACE: usize = GRAPHICS_OFFSET_VULKAN_WSI + 8;
pub(crate) const GRAPHICS_OFFSET_VULKAN_SURFACE_HWND: usize = GRAPHICS_OFFSET_VULKAN_SURFACE + 8;
pub(crate) const GRAPHICS_OFFSET_VULKAN_SWAPCHAIN: usize = GRAPHICS_OFFSET_VULKAN_SURFACE_HWND + 8;
pub(crate) const GRAPHICS_OFFSET_VULKAN_SC_W: usize = GRAPHICS_OFFSET_VULKAN_SWAPCHAIN + 8;
pub(crate) const GRAPHICS_OFFSET_VULKAN_SC_H: usize = GRAPHICS_OFFSET_VULKAN_SC_W + 8;
pub(crate) const GRAPHICS_OFFSET_VULKAN_SC_COUNT: usize = GRAPHICS_OFFSET_VULKAN_SC_H + 8;
pub(crate) const GRAPHICS_OFFSET_VULKAN_SC_STALE: usize = GRAPHICS_OFFSET_VULKAN_SC_COUNT + 8;
pub(crate) const GRAPHICS_OFFSET_VULKAN_SC_FAILED: usize = GRAPHICS_OFFSET_VULKAN_SC_STALE + 8;
pub(crate) const GRAPHICS_OFFSET_VULKAN_PRESENT_PASS: usize = GRAPHICS_OFFSET_VULKAN_SC_FAILED + 8;
pub(crate) const GRAPHICS_OFFSET_VULKAN_SEM_ACQUIRE: usize =
    GRAPHICS_OFFSET_VULKAN_PRESENT_PASS + 8;
pub(crate) const GRAPHICS_OFFSET_VULKAN_SEM_RENDER: usize = GRAPHICS_OFFSET_VULKAN_SEM_ACQUIRE + 8;
/// The most swapchain images the present path tracks; a surface asking for more is
/// given this many (`minImageCount + 1`, capped).
pub(crate) const VULKAN_MAX_SWAPCHAIN_IMAGES: usize = 4;
pub(crate) const GRAPHICS_OFFSET_VULKAN_SC_IMAGES: usize = GRAPHICS_OFFSET_VULKAN_SEM_RENDER + 8;
pub(crate) const GRAPHICS_OFFSET_VULKAN_SC_VIEWS: usize =
    GRAPHICS_OFFSET_VULKAN_SC_IMAGES + VULKAN_MAX_SWAPCHAIN_IMAGES * 8;
pub(crate) const GRAPHICS_OFFSET_VULKAN_SC_FRAMEBUFFERS: usize =
    GRAPHICS_OFFSET_VULKAN_SC_VIEWS + VULKAN_MAX_SWAPCHAIN_IMAGES * 8;
/// bug-765: what a Vulkan frame of several passes keeps between them, **graphics
/// thread only** like the swapchain words above. A frame of one pass reads none of them.
///
/// * `LOAD_PASS` — the offscreen render pass with `loadOp = LOAD`, entered and left in
///   `TRANSFER_SRC_OPTIMAL` (where `RENDER_PASS` leaves the image): what every pass
///   after a frame's first records into. Made on the first frame that needs it.
/// * `PRESENT_LOAD_PASS` — its swapchain twin, entered and left in `PRESENT_SRC_KHR`
///   (where `PRESENT_PASS` leaves the image).
/// * `SC_HELD` — one more than the index of the swapchain image a frame's first pass
///   acquired, while a later pass of that frame is still to come; 0 otherwise. The
///   image is acquired once a frame and handed to the queue once, by the last pass.
///
/// Both passes differ from the ones the pipelines were built against only in load
/// operation and layouts, which render pass compatibility does not look at, so no
/// pipeline is rebuilt.
pub(crate) const GRAPHICS_OFFSET_VULKAN_LOAD_PASS: usize =
    GRAPHICS_OFFSET_VULKAN_SC_FRAMEBUFFERS + VULKAN_MAX_SWAPCHAIN_IMAGES * 8;
pub(crate) const GRAPHICS_OFFSET_VULKAN_PRESENT_LOAD_PASS: usize =
    GRAPHICS_OFFSET_VULKAN_LOAD_PASS + 8;
pub(crate) const GRAPHICS_OFFSET_VULKAN_SC_HELD: usize =
    GRAPHICS_OFFSET_VULKAN_PRESENT_LOAD_PASS + 8;
/// plan-172-J: where the pointer is, published by each platform's MAIN thread and read
/// by the graphics thread — the worker never touches these.
///
/// * `POINTER_X` / `POINTER_Y` — surface pixels, the units an SGR mouse report carries.
///   Meaningless while `POINTER_INSIDE` is 0.
/// * `POINTER_BUTTONS` — bit 0 left, bit 1 middle, bit 2 right, set while held.
/// * `POINTER_INSIDE` — 1 while the pointer is over the surface.
/// * `POINTER_SEQ` — the sequence word of a sequence lock over the other four: odd
///   while a publish is writing them, bumped to even when it is done
///   ([`emit_publish_pointer`]). A reader takes the four only between two equal, even
///   reads of it (`canvas::pointerSnapshot`).
///
/// A sequence word rather than the render mutex, for the reason
/// [`emit_publish_surface_size`] takes no lock: a pointer move is a main-thread
/// callback, and it must not wait behind a frame. Unlike the size, a torn read here is
/// not harmless — an `x` from one move with an `inside` from another is a position the
/// pointer never had — so the four are read as one.
pub(crate) const GRAPHICS_OFFSET_POINTER_SEQ: usize = GRAPHICS_OFFSET_VULKAN_SC_HELD + 8;
pub(crate) const GRAPHICS_OFFSET_POINTER_X: usize = GRAPHICS_OFFSET_POINTER_SEQ + 8;
pub(crate) const GRAPHICS_OFFSET_POINTER_Y: usize = GRAPHICS_OFFSET_POINTER_X + 8;
pub(crate) const GRAPHICS_OFFSET_POINTER_BUTTONS: usize = GRAPHICS_OFFSET_POINTER_Y + 8;
pub(crate) const GRAPHICS_OFFSET_POINTER_INSIDE: usize = GRAPHICS_OFFSET_POINTER_BUTTONS + 8;
/// The four published words, in the order `canvas::pointerSnapshot` returns them.
pub(crate) const GRAPHICS_POINTER_WORDS: [usize; 4] = [
    GRAPHICS_OFFSET_POINTER_X,
    GRAPHICS_OFFSET_POINTER_Y,
    GRAPHICS_OFFSET_POINTER_BUTTONS,
    GRAPHICS_OFFSET_POINTER_INSIDE,
];
/// plan-172-K: 1 while the scene the graphics thread last drew holds a
/// `canvas::AutoPaint`. Written by the graphics thread at the end of each frame's
/// resolve (`canvas::autoPublish`), read by each platform's MAIN thread after it
/// publishes the pointer: only then is a pointer change a redraw
/// ([`emit_skip_unless_autopaint_live`]), so a scene with no `AutoPaint` costs no frame
/// however the pointer moves.
///
/// One plain word, no lock and no sequence: it is a hint in both directions. Read
/// stale as 1, main signals a frame that resolves nothing and is skipped. Read stale
/// as 0 across the frame that first sets it, main skips a signal — which is why the
/// resolve re-reads the pointer after setting it (`helper_autopaint.rs`).
pub(crate) const GRAPHICS_OFFSET_AUTOPAINT_LIVE: usize = GRAPHICS_OFFSET_POINTER_INSIDE + 8;
/// plan-172-K: 1 while a pointer change is waiting to be looked at — the pointer's own
/// `PENDING`. Set by `canvas::signalRedraw(TRUE)` and cleared by the render loop's wait,
/// both under the graphics mutex.
///
/// Its own word, and not `PENDING`, because the two ask for different things. `PENDING`
/// is a redraw that is OWED: the scene, the size or an image's pixels changed, and the
/// frame is drawn whatever it finds. This one is a question — "does any `AutoPaint` show
/// a different paint now?" — and a frame woken by it alone that answers no is not
/// drawn. Folding the two together would either draw a frame for every pointer move
/// over nothing, or let a pointer move swallow a `present`.
pub(crate) const GRAPHICS_OFFSET_AUTO_WAKE: usize = GRAPHICS_OFFSET_AUTOPAINT_LIVE + 8;
/// plan-172-K: how long a `canvas::AutoPaint` holds its enter and exit paints, in
/// milliseconds — the project's `options.canvasAutoPaintMS`. The one word of the block
/// that does not start as zero: [`graphics_state_data_object`] writes it into the
/// executable, nothing writes it afterwards, and `canvas::autoWindowMs` reads it.
///
/// In the block because it is the one piece of canvas data every thread can already
/// find, and a constant baked into the data is a value the companion source can read
/// without the build having to rewrite that source per project.
pub(crate) const GRAPHICS_OFFSET_AUTO_WINDOW_MS: usize = GRAPHICS_OFFSET_AUTO_WAKE + 8;
/// Total block size.
pub(crate) const GRAPHICS_STATE_SIZE: usize = GRAPHICS_OFFSET_AUTO_WINDOW_MS + 8;

/// What `canvas::waitForRedraw` answers: the loop is stopping; a redraw is owed; or
/// (plan-172-K) nothing is owed but the pointer changed or an `AutoPaint` window ended.
pub(crate) const WAKE_STOP: &str = "0";
pub(crate) const WAKE_REDRAW: &str = "1";
pub(crate) const WAKE_AUTO_ONLY: &str = "2";
