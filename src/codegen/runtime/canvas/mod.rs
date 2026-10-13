//! The canvas graphics thread (plan-98-D Phase 2).
//!
//! One extra thread per canvas program, spawned lazily on the first `present`, which
//! owns the render loop. `canvas::present` stops rendering inline and instead
//! publishes the scene and signals; the graphics thread wakes, renders, and blits.
//!
//! `.ai/canvas-threading.md` is the normative protocol; this module implements §1
//! (the thread), §4 trigger 1 (a scene was published) and the redraw condition the
//! other triggers will also use.
//!
//! ## Why the thread runs MFBASIC
//!
//! The rasteriser is MFBASIC source (plan-98-C), which means the graphics thread
//! needs a real MFB execution context: an arena-state block for its allocations and
//! its module globals — including the sRGB transfer table `__COLOR_SRGB` (which
//! plan-122-B moved out of canvas into the `color` package, where it backs
//! `color::toLinear`), whose absence would silently render every antialiased pixel
//! black. It gets both the same way a `thread::start`
//! worker does: the spawner allocates and zeroes a child arena-state block, and the
//! trampoline runs the module's `LINK` and global initializers on the new thread
//! before entering the loop.
//!
//! Its globals are therefore **its own** — in particular its own geometry cache,
//! which is exactly right: the cache is renderer state, and the worker has no
//! business in it.
//!
//! ## Why the sync primitives are spelled `pthread_*`
//!
//! `emit_thread_external_call` translates each POSIX primitive to its Win32
//! equivalent (SRWLOCK / CONDITION_VARIABLE, both pointer-sized and valid when
//! zeroed), so the mutex and condition code below is written once and works on all
//! three families.

pub(crate) mod curve_shader;
pub(crate) mod metal;
pub(crate) mod vulkan;
pub(crate) mod vulkan_handoff;

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::{Operand, VirtualRegister};
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::runtime::thread::{emit_thread_external_call, thread_symbol};
use crate::target::shared::abi;
use std::collections::HashMap;

/// Scratch registers for the graphics emitters, allocated by the caller.
///
/// The caller must own the numbering: a fresh `Vregs::new()` starts at `%v0`, and
/// inside a `CodeBuilder`-managed body that is a register the builder has already
/// handed out. The two streams then overwrite each other's values — a corruption
/// with no local symptom, which cost an afternoon of chasing a libmalloc abort. So
/// the scratch is taken from whichever allocator owns the function being emitted
/// into, and taken *before* the call so the borrow does not outlive it.
pub(crate) struct GraphicsScratch {
    base: VirtualRegister,
    scratch: VirtualRegister,
    verdict: VirtualRegister,
    arena: VirtualRegister,
    cursor: VirtualRegister,
    end: VirtualRegister,
}

impl GraphicsScratch {
    pub(crate) fn new(vreg: &mut dyn FnMut() -> VirtualRegister) -> Self {
        GraphicsScratch {
            base: vreg(),
            scratch: vreg(),
            verdict: vreg(),
            arena: vreg(),
            cursor: vreg(),
            end: vreg(),
        }
    }
}

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

/// The per-item parameter block both GPU backends push to their shaders.
///
/// **One contract, two emitters.** The block is byte-identical between the Metal
/// pipeline (an MSL `constant MfbItem&`) and the Vulkan one (a GLSL push-constant
/// block) — glslang's reflection reports exactly these offsets and this size — so
/// the shaders agree by construction rather than by two hand-maintained layouts
/// staying in step. Each backend emits the *stores* in its own IR flavour; the
/// *layout* is here, once.
///
/// Seventeen `ivec4`s, one of them holding the surface size. Every member is an `ivec4`
/// so the two shading languages' packing rules cannot disagree.
///
/// **The bound is the item buffer, not a push-constant range** (plan-116-A). Until
/// then the block rode `vkCmdPushConstants` / `setVertexBytes:` and 112 was chosen to
/// fit Vulkan's *guaranteed* 128-byte range — a hard ceiling, and the only guaranteed
/// one, so widening the block past it would have made the feature set depend on the
/// device. It now travels in `…_VULKAN_ITEM_BUFFER` (Metal:
/// `…_MTL_ITEM_BUFFER`), one record per drawn quad indexed by instance, so the only
/// limit left is capacity: `VULKAN_MAX_FRAME_ITEMS` records must fit
/// `VULKAN_ITEM_BUFFER_BYTES`, which is defined *from* this constant and therefore
/// cannot fall out of step.
///
/// What still constrains the value is **agreement between the two shading languages**,
/// and that is gated rather than trusted: glslang's reflection for the GLSL
/// `ItemBlock` must report `topLevelArrayStride` equal to this size with one member per
/// 16 bytes (`glslangValidator -V -q mfb_canvas.vert`; 208 with members at
/// 0/16/.../192 when plan-116-F added the gradient axis `ivec4`, 272 with the four
/// residual `ivec4`s at 208/224/240/256 since bug-735, 288 with plan-163-B's
/// `strokeStyle` at 272; plan-163-C's `clipBand` at 288 and `clipPath` at 304; plan-163-D's
/// `gradInv0`/`gradInv1` at 320/336 and `gradFocal` at 352), matching the
/// `ITEM_OFFSET_*` constants below one for one. `the_item_block_matches_the_std430_stride`
/// in `vulkan.rs` pins it. Widening the block means keeping every member `ivec4`-sized
/// so std430's stride stays equal to the size, then re-running that reflection.
pub(crate) const ITEM_BLOCK_SIZE: usize = 368;
/// Bounds `minX, minY, maxX, maxY`, 16.16 fixed point.
pub(crate) const ITEM_OFFSET_QUAD: usize = 0;
/// The shape parameters `p0..p3`, 16.16.
pub(crate) const ITEM_OFFSET_SHAPE: usize = 16;
/// Fill and stroke `RGBA`, whole 0–255 values.
pub(crate) const ITEM_OFFSET_FILL: usize = 32;
pub(crate) const ITEM_OFFSET_STROKE: usize = 48;
/// `kind`, `radius` (16.16), `strokeHalf` (16.16), `edgeCount`.
pub(crate) const ITEM_OFFSET_MISC: usize = 64;
/// A glyph's or picture's bitmap height (`ITEM_ARC_GLYPH_HEIGHT`, 0 otherwise), an
/// arc's `reflex` flag (`ITEM_ARC_REFLEX`), then the polygon's first-edge index into the
/// frame's edge region, then the cap tag. The first two words held the arc's 16.16
/// angles until bug-735.
///
/// The words are per-kind the same way `HEADER_AUX0` is: an arc never reads the
/// edge base and a polygon never reads the flag, so one `ivec4` carries both rather
/// than each taking its own. plan-163-A gives a polygon's copy of the flag word its
/// fill rule instead ([`emit_item_polygon_rule`]). Both backends carry the edge base since plan-116-A: each
/// polygon takes a slice of one buffer that serves the whole frame, and this is where
/// the offset travels.
pub(crate) const ITEM_OFFSET_ARC: usize = 80;
/// The word inside `ITEM_OFFSET_ARC` holding the polygon's first-edge index.
///
/// For a glyph (`GEO_KIND_TEXT`) the same word holds the first-*sample* index into the
/// buffer's glyph region, for exactly the same reason: Vulkan records one buffer for the
/// frame, so a per-draw offset is the only way each glyph can see its own bitmap.
pub(crate) const ITEM_ARC_EDGE_BASE: usize = 8;
/// The word inside `ITEM_OFFSET_ARC` holding a glyph's bitmap height.
///
/// A glyph reads neither arc angle, so `arc.x` carries the height beside `misc.w`'s
/// width — the same per-kind reuse the arc/polygon pair already makes of this block.
pub(crate) const ITEM_ARC_GLYPH_HEIGHT: usize = 0;
/// The word inside `ITEM_OFFSET_ARC` holding the `CapStyle` tag, 0 or 1 (plan-116-D).
///
/// The block's last free word, and the right one: only `Line` and `Arc` read it, and
/// they are the two kinds this block already serves — a polygon reads
/// `ITEM_ARC_EDGE_BASE`, a glyph reads `ITEM_ARC_GLYPH_HEIGHT`, and none of the three
/// uses overlaps another because no item is two kinds at once. Taking a word here is
/// what keeps `ITEM_BLOCK_SIZE` at 160 rather than growing every item in every scene
/// to carry one bit for two primitives.
pub(crate) const ITEM_ARC_CAP: usize = 12;
/// The surface's width and height, in whole pixels, then the blend mode.
pub(crate) const ITEM_OFFSET_SURFACE: usize = 96;
/// The word inside `ITEM_OFFSET_SURFACE` holding the `BlendMode` tag, 0..3
/// (plan-116-B).
///
/// It lands here rather than in a new `ivec4` because plan-116-A's audit found this
/// word free — the surface size needs two of the four, and the block is declared as a
/// full `ivec4` on both sides precisely so trailing padding cannot differ between the
/// languages. Using the space costs nothing and keeps the block one `ivec4` narrower.
pub(crate) const ITEM_SURFACE_BLEND: usize = 8;
/// The item's clip rectangle, resolved to `x0, y0, x1, y1` in 16.16 fixed point
/// (plan-116-B).
///
/// The `ivec4` that took the block from 112 to 128 bytes — which the push-constant
/// transport could not have afforded, since 128 is the whole guaranteed range and the
/// pipeline layout would have had nothing left. plan-116-A moved the block into a
/// buffer for exactly this.
///
/// A zero-area rectangle means **unclipped** and is recognised by `x0 >= x1 || y0 >=
/// y1`, the same test the geometry header uses, so the two agree by construction.
pub(crate) const ITEM_OFFSET_CLIP: usize = 112;
/// The inverted transform, as **raw IEEE-754 `float32` bits** in two `ivec4`s
/// (plan-116-C): `ia, ib, ic, id` at 128 and `itx, ity, hasTransform, unused` at 144.
///
/// **Not 16.16, unlike every other geometric field in the block.** An item scaled up
/// 100× has inverse terms near `0.01`, which 16.16 holds to about four significant
/// digits — a precision cliff exactly where a transform is doing the most work. Float32
/// has no such cliff, costs the same four bytes, needs no conversion on the CPU side
/// (the header slots are already `Float`), and is what the shader arithmetic uses
/// anyway. The shaders decode with `intBitsToFloat` / `as_type<float>`.
///
/// `hasTransform` is a whole 0 or 1, not a float bit pattern: it is compared, never
/// arithmetic.
pub(crate) const ITEM_OFFSET_TRANSFORM: usize = 128;
/// An arc's two sweep endpoints in 16.16 surface pixels — `startX, startY, endX, endY`
/// (plan-116-D).
///
/// The eleventh `ivec4`, and the one member of the block that had to grow it: the
/// per-kind `arc` block's last free word went to the cap tag, and four coordinates do
/// not fit in the two words left elsewhere. 160 → 176 bytes, which stays a multiple of
/// 16 so std430's array stride still equals the size —
/// `the_item_block_matches_the_std430_stride` is what pins that.
///
/// Every arc reads them since bug-735: a `Round` cap's two discs, and for every cap the
/// sweep test's direction vectors, which are the endpoints less the centre -- a positive
/// multiple of the oracle's Taylor `(cos, sin)`, so the shaders need no trigonometry at
/// all. Every item pays 16 bytes of buffer for it, which is the trade: the alternative
/// is a per-pixel `sin`/`cos` pair in three renderers, and the deterministic series the
/// oracle requires is far more expensive than a fetch. Their residuals are in
/// `ITEM_OFFSET_CAPS_LO`.
pub(crate) const ITEM_OFFSET_ARC_CAPS: usize = 160;
/// A **polygon**'s band-index header in the same `ivec4` (bug-686 on Metal, bug-688 on
/// Vulkan): `bandTop`, `bandHeight` (16.16, shape space), the band count (0 = loop over
/// every edge) and its table's word offset in the band region (`METAL_BAND_BASE_WORDS`,
/// `VULKAN_BAND_BASE_WORDS`).
///
/// The per-kind sharing `ITEM_OFFSET_ARC` already does: only an arc reads its caps and
/// only a polygon reads its bands, and no item is both. Each backend's `emit_band_index`
/// writes these after `emit_item_block`'s generic arc-caps store, for polygons only; the
/// shaders read them only when `misc.x` is the polygon kind.
pub(crate) const ITEM_BAND_TOP: usize = 0;
pub(crate) const ITEM_BAND_HEIGHT: usize = 4;
pub(crate) const ITEM_BAND_COUNT: usize = 8;
pub(crate) const ITEM_BAND_START: usize = 12;
/// An ellipse's rotation, as `cos, sin` in 16.16, then two unused words (plan-116-E).
///
/// The twelfth `ivec4`. Two words would have fit in an existing block's spare, but
/// std430 rounds the array stride to a multiple of 16 anyway, so a half-used `ivec4`
/// costs exactly what a full one does and keeps every member `ivec4`-sized — which is
/// the property `the_item_block_matches_the_std430_stride` exists to hold.
///
/// **16.16 alone was not enough here**, though this said it was: the fixed-point step
/// is an angular error up to 7.6e-6 rad, which at radius 26 moves the rim by 2e-4 px,
/// and a pixel within that of a coverage step rounds to the neighbouring step -- 3-9
/// sRGB levels near black (bug-735). Both words carry a residual in `ITEM_OFFSET_MISC_LO`
/// since. The CPU still evaluates the deterministic Taylor pair; this is only how the
/// answer travels.
pub(crate) const ITEM_OFFSET_ELLIPSE: usize = 176;
/// A gradient's axis in 16.16 — `startPoint.x, startPoint.y, endPoint.x, endPoint.y`
/// (plan-116-F).
///
/// The thirteenth `ivec4`, and the block had to grow for it: the `ellipse` block's two
/// spare words went to the stop count and base, and four coordinates do not fit in the
/// one word left elsewhere. The gradient's *kind* does fit there, and takes it —
/// `ITEM_SURFACE_GRADIENT_KIND`.
///
/// Growing the block moves Metal's hand-assigned frame and `METAL_EDGE_BASE` with it;
/// `.ai/canvas-threading.md` lists the five things that move and which three are
/// silent.
pub(crate) const ITEM_OFFSET_GRADIENT: usize = 192;
/// bug-735: the **residual words** of the fields a distance reads, one `ivec4` per
/// 16.16 `ivec4` they refine, each `round((v * 65536 - word) * 2^31)` in units of
/// 2^-47 px (`EDGE_RESIDUAL_SCALE`, the encoding bug-687 gave the polygon edges).
///
/// A field narrowed to 16.16 alone moves a distance by up to ~1.5e-5 px, and where the
/// oracle's f64 distance lies closer than that to a coverage quantisation step the GPU
/// rounds to the neighbouring one -- near black, 3-12 sRGB levels, past `GPU_DEFAULT`.
/// bug-735's C model showed the narrowing alone does it for rectangles and clips whose
/// edges sit near a step (the arithmetic there is nearly exact in fp32), so every field
/// a distance reads carries its residual and both shaders add it back in double-float:
///
/// * `shapeLo` (208) -- `shape.xyzw`: centres, half-extents, radii, line endpoints,
///   ellipse axes;
/// * `capsLo` (224) -- `arcCaps.xyzw`, an arc's sweep endpoints (a polygon's band
///   header there has none: its header slots are zero, so these are too);
/// * `clipLo` (240) -- `clip.xyzw`;
/// * `miscLo` (256) -- the residuals of `misc.y` (radius) and `misc.z` (stroke
///   half-width), then of `ellipse.x`/`.y` (the rotation's cosine and sine).
///
/// Four `ivec4`s at the block's end rather than a parallel region like the edges':
/// every item has these fields, so a per-item index would cost as much as the words.
pub(crate) const ITEM_OFFSET_SHAPE_LO: usize = 208;
pub(crate) const ITEM_OFFSET_CAPS_LO: usize = 224;
pub(crate) const ITEM_OFFSET_CLIP_LO: usize = 240;
pub(crate) const ITEM_OFFSET_MISC_LO: usize = 256;
/// plan-163-B: the stroke's style words -- the first edge slot of the item's join
/// records in the frame's edge region and how many records there are (each takes two
/// slots), then the first entry of its dash pattern in the frame's dash region and how
/// many entries. All four are 0 for an item that draws no join record and no dash.
pub(crate) const ITEM_OFFSET_STROKE_STYLE: usize = 272;
pub(crate) const ITEM_STROKE_JOIN_BASE: usize = 0;
pub(crate) const ITEM_STROKE_JOIN_COUNT: usize = 4;
pub(crate) const ITEM_STROKE_DASH_BASE: usize = 8;
pub(crate) const ITEM_STROKE_DASH_COUNT: usize = 12;
/// plan-163-C: a path clip's band header -- top, height, band count, table offset, the
/// shape of `arcCaps`'s polygon band header -- for the clip's own edges; band count 0
/// walks every clip edge.
pub(crate) const ITEM_OFFSET_CLIP_BAND: usize = 288;
/// plan-163-C: the clip's words -- its first edge in the frame's edge region and its edge
/// count (a path clip), its mode (0 none, 1 rectangle, 2 path) with its FillRule ordinal
/// shifted left 2, and a picture's transparency step T.
pub(crate) const ITEM_OFFSET_CLIP_PATH: usize = 304;
pub(crate) const ITEM_CLIP_PATH_BASE: usize = 0;
pub(crate) const ITEM_CLIP_PATH_COUNT: usize = 4;
pub(crate) const ITEM_CLIP_PATH_MODE: usize = 8;
pub(crate) const ITEM_CLIP_PATH_PICTURE_T: usize = 12;
/// plan-163-D: a radial gradient's own inverse transform -- `ia, ib, ic, id` then `itx,
/// ity` as float32 BIT PATTERNS and its flag (0 or 1), the `xform0`/`xform1` layout --
/// copied word for word from header slots 57-63 into `gradInv0` and `gradInv1`.
pub(crate) const ITEM_OFFSET_GRADIENT_INV: usize = 320;
/// plan-163-D: a radial gradient's focal offset from its centre and focal radius, 16.16
/// (`gradFocal`), already clamped by the builder.
pub(crate) const ITEM_OFFSET_GRADIENT_FOCAL: usize = 352;
/// The word inside `ITEM_OFFSET_ARC` holding an arc's `reflex` flag, 0 or 1 (bug-735):
/// `endAngle - startAngle > PI`, decided by the emitter in the oracle's own f64
/// (`__canvas_drawGeometry`), so the sweep test's union-or-intersection choice cannot
/// differ from the oracle's. It replaced the 16.16 end angle, which the shaders turned
/// into a sweep vector with GPU `cos`/`sin` -- transcendentals that do not match the
/// oracle's deterministic Taylor pair. The sweep vectors now come from the arc's cap
/// endpoints (`arcCaps` minus the centre, a positive multiple of `(cos, sin)`), so no
/// angle reaches either shader and `arc.x` is written 0 for every kind but a glyph or
/// picture, which put their bitmap height there.
///
/// For a polygon (kind 4) the same word is its `FillRule` ordinal, 0 EvenOdd or 1
/// NonZero (plan-163-A, [`emit_item_polygon_rule`]): only an arc reads it as the flag.
pub(crate) const ITEM_ARC_REFLEX: usize = 4;

/// bug-735: every item-block field a distance reads, as `(header slot, word offset,
/// residual offset)` -- offsets into the block. Both emitters narrow each one with
/// [`emit_item_residual_field`], so the Metal and Vulkan blocks carry the same residuals
/// by construction.
pub(crate) const ITEM_RESIDUAL_FIELDS: [(usize, usize, usize); 16] = [
    (HEADER_SHAPE, ITEM_OFFSET_SHAPE, ITEM_OFFSET_SHAPE_LO),
    (
        HEADER_SHAPE + 1,
        ITEM_OFFSET_SHAPE + 4,
        ITEM_OFFSET_SHAPE_LO + 4,
    ),
    (
        HEADER_SHAPE + 2,
        ITEM_OFFSET_SHAPE + 8,
        ITEM_OFFSET_SHAPE_LO + 8,
    ),
    (
        HEADER_SHAPE + 3,
        ITEM_OFFSET_SHAPE + 12,
        ITEM_OFFSET_SHAPE_LO + 12,
    ),
    (
        HEADER_CAP_START_X,
        ITEM_OFFSET_ARC_CAPS,
        ITEM_OFFSET_CAPS_LO,
    ),
    (
        HEADER_CAP_START_X + 1,
        ITEM_OFFSET_ARC_CAPS + 4,
        ITEM_OFFSET_CAPS_LO + 4,
    ),
    (
        HEADER_CAP_END_X,
        ITEM_OFFSET_ARC_CAPS + 8,
        ITEM_OFFSET_CAPS_LO + 8,
    ),
    (
        HEADER_CAP_END_X + 1,
        ITEM_OFFSET_ARC_CAPS + 12,
        ITEM_OFFSET_CAPS_LO + 12,
    ),
    (HEADER_CLIP_X0, ITEM_OFFSET_CLIP, ITEM_OFFSET_CLIP_LO),
    (
        HEADER_CLIP_Y0,
        ITEM_OFFSET_CLIP + 4,
        ITEM_OFFSET_CLIP_LO + 4,
    ),
    (
        HEADER_CLIP_X1,
        ITEM_OFFSET_CLIP + 8,
        ITEM_OFFSET_CLIP_LO + 8,
    ),
    (
        HEADER_CLIP_Y1,
        ITEM_OFFSET_CLIP + 12,
        ITEM_OFFSET_CLIP_LO + 12,
    ),
    (HEADER_RADIUS, ITEM_OFFSET_MISC + 4, ITEM_OFFSET_MISC_LO),
    (
        HEADER_STROKE_HALF,
        ITEM_OFFSET_MISC + 8,
        ITEM_OFFSET_MISC_LO + 4,
    ),
    (
        HEADER_ELLIPSE_COS,
        ITEM_OFFSET_ELLIPSE,
        ITEM_OFFSET_MISC_LO + 8,
    ),
    (
        HEADER_ELLIPSE_SIN,
        ITEM_OFFSET_ELLIPSE + 4,
        ITEM_OFFSET_MISC_LO + 12,
    ),
];

/// `PI` (`3.141592653589793`, the constant `__canvas_drawGeometry` compares an arc's
/// sweep against) as its IEEE-754 double bit pattern.
const PI_F64_BITS: &str = "4614256656552045848";

/// bug-735: narrow one header slot to its 16.16 word at `block + word_at` and its
/// residual `round((v * 65536 - word) * 2^31)` at `block + residual_at` (both offsets
/// from the stack pointer) -- the encoding bug-687 gave the polygon edges, for a field
/// of the item block. `v * 65536 - word` is exact: the two are within half a step.
///
/// Expects `FP_SCRATCH[0]` to hold 65536.0 (`FIXED_POINT_SCALE`) and `FP_SCRATCH[3]`
/// 2^31 (`EDGE_RESIDUAL_SCALE`); clobbers `FP_SCRATCH[1..=2]` and `SCRATCH[1]`. Plain
/// instructions, so the Metal `Asm` and the Vulkan `CodeBuilder` emit the same ones.
pub(crate) fn emit_item_residual_field(
    header: abi::Reg,
    slot: usize,
    word_at: usize,
    residual_at: usize,
) -> Vec<CodeInstruction> {
    vec![
        abi::load_double(abi::FP_SCRATCH[1], header, slot * 8),
        abi::float_multiply_d(abi::FP_SCRATCH[1], abi::FP_SCRATCH[1], abi::FP_SCRATCH[0]),
        abi::float_round_to_signed_x(abi::SCRATCH[1], abi::FP_SCRATCH[1]),
        abi::store_u32(abi::SCRATCH[1], abi::stack_pointer(), word_at),
        abi::signed_convert_to_float_d(abi::FP_SCRATCH[2], abi::SCRATCH[1]),
        abi::float_subtract_d(abi::FP_SCRATCH[1], abi::FP_SCRATCH[1], abi::FP_SCRATCH[2]),
        abi::float_multiply_d(abi::FP_SCRATCH[1], abi::FP_SCRATCH[1], abi::FP_SCRATCH[3]),
        abi::float_round_to_signed_x(abi::SCRATCH[1], abi::FP_SCRATCH[1]),
        abi::store_u32(abi::SCRATCH[1], abi::stack_pointer(), residual_at),
    ]
}

/// bug-735: make the stroke half-width's `(misc.z, miscLo.y)` pair positive exactly when
/// the header's f64 half-width is -- the test the software rasteriser strokes on
/// (`half > 0.0`) and `__canvas_blockInstances` splits a blended item on.
///
/// `__canvas_strokeHalf` answers -1 or `strokeWidth / 2`, so only a half-width under
/// 2^-48 px can narrow to a zero word AND a zero residual; this makes its residual 1
/// (2^-47 px). Without it the shaders and the emitters' split test would read it as "does
/// not stroke" while the oracle strokes it and the draw list counts two records for it.
/// `positive` and `done` are two fresh labels; clobbers `FP_SCRATCH[1..=2]` and
/// `SCRATCH[1]`. Runs after [`emit_item_residual_field`] wrote the pair.
pub(crate) fn emit_item_stroke_sign(
    header: abi::Reg,
    block_at: usize,
    positive: &str,
    done: &str,
) -> Vec<CodeInstruction> {
    let residual_at = block_at + ITEM_OFFSET_MISC_LO + 4;
    vec![
        abi::load_double(abi::FP_SCRATCH[1], header, HEADER_STROKE_HALF * 8),
        abi::move_immediate(abi::SCRATCH[1], "Integer", "0"),
        abi::signed_convert_to_float_d(abi::FP_SCRATCH[2], abi::SCRATCH[1]),
        abi::float_compare_d(abi::FP_SCRATCH[1], abi::FP_SCRATCH[2]),
        abi::branch_gt(positive),
        abi::branch(done),
        abi::label(positive),
        abi::load_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            block_at + ITEM_OFFSET_MISC + 8,
        ),
        abi::compare_immediate(abi::SCRATCH[1], "0"),
        abi::branch_ne(done),
        abi::load_u32(abi::SCRATCH[1], abi::stack_pointer(), residual_at),
        abi::compare_immediate(abi::SCRATCH[1], "0"),
        abi::branch_ne(done),
        abi::move_immediate(abi::SCRATCH[1], "Integer", "1"),
        abi::store_u32(abi::SCRATCH[1], abi::stack_pointer(), residual_at),
        abi::label(done),
    ]
}

/// bug-735: the item block's `arc.x` (0) and `arc.y` (`ITEM_ARC_REFLEX`) at
/// `block_at` from the stack pointer: `endAngle - startAngle > PI` in f64, the oracle's
/// own test, for every kind -- only an arc reads it. `set` and `done` are two fresh
/// labels. Clobbers `FP_SCRATCH[1..=2]` and `SCRATCH[1]`.
pub(crate) fn emit_item_arc_flags(
    header: abi::Reg,
    block_at: usize,
    set: &str,
    done: &str,
) -> Vec<CodeInstruction> {
    let reflex_at = block_at + ITEM_OFFSET_ARC + ITEM_ARC_REFLEX;
    vec![
        abi::move_immediate(abi::SCRATCH[1], "Integer", "0"),
        abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            block_at + ITEM_OFFSET_ARC,
        ),
        abi::store_u32(abi::SCRATCH[1], abi::stack_pointer(), reflex_at),
        abi::load_double(abi::FP_SCRATCH[1], header, HEADER_AUX1 * 8),
        abi::load_double(abi::FP_SCRATCH[2], header, HEADER_AUX0 * 8),
        abi::float_subtract_d(abi::FP_SCRATCH[1], abi::FP_SCRATCH[1], abi::FP_SCRATCH[2]),
        abi::move_immediate(abi::SCRATCH[1], "Integer", PI_F64_BITS),
        abi::float_move_d_from_x(abi::FP_SCRATCH[2], abi::SCRATCH[1]),
        abi::float_compare_d(abi::FP_SCRATCH[1], abi::FP_SCRATCH[2]),
        abi::branch_gt(set),
        abi::branch(done),
        abi::label(set),
        abi::move_immediate(abi::SCRATCH[1], "Integer", "1"),
        abi::store_u32(abi::SCRATCH[1], abi::stack_pointer(), reflex_at),
        abi::label(done),
    ]
}
/// `std::f64::consts::SQRT_2` as its IEEE-754 bit pattern, for `float_move_d_from_x`.
const SQRT_2_F64_BITS: u64 = 0x3FF6_A09E_667F_3BCD;

/// plan-163-B: widen a band reach's half-width in `half` to `half * sqrt(2)` when the
/// item's cap tag (`HEADER_CAP`) is Square (2): a square end's corners lie a diagonal
/// half-width from its endpoint, so an edge that far from a row can still cover a pixel
/// on it. Any other tag leaves `half` unchanged, so every other item's band index is
/// what it was. `skip` is a fresh label; clobbers `int_scratch` and `fp_scratch`.
pub(crate) fn emit_square_cap_reach(
    header: abi::Reg,
    half: abi::Reg,
    int_scratch: abi::Reg,
    fp_scratch: abi::Reg,
    skip: &str,
) -> Vec<CodeInstruction> {
    vec![
        abi::load_double(fp_scratch, header, HEADER_CAP * 8),
        abi::float_convert_to_signed_x(int_scratch, fp_scratch),
        abi::compare_immediate(int_scratch, "2"),
        abi::branch_ne(skip),
        abi::move_immediate(int_scratch, "Integer", &SQRT_2_F64_BITS.to_string()),
        abi::float_move_d_from_x(fp_scratch, int_scratch),
        abi::float_multiply_d(half, half, fp_scratch),
        abi::label(skip),
    ]
}

/// plan-163-A: a polygon's or a curve polygon's `FillRule` ordinal
/// (`HEADER_POLYGON_RULE`, 0 EvenOdd, 1 NonZero) into `arc.y` (`ITEM_ARC_REFLEX`) of the item block at `block_at` from the
/// stack pointer. Only an arc reads `arc.y` as its reflex flag, so a polygon's copy of
/// the word is free; call this AFTER [`emit_item_arc_flags`], which writes the flag for
/// every kind. Any other kind is skipped and keeps the flag. Stroked text is a polygon
/// with slot 21 at 0, so it reads EvenOdd. `skip` is a fresh label. Clobbers
/// `FP_SCRATCH[1]` and `SCRATCH[1]`.
pub(crate) fn emit_item_polygon_rule(
    header: abi::Reg,
    block_at: usize,
    skip: &str,
) -> Vec<CodeInstruction> {
    // plan-164-C: a `CurvePolygon` keeps its rule in the same slot and reads it from the
    // same word (`curveWalk`).
    let has_rule = format!("{skip}_has");
    vec![
        abi::load_double(abi::FP_SCRATCH[1], header, HEADER_KIND * 8),
        abi::float_convert_to_signed_x(abi::SCRATCH[1], abi::FP_SCRATCH[1]),
        abi::compare_immediate(abi::SCRATCH[1], GEO_KIND_POLYGON),
        abi::branch_eq(&has_rule),
        abi::compare_immediate(abi::SCRATCH[1], GEO_KIND_CURVEPOLYGON),
        abi::branch_ne(skip),
        abi::label(&has_rule),
        abi::load_double(abi::FP_SCRATCH[1], header, HEADER_POLYGON_RULE * 8),
        abi::float_convert_to_signed_x(abi::SCRATCH[1], abi::FP_SCRATCH[1]),
        abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            block_at + ITEM_OFFSET_ARC + ITEM_ARC_REFLEX,
        ),
        abi::label(skip),
    ]
}
/// The gradient's kind (0 linear, 1 radial) in `surface.w`, which was the block's last
/// spare word -- since plan-163-D packed with the spread and the stroke apply as `kind |
/// spread << 1 | apply << 3` (spread 0 Pad, 1 Reflect, 2 Repeat), which the shaders
/// unpack with `& 1`, `>> 1 & 3` and `& 8`.
pub(crate) const ITEM_SURFACE_GRADIENT_KIND: usize = 12;
/// plan-163-D: `__CANVAS_GEO_GRADIENT_SPREAD`, `_APPLY`, the radial inverse's first word
/// (`_INV`, six float32 bit patterns) and its flag (`_HAS_INV`), and the focal offset's x
/// (`_FOCALX`, then y and the focal radius).
pub(crate) const HEADER_GRADIENT_SPREAD: usize = 55;
pub(crate) const HEADER_GRADIENT_APPLY: usize = 56;
pub(crate) const HEADER_GRADIENT_INV: usize = 57;
pub(crate) const HEADER_GRADIENT_HAS_INV: usize = 63;
pub(crate) const HEADER_GRADIENT_FOCAL_X: usize = 64;
/// The gradient's axis in the geometry header — `startPoint.x` at 43, running to
/// `endPoint.y` at 46 — and its kind at 42.
pub(crate) const HEADER_GRADIENT_KIND: usize = 42;
pub(crate) const HEADER_GRADIENT_FROM_X: usize = 43;
/// An ellipse's rotation in the geometry header, as its cosine then its sine
/// (plan-116-E). Evaluated once per ellipse by `__canvas_ellipseHeader` with the
/// deterministic Taylor pair, so all three renderers read the same two numbers.
/// The gradient's stop count in the geometry header, 0 when there is none
/// (plan-116-F). The header also carries the kind and the two points at 42–46; the
/// emitters need only the count, because the shader reads the rest out of the block.
pub(crate) const HEADER_GRADIENT_COUNT: usize = 41;
/// The gradient's stop count and first-stop index, in the item block's `ellipse`
/// `ivec4` (plan-116-F).
///
/// Words 2 and 3 of that block were spare: an ellipse reads only `cos` and `sin` from
/// it, and no item is an ellipse *and* something else. Reusing them is the same
/// per-kind sharing `ITEM_OFFSET_ARC` already does three ways (edge base, glyph
/// height, cap), and it keeps `ITEM_BLOCK_SIZE` at 192 — which matters more than it
/// looks, because growing the block shifts Metal's whole hand-assigned frame and
/// `METAL_EDGE_BASE` with it (`.ai/canvas-threading.md`).
pub(crate) const ITEM_ELLIPSE_GRADIENT_COUNT: usize = 8;
pub(crate) const ITEM_ELLIPSE_GRADIENT_BASE: usize = 12;
pub(crate) const HEADER_ELLIPSE_COS: usize = 39;
pub(crate) const HEADER_ELLIPSE_SIN: usize = 40;
// The `hasTransform` flag is the seventh word from `ITEM_OFFSET_TRANSFORM`, written
// positionally by the same loop that writes the six terms — a named offset for it
// would be a second way to say where it lives, and one of the two would eventually be
// wrong. The shaders read it as `xform1.z`, which the struct comment records.

/// `__CANVAS_GEO_POLYGON` — the one geometry kind whose payload does not fit in the
/// item block, so both backends have to test for it by hand.
///
/// The rest of the dispatch happens inside the shaders, which read `misc.x`; this is
/// here because the *emitters* branch on it too, to decide whether to build an edge
/// payload at all. Spelled once so a renumbering in `helper_geometry.rs`'s
/// `__CANVAS_GEO_POLYGON` cannot leave two backends disagreeing with the source of
/// truth in different ways.
pub(crate) const GEO_KIND_POLYGON: &str = "4";

/// `__CANVAS_GEO_TEXT` — a glyph run, the other kind whose payload does not fit in the
/// item block, and the only one that is not *one* draw.
///
/// A run's tail is `(cacheEntry, penX, penY)` per glyph, and each glyph is its own quad
/// with its own coverage bitmap, so a text item becomes N draws rather than one. Both
/// emitters therefore branch on this before they build an item block at all.
pub(crate) const GEO_KIND_TEXT: &str = "6";

/// A `canvas::Group` node's kind (plan-116-G).
///
/// A group draws nothing itself, so this is not a shape the renderers dispatch on —
/// it is what lets a group node be told apart from `GEO_KIND_NONE`, which means "an
/// item with no geometry". Spelled here and once in MFBASIC as `__CANVAS_GEO_GROUP`,
/// with `the_geo_layout_constants_match_their_rust_counterparts` as the only thing
/// relating the two.
///
/// `#[cfg(test)]` because that guard is genuinely its only consumer right now: no
/// emitter writes kind 8 yet, since a Phase 2 group node still carries an empty
/// header. That is not a placeholder — the pin is the whole reason the Rust spelling
/// exists at this point — and it is self-correcting: the phase that teaches an emitter
/// about groups cannot compile until the attribute comes off.
#[cfg(test)]
pub(crate) const GEO_KIND_GROUP: &str = "8";
/// `__CANVAS_GEO_PICTURE` — a `canvas::Picture` (bug-484).
///
/// To the shaders a rectangle whose fill colour is sampled from an image; to the
/// emitters the third kind with a payload, after the polygon's edges and the glyph's
/// coverage. Its texels ride the frame buffer's **glyph region**, one packed RGBA word
/// per texel (`r | g << 8 | b << 16 | a << 24`, the shadow's own byte order), and the
/// block names its slice exactly as a glyph's does: width in `misc.w`, height in
/// `arc.x` (`ITEM_ARC_GLYPH_HEIGHT`), base in `arc.z` (`ITEM_ARC_EDGE_BASE`). Sharing
/// the glyph region rather than adding a fourth means sharing its cursor and its frame
/// cap, and the predicates count a picture's texels against that cap.
pub(crate) const GEO_KIND_PICTURE: &str = "9";
/// `__CANVAS_GEO_POLYLINE` — a `canvas::PolyLine` (plan-154-A).
///
/// Its tail is the polygon edge layout (without the wrap edge) followed by its per-point
/// colours as gradient stops, so an emitter that uploads it needs the polygon's edge path
/// and the gradient's stop path, and nothing new.
///
/// Spelled here and once in MFBASIC; `the_geo_layout_constants_match_their_rust_counterparts`
/// holds the two equal.
pub(crate) const GEO_KIND_POLYLINE: &str = "10";
/// `__CANVAS_GEO_POLYCURVE` and `__CANVAS_GEO_CURVEPOLYGON` — a `canvas::PolyCurve` and
/// a `canvas::CurvePolygon` (plan-164-A). Two kinds over one tail of rational-quadratic
/// pieces, as `POLYLINE` and `POLYGON` are two kinds over one edge layout: the kind says
/// whether there is an interior.
///
/// Spelled here and once in MFBASIC;
/// `the_geo_layout_constants_match_their_rust_counterparts` holds the two equal.
pub(crate) const GEO_KIND_POLYCURVE: &str = "11";
pub(crate) const GEO_KIND_CURVEPOLYGON: &str = "12";
/// The doubles one piece of a curve record's tail occupies: `x0, y0, x1, y1, x2, y2,
/// w1` (`__CANVAS_CURVE_PIECE` in `builtins/canvas/helper_curves.rs`, which
/// `canvas_curve_piece_width_is_pinned` holds equal to this).
///
/// On both GPUs a piece is TWO slots of the frame's edge region (plan-164-C): its
/// chord `x0, y0, x2, y2` among the item's first `count` slots, where the band index
/// reads it as the edge it is in y; and `x1, y1, w1, 0` in the `count` slots after.
pub(crate) const CURVE_PIECE_SLOTS: usize = 7;
/// A polyline's first and last segments with length, in the header's gradient axis
/// slots (`__CANVAS_GEO_POLYLINE_FIRST` / `_LAST`), which a polyline does not otherwise
/// use.
///
/// The emitters copy them into the item block's `gradient.x`/`.y` as **whole numbers**,
/// not the 16.16 the axis is written as: 16.16 holds a segment index only up to 32,767,
/// and a plotted path can have more segments than that. The butt planes cut only these
/// two segments, each on its outer end.
pub(crate) const HEADER_POLYLINE_FIRST: usize = 43;
pub(crate) const HEADER_POLYLINE_LAST: usize = 44;
/// The header slots holding a picture's pixel-block address, high and low 24 bits.
///
/// Split because every header slot is hashed through `__canvas_hashFloat`, which
/// scales by 65536 before `toInt` — a whole 48-bit address overflows it. The emitters
/// rebuild the address as `hi << PICTURE_SHADOW_SPLIT_BITS | lo`; the block is a
/// `List OF Byte`, so the texels start `COLLECTION_HEADER_SIZE` bytes in.
pub(crate) const HEADER_PICTURE_SHADOW_HI: usize = 35;
pub(crate) const HEADER_PICTURE_SHADOW_LO: usize = 36;
pub(crate) const PICTURE_SHADOW_SPLIT_BITS: usize = 24;
/// Floats per glyph in a `__CANVAS_GEO_TEXT` tail: `cacheEntry, penX, penY`.
pub(crate) const GLYPH_RUN_SLOTS: usize = 3;
/// Integers per `__CANVAS_GLYPH_META` entry: `x0, y0, w, h, covStart`.
///
/// `x0`/`y0` are the bitmap's offset from the pen, which is what lets the same bitmap
/// serve the same glyph wherever it lands; `covStart` indexes `__CANVAS_GLYPH_COV`.
pub(crate) const GLYPH_META_SLOTS: usize = 5;
pub(crate) const GLYPH_META_X0: usize = 0;
pub(crate) const GLYPH_META_Y0: usize = 1;
pub(crate) const GLYPH_META_W: usize = 2;
pub(crate) const GLYPH_META_H: usize = 3;
pub(crate) const GLYPH_META_START: usize = 4;

/// 16.16 fixed point: the scale both shaders divide positions by.
///
/// Positions narrow to fixed point on the CPU because the geometry header is
/// `Float` (IEEE double), neither shading language has a double, and the AArch64
/// assembler has no double→single convert and no 32-bit FP store. 16.16 covers
/// ±32768 px at 1/65536 px — finer than `float`'s own resolution above 512 px, over
/// a coordinate space a few thousand pixels wide.
pub(crate) const FIXED_POINT_SCALE: &str = "65536";

/// The scale of a polygon vertex's **residual word** (bug-687): `2^31` units of the
/// 16.16 step, so the residual is in units of `2^-47` px.
///
/// 16.16 alone is not the oracle's polygon. Rounding each vertex to 1/65536 px moves a
/// distance by up to ~1e-5 px, and where the oracle's distance lies closer than that to
/// a coverage quantisation boundary the GPU rounds to the neighbouring step — near
/// black, 3-4 sRGB levels, past `GPU_DEFAULT` (bug-687's {301/97} star: 2.07e-6 px of
/// narrowing against a boundary 1.24e-6 px away). So every polygon edge endpoint also
/// carries `round((x * 65536 - word) * 2^31)`, which the shaders add back in
/// double-float: `x = word / 2^16 + residual / 2^47`, good to ~2^-41 px after the
/// shader's float conversion. The residual is at most half a 16.16 step, `2^30`
/// units, so it fits an `int` with room; it is computed with double multiplies,
/// subtracts and a round, the only float operations every assembler has.
pub(crate) const EDGE_RESIDUAL_SCALE: &str = "2147483648";

/// Slots of `__canvas_headerFor`'s fixed 51-float geometry header that the GPU
/// backends read. The software rasteriser indexes the same layout.
pub(crate) const HEADER_KIND: usize = 0;
pub(crate) const HEADER_SHAPE: usize = 2;
pub(crate) const HEADER_RADIUS: usize = 6;
pub(crate) const HEADER_STROKE_HALF: usize = 7;
pub(crate) const HEADER_FILL_R: usize = 8;
pub(crate) const HEADER_STROKE_R: usize = 12;
pub(crate) const HEADER_BOUNDS: usize = 16;
/// Slot 20 is the polygon's edge count *and* the arc's start angle — the header
/// reuses it per kind, and so does the item block. Writing both unconditionally is
/// cheaper than branching and can never be wrong: a shader reads only the one its
/// `kind` selects.
pub(crate) const HEADER_AUX0: usize = 20;
pub(crate) const HEADER_AUX1: usize = 21;
/// A polygon's `canvas::FillRule` ordinal (0 EvenOdd, 1 NonZero) — `HEADER_AUX1`
/// under its polygon meaning (plan-163-A, `__CANVAS_GEO_POLYGON_RULE`). Stroked text
/// is a polygon too and leaves it 0.
pub(crate) const HEADER_POLYGON_RULE: usize = HEADER_AUX1;
/// The item's clip rectangle, **resolved** to `x0, y0, x1, y1` in surface pixels
/// (plan-116-B).
///
/// Resolved rather than `x/y/w/h` so neither the rasteriser nor either shader repeats
/// the addition — the clip is tested once per covered pixel on the boundary and once
/// per item everywhere else, and `x + w` is not a term worth recomputing there.
///
/// A zero-area clip means **no clipping**, which is what an unset `Paint.clip` reads
/// as (`canvas::Paint.clip`'s description). It is stored as four zeros and recognised
/// by `x0 >= x1 OR y0 >= y1` — a test that also catches a caller passing a negative
/// extent, which `Bounds` cannot forbid.
pub(crate) const HEADER_CLIP_X0: usize = 22;
pub(crate) const HEADER_CLIP_Y0: usize = 23;
pub(crate) const HEADER_CLIP_X1: usize = 24;
pub(crate) const HEADER_CLIP_Y1: usize = 25;
/// The item's transform, **already inverted**, as `ia, ib, ic, id, itx, ity`
/// (plan-116-C).
///
/// Applied as `x' = ia*x + ic*y + itx`, `y' = ib*x + id*y + ity` — the same convention
/// `canvas::Transform` documents for the forward matrix.
///
/// Inverted **once, on the CPU, at header-build time**, because the renderers need
/// `T⁻¹` and nothing else: a shape is drawn by evaluating its distance field at the
/// inverse-mapped query point. Inverting per pixel, or per frame in each of three
/// renderers, would be the same arithmetic done between 1 and 10^6 times more often —
/// and would be three places for the all-zero-means-identity rule to live.
/// `__canvas_invertTransform` is that single place.
pub(crate) const HEADER_TRANSFORM_IA: usize = 27;
pub(crate) const HEADER_TRANSFORM_IB: usize = 28;
pub(crate) const HEADER_TRANSFORM_IC: usize = 29;
pub(crate) const HEADER_TRANSFORM_ID: usize = 30;
pub(crate) const HEADER_TRANSFORM_ITX: usize = 31;
pub(crate) const HEADER_TRANSFORM_ITY: usize = 32;
/// 1 when the item carries a transform that is not the identity, 0 otherwise
/// (plan-116-C).
///
/// A flag rather than six compares, because the per-pixel gate is what an *untransformed*
/// item pays — and that is every item in every scene written before this letter. It is
/// also what gates the two extra distance evaluations the gradient correction needs
/// (`ITEM_OFFSET_TRANSFORM`), which are the real cost.
pub(crate) const HEADER_HAS_TRANSFORM: usize = 33;
/// The item's `BlendMode`, as the enum tag 0..3 (plan-116-B).
///
/// `BlendMode.Normal` is 0, so an unset `Paint.blend` is the source-over behaviour
/// every scene had before this field was read — the zero value is the no-op, the same
/// rule the rest of `Paint` follows.
pub(crate) const HEADER_BLEND: usize = 26;
/// The item's `CapStyle`, as the enum tag 0..1 (plan-116-D).
///
/// Only `Line` and `Arc` write it; every other kind leaves it at the blank header's
/// zero. That is safe rather than merely unread — a kind with no ends never reaches a
/// cap branch in any of the three renderers.
///
/// **The zero is not "today's behaviour" for both variants**, which is the thing to
/// know here. A `Line` was round before this letter (`__canvas_segmentDistance` clamps
/// `t` to `0..1`) and an `Arc` was butt (the sweep test cuts it along a radius), so
/// preserving today's rendering meant splitting the existing sites: `Line` → `Round`,
/// `Arc` → `Butt`. `Butt` is still the enum's zero, because that is how the feature was
/// specified and a defaulted `cap` cannot arise — MFBASIC named construction requires
/// every field.
pub(crate) const HEADER_CAP: usize = 34;
/// An arc's two sweep endpoints in surface pixels, `startX, startY, endX, endY`
/// (plan-116-D).
///
/// Written by every arc and, on the GPU since bug-735, read by every arc (the sweep
/// test's direction vectors; the software rasteriser reads them for a `Round` cap
/// only), because a per-shape constant computed once at header-build time is the cheap
/// half of this letter: a cap disc centred on the endpoint would otherwise need a `sin`/`cos` pair
/// **per pixel** in each of three renderers, and `__canvas_cos`/`__canvas_sin` are the
/// deterministic Taylor series the oracle needs rather than libm. The arc header
/// already calls them for its sweep vectors, so this costs two multiply-adds each.
pub(crate) const HEADER_CAP_START_X: usize = 35;
pub(crate) const HEADER_CAP_END_X: usize = 37;
/// The fixed header length in slots — where a polygon's edge tail begins.
///
/// Spelled again in MFBASIC as `__CANVAS_GEO_HEADER` (`helper_geometry.rs`), with no
/// compiler between the two; `the_geo_layout_constants_match_their_rust_counterparts`
/// is what keeps them equal. Changing this without changing that one makes a polygon's
/// first edge coordinate read as a header field.
pub(crate) const HEADER_SLOTS: usize = 67;
/// plan-163-B: `__CANVAS_GEO_JOIN_COUNT` -- a stroke's join records in the tail, one
/// per outline edge -- and `__CANVAS_GEO_JOIN_REACH`, how far a record reaches from its
/// vertex, which a polygon's or polyline's band index widens to.
pub(crate) const HEADER_JOIN_COUNT: usize = 47;
pub(crate) const HEADER_JOIN_REACH: usize = 48;
/// plan-163-B: `__CANVAS_GEO_DASH_COUNT` -- the dash tail's floats, which sit between
/// the join records and the gradient stops -- and `__CANVAS_GEO_DASH_PHASE`, the dash
/// offset wrapped into one period, which the emitters upload as the first dash word.
pub(crate) const HEADER_DASH_COUNT: usize = 49;
pub(crate) const HEADER_DASH_PHASE: usize = 50;
/// plan-163-C: `__CANVAS_GEO_CLIP_EDGE_COUNT` -- a path clip's edges, five floats each,
/// which sit between the dash tail and the gradient stops.
pub(crate) const HEADER_CLIP_EDGE_COUNT: usize = 52;
/// plan-163-C: `__CANVAS_GEO_CLIP_MODE`, `__CANVAS_GEO_CLIP_RULE` and
/// `__CANVAS_GEO_PICTURE_TRANSPARENCY`, copied into the item block's `clipPath`.
pub(crate) const HEADER_CLIP_MODE: usize = 51;
pub(crate) const HEADER_CLIP_RULE: usize = 53;
pub(crate) const HEADER_PICTURE_TRANSPARENCY: usize = 54;
/// Doubles per cached polygon edge: `x0, y0, dx, dy, invLenSq`.
pub(crate) const EDGE_SLOTS: usize = 5;
/// The most edges one **pass** may carry on the Vulkan path — the size of the frame
/// buffer's edge region (bug-765: a frame past it is drawn as several passes).
///
/// Vulkan has no per-item limit — the edges live in a storage buffer, not in the
/// push constants — so the real constraint is the buffer, and it is a whole-buffer
/// one: every polygon of a pass takes a slice. `__canvas_passNext` sums the blocks'
/// edges (`__canvas_blockCost`) against this and ends the pass before the total
/// passes it, and a block past it alone sends the frame to software, so the emitter's
/// own bound check is unreachable.
///
/// 262,144 since bug-688 (it was 16,384, which `examples/weather`'s coastline alone
/// exceeds): Metal's number since bug-686, and 4 MiB of region. A large polygon no
/// longer costs every edge per covered pixel — the band index (`VULKAN_BAND_BASE_WORDS`)
/// has the shader walk only its own band's edges.
pub(crate) const VULKAN_MAX_FRAME_EDGES: usize = 262_144;
/// Four 16.16 words per edge — the two endpoints.
pub(crate) const VULKAN_EDGE_BYTES: usize = VULKAN_MAX_FRAME_EDGES * 16;
/// Four residual words per edge (bug-687, `EDGE_RESIDUAL_SCALE`), one per endpoint
/// coordinate, at the same edge index as its 16.16 words in the region after them.
pub(crate) const VULKAN_EDGE_RESIDUAL_BYTES: usize = VULKAN_MAX_FRAME_EDGES * 16;

/// The most **item blocks** one Vulkan frame may carry — the capacity of the item
/// buffer.
///
/// A *drawn quad*, not a scene item: every non-text item takes one block, and a glyph
/// run takes one per glyph, because each glyph is its own quad with its own block
/// (`GEO_KIND_TEXT`). So the count both predicates sum is "quads", and that is the
/// number this bounds.
///
/// It is the size of the frame BUFFER's item region, and so of one PASS (bug-765):
/// `__canvas_passNext` sums a frame's quads against it and ends a pass before the
/// total passes it. Only a block of more quads than this by itself (a `Text` of more
/// glyphs) sends the whole frame to software, for the reason `VULKAN_MAX_FRAME_EDGES`
/// gives: a truncated scene is a *different scene*, and software is the oracle, so
/// declining is never worse than drawing.
///
/// 65,536 since bug-688, Metal's `METAL_MAX_FRAME_ITEMS`: 4,096 (the old shared
/// `CANVAS_MAX_FRAME_ITEMS`) sent `examples/weather` (about 5,700 blocks), a 5,000-quad
/// scene and a terminal screen of text to software. 13 MiB of item buffer at the
/// current `ITEM_BLOCK_SIZE`, host-visible and mapped for the renderer's life.
pub(crate) const VULKAN_MAX_FRAME_ITEMS: usize = 65536;

/// The most **item blocks** one Metal frame may carry — `VULKAN_MAX_FRAME_ITEMS`'s
/// Metal twin (bug-686; the Vulkan cap was the shared `CANVAS_MAX_FRAME_ITEMS` until bug-688).
///
/// 4096 quads sent ordinary scenes to software: a terminal screen of text is one quad
/// per glyph, `examples/weather` publishes about 5,700 blocks a frame, and the 1,000-item
/// scenes bug-686 measured were a quarter of the way there. 65,536 blocks is 13 MiB of
/// item buffer at the current `ITEM_BLOCK_SIZE`, and every frame region after it moves
/// with it — which is why the shader's region bases are generated from these constants
/// rather than spelled in the MSL (`metal_shader_source`).
///
/// A frame past it is drawn as several passes (bug-765, `__canvas_passNext`); the
/// MFBASIC copy is generated from this constant (`helper_render.rs`).
pub(crate) const METAL_MAX_FRAME_ITEMS: usize = 65536;

/// How many 64-bit words one entry of the GPU draw list occupies.
///
/// The draw list is built in MFBASIC (`__canvas_pushOneDraw`) and walked by the native
/// emitters, so this width exists on both sides of a boundary the Rust compiler cannot
/// see across -- the MFBASIC side is a `&str`. It is a power of two because the emitter
/// addresses an entry with a shift rather than a multiply.
///
/// When the two sides disagreed (plan-116-H13: the emitter at eight words, the builtin
/// still appending four) the emitter strode 64 bytes through a 32-byte array. It then
/// read every *other* entry, and took the following entry's `base` as a blend mode --
/// which indexes the pipeline table out of range and hands Vulkan a junk `VkPipeline`.
/// That does not fail as a wrong picture: it SIGSEGVs inside the driver's JIT-compiled
/// code, with no MFBASIC frame anywhere in the backtrace.
///
/// plan-172-G: sixteen words. The first seven are what an entry always was — block base
/// and count, the translation as 16.16, the blend mode, the translation's residuals —
/// so a translate-only entry is its old eight words followed by zeros. Word 7 says
/// whether the entry carries a matrix, and words 8–11 are that matrix: the linear part
/// of the INVERSE of the enclosing groups' composed affine, as float32 bit patterns.
/// Word 12 is the pass word ([`CANVAS_DRAW_PASS_WORD`]); words 13–15 are reserved and
/// zero.
pub(crate) const CANVAS_DRAW_ENTRY_WORDS: usize = 16;

/// bug-765: the word of a draw list's FIRST entry that says where the list stands in
/// its frame, for a frame drawn as several passes (`__canvas_passNext`). An emitter is
/// handed one pass's blocks and entries exactly as it is handed a whole frame's, and
/// this word is the only thing that tells the two apart:
///
/// * [`CANVAS_DRAW_PASS_CONTINUES`] — a pass of this frame was drawn before this one.
///   The target holds it, so it is LOADED, not cleared, and the image a direct present
///   draws into is the one the first pass acquired.
/// * [`CANVAS_DRAW_PASS_MORE`] — a pass follows. This one is submitted and waited for
///   (the frame buffer is single-buffered, and the next pass refills it), but the frame
///   is neither presented nor read back.
///
/// Zero is a frame of one pass, which is every list `__canvas_pushOneDraw` and
/// `canvas::sceneDrawsFlat` lay out: they write 0 here, so such a frame is drawn by the
/// instructions that drew it before there were passes.
///
/// It rides the list because the list is what makes a pass a pass, and because the
/// seam's argument bank is full (`func_metal_draw.rs`). It is its own word for the
/// reason the blend mode is: a flag packed into the high bits of `count` survives
/// exactly until someone draws that many instances.
pub(crate) const CANVAS_DRAW_PASS_WORD: usize = 12;
pub(crate) const CANVAS_DRAW_PASS_CONTINUES: usize = 1;
pub(crate) const CANVAS_DRAW_PASS_MORE: usize = 2;

/// `log2(CANVAS_DRAW_ENTRY_WORDS * 8)` -- entry index to byte offset.
pub(crate) const CANVAS_DRAW_ENTRY_SHIFT: u32 = CANVAS_DRAW_ENTRY_WORDS.trailing_zeros() + 3;

/// `log2(CANVAS_DRAW_ENTRY_WORDS)` -- element count to entry count.
pub(crate) const CANVAS_DRAW_ENTRY_COUNT_SHIFT: u32 = CANVAS_DRAW_ENTRY_WORDS.trailing_zeros();

/// Byte offset of the blend mode within an entry (word 4).
pub(crate) const CANVAS_DRAW_ENTRY_MODE: usize = 32;
/// Vulkan's item buffer in bytes — one `ITEM_BLOCK_SIZE` record per quad, for
/// `VULKAN_MAX_FRAME_ITEMS` quads. A buffer of its own (binding 1), unlike Metal's item
/// region, which heads its one frame buffer.
pub(crate) const VULKAN_ITEM_BUFFER_BYTES: usize = VULKAN_MAX_FRAME_ITEMS * ITEM_BLOCK_SIZE;
/// Metal's item region in bytes — one `ITEM_BLOCK_SIZE` record per quad, for
/// `METAL_MAX_FRAME_ITEMS` quads. The first region of the Metal frame buffer.
pub(crate) const METAL_ITEM_BUFFER_BYTES: usize = METAL_MAX_FRAME_ITEMS * ITEM_BLOCK_SIZE;

/// The most edges one **frame** may carry on the Metal path, mirroring
/// `VULKAN_MAX_FRAME_EDGES` (plan-116-A).
///
/// Metal's edges used to ride a per-item `setFragmentBytes:` payload, which is copied
/// into the command buffer at record time — so the cap was per *item* (256 edges)
/// and there was no frame total at all. An instanced draw cannot rebind that payload
/// between instances, so the edges moved into a region of the frame buffer, exactly
/// where Vulkan has always kept them, and the cap became a frame total to match.
///
/// A Metal scene whose polygon edges sum past this is declined to software. It was
/// 16384 until bug-686, which `examples/weather`'s coastline alone exceeds; 262,144 edges
/// is 4 MiB of region, and a 40,000-edge frame measured within max channel delta 1 of
/// the software oracle (bug-686 spike C). Software is the oracle, so a declined frame
/// is at least as correct — truncating instead would draw a *different shape*.
///
/// There is **no per-polygon cap** since bug-686. The 256-edge one was kept "by policy"
/// after plan-116-A moved the edges into this region, and it existed twice — in the
/// predicate and as a branch in `emit_edge_buffer` that drew an over-long polygon as
/// nothing; spike B drew 300-4,000-edge polygons on Metal within max channel delta 1.
pub(crate) const METAL_MAX_FRAME_EDGES: usize = 262_144;
/// Where Metal's edge region starts inside the frame buffer, in 32-bit words.
///
/// The item blocks come first, so this is simply past them. The shader adds it to each
/// polygon's `ITEM_ARC_EDGE_BASE` rather than reading a separately-offset binding,
/// which is the same shape `VULKAN_GLYPH_BASE_WORDS` already uses — and it sidesteps
/// `MTLBuffer` offset alignment entirely, since nothing is ever bound at a non-zero
/// offset. `the_metal_shader_region_bases_match_the_buffer_layout` pins the number
/// against the copy inside the MSL string.
pub(crate) const METAL_EDGE_BASE_WORDS: usize = METAL_ITEM_BUFFER_BYTES / 4;
/// Where Metal's **edge residual** region starts, in 32-bit words — right after the
/// edges (bug-687). Edge `e`'s four residual words (`EDGE_RESIDUAL_SCALE`) sit at
/// `METAL_EDGE_RESIDUAL_BASE + e * 4`, parallel to its 16.16 words, so everything that
/// reads only the 16.16 words (the band builder, a polyline) is untouched.
pub(crate) const METAL_EDGE_RESIDUAL_BASE_WORDS: usize =
    METAL_EDGE_BASE_WORDS + METAL_MAX_FRAME_EDGES * 4;

/// A polygon's **band index** (bug-686): which of its edges a pixel at a given height
/// has to visit, so the fragment shader's `edgeDistance` stops looping over every edge
/// for every covered pixel — O(edges × pixels), 252 ms of GPU for one 64,000-edge
/// polygon (bug-686 section E).
///
/// `emit_band_index` splits the polygon's shape-space y-range into bands and puts each
/// edge in every band its `[ymin − r, ymax + r]` meets, r being the distance past which
/// an edge cannot change the pixel's coverage. The shader evaluates the SAME loop body
/// over its own band's list, so the result is bit-identical to the full loop (the
/// argument is at `emit_band_index`). Per polygon, at the word offset its block carries
/// in `ITEM_BAND_START`:
///
/// * `2 × bands` words of table — `(first, count)` per band, `first` relative to the
///   list that follows;
/// * the list — edge numbers, `0..edgeCount`, band after band.
///
/// The polygon's block carries `bandTop`, `bandHeight` (16.16, shape space), the band
/// count and that offset in its `arcCaps` `ivec4`, which only an arc reads
/// (`ITEM_BAND_*`). A band count
/// of 0 means "loop over every edge": what a polygon under `BAND_MIN_EDGES` gets,
/// and what any polygon gets whose index would not fit — correct, only slower. The
/// index is never truncated and a frame is never declined for it.
///
/// Budget: bands are at most the polygon's edge count, and the list at most
/// `BAND_LIST_PER_EDGE` entries per edge (a polygon over it gets taller bands), so
/// one polygon needs at most `BAND_WORDS_PER_EDGE` words per edge — and the frame's
/// edges are capped at `METAL_MAX_FRAME_EDGES` (`VULKAN_MAX_FRAME_EDGES`), so each
/// backend's band region (`METAL_MAX_FRAME_BAND_WORDS`, `VULKAN_MAX_FRAME_BAND_WORDS`)
/// always has room. Both backends build the same index (bug-688 ported it to Vulkan).
pub(crate) const BAND_LIST_PER_EDGE: usize = 6;
/// Table (two words per band, at most one band per edge) plus list, per edge.
pub(crate) const BAND_WORDS_PER_EDGE: usize = 2 + BAND_LIST_PER_EDGE;
/// The band region's size in 32-bit words.
pub(crate) const METAL_MAX_FRAME_BAND_WORDS: usize = BAND_WORDS_PER_EDGE * METAL_MAX_FRAME_EDGES;
/// The most bands one polygon is split into. Past ~1,000 the table costs more to build
/// than a thinner list saves; a 600 px polygon gets 2-px bands up to here.
pub(crate) const BAND_MAX: usize = 1024;
/// Polygons with fewer edges than this loop over all of them (band count 0): the full
/// loop over a handful of edges costs less than finding the band.
pub(crate) const BAND_MIN_EDGES: usize = 32;
/// Where Metal's band region starts, in 32-bit words — right after the edges it indexes
/// and their residuals (bug-687).
pub(crate) const METAL_BAND_BASE_WORDS: usize =
    METAL_EDGE_RESIDUAL_BASE_WORDS + METAL_MAX_FRAME_EDGES * 4;
/// The most gradient stops one **Vulkan** frame may carry (plan-116-F; bug-688).
///
/// Two stops per quad of `VULKAN_MAX_FRAME_ITEMS`, as Metal's is per quad of its own:
/// a gradient has at least two, so a frame of nothing but simple gradient-filled shapes
/// is declined by the item cap before this one. It was 4,096 on both backends until
/// bug-686 found 2,100 two-stop gradient rectangles — an ordinary chart — declined by it.
pub(crate) const VULKAN_MAX_FRAME_GRADIENT_STOPS: usize = 2 * VULKAN_MAX_FRAME_ITEMS;
/// The most gradient stops one **Metal** frame may carry (bug-686).
///
/// Two stops per quad of `METAL_MAX_FRAME_ITEMS`: a gradient has at least two, so this
/// is the first cap at which a frame of nothing but simple gradient-filled shapes is
/// declined by the item cap rather than by this one. 2.5 MiB of region.
pub(crate) const METAL_MAX_FRAME_GRADIENT_STOPS: usize = 2 * METAL_MAX_FRAME_ITEMS;
/// Five 32-bit words a stop: offset, then the four colour channels.
pub(crate) const GRADIENT_STOP_WORDS: usize = 5;
/// Where Metal's gradient region starts, in 32-bit words — after the items, edges and
/// band index.
pub(crate) const METAL_GRADIENT_BASE_WORDS: usize =
    METAL_BAND_BASE_WORDS + METAL_MAX_FRAME_BAND_WORDS;
/// The most coverage samples one **frame**'s glyphs may carry on the Metal path — the
/// same number, and the same one-sample-per-word layout, as
/// `VULKAN_MAX_FRAME_GLYPH_SAMPLES` (bug-670).
///
/// Metal's glyph bitmaps used to ride a per-glyph `setFragmentBytes:` payload. Once
/// plan-116-H made a text item ONE instanced draw, that payload was overwritten by each
/// glyph's publish before the draw ran, and every glyph drew the run's last bitmap. A
/// frame-wide region, each glyph's slice named by the offset in its own block, is what
/// Vulkan always did, and it is the only shape an instanced run can read.
///
/// `1 << 23` since bug-686 (it was `1 << 20`, Vulkan's size): a picture's texels ride
/// this region too, and one 1024×1024 background plus anything else overflowed a
/// million words. Eight million is a 2560×1440 background with room left for tiles and
/// text, and 32 MiB of region.
pub(crate) const METAL_MAX_FRAME_GLYPH_SAMPLES: usize = 1 << 23;
/// The most dash words one **Metal** frame may carry (plan-163-B), summed over its
/// dashed items: each takes its dash tail (`__CANVAS_GEO_DASH_COUNT` floats) plus the
/// phase, as float32 bit patterns. A frame-wide region like the gradient stops', its
/// slice named in the item block's `strokeStyle.z`/`.w`. `1 << 20` holds three words for
/// every edge of `METAL_MAX_FRAME_EDGES` with room for patterns and ellipse tables
/// (257 words each); 4 MiB of region.
pub(crate) const METAL_MAX_FRAME_DASH_WORDS: usize = 1 << 20;
/// Where Metal's dash region starts, in 32-bit words — after the gradient stops.
pub(crate) const METAL_DASH_BASE_WORDS: usize =
    METAL_GRADIENT_BASE_WORDS + METAL_MAX_FRAME_GRADIENT_STOPS * GRADIENT_STOP_WORDS;
/// Where Metal's glyph region starts, in 32-bit words — after the dash words.
pub(crate) const METAL_GLYPH_BASE_WORDS: usize = METAL_DASH_BASE_WORDS + METAL_MAX_FRAME_DASH_WORDS;
/// The whole Metal frame buffer: item blocks, then edges (four 16.16 words each), then
/// their residuals (four words each, bug-687), then the band index (bug-686), then gradient stops (five each), then dash words (one
/// float32 each, plan-163-B), then glyph coverage (one sample a word), then the picture
/// table (bug-686, `METAL_PICTURE_TABLE_BYTES`).
///
/// A sum of the regions' sizes rather than "the last base plus the last size", so
/// `the_metal_shader_region_bases_match_the_buffer_layout` compares two independent
/// computations and a region left out of the chain shows up there.
pub(crate) const METAL_BUFFER_BYTES: usize = METAL_ITEM_BUFFER_BYTES
    + METAL_MAX_FRAME_EDGES * 16
    + METAL_MAX_FRAME_EDGES * 16
    + METAL_MAX_FRAME_BAND_WORDS * 4
    + METAL_MAX_FRAME_GRADIENT_STOPS * GRADIENT_STOP_WORDS * 4
    + METAL_MAX_FRAME_DASH_WORDS * 4
    + METAL_MAX_FRAME_GLYPH_SAMPLES * 4
    + METAL_PICTURE_TABLE_BYTES;

/// The per-frame **picture table**: which image blocks this frame has already copied
/// into the glyph region, and where (bug-686). The last region of the Metal frame
/// buffer, read only by the emitter — the shader never sees it, so it has no MSL base.
///
/// `emit_picture_buffer` used to copy a picture's texels for every `Picture` item, so a
/// tilemap of 2,000 tiles drawn from two images copied 2,000 images and counted them
/// against the glyph region's cap. It now looks the image's block up here first and, on
/// a hit, points the item at the texels already uploaded; the pass walk
/// (`__canvas_passNext`) counts each distinct block once a pass to match. The table is
/// a PASS's: each pass is one call of the renderer, which starts it empty.
///
/// Two parts, a record array and an open-addressed index over it:
///
/// * `METAL_PICTURE_RECORDS` records of `PICTURE_RECORD_BYTES`:
///   `block` (u64, the pixel block's address), `base` (u32, its first texel in the
///   glyph region), `slot` (u32, the index slot that names this record);
/// * `METAL_PICTURE_INDEX_SLOTS` u32 slots, each a record number, probed linearly from
///   a multiplicative hash of the block address.
///
/// **Nothing is cleared between frames.** A slot is live only if its record number is
/// below this frame's record count AND that record's `slot` names it back — the
/// sparse-set test — so last frame's slots, or whatever the buffer held at creation,
/// read as empty without a pass over the index. Every record this frame writes is
/// written before its slot, and nothing is deleted within a frame, so every live
/// slot's probe chain is intact.
///
/// One record per quad of `METAL_MAX_FRAME_ITEMS` — a picture is one item and every
/// item at least one quad, so the predicate's own cap means the table is never full;
/// the index has twice as many slots, so a probe always reaches an empty one.
pub(crate) const METAL_PICTURE_RECORDS: usize = METAL_MAX_FRAME_ITEMS;
/// Bytes per picture record: block (8), base (4), index slot (4).
pub(crate) const PICTURE_RECORD_BYTES: usize = 16;
/// Slots in the picture index — a power of two, twice the records.
pub(crate) const METAL_PICTURE_INDEX_SLOTS: usize = 2 * METAL_PICTURE_RECORDS;
const _: () = assert!(
    METAL_PICTURE_INDEX_SLOTS.is_power_of_two(),
    "the picture index is masked, not divided"
);
/// Where the picture records start, in bytes — after the glyph region.
pub(crate) const METAL_PICTURE_RECORDS_OFFSET: usize =
    METAL_GLYPH_BASE_WORDS * 4 + METAL_MAX_FRAME_GLYPH_SAMPLES * 4;
/// Where the picture index starts, in bytes — after the records.
pub(crate) const METAL_PICTURE_INDEX_OFFSET: usize =
    METAL_PICTURE_RECORDS_OFFSET + METAL_PICTURE_RECORDS * PICTURE_RECORD_BYTES;
/// The picture table's whole size: records, then index slots.
pub(crate) const METAL_PICTURE_TABLE_BYTES: usize =
    METAL_PICTURE_RECORDS * PICTURE_RECORD_BYTES + METAL_PICTURE_INDEX_SLOTS * 4;

/// The most coverage samples one **frame**'s glyphs may carry on the Vulkan path.
///
/// Glyph bitmaps ride the same buffer as the edges, in a region after them, for the
/// reason a second buffer would have to be justified rather than assumed: it would need
/// its own allocation, its own memory-type search, its own descriptor binding and its
/// own upload, to hold data with exactly the edges' lifetime and exactly their access
/// pattern. One buffer, several regions, one binding.
///
/// One sample per 32-bit word rather than four packed per word. That wastes three
/// quarters of the region and buys a shader arm with no shifting and no masking, in the
/// only place where a packing mistake would be invisible — a wrongly unpacked coverage
/// byte still produces a glyph, just a wrong one.
///
/// `1 << 23` since bug-688 (it was `1 << 20`): a picture's texels ride this region
/// too, and a 1024×1024 background plus anything else overflowed a million words.
/// Metal's `METAL_MAX_FRAME_GLYPH_SAMPLES`, and 32 MiB of region.
pub(crate) const VULKAN_MAX_FRAME_GLYPH_SAMPLES: usize = 1 << 23;

// --- the Vulkan shared buffer (binding 0) ------------------------------------------
//
// Edges from word 0, then their residuals (bug-687), then the band index, the gradient
// stops, the glyph/picture texels and the picture table. The shader reaches the
// residual, band, gradient and glyph regions through BASES THAT ARE SPECIALIZATION
// CONSTANTS (bug-688): the fragment shader declares them `layout(constant_id = N) const int`, and the pipeline feeds the
// values below through `VkSpecializationInfo` (`VULKAN_SPEC_CONSTANTS`). Until then
// `GRADIENT_BASE` and `GLYPH_BASE` were literals in the checked-in GLSL, kept equal to
// these by a unit test — so every cap before them was frozen, because moving one meant
// a hand edit of the shader and a SPIR-V rebuild or a plausible wrong picture.

/// The band region's size in 32-bit words — `BAND_WORDS_PER_EDGE` per edge of
/// `VULKAN_MAX_FRAME_EDGES`, the budget `emit_band_index` holds every polygon to.
pub(crate) const VULKAN_MAX_FRAME_BAND_WORDS: usize = BAND_WORDS_PER_EDGE * VULKAN_MAX_FRAME_EDGES;
/// Where the edge residual region starts, in 32-bit words — right after the edges
/// (bug-687). Edge `e`'s four residual words sit at `EDGE_RESIDUAL_BASE + e * 4`,
/// parallel to its 16.16 words, so the band builder and a polyline, which read only the
/// 16.16 words, are untouched.
pub(crate) const VULKAN_EDGE_RESIDUAL_BASE_WORDS: usize = VULKAN_EDGE_BYTES / 4;
/// Where the band region starts, in 32-bit words — right after the edges it indexes and
/// their residuals.
pub(crate) const VULKAN_BAND_BASE_WORDS: usize =
    (VULKAN_EDGE_BYTES + VULKAN_EDGE_RESIDUAL_BYTES) / 4;
/// Where the gradient region starts, in 32-bit words — after the band index.
pub(crate) const VULKAN_GRADIENT_BASE_WORDS: usize =
    VULKAN_BAND_BASE_WORDS + VULKAN_MAX_FRAME_BAND_WORDS;
/// The most dash words one **Vulkan** frame may carry (plan-163-B) — Metal's
/// `METAL_MAX_FRAME_DASH_WORDS`, the same layout.
pub(crate) const VULKAN_MAX_FRAME_DASH_WORDS: usize = 1 << 20;
/// Where the dash region starts, in 32-bit words — after the gradient stops.
pub(crate) const VULKAN_DASH_BASE_WORDS: usize =
    VULKAN_GRADIENT_BASE_WORDS + VULKAN_MAX_FRAME_GRADIENT_STOPS * GRADIENT_STOP_WORDS;
/// Where the glyph (and picture texel) region starts, in 32-bit words — after the
/// dash words.
pub(crate) const VULKAN_GLYPH_BASE_WORDS: usize =
    VULKAN_DASH_BASE_WORDS + VULKAN_MAX_FRAME_DASH_WORDS;
/// Vulkan's per-frame picture table (bug-688) — Metal's (`METAL_PICTURE_TABLE_BYTES`)
/// with the same record and index layout, one record per quad of
/// `VULKAN_MAX_FRAME_ITEMS`. The last region of the shared buffer, read only by the
/// emitter: the shader never sees it, so it has no specialization constant.
pub(crate) const VULKAN_PICTURE_RECORDS: usize = VULKAN_MAX_FRAME_ITEMS;
/// Slots in the picture index — a power of two, twice the records.
pub(crate) const VULKAN_PICTURE_INDEX_SLOTS: usize = 2 * VULKAN_PICTURE_RECORDS;
const _: () = assert!(
    VULKAN_PICTURE_INDEX_SLOTS.is_power_of_two(),
    "the picture index is masked, not divided"
);
/// Where the picture records start, in bytes — after the glyph region.
pub(crate) const VULKAN_PICTURE_RECORDS_OFFSET: usize =
    VULKAN_GLYPH_BASE_WORDS * 4 + VULKAN_MAX_FRAME_GLYPH_SAMPLES * 4;
/// Where the picture index starts, in bytes — after the records.
pub(crate) const VULKAN_PICTURE_INDEX_OFFSET: usize =
    VULKAN_PICTURE_RECORDS_OFFSET + VULKAN_PICTURE_RECORDS * PICTURE_RECORD_BYTES;
/// The picture table's whole size: records, then index slots.
pub(crate) const VULKAN_PICTURE_TABLE_BYTES: usize =
    VULKAN_PICTURE_RECORDS * PICTURE_RECORD_BYTES + VULKAN_PICTURE_INDEX_SLOTS * 4;
/// The whole shared buffer: edges, edge residuals (bug-687), band index, gradient stops,
/// dash words (plan-163-B), glyph/picture texels, picture table.
///
/// A sum of the regions' sizes rather than "the last base plus the last size", so
/// `the_shared_buffer_holds_every_region` compares two independent computations and a
/// region left out of the chain shows up there.
pub(crate) const VULKAN_BUFFER_BYTES: usize = VULKAN_EDGE_BYTES
    + VULKAN_EDGE_RESIDUAL_BYTES
    + VULKAN_MAX_FRAME_BAND_WORDS * 4
    + VULKAN_MAX_FRAME_GRADIENT_STOPS * GRADIENT_STOP_WORDS * 4
    + VULKAN_MAX_FRAME_DASH_WORDS * 4
    + VULKAN_MAX_FRAME_GLYPH_SAMPLES * 4
    + VULKAN_PICTURE_TABLE_BYTES;
/// The fragment shader's specialization constants, as `(constant_id, value)`: the
/// region bases it reads, in words. The ids are the `layout(constant_id = N)` numbers in
/// `shaders/mfb_canvas.frag`; `the_fragment_shader_declares_every_spec_constant` pins
/// the two lists to each other.
pub(crate) const VULKAN_SPEC_CONSTANTS: [(u32, usize); 5] = [
    (0, VULKAN_GLYPH_BASE_WORDS),
    (1, VULKAN_GRADIENT_BASE_WORDS),
    (2, VULKAN_BAND_BASE_WORDS),
    (3, VULKAN_EDGE_RESIDUAL_BASE_WORDS),
    (4, VULKAN_DASH_BASE_WORDS),
];

/// The Win64 shadow space this trampoline owes its callees: 32 bytes on Windows, none
/// elsewhere. It sits at the BOTTOM of the frame, so the saves above it are out of reach
/// of a callee that spills its register arguments into it.
pub(crate) fn graphics_trampoline_shadow(windows: bool) -> usize {
    usize::from(windows) * 32
}

/// A canvas thread entry's frame size for `saved` saved registers, split out from the
/// emitter so the alignment invariant it exists to satisfy can be asserted directly
/// (bug-479).
///
/// **Every x86-64 thread entry is reached by a `call`** — `_pthread_start` on Unix and
/// `BaseThreadInitThunk` on Windows alike — so the routine begins at `rsp % 16 == 8`.
/// A callee must in turn be entered at `rsp % 16 == 8`, which means `rsp % 16 == 0`
/// immediately before the `call`. This trampoline only subtracts (it pushes nothing), so
/// the frame itself has to carry the odd 8: `frame % 16 == 8` is the whole requirement.
///
/// AArch64 needs no such thing — the entry `sp` is 16-aligned and the return address
/// arrives in `lr` rather than on the stack.
pub(crate) fn thread_entry_frame(arch: &str, windows: bool, saved: usize) -> usize {
    let realign = usize::from(arch == "x86_64") * 8;
    // `lr`, then one slot per saved register, rounded to the 16 every frame keeps.
    let saves = 8 * (1 + saved);
    saves.div_ceil(16) * 16 + realign + graphics_trampoline_shadow(windows)
}

/// The callee-saved registers the graphics thread entry saves for its caller, in
/// slot order above `lr` (bug-982 audit).
///
/// The caller is `_pthread_start` / `start_thread` / `BaseThreadInitThunk`, which
/// keeps its own state in the callee-saved registers across the call, and the
/// body runs compiled MFB code. MFB code preserves only the callee-saved registers
/// the allocator hands out; the rest of the bank is the convention's to change, and
/// every one of them must reach the thread library intact:
///
/// - the arena-state, current-thread and closure-environment registers MFB code
///   pins (`x19`/`x20`/`x28` on AArch64, `r15`/`rbx`/`r13` on x86-64; `rbx`/`r13`
///   are also the x86 homes of the hand-written runtime helpers' scratch, which
///   the render loop reaches);
/// - the 8th internal argument register, `rbp` on x86-64 (see the emitter);
/// - on Win64, `rdi`/`rsi`, the internal 5th and 6th argument registers, which
///   Win64 (unlike SysV) makes callee-saved.
///
/// The arch parameter is unused today because `canvas` needs app mode and app mode
/// has no RISC-V target; it stays in the signature beside `graphics_trampoline_frame`'s.
pub(crate) fn graphics_trampoline_saved(_arch: &str, windows: bool) -> Vec<abi::Reg> {
    let mut saved = vec![
        ARENA_STATE_REGISTER,
        abi::CURRENT_THREAD,
        CLOSURE_ENV_REGISTER,
        abi::mfb_arg(7),
    ];
    if windows {
        saved.extend([abi::mfb_arg(4), abi::mfb_arg(5)]);
    }
    saved
}

/// The trampoline `pthread_create` starts: establishes the MFB context, then loops.
pub(crate) const GRAPHICS_TRAMPOLINE_SYMBOL: &str = "_mfb_rt_canvas_graphics_entry";

/// The graphics thread's shared-state data object: all zero but
/// [`GRAPHICS_OFFSET_AUTO_WINDOW_MS`], which holds `auto_paint_ms` little-endian.
pub(crate) fn graphics_state_data_object(auto_paint_ms: u64) -> CodeDataObject {
    let mut bytes = vec![0u8; GRAPHICS_STATE_SIZE];
    bytes[GRAPHICS_OFFSET_AUTO_WINDOW_MS..GRAPHICS_OFFSET_AUTO_WINDOW_MS + 8]
        .copy_from_slice(&auto_paint_ms.to_le_bytes());
    let value: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    CodeDataObject {
        symbol: GRAPHICS_STATE_SYMBOL.to_string(),
        kind: "raw".to_string(),
        layout: "mfb.runtime.canvas_graphics.v1 { u64 started, pending; mutex; cond; \
                 u64 arenaState }"
            .to_string(),
        align: 8,
        size: GRAPHICS_STATE_SIZE,
        value,
    }
}

/// The MFBASIC render loop's native symbol.
///
/// Derived from the helper's own name rather than hard-coded, so a rename of
/// `__canvas_renderLoop` breaks the build here instead of producing a trampoline
/// that jumps to a symbol nothing defines.
pub(crate) fn render_loop_symbol() -> String {
    crate::target::shared::nir::function_symbol(&crate::internal_name::internalize(
        "__canvas_renderLoop",
    ))
}

/// `_mfb_rt_canvas_graphics_entry` — the graphics thread's first instruction.
///
/// The thread argument (`c_arg(0)`) is the child arena-state block the spawner
/// allocated and zeroed. This pins it into the arena-state register, runs the
/// module's `LINK` and global initializers **on this thread** — which is what gives
/// the loop its own geometry cache and, critically, a populated `__COLOR_SRGB` (the
/// sRGB transfer table, in the `color` package since plan-122-B) — and then enters
/// `__canvas_renderLoop`, which never returns.
///
/// The loop returns when shutdown asks it to (`canvas::waitForRedraw` reports
/// `FALSE`), and this returns straight after — which is how a pthread entry exits,
/// and what the shutdown join is waiting for.
pub(crate) fn emit_graphics_trampoline(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    arena_init: ArenaInitSymbols,
) -> Result<CodeFunction, String> {
    let from = GRAPHICS_TRAMPOLINE_SYMBOL;
    let _ = platform_imports;
    let windows = platform.family() == PlatformFamily::Windows;
    Ok(emit_thread_entry(
        from,
        "canvas.graphics_entry",
        platform,
        graphics_trampoline_saved(platform.arch(), windows),
        |instructions, relocations| {
            // Pin the child arena state. Every MFB global access on this thread — including
            // the geometry cache and the sRGB table — is addressed off it.
            instructions.push(abi::move_register(ARENA_STATE_REGISTER, abi::c_arg(0)));

            // The two initializers, in the order the entry runs them (bug-369): `LINK` first,
            // because a global's initializer may call a `LINK` symbol.
            for symbol in [arena_init.link_init, arena_init.global_init]
                .into_iter()
                .flatten()
            {
                instructions.push(abi::branch_link(symbol));
                relocations.push(internal_branch(from, symbol));
            }

            let render_loop = render_loop_symbol();
            instructions.push(abi::branch_link(&render_loop));
            relocations.push(internal_branch(from, &render_loop));
            // bug-806: this thread ran the module's global initializer into its own
            // instance; close what that instance's module-level bindings own.
            if let Some(symbol) = arena_init.teardown {
                instructions.push(abi::branch_link(symbol));
                relocations.push(internal_branch(from, symbol));
            }
        },
    ))
}

/// A thread start routine: the frame every canvas thread entry needs, around `body`.
///
/// The graphics thread's entry and (bug-996) the Vulkan builder's are both called by
/// the thread library, so they owe it the same things: the stack alignment a `call`
/// leaves, Windows' shadow space, and every register in `saved` handed back as it
/// arrived. Which registers that is depends on what the body runs, so the caller says
/// ([`graphics_trampoline_saved`], `vulkan_handoff::builder_entry_saved`). `body` runs
/// with the thread argument still in `c_arg(0)`; the routine returns 0.
pub(crate) fn emit_thread_entry(
    from: &str,
    name: &str,
    platform: &dyn CodegenPlatform,
    saved: Vec<abi::Reg>,
    body: impl FnOnce(&mut Vec<CodeInstruction>, &mut Vec<CodeRelocation>),
) -> CodeFunction {
    let mut instructions = Vec::new();
    let mut relocations = Vec::new();
    // **The x86-64 stack realign.** Every x86-64 thread library reaches a
    // start-routine through a `call`, so the routine begins at `sp % 16 == 8` — the
    // return address has been pushed and nothing has re-aligned. A frame that is a
    // multiple of 16 therefore leaves every call this trampoline makes misaligned,
    // and the first callee that uses an aligned SSE store faults. It does not fault
    // here: it faults far away, inside `calloc` under `g_idle_add`, as heap
    // corruption in a thread whose own frames are all correct.
    //
    // `lower_thread_trampoline` has carried this same +8 since bug-408, box-proven on
    // both libcs (glibc 2228: the 88-byte frame runs, the 80-byte frame SIGSEGVs 5/5;
    // musl 2227: 88 runs, 96 SIGSEGVs 5/5). The graphics thread is a second
    // start-routine and needs it for exactly the same reason — it did not have it,
    // which is why canvas mode segfaulted on Linux from the moment plan-98-D moved
    // rendering onto a thread. AArch64 takes no realign: `pthread_create` enters with
    // a 16-aligned `sp` and the return address in `lr`.
    //
    // **Windows takes the realign too — measured, not assumed.** This comment used to
    // claim `BaseThreadInitThunk` enters a `CreateThread` start routine ALREADY
    // 16-aligned, and skipped the `+8` on that basis. It is false: the thunk reaches
    // the start routine through an ordinary `call`, so the routine begins at
    // `rsp % 16 == 8` exactly like `_pthread_start`.
    //
    // The skew is invisible until something on the graphics thread calls a Win32
    // function that cares. `SleepConditionVariableSRW` cares, because ntdll builds its
    // wait block on the caller's stack and tags the pointer in the block's low 4 bits
    // (`and rdx,0FFFFFFFFFFFFFFF0h` at `RtlSleepConditionVariableSRW+0x13d`). Eight
    // bytes out, that mask yields a pointer into the middle of the block, the wait-list
    // walk below it loads a NULL `Next`, and `mov [rcx+10h],rax` faults with `rcx=0` —
    // with every argument correct, on the very first wait. That is bug-479.
    //
    // Measured on box 2230 with a breakpoint on `ntdll!RtlAcquireSRWLockExclusive`
    // printing `@rsp` per thread, both acquiring the SAME canvas mutex:
    //
    //     worker   t=1728  rsp=0x332de68  -> % 16 == 8   correct
    //     graphics t=1a28  rsp=0x3b2fd30  -> % 16 == 0   skewed
    //
    // The worker is right because its entry pushes a register before its frame; this
    // trampoline only subtracts, so the frame itself has to carry the odd 8. And
    // because a body's alignment is its caller's call site, the skew was inherited by
    // every Win32 call the render loop ever made (bug-478 is the same mistake in
    // `win_x86_64/app/mod.rs`).
    //
    // Windows also needs the callee's **shadow space**: 32 bytes at the bottom of this
    // frame that any callee may spill its register arguments into. The shared
    // `finalize_frame` reserves it for every *allocated* function
    // (`outgoing_args_base_offset`), but this trampoline is hand-built and gets none —
    // so without it the saves below sit exactly where a callee is entitled to write.
    let windows = platform.family() == PlatformFamily::Windows;
    let shadow = graphics_trampoline_shadow(windows);
    let frame = thread_entry_frame(platform.arch(), windows, saved.len());
    instructions.push(abi::label("entry"));
    instructions.push(abi::subtract_stack(frame));
    instructions.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        shadow,
    ));
    // **Save the arena register.** It is callee-saved, and the caller here is
    // `_pthread_start`, which has its own live state in it — clobbering it corrupts
    // pthread's frame and the thread dies at exit inside `_pthread_terminate` with a
    // pointer-authentication failure, having run the whole body correctly first. The
    // thread runtime's own trampoline saves the same register for the same reason;
    // "a thread entry has nobody to return to" is wrong, because pthread is the
    // caller.
    // (Every register `graphics_trampoline_saved` lists is stored below, after
    // the two that cost the most to find.)
    //
    // **Save the 8th internal argument register too.**
    //
    // MFBASIC functions take up to 8 parameters and AArch64 has 8 argument
    // registers, but SysV x86-64 has only six — so the internal convention extends
    // the list with `rax` and `rbp` for arguments 7 and 8 (bug-296,
    // `CALL_ARGS` in `x86_64/select.rs`). That is self-consistent between MFB
    // caller and MFB callee, and wrong at a boundary: `rbp` is CALLEE-saved under
    // SysV, and the caller here is glibc's `start_thread`, which keeps its own frame
    // pointer in it across the call to this routine.
    //
    // So any MFB function that stages an 8th argument destroys it. `__canvas_
    // drawGeometry` does exactly that, six times, calling `__canvas_geoDistance`:
    //
    //     mov r8, r10 / mov r9, r10 / mov rax, r10 / mov rbp, r10
    //     bl _mfb_ifn_canvas_5FgeoDistance
    //
    // and its own frame saves only `["r12", "r14", "lr"]`, because the frame's
    // callee-saved set is computed from ALLOCATED registers and this `rbp` is an
    // ABI-staged one the allocator never assigned.
    //
    // On AArch64 the 8th argument is `x7`, which is caller-saved, so saving it here
    // costs one store and changes nothing.
    //
    // Measured on an x86_64 ubuntu-24.04 runner under gdb: the graphics thread
    // returns with `rbp = 0x404e000000000000`, which is not a pointer at all — it is
    // the double `60.0`, the `radius` of the circle in the scene being drawn.
    // `start_thread` then executes `mov -0x98(%rbp),%rax` and dies with SIGBUS. That
    // is the whole of the canvas failure on the x86_64 Linux rows: 68 of ~90 canvas
    // tests, 55 SIGBUS and 13 SIGSEGV, the two alternating exactly as one wild
    // address does depending on where it lands.
    //
    // It reproduces on no machine I can reach — boxes 2228/2227/2223 and an
    // ubuntu:24.04 container with the same GTK run the same binaries clean, dozens
    // of times — because whether a clobbered `rbp` is ever *dereferenced* depends on
    // the libc's own code path after the start routine returns.
    for (index, register) in saved.iter().enumerate() {
        instructions.push(abi::store_u64(
            *register,
            abi::stack_pointer(),
            shadow + 8 + 8 * index,
        ));
    }
    body(&mut instructions, &mut relocations);

    // Return, which is how a pthread entry exits. It used to park here on the
    // theory that a render loop returning was a bug — but shutdown asks it to
    // return, and parking made the shutdown join wait forever on a thread that
    // was spinning two instructions away from finishing.
    instructions.push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
    // Every saved register, the arena register (`r15` on x86-64) and the 8th
    // argument register (`rbp`) among them: all must reach `start_thread` intact.
    for (index, register) in saved.iter().enumerate().rev() {
        instructions.push(abi::load_u64(
            *register,
            abi::stack_pointer(),
            shadow + 8 + 8 * index,
        ));
    }
    instructions.push(abi::load_u64(
        abi::link_register(),
        abi::stack_pointer(),
        shadow,
    ));
    instructions.push(abi::add_stack(frame));
    instructions.push(abi::return_());

    CodeFunction {
        name: name.to_string(),
        symbol: from.to_string(),
        params: Vec::new(),
        returns: "Pointer".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    }
}

/// Load the graphics-state block's address into `dst`.
pub(crate) fn state_base(
    from: &str,
    dst: impl Into<abi::Reg>,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    let dst: abi::Reg = dst.into();
    push_symbol_address(from, GRAPHICS_STATE_SYMBOL, dst, ins, rel);
}

/// `canvas::signalRedraw(pointerOnly)` — a redraw is owed, or the pointer changed.
///
/// Sets the flag and wakes the loop. A **flag**, not a counter: two presents before a
/// frame must produce one frame, not two (`.ai/canvas-threading.md` §3), and a
/// counter would make the renderer draw the same scene twice.
///
/// **plan-172-K: `pointer_only`.** Exactly 1 is a platform's pointer handler saying
/// the pointer changed under a scene that holds an `AutoPaint`: it sets
/// [`GRAPHICS_OFFSET_AUTO_WAKE`] and nothing else — not `PENDING`, and not `WANTED`,
/// which is a `present`'s. Any other value is a redraw, so a caller that has to be
/// wrong is wrong in the direction that draws a frame.
pub(crate) fn emit_signal_redraw(
    symbol: &str,
    scratch: &GraphicsScratch,
    pointer_only: &VirtualRegister,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let skip = format!("{symbol}_no_thread");
    state_base(symbol, &scratch.base, instructions, relocations);
    // Nothing to wake before the thread exists, and the mutex is not initialized
    // until it is spawned — locking it here would be locking garbage.
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_STARTED,
    ));
    instructions.push(abi::compare_immediate(&scratch.scratch, "0"));
    instructions.push(abi::branch_eq(&skip));

    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_MUTEX,
    ));
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    };
    emit_thread_external_call(&mut ctx, "pthread_mutex_lock")?;

    let redraw = format!("{symbol}_redraw");
    let wake = format!("{symbol}_wake");
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::move_immediate(&scratch.scratch, "Integer", "1"));
    instructions.push(abi::compare_immediate(pointer_only, "1"));
    instructions.push(abi::branch_ne(&redraw));
    instructions.push(abi::store_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_AUTO_WAKE,
    ));
    instructions.push(abi::branch(&wake));
    instructions.push(abi::label(&redraw));
    instructions.push(abi::store_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_PENDING,
    ));
    // Record which frame this present is asking for, so `canvas::syncFrame` knows
    // what to wait for. Under the mutex, like everything else here.
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_FRAMES,
    ));
    instructions.push(abi::add_immediate(&scratch.scratch, &scratch.scratch, 1));
    instructions.push(abi::store_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_WANTED,
    ));
    instructions.push(abi::label(&wake));
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_COND,
    ));
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    };
    emit_thread_external_call(&mut ctx, "pthread_cond_signal")?;

    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_MUTEX,
    ));
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    };
    emit_thread_external_call(&mut ctx, "pthread_mutex_unlock")?;
    instructions.push(abi::label(&skip));
    Ok(())
}

/// `canvas::waitForRedraw(timeoutNanos)` — block until a redraw is owed, then take it.
///
/// The wait is a `while`, not an `if`: a condition variable may wake spuriously, and
/// a spurious wake that fell through would render a frame nobody asked for — which is
/// not a correctness bug but is exactly the "a static scene costs nothing" guarantee
/// (`.ai/canvas-threading.md` §4) quietly failing.
///
/// **plan-172-K: the one wait that can run out, and the answer says why it ended.**
/// [`WAKE_STOP`], [`WAKE_REDRAW`] — `PENDING` was set and has been taken — or
/// [`WAKE_AUTO_ONLY`]: nothing is owed, but the pointer changed
/// ([`GRAPHICS_OFFSET_AUTO_WAKE`]) or `timeoutNanos` ran out. With `timeoutNanos`
/// above zero the wait is timed — the frame a `canvas::AutoPaint`'s enter or exit
/// window asks for when it ends, with the pointer still. Zero is the untimed wait this
/// always was, and it is what every scene with no window running passes.
///
/// A redraw wins: a pointer change that arrives with one is answered by the same
/// frame, so `AUTO_WAKE` is cleared with `PENDING`.
///
/// An auto-only wake takes nothing else: `PENDING` was clear and stays clear, `WANTED`
/// is not touched, and nothing is broadcast, so a `present` and a `waitFrame` see
/// exactly what they would have seen had the wait gone on. The deadline is an absolute
/// `CLOCK_REALTIME` reading taken once, before the lock — the clock
/// `pthread_cond_timedwait` takes — and is compared again after every wake, so a
/// spurious wake neither returns early nor restarts the timeout.
pub(crate) fn emit_wait_for_redraw(
    builder: &mut CodeBuilder,
    timeout: &Operand,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(), String> {
    let symbol_owned = builder.current_symbol.clone();
    let symbol = symbol_owned.as_str();
    let retry = format!("{symbol}_wait");
    let have = format!("{symbol}_have");
    let stop = format!("{symbol}_stop");
    let done = format!("{symbol}_wait_done");
    let untimed = format!("{symbol}_untimed");
    let plain = format!("{symbol}_plain_wait");
    let expired = format!("{symbol}_expired");

    // The argument leaves its register before the first call, and the deadline is
    // worked out before the lock is taken.
    let timeout_value = builder.temporary_vreg();
    builder.emit(abi::move_register(&timeout_value, timeout));
    let ts = builder.allocate_stack_object("canvas_wait_ts", 16);
    let scratch_owned = GraphicsScratch::new(&mut || builder.temporary_vreg());
    let scratch = &scratch_owned;
    let now = builder.temporary_vreg();
    let deadline = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&deadline, "Integer", "0"));
    builder.emit(abi::compare_immediate(&timeout_value, "0"));
    builder.emit(abi::branch_le(&untimed));
    emit_realtime_nanos(builder, ts, &now, platform_imports, platform)?;
    builder.emit(abi::add_registers(&deadline, &now, &timeout_value));
    builder.emit(abi::label(&untimed));

    let instructions = &mut builder.instructions;
    let relocations = &mut builder.relocations;
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_MUTEX,
    ));
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    };
    emit_thread_external_call(&mut ctx, "pthread_mutex_lock")?;

    instructions.push(abi::label(&retry));
    state_base(symbol, &scratch.base, instructions, relocations);
    // **A pending frame wins over stopping** — the loop DRAINS before it exits.
    //
    // The other order looks tidier ("we are shutting down, why draw?") and is
    // wrong: a program whose whole body is `present` then return races its own
    // shutdown, and loses often enough that the frame is simply never drawn. It is
    // not even a reliable failure — run from a shell it rendered, run under
    // `cargo test` it did not.
    //
    // Draining terminates: shutdown sets `stopping` once, and the worker is inside
    // shutdown, so no further `present` can arrive to keep the loop fed.
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_PENDING,
    ));
    instructions.push(abi::compare_immediate(&scratch.scratch, "0"));
    instructions.push(abi::branch_ne(&have));
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_STOPPING,
    ));
    instructions.push(abi::compare_immediate(&scratch.scratch, "0"));
    instructions.push(abi::branch_ne(&stop));
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_AUTO_WAKE,
    ));
    instructions.push(abi::compare_immediate(&scratch.scratch, "0"));
    instructions.push(abi::branch_ne(&expired));
    instructions.push(abi::compare_immediate(&deadline, "0"));
    instructions.push(abi::branch_eq(&plain));
    // Timed: out of time is a redraw; otherwise wait until the deadline, as an
    // absolute timespec.
    emit_realtime_nanos(builder, ts, &now, platform_imports, platform)?;
    builder.emit(abi::compare_registers(&now, &deadline));
    builder.emit(abi::branch_ge(&expired));
    {
        let scale = builder.temporary_vreg();
        let seconds = builder.temporary_vreg();
        let rest = builder.temporary_vreg();
        builder.emit(abi::move_immediate(&scale, "Integer", NANOS_PER_SECOND));
        builder.emit(abi::unsigned_divide_registers(&seconds, &deadline, &scale));
        builder.emit(abi::multiply_subtract_registers(
            &rest, &seconds, &scale, &deadline,
        ));
        builder.emit(abi::store_u64(&seconds, abi::stack_pointer(), ts));
        builder.emit(abi::store_u64(&rest, abi::stack_pointer(), ts + 8));
    }
    let instructions = &mut builder.instructions;
    let relocations = &mut builder.relocations;
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_COND,
    ));
    instructions.push(abi::add_immediate(
        abi::c_arg(1),
        &scratch.base,
        GRAPHICS_OFFSET_MUTEX,
    ));
    instructions.push(abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), ts));
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    };
    emit_thread_external_call(&mut ctx, "pthread_cond_timedwait")?;
    instructions.push(abi::branch(&retry));

    instructions.push(abi::label(&plain));
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_COND,
    ));
    instructions.push(abi::add_immediate(
        abi::c_arg(1),
        &scratch.base,
        GRAPHICS_OFFSET_MUTEX,
    ));
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    };
    emit_thread_external_call(&mut ctx, "pthread_cond_wait")?;
    instructions.push(abi::branch(&retry));

    // The pointer changed, or the deadline passed, with nothing owed: the pointer's
    // flag is taken and nothing else is.
    instructions.push(abi::label(&expired));
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::store_u64(
        abi::ZERO,
        &scratch.base,
        GRAPHICS_OFFSET_AUTO_WAKE,
    ));
    instructions.push(abi::move_immediate(
        &scratch.verdict,
        "Integer",
        WAKE_AUTO_ONLY,
    ));
    instructions.push(abi::branch(&done));

    instructions.push(abi::label(&have));
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::store_u64(
        abi::ZERO,
        &scratch.base,
        GRAPHICS_OFFSET_PENDING,
    ));
    instructions.push(abi::store_u64(
        abi::ZERO,
        &scratch.base,
        GRAPHICS_OFFSET_AUTO_WAKE,
    ));
    // plan-155-C: the redraw has been picked up — the moment `canvas::waitFrame`
    // waits for. Broadcast while the mutex is still held, so the waiter reads the
    // cleared flag when it reacquires it.
    emit_frame_cond_broadcast(
        symbol,
        scratch,
        platform_imports,
        platform,
        instructions,
        relocations,
    )?;
    instructions.push(abi::move_immediate(
        &scratch.verdict,
        "Integer",
        WAKE_REDRAW,
    ));
    instructions.push(abi::branch(&done));

    instructions.push(abi::label(&stop));
    instructions.push(abi::move_immediate(&scratch.verdict, "Integer", WAKE_STOP));

    instructions.push(abi::label(&done));
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_MUTEX,
    ));
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    };
    emit_thread_external_call(&mut ctx, "pthread_mutex_unlock")?;
    instructions.push(abi::move_register(RESULT_VALUE_REGISTER, &scratch.verdict));
    Ok(())
}

/// `_mfb_rt_canvas_stop_graphics` — stop the render thread and wait for it.
///
/// Called from `_mfb_shutdown`, **before** the arena is destroyed. This closes R12
/// in `.ai/canvas-threading.md` §8: the scene blocks live in the worker's arena and
/// the worker's arena state lives on the worker's stack frame, so a graphics thread
/// still rendering when the worker unwinds is reading freed memory. Without the
/// join, a program that presents and returns segfaults — which is exactly what it
/// did before this existed.
///
/// A no-op when the thread was never started, which is every non-canvas program and
/// every canvas program that never presented.
pub(crate) fn emit_stop_graphics(
    symbol: &str,
    scratch: &GraphicsScratch,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let skip = format!("{symbol}_no_graphics");
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_STARTED,
    ));
    instructions.push(abi::compare_immediate(&scratch.scratch, "0"));
    instructions.push(abi::branch_eq(&skip));

    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_MUTEX,
    ));
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    };
    emit_thread_external_call(&mut ctx, "pthread_mutex_lock")?;

    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::move_immediate(&scratch.scratch, "Integer", "1"));
    instructions.push(abi::store_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_STOPPING,
    ));
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_COND,
    ));
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    };
    emit_thread_external_call(&mut ctx, "pthread_cond_signal")?;
    // plan-155-C: a `canvas::waitFrame` in progress must see `STOPPING` too.
    emit_frame_cond_broadcast(
        symbol,
        scratch,
        platform_imports,
        platform,
        instructions,
        relocations,
    )?;

    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_MUTEX,
    ));
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    };
    emit_thread_external_call(&mut ctx, "pthread_mutex_unlock")?;

    // Join. The wait must be *outside* the mutex the loop is about to reacquire.
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::load_u64(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_TID,
    ));
    instructions.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    if platform.family() == PlatformFamily::Windows {
        // WaitForSingleObject(handle, INFINITE)
        instructions.push(abi::move_immediate(abi::c_arg(1), "Integer", "4294967295"));
        instructions.push(abi::branch_link("WaitForSingleObject"));
        relocations.push(external_branch(
            symbol,
            "WaitForSingleObject",
            platform_imports,
        )?);
    } else {
        let join = thread_symbol(platform, "pthread_join");
        instructions.push(abi::branch_link(&join));
        relocations.push(external_branch(symbol, &join, platform_imports)?);
    }
    instructions.push(abi::label(&skip));
    Ok(())
}

/// `canvas::startGraphics()` — spawn the render thread, once.
///
/// **No lock guards `started`,** and that is sound rather than sloppy: `present` is
/// the only caller and the language is single-worker (`.ai/canvas-threading.md` §9),
/// so there is no second thread that could race the check. It is also what breaks the
/// obvious chicken-and-egg — the mutex cannot guard its own initialization, and a
/// zeroed `pthread_mutex_t` is a valid unlocked mutex on Linux but **not** on macOS,
/// whose initializer carries a signature word. Initializing both primitives here,
/// before the thread that uses them exists, is the ordering that makes them safe.
///
/// The child arena state is allocated from the *caller's* arena and handed over. That
/// is the same thing `thread::start` does, and it is why the graphics thread must
/// never free it: an arena is per-thread.
pub(crate) fn emit_start_graphics(
    symbol: &str,
    scratch: &GraphicsScratch,
    arena_global_slots: usize,
    debug_arena_registry: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    alloc_fail: &str,
) -> Result<(), String> {
    let done = format!("{symbol}_graphics_running");
    let zero_loop = format!("{symbol}_graphics_zero");

    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_STARTED,
    ));
    instructions.push(abi::compare_immediate(&scratch.scratch, "0"));
    instructions.push(abi::branch_ne(&done));

    // pthread_mutex_init(&mutex, NULL); pthread_cond_init(&cond, NULL)
    for (offset, call) in [
        (GRAPHICS_OFFSET_MUTEX, "pthread_mutex_init"),
        (GRAPHICS_OFFSET_COND, "pthread_cond_init"),
        (GRAPHICS_OFFSET_FRAME_COND, "pthread_cond_init"),
    ] {
        state_base(symbol, &scratch.base, instructions, relocations);
        instructions.push(abi::add_immediate(abi::c_arg(0), &scratch.base, offset));
        instructions.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
        let mut ctx = EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions,
            relocations,
        };
        emit_thread_external_call(&mut ctx, call)?;
    }

    // The child's arena state: the same layout the entry frame reserves, so every
    // global slot lands at the same offset on both paths (bug-369).
    let child_arena_size = ENTRY_GLOBALS_OFFSET + arena_global_slots * 8;
    instructions.push(abi::move_immediate(
        abi::return_register(),
        "Integer",
        &child_arena_size.to_string(),
    ));
    instructions.push(abi::move_immediate(abi::c_arg(1), "Integer", "1"));
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.push(abi::move_register(&scratch.arena, abi::mfb_return(1)));

    // Zero it: arena memory is not zero-filled, and an uninitialized global slot is a
    // garbage pointer the first global access dereferences.
    instructions.push(abi::move_register(&scratch.cursor, &scratch.arena));
    instructions.push(abi::add_immediate(
        &scratch.end,
        &scratch.arena,
        child_arena_size,
    ));
    instructions.push(abi::label(&zero_loop));
    instructions.push(abi::store_u64(abi::ZERO, &scratch.cursor, 0));
    instructions.push(abi::add_immediate(&scratch.cursor, &scratch.cursor, 8));
    instructions.push(abi::compare_registers(&scratch.cursor, &scratch.end));
    instructions.push(abi::branch_lo(&zero_loop));

    // plan-130-C: register the graphics thread's arena with the `--debug` arena registry,
    // before the thread that owns it exists.
    if debug_arena_registry {
        instructions.push(abi::move_register(abi::c_arg(0), &scratch.arena));
        instructions.push(abi::move_immediate(
            abi::c_arg(1),
            "Integer",
            crate::codegen::debug::ARENA_KIND_GRAPHICS,
        ));
        instructions.push(abi::branch_link(
            crate::codegen::debug::DEBUG_ARENA_REGISTER_SYMBOL,
        ));
        relocations.push(crate::codegen::engine::builder::internal_branch(
            symbol,
            crate::codegen::debug::DEBUG_ARENA_REGISTER_SYMBOL,
        ));
    }
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::store_u64(
        &scratch.arena,
        &scratch.base,
        GRAPHICS_OFFSET_ARENA,
    ));
    // Publish `started` BEFORE the spawn: the new thread's first act is to wait on
    // the condition, and `signalRedraw` skips entirely while `started` is zero. A
    // present that raced in between would otherwise drop its own wake-up.
    instructions.push(abi::move_immediate(&scratch.scratch, "Integer", "1"));
    instructions.push(abi::store_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_STARTED,
    ));

    emit_graphics_spawn(
        symbol,
        scratch,
        platform_imports,
        platform,
        instructions,
        relocations,
    )?;
    instructions.push(abi::label(&done));
    Ok(())
}

/// The `pthread_create` (or `CreateThread`) itself.
///
/// `SCRATCH[6]` holds the child arena state, which becomes the thread argument.
fn emit_graphics_spawn(
    symbol: &str,
    scratch: &GraphicsScratch,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    if platform.family() == PlatformFamily::Windows {
        // CreateThread(NULL, 8 MiB, entry, arg, STACK_SIZE_PARAM_IS_A_RESERVATION, NULL).
        //
        // **The stack size is not optional**, for the reason the pthread arm below
        // spells out: `dwStackSize = 0` takes the PE header's default, which is 1 MiB,
        // and the renderer is not a 1 MiB kind of code — `thread::start` asks for 8 MiB
        // on POSIX and the graphics thread runs the same sort of frames. macOS learned
        // this the hard way with a 512 KiB pthread default, where the render *completed*
        // and the thread then died at exit inside libmalloc.
        //
        // `STACK_SIZE_PARAM_IS_A_RESERVATION` (0x00010000) makes the number a
        // *reservation* rather than a commit, so the cost is address space rather than
        // RSS — the same trade the pthread arm makes.
        instructions.push(abi::move_immediate(abi::c_arg(0), "Integer", "0"));
        instructions.push(abi::move_immediate(
            abi::c_arg(1),
            "Integer",
            &(8 * 1024 * 1024).to_string(),
        ));
        push_symbol_address(
            symbol,
            GRAPHICS_TRAMPOLINE_SYMBOL,
            abi::c_arg(2),
            instructions,
            relocations,
        );
        instructions.push(abi::move_register(abi::c_arg(3), &scratch.arena));
        // `outgoing_stack_arg_store`, **not** a raw `[sp+0x20]` store. This seam is
        // emitted into an allocated function, and `finalize_frame` shifts every
        // sp-relative access in the body by the frame it builds — so a hand-written
        // `[sp+0x20]` does not stay at `rsp+0x20`, and `CreateThread` reads whatever
        // is really there for `dwCreationFlags` and `lpThreadId`. The second of those
        // is an OUT pointer, so a garbage value is not a wrong flag, it is a wild
        // write. The sentinel base is the only spelling the finalizer accounts for:
        // it both places the store correctly and grows the outgoing area to fit it.
        instructions.push(abi::move_immediate(
            &scratch.scratch,
            "Integer",
            "65536", // STACK_SIZE_PARAM_IS_A_RESERVATION
        ));
        instructions.push(abi::outgoing_stack_arg_store(&scratch.scratch, 0)); // dwCreationFlags
        instructions.push(abi::outgoing_stack_arg_store(abi::ZERO, 1)); // lpThreadId
        instructions.push(abi::branch_link("CreateThread"));
        relocations.push(external_branch(symbol, "CreateThread", platform_imports)?);
        // Keep the handle: shutdown waits on it, and losing it would leave the join
        // waiting on garbage.
        state_base(symbol, &scratch.base, instructions, relocations);
        instructions.push(abi::store_u64(
            abi::c_return(0),
            &scratch.base,
            GRAPHICS_OFFSET_TID,
        ));
        return Ok(());
    }

    // pthread_create(&tid, &attr, entry, arg).
    //
    // **The attr is not optional.** A NULL attr gives macOS's 512 KiB default stack,
    // and the renderer overflowed it — not with a crash in the render, which
    // completed and wrote a correct frame, but by smashing the thread's TSD block at
    // the base of its stack, which then aborted in libmalloc during
    // `_pthread_tsd_cleanup` at thread exit. `thread::start` sets 8 MiB for exactly
    // this reason (a large MFB frame is normal; the regex engine's is ~230 KiB), and
    // the graphics thread runs the same kind of code. The memory is reserved lazily,
    // so the cost is address space rather than RSS.
    let create = thread_symbol(platform, "pthread_create");
    let attr_init = thread_symbol(platform, "pthread_attr_init");
    let attr_setstacksize = thread_symbol(platform, "pthread_attr_setstacksize");
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_ATTR,
    ));
    instructions.push(abi::branch_link(&attr_init));
    relocations.push(external_branch(symbol, &attr_init, platform_imports)?);
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_ATTR,
    ));
    instructions.push(abi::move_immediate(
        abi::c_arg(1),
        "Integer",
        &(8 * 1024 * 1024).to_string(),
    ));
    instructions.push(abi::branch_link(&attr_setstacksize));
    relocations.push(external_branch(
        symbol,
        &attr_setstacksize,
        platform_imports,
    )?);

    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_TID,
    ));
    instructions.push(abi::add_immediate(
        abi::c_arg(1),
        &scratch.base,
        GRAPHICS_OFFSET_ATTR,
    ));
    push_symbol_address(
        symbol,
        GRAPHICS_TRAMPOLINE_SYMBOL,
        abi::c_arg(2),
        instructions,
        relocations,
    );
    instructions.push(abi::move_register(abi::c_arg(3), &scratch.arena));
    instructions.push(abi::branch_link(&create));
    relocations.push(external_branch(symbol, &create, platform_imports)?);
    Ok(())
}

/// The four values `canvas::frameDone` records, each already in a register the caller
/// allocated (a value in an argument register would not survive the mutex call).
pub(crate) struct FrameRecord<'a> {
    /// Non-zero when a backend drew the frame; zero for a wake-up with nothing to draw.
    pub(crate) drew: &'a VirtualRegister,
    /// The `canvas::Renderer` ordinal that drew it.
    pub(crate) renderer: &'a VirtualRegister,
    /// How long `__canvas_renderFrame` took, in nanoseconds.
    pub(crate) render_nanos: &'a VirtualRegister,
    /// `canvas::frameNanos` at the end of the frame — the drawn ring's timestamp.
    pub(crate) now: &'a VirtualRegister,
}

/// Push `value` onto the ring at `ring` whose head is at `head`, and advance the head.
/// `scratch.base` must hold the graphics state; the caller holds the mutex.
fn emit_ring_push(
    scratch: &GraphicsScratch,
    head: usize,
    ring: usize,
    value: impl Into<abi::Reg>,
    instructions: &mut Vec<CodeInstruction>,
) {
    let value: abi::Reg = value.into();
    instructions.push(abi::load_u64(&scratch.cursor, &scratch.base, head));
    instructions.push(abi::shift_left_immediate(&scratch.end, &scratch.cursor, 3));
    instructions.push(abi::add_registers(
        &scratch.end,
        &scratch.end,
        &scratch.base,
    ));
    instructions.push(abi::store_u64(value, &scratch.end, ring));
    instructions.push(abi::add_immediate(&scratch.cursor, &scratch.cursor, 1));
    instructions.push(abi::move_immediate(
        &scratch.end,
        "Integer",
        &(FRAME_RING_SLOTS - 1).to_string(),
    ));
    instructions.push(abi::and_registers(
        &scratch.cursor,
        &scratch.cursor,
        &scratch.end,
    ));
    instructions.push(abi::store_u64(&scratch.cursor, &scratch.base, head));
}

/// `global += 1` for the graphics-state word at `offset`.
fn emit_bump(scratch: &GraphicsScratch, offset: usize, instructions: &mut Vec<CodeInstruction>) {
    instructions.push(abi::load_u64(&scratch.scratch, &scratch.base, offset));
    instructions.push(abi::add_immediate(&scratch.scratch, &scratch.scratch, 1));
    instructions.push(abi::store_u64(&scratch.scratch, &scratch.base, offset));
}

/// `canvas::notePresent(now)` — count a present and log its time (plan-155-B).
///
/// Every `present` and `presentLayers` calls this, identical re-presents included:
/// `presents` is "what the program offered", which is the comparison `fps` is read
/// against. Under the mutex because `canvas::frameCounters` reads the same words under
/// it, so a snapshot never sees a half-written ring. A no-op before the thread starts —
/// the mutex is not initialized until then, and both wrappers start it first.
pub(crate) fn emit_note_present(
    symbol: &str,
    scratch: &GraphicsScratch,
    now: impl Into<abi::Reg>,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let now: abi::Reg = now.into();
    let skip = format!("{symbol}_no_thread");
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_STARTED,
    ));
    instructions.push(abi::compare_immediate(&scratch.scratch, "0"));
    instructions.push(abi::branch_eq(&skip));
    emit_graphics_lock(
        symbol,
        scratch,
        "pthread_mutex_lock",
        platform_imports,
        platform,
        instructions,
        relocations,
    )?;
    state_base(symbol, &scratch.base, instructions, relocations);
    emit_bump(scratch, GRAPHICS_OFFSET_PRESENTS, instructions);
    emit_ring_push(
        scratch,
        GRAPHICS_OFFSET_PRESENT_HEAD,
        GRAPHICS_OFFSET_PRESENT_RING,
        now,
        instructions,
    );
    emit_graphics_lock(
        symbol,
        scratch,
        "pthread_mutex_unlock",
        platform_imports,
        platform,
        instructions,
        relocations,
    )?;
    instructions.push(abi::label(&skip));
    Ok(())
}

/// Lock or unlock (`call`) the graphics mutex.
fn emit_graphics_lock(
    symbol: &str,
    scratch: &GraphicsScratch,
    call: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_MUTEX,
    ));
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    };
    emit_thread_external_call(&mut ctx, call)
}

/// `canvas::frameCounters(now) AS List OF Integer` — one consistent snapshot of the
/// frame statistics (plan-155-B), laid out as [`FRAME_COUNTER_WORDS`] says.
///
/// **One lock, one moment.** `presents` and `drawn` are written by different threads,
/// and `pps - fps` is only meaningful if both are read at the same instant, so every word
/// is copied inside a single critical section. The list is allocated before the lock is
/// taken, so the critical section is loads and stores only.
///
/// Before the graphics thread exists there is no mutex to take and nothing has written
/// a word (both writers run only once it has started), so the words are copied without
/// it — all zeros, with `now` and the display rate filled in.
pub(crate) fn emit_frame_counters(
    builder: &mut CodeBuilder,
    now: &Operand,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(), String> {
    let symbol = builder.current_symbol.clone();
    let now_value = builder.temporary_vreg();
    builder.emit(abi::move_register(&now_value, now));
    let now_slot = builder.spill_to_slot("canvas_counters_now", &now_value);

    let list_type = crate::types::ParameterType::list_of(crate::types::ParameterType::Integer);
    let layout = CollectionTypeLayout::from_type(&list_type)
        .ok_or_else(|| format!("canvas: no layout for {list_type}"))?;
    let block_slot = builder.allocate_stack_object("canvas_counters_block", 8);
    let ok = builder.label("canvas_counters_alloc_ok");
    builder.emit(abi::move_immediate(
        abi::return_register(),
        "Integer",
        &(COLLECTION_HEADER_SIZE + FRAME_COUNTER_WORDS * 8).to_string(),
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&ok));
    builder.emit(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        block_slot,
    ));
    let block = builder.temporary_vreg();
    let words = builder.temporary_vreg();
    let bytes = builder.temporary_vreg();
    builder.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
    builder.emit(abi::move_immediate(
        &words,
        "Integer",
        &FRAME_COUNTER_WORDS.to_string(),
    ));
    builder.emit(abi::move_immediate(
        &bytes,
        "Integer",
        &(FRAME_COUNTER_WORDS * 8).to_string(),
    ));
    builder.emit_write_collection_header_full(&layout, &block, &words, &words, &bytes, &bytes);

    let scratch = GraphicsScratch::new(&mut || builder.temporary_vreg());
    let unlocked = builder.label("canvas_counters_unlocked");
    let copied = builder.label("canvas_counters_copied");
    let started = builder.temporary_vreg();
    // Whether the thread exists cannot change under us: only this thread (the worker)
    // starts it, and nothing stops it before the program ends.
    state_base(
        &symbol,
        &scratch.base,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.emit(abi::load_u64(
        &started,
        &scratch.base,
        GRAPHICS_OFFSET_STARTED,
    ));
    builder.emit(abi::compare_immediate(&started, "0"));
    builder.emit(abi::branch_eq(&unlocked));
    emit_graphics_lock(
        &symbol,
        &scratch,
        "pthread_mutex_lock",
        platform_imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.emit(abi::label(&unlocked));

    // The copy: loads and stores only.
    state_base(
        &symbol,
        &scratch.base,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    let data = builder.temporary_vreg();
    builder.emit(abi::load_u64(&data, abi::stack_pointer(), block_slot));
    builder.emit(abi::add_immediate(&data, &data, COLLECTION_HEADER_SIZE));
    builder.emit(abi::load_u64(
        &scratch.scratch,
        abi::stack_pointer(),
        now_slot,
    ));
    builder.emit(abi::store_u64(&scratch.scratch, &data, 0));
    for (word, offset) in [
        (1, GRAPHICS_OFFSET_PRESENTS),
        (2, GRAPHICS_OFFSET_PRESENT_HEAD),
        (3, GRAPHICS_OFFSET_DRAWN),
        (4, GRAPHICS_OFFSET_DRAWN_HEAD),
        (5, GRAPHICS_OFFSET_LAST_RENDER_NANOS),
        (6, GRAPHICS_OFFSET_LAST_RENDERER),
    ] {
        builder.emit(abi::load_u64(&scratch.scratch, &scratch.base, offset));
        builder.emit(abi::store_u64(&scratch.scratch, &data, word * 8));
    }
    // Word 7: the display rate, override first — `canvas::displayMilliHz`'s rule.
    let chosen = builder.label("canvas_counters_display");
    builder.emit(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_DISPLAY_OVERRIDE,
    ));
    builder.emit(abi::compare_immediate(&scratch.scratch, "0"));
    builder.emit(abi::branch_ne(&chosen));
    builder.emit(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_DISPLAY_MHZ,
    ));
    builder.emit(abi::label(&chosen));
    builder.emit(abi::store_u64(&scratch.scratch, &data, 7 * 8));
    for (ring, first_word) in [
        (GRAPHICS_OFFSET_PRESENT_RING, FRAME_COUNTER_HEADER_WORDS),
        (
            GRAPHICS_OFFSET_DRAWN_RING,
            FRAME_COUNTER_HEADER_WORDS + FRAME_RING_SLOTS,
        ),
    ] {
        let again = builder.label("canvas_counters_ring");
        builder.emit(abi::add_immediate(&scratch.cursor, &scratch.base, ring));
        builder.emit(abi::add_immediate(
            &scratch.end,
            &scratch.cursor,
            FRAME_RING_SLOTS * 8,
        ));
        builder.emit(abi::add_immediate(&scratch.arena, &data, first_word * 8));
        builder.emit(abi::label(&again));
        builder.emit(abi::load_u64(&scratch.scratch, &scratch.cursor, 0));
        builder.emit(abi::store_u64(&scratch.scratch, &scratch.arena, 0));
        builder.emit(abi::add_immediate(&scratch.cursor, &scratch.cursor, 8));
        builder.emit(abi::add_immediate(&scratch.arena, &scratch.arena, 8));
        builder.emit(abi::compare_registers(&scratch.cursor, &scratch.end));
        builder.emit(abi::branch_lo(&again));
    }

    builder.emit(abi::compare_immediate(&started, "0"));
    builder.emit(abi::branch_eq(&copied));
    emit_graphics_lock(
        &symbol,
        &scratch,
        "pthread_mutex_unlock",
        platform_imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.emit(abi::label(&copied));
    builder.emit(abi::load_u64(
        RESULT_VALUE_REGISTER,
        abi::stack_pointer(),
        block_slot,
    ));
    Ok(())
}

/// `canvas::frameDone(drew, renderer, renderNanos, now)` — the render loop reports a
/// completed frame.
///
/// Advances the frame counter and wakes anything waiting on it. Broadcast rather
/// than signal: the waiter is a different party from the render loop, and a signal
/// could wake the loop's own wait instead.
///
/// When the frame was drawn (plan-155-B) it also logs it for `canvas::getFrameStats`:
/// the drawn total and ring, the render time and the renderer — inside the same critical
/// section, so the record adds no lock. A wake-up that drew nothing still advances
/// `FRAMES` (a `syncFrame` waiter is released by it) but is not a drawn frame.
pub(crate) fn emit_frame_done(
    symbol: &str,
    scratch: &GraphicsScratch,
    record: &FrameRecord<'_>,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_MUTEX,
    ));
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    };
    emit_thread_external_call(&mut ctx, "pthread_mutex_lock")?;

    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_FRAMES,
    ));
    instructions.push(abi::add_immediate(&scratch.scratch, &scratch.scratch, 1));
    instructions.push(abi::store_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_FRAMES,
    ));
    let not_drawn = format!("{symbol}_not_drawn");
    instructions.push(abi::compare_immediate(record.drew, "0"));
    instructions.push(abi::branch_eq(&not_drawn));
    emit_bump(scratch, GRAPHICS_OFFSET_DRAWN, instructions);
    emit_ring_push(
        scratch,
        GRAPHICS_OFFSET_DRAWN_HEAD,
        GRAPHICS_OFFSET_DRAWN_RING,
        record.now,
        instructions,
    );
    instructions.push(abi::store_u64(
        record.render_nanos,
        &scratch.base,
        GRAPHICS_OFFSET_LAST_RENDER_NANOS,
    ));
    instructions.push(abi::store_u64(
        record.renderer,
        &scratch.base,
        GRAPHICS_OFFSET_LAST_RENDERER,
    ));
    instructions.push(abi::label(&not_drawn));
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_COND,
    ));
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    };
    emit_thread_external_call(&mut ctx, "pthread_cond_broadcast")?;

    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_MUTEX,
    ));
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    };
    emit_thread_external_call(&mut ctx, "pthread_mutex_unlock")?;
    Ok(())
}

/// Wake every `canvas::waitFrame` waiter (plan-155-C). The caller holds the mutex.
fn emit_frame_cond_broadcast(
    symbol: &str,
    scratch: &GraphicsScratch,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_FRAME_COND,
    ));
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    };
    emit_thread_external_call(&mut ctx, "pthread_cond_broadcast")
}

/// `CLOCK_REALTIME`, the clock a default condition variable's deadline is in — the
/// same number on Linux and Darwin, and ignored by the Windows `clock_gettime` shim,
/// whose single clock is the one its `pthread_cond_timedwait` arm reads.
const CLOCK_REALTIME: &str = "0";
const NANOS_PER_SECOND: &str = "1000000000";

/// `now` = `CLOCK_REALTIME` in nanoseconds, through the timespec at `[sp + ts]`.
fn emit_realtime_nanos(
    builder: &mut CodeBuilder,
    ts: usize,
    now: &crate::codegen::engine::operand::VirtualRegister,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(), String> {
    let symbol = builder.current_symbol.clone();
    builder.emit(abi::move_immediate(
        abi::c_arg(0),
        "Integer",
        CLOCK_REALTIME,
    ));
    builder.emit(abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), ts));
    let mut ctx = EmitCtx {
        symbol: &symbol,
        platform_imports,
        platform,
        instructions: &mut builder.instructions,
        relocations: &mut builder.relocations,
    };
    emit_thread_external_call(&mut ctx, "clock_gettime")?;
    let scale = builder.temporary_vreg();
    let nanos = builder.temporary_vreg();
    builder.emit(abi::load_u64(now, abi::stack_pointer(), ts));
    builder.emit(abi::load_u64(&nanos, abi::stack_pointer(), ts + 8));
    builder.emit(abi::move_immediate(&scale, "Integer", NANOS_PER_SECOND));
    builder.emit(abi::multiply_registers(now, now, &scale));
    builder.emit(abi::add_registers(now, now, &nanos));
    Ok(())
}

/// `canvas::awaitPickup(intervalNanos)` — the wait inside `canvas::waitFrame`
/// (plan-155-C). The caller has started the graphics thread, so the mutex exists.
///
/// Returns once **both** hold: the render loop has picked up every redraw asked for
/// (`PENDING` clear), and `intervalNanos` have passed since the last return
/// (`LAST_WAIT`). Or at once, when the thread is stopping. It waits on
/// `GRAPHICS_OFFSET_FRAME_COND` only — untimed while a redraw is owed, timed until the
/// deadline otherwise — and never signals anything, so it cannot cause a frame.
///
/// **Picked up, not drawn.** Waiting for the frame to finish would serialize the two
/// threads: the worker idle while the frame renders, the renderer idle while the worker
/// builds the next scene. Pickup leaves one frame in flight.
///
/// Times are `CLOCK_REALTIME`, the clock `pthread_cond_timedwait` takes. So that a
/// wall-clock step backwards cannot stall the loop, the deadline is never later than
/// `now + interval`; a step costs at most one interval. `LAST_WAIT` becomes the deadline
/// when the return is less than one interval late, so the wake-up latency does not
/// stretch every interval, and the time of the return otherwise, so a loop that fell
/// behind gets no burst of catch-up returns.
pub(crate) fn emit_await_pickup(
    builder: &mut CodeBuilder,
    interval: &Operand,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(), String> {
    let symbol = builder.current_symbol.clone();
    let interval_value = builder.temporary_vreg();
    builder.emit(abi::move_register(&interval_value, interval));
    let ts = builder.allocate_stack_object("canvas_await_ts", 16);
    let scratch = GraphicsScratch::new(&mut || builder.temporary_vreg());
    let now = builder.temporary_vreg();
    let deadline = builder.temporary_vreg();
    let cap = builder.temporary_vreg();
    let again = builder.label("canvas_await_again");
    let owed = builder.label("canvas_await_owed");
    let done = builder.label("canvas_await_done");
    let capped = builder.label("canvas_await_capped");

    emit_graphics_lock(
        &symbol,
        &scratch,
        "pthread_mutex_lock",
        platform_imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    // deadline = min(LAST_WAIT + interval, now + interval)
    emit_realtime_nanos(builder, ts, &now, platform_imports, platform)?;
    state_base(
        &symbol,
        &scratch.base,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.emit(abi::load_u64(
        &deadline,
        &scratch.base,
        GRAPHICS_OFFSET_LAST_WAIT,
    ));
    builder.emit(abi::add_registers(&deadline, &deadline, &interval_value));
    builder.emit(abi::add_registers(&cap, &now, &interval_value));
    builder.emit(abi::compare_registers(&deadline, &cap));
    builder.emit(abi::branch_le(&capped));
    builder.emit(abi::move_register(&deadline, &cap));
    builder.emit(abi::label(&capped));

    builder.emit(abi::label(&again));
    state_base(
        &symbol,
        &scratch.base,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.emit(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_STOPPING,
    ));
    builder.emit(abi::compare_immediate(&scratch.scratch, "0"));
    builder.emit(abi::branch_ne(&done));
    builder.emit(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_PENDING,
    ));
    builder.emit(abi::compare_immediate(&scratch.scratch, "0"));
    builder.emit(abi::branch_ne(&owed));
    emit_realtime_nanos(builder, ts, &now, platform_imports, platform)?;
    builder.emit(abi::compare_registers(&now, &deadline));
    builder.emit(abi::branch_ge(&done));
    // The floor: a timed wait until the deadline, as an absolute timespec.
    let scale = builder.temporary_vreg();
    let seconds = builder.temporary_vreg();
    let rest = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&scale, "Integer", NANOS_PER_SECOND));
    builder.emit(abi::unsigned_divide_registers(&seconds, &deadline, &scale));
    builder.emit(abi::multiply_subtract_registers(
        &rest, &seconds, &scale, &deadline,
    ));
    builder.emit(abi::store_u64(&seconds, abi::stack_pointer(), ts));
    builder.emit(abi::store_u64(&rest, abi::stack_pointer(), ts + 8));
    state_base(
        &symbol,
        &scratch.base,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.emit(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_FRAME_COND,
    ));
    builder.emit(abi::add_immediate(
        abi::c_arg(1),
        &scratch.base,
        GRAPHICS_OFFSET_MUTEX,
    ));
    builder.emit(abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), ts));
    {
        let mut ctx = EmitCtx {
            symbol: &symbol,
            platform_imports,
            platform,
            instructions: &mut builder.instructions,
            relocations: &mut builder.relocations,
        };
        emit_thread_external_call(&mut ctx, "pthread_cond_timedwait")?;
    }
    builder.emit(abi::branch(&again));

    // A redraw is owed: wait, untimed, for the render loop to take it (or stop).
    builder.emit(abi::label(&owed));
    builder.emit(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_FRAME_COND,
    ));
    builder.emit(abi::add_immediate(
        abi::c_arg(1),
        &scratch.base,
        GRAPHICS_OFFSET_MUTEX,
    ));
    {
        let mut ctx = EmitCtx {
            symbol: &symbol,
            platform_imports,
            platform,
            instructions: &mut builder.instructions,
            relocations: &mut builder.relocations,
        };
        emit_thread_external_call(&mut ctx, "pthread_cond_wait")?;
    }
    builder.emit(abi::branch(&again));

    // The next interval runs from the deadline when this return is on schedule (less
    // than one interval late), and from now when it is not. Measuring from the return
    // alone adds the wake-up latency to every interval — measured: a 120 Hz loop ran
    // at 99-100 presents a second — while measuring from the deadline alone would let a
    // loop that fell behind return back-to-back until it caught up.
    builder.emit(abi::label(&done));
    let on_time = builder.label("canvas_await_on_time");
    let stamped = builder.label("canvas_await_stamped");
    emit_realtime_nanos(builder, ts, &now, platform_imports, platform)?;
    state_base(
        &symbol,
        &scratch.base,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.emit(abi::subtract_registers(&cap, &now, &deadline));
    builder.emit(abi::compare_registers(&cap, &interval_value));
    builder.emit(abi::branch_lt(&on_time));
    builder.emit(abi::store_u64(
        &now,
        &scratch.base,
        GRAPHICS_OFFSET_LAST_WAIT,
    ));
    builder.emit(abi::branch(&stamped));
    builder.emit(abi::label(&on_time));
    builder.emit(abi::store_u64(
        &deadline,
        &scratch.base,
        GRAPHICS_OFFSET_LAST_WAIT,
    ));
    builder.emit(abi::label(&stamped));
    emit_graphics_lock(
        &symbol,
        &scratch,
        "pthread_mutex_unlock",
        platform_imports,
        platform,
        &mut builder.instructions,
        &mut builder.relocations,
    )
}

/// `canvas::syncFrame()` — wait for the frame this present asked for.
///
/// A no-op unless `MFB_CANVAS_SYNC` was set, which is the whole point: it exists so
/// a test can make frame-level assertions without racing the scheduler, and it must
/// not put a wait on the production present path.
pub(crate) fn emit_sync_frame(
    symbol: &str,
    scratch: &GraphicsScratch,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let skip = format!("{symbol}_no_sync");
    let retry = format!("{symbol}_sync_wait");
    let ready = format!("{symbol}_sync_ready");

    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_SYNC,
    ));
    instructions.push(abi::compare_immediate(&scratch.scratch, "0"));
    instructions.push(abi::branch_eq(&skip));

    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_MUTEX,
    ));
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    };
    emit_thread_external_call(&mut ctx, "pthread_mutex_lock")?;

    instructions.push(abi::label(&retry));
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_FRAMES,
    ));
    instructions.push(abi::load_u64(
        &scratch.verdict,
        &scratch.base,
        GRAPHICS_OFFSET_WANTED,
    ));
    instructions.push(abi::compare_registers(&scratch.scratch, &scratch.verdict));
    instructions.push(abi::branch_ge(&ready));
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_COND,
    ));
    instructions.push(abi::add_immediate(
        abi::c_arg(1),
        &scratch.base,
        GRAPHICS_OFFSET_MUTEX,
    ));
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    };
    emit_thread_external_call(&mut ctx, "pthread_cond_wait")?;
    instructions.push(abi::branch(&retry));

    instructions.push(abi::label(&ready));
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        &scratch.base,
        GRAPHICS_OFFSET_MUTEX,
    ));
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions,
        relocations,
    };
    emit_thread_external_call(&mut ctx, "pthread_mutex_unlock")?;
    instructions.push(abi::label(&skip));
    Ok(())
}

/// `canvas::setSyncMode(on)` — record whether `present` should wait for its frame.
///
/// Set from MFBASIC, which is where the environment is portably readable, rather
/// than by calling `getenv` from the spawn: the env plumbing already exists there
/// and differs per platform (`GetEnvironmentVariableW` on Windows), and this is one
/// boolean read once.
pub(crate) fn emit_set_sync_mode(
    symbol: &str,
    scratch: &GraphicsScratch,
    value: &Operand,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::store_u64(value, &scratch.base, GRAPHICS_OFFSET_SYNC));
}

/// The surface's startup size, and the value a zero field reads as.
///
/// Matches what all three platform surfaces are created at
/// (`emit_reconcile_canvas_helper`, `RECONCILE_BUILD_SYMBOL`, the Win32
/// `CreateWindowExW`), so a program that never resizes is unaffected by the resize
/// path existing.
pub(crate) const DEFAULT_SURFACE_WIDTH: usize = 900;
pub(crate) const DEFAULT_SURFACE_HEIGHT: usize = 640;

/// One of the surface's two dimensions: where its word is in the graphics state, and
/// what a zero there reads as.
#[derive(Clone, Copy)]
pub(crate) enum SurfaceDimension {
    Width,
    Height,
}

impl SurfaceDimension {
    fn offset(self) -> usize {
        match self {
            SurfaceDimension::Width => GRAPHICS_OFFSET_WIDTH,
            SurfaceDimension::Height => GRAPHICS_OFFSET_HEIGHT,
        }
    }

    fn startup(self) -> usize {
        match self {
            SurfaceDimension::Width => DEFAULT_SURFACE_WIDTH,
            SurfaceDimension::Height => DEFAULT_SURFACE_HEIGHT,
        }
    }

    /// Load this dimension of the graphics state at `base` into `dst`, as the program
    /// sees it: a zero word is the startup size.
    ///
    /// The ONE reading of a size word. `canvas::getSize` answers through it
    /// ([`emit_surface_dimension`]) and the resize publisher decides "did the size
    /// change" through it ([`emit_publish_surface_size_tagged`]), so the two cannot
    /// give 0 different meanings (bug-970). `ready` is a label the caller owns.
    fn emit_load(
        self,
        dst: abi::Reg,
        base: abi::Reg,
        ready: &str,
        instructions: &mut Vec<CodeInstruction>,
    ) {
        instructions.push(abi::load_u64(dst, base, self.offset()));
        instructions.push(abi::compare_immediate(dst, "0"));
        instructions.push(abi::branch_ne(ready));
        instructions.push(abi::move_immediate(
            dst,
            "Integer",
            &self.startup().to_string(),
        ));
        instructions.push(abi::label(ready));
    }
}

/// Read one published surface dimension, falling back to the startup size.
pub(crate) fn emit_surface_dimension(
    symbol: &str,
    scratch: &GraphicsScratch,
    dimension: SurfaceDimension,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let ready = format!("{symbol}_dim_ready_{}", dimension.offset());
    state_base(symbol, &scratch.base, instructions, relocations);
    dimension.emit_load(
        (&scratch.scratch).into(),
        (&scratch.base).into(),
        &ready,
        instructions,
    );
    instructions.push(abi::move_register(RESULT_VALUE_REGISTER, &scratch.scratch));
}

/// plan-172-K, trigger 6: branch to `skip` unless the scene the graphics thread last
/// drew holds a `canvas::AutoPaint` ([`GRAPHICS_OFFSET_AUTOPAINT_LIVE`]).
///
/// Every platform's pointer handler emits this after its publish and then calls
/// `canvas::signalRedraw` with 1 in the first argument register — "the pointer
/// changed", not "a redraw is owed" ([`emit_signal_redraw`]) — so a pointer change
/// wakes the render loop exactly when something in the scene could change its paint
/// for it. `base` is overwritten. It is not part of
/// [`emit_publish_pointer`] because the signal is a call — it takes the graphics mutex
/// — and that writer is emitted into leaf functions with no frame.
pub(crate) fn emit_skip_unless_autopaint_live(
    symbol: &str,
    base: impl Into<abi::Reg>,
    skip: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let base: abi::Reg = base.into();
    state_base(symbol, base, instructions, relocations);
    instructions.push(abi::load_u64(base, base, GRAPHICS_OFFSET_AUTOPAINT_LIVE));
    instructions.push(abi::compare_immediate(base, "0"));
    instructions.push(abi::branch_eq(skip));
}

/// The argument each call to `canvas::signalRedraw` in `instructions` is made with, in
/// order: the immediate moved into the first argument register by the instruction
/// right before the call. A call with anything else in front of it is a panic — a
/// platform handler that signals without saying which kind of signal it is.
#[cfg(test)]
pub(crate) fn signal_redraw_arguments(instructions: &[CodeInstruction]) -> Vec<String> {
    let first_argument = abi::Reg::from(abi::mfb_arg(0)).to_string();
    let mut out = Vec::new();
    for (index, instruction) in instructions.iter().enumerate() {
        if instruction.get("target").as_deref() != Some("_mfb_rt_canvas_canvas_signalRedraw") {
            continue;
        }
        let before = index
            .checked_sub(1)
            .map(|at| &instructions[at])
            .expect("a signal is never a function's first instruction");
        assert_eq!(
            before.get("dst").as_deref(),
            Some(first_argument.as_str()),
            "canvas::signalRedraw is called with its argument register unset"
        );
        out.push(before.get("value").expect("an immediate argument"));
    }
    out
}

/// The registers [`emit_publish_pointer`] is given: the four values to publish, and
/// three it may overwrite. A struct because every caller is a hand-written platform
/// handler that spells its own registers, and seven positional registers of one type
/// is seven chances to swap two.
pub(crate) struct PointerRegs {
    pub(crate) x: abi::Reg,
    pub(crate) y: abi::Reg,
    pub(crate) buttons: abi::Reg,
    pub(crate) inside: abi::Reg,
    /// Overwritten: the graphics-state base.
    pub(crate) base: abi::Reg,
    /// Overwritten: the sequence word's value.
    pub(crate) seq: abi::Reg,
    /// Overwritten: an address, where the stores are store-releases.
    pub(crate) addr: abi::Reg,
}

/// Publish the pointer from a platform's pointer event (plan-172-J).
///
/// Called on the **main** thread — or, for the debug script, on the graphics thread
/// through `canvas::publishPointer`; never both at once in a run that means anything —
/// with the four values in registers. One writer at a time, so the sequence word is a
/// plain increment, not a compare-and-swap: odd, the four stores, even.
///
/// `ordered` is whether the target needs the accesses ordered explicitly
/// (`scene_base::orders_scene_accesses`: AArch64). There every store is a
/// store-release, which is visible only after every earlier access of this thread —
/// so no data word can be seen before the odd mark, nor the even mark before the data
/// — and the reader's loads are load-acquires. x86-64 keeps stores in order and loads
/// in order by itself.
///
/// Makes no call and touches no register outside [`PointerRegs`]. Signals nothing: a
/// pointer change is a redraw only while the scene holds a `canvas::AutoPaint`, and the
/// caller decides that ([`emit_skip_unless_autopaint_live`]).
pub(crate) fn emit_publish_pointer(
    symbol: &str,
    ordered: bool,
    regs: &PointerRegs,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let store = |value: abi::Reg, offset: usize, ins: &mut Vec<CodeInstruction>| {
        if ordered {
            ins.push(abi::add_immediate(regs.addr, regs.base, offset));
            ins.push(abi::store_release_u64(value, regs.addr));
        } else {
            ins.push(abi::store_u64(value, regs.base, offset));
        }
    };
    state_base(symbol, regs.base, instructions, relocations);
    instructions.push(abi::load_u64(
        regs.seq,
        regs.base,
        GRAPHICS_OFFSET_POINTER_SEQ,
    ));
    instructions.push(abi::add_immediate(regs.seq, regs.seq, 1));
    store(regs.seq, GRAPHICS_OFFSET_POINTER_SEQ, instructions);
    for (value, offset) in [regs.x, regs.y, regs.buttons, regs.inside]
        .into_iter()
        .zip(GRAPHICS_POINTER_WORDS)
    {
        store(value, offset, instructions);
    }
    instructions.push(abi::add_immediate(regs.seq, regs.seq, 1));
    store(regs.seq, GRAPHICS_OFFSET_POINTER_SEQ, instructions);
}

/// Publish a new surface size from the platform's resize event.
///
/// Called on the **main** thread with the new dimensions already in the two given
/// registers. Two plain aligned stores, no lock: the graphics thread reads them at
/// frame start and a torn pair would at worst render one frame at a mixed size,
/// which the next frame corrects — whereas taking the render mutex from a resize
/// callback would block the UI thread behind a frame.
///
/// The caller signals a redraw afterwards; publishing without signalling would leave
/// the new size unused until something else asked for a frame.
pub(crate) fn emit_publish_surface_size(
    symbol: &str,
    base: impl Into<abi::Reg>,
    width: impl Into<abi::Reg>,
    height: impl Into<abi::Reg>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    emit_publish_surface_size_tagged(symbol, "", base, width, height, instructions, relocations);
}

/// [`emit_publish_surface_size`] for a function that publishes from more than one
/// site: `tag` keeps each site's labels distinct (bug-688 V6, the Windows window
/// procedure publishes from `WM_SIZE` and from the canvas reconcile).
pub(crate) fn emit_publish_surface_size_tagged(
    symbol: &str,
    tag: &str,
    base: impl Into<abi::Reg>,
    width: impl Into<abi::Reg>,
    height: impl Into<abi::Reg>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let base: abi::Reg = base.into();
    let height: abi::Reg = height.into();
    let width: abi::Reg = width.into();
    let unchanged = format!("{symbol}{tag}_resize_same");
    let changed = format!("{symbol}{tag}_resize_changed");
    state_base(symbol, base, instructions, relocations);
    // Count the resize only when the size actually changed. Every platform re-publishes
    // on events that are not resizes — AppKit sends `setFrameSize:` with the size it
    // already has, and the headless scripted resize does the same — and
    // `canvas::didResize` promises the program a *change*, not an event.
    //
    // "Changed" is against the size the program SEES, which is the startup size while
    // the words are still zero (bug-970): a window that opens at that size publishes
    // what `canvas::getSize` already answered, and one that opens at another size
    // publishes a change. Nothing here knows whether a publish is the first.
    //
    // The compare reads the old values before the stores overwrite them, and uses the
    // scratch the caller already gave us rather than a fifth register: this runs in a
    // platform resize callback, where the register budget is whatever the caller had.
    SurfaceDimension::Width.emit_load(
        SCRATCH_RESIZE,
        base,
        &format!("{symbol}{tag}_resize_old_width"),
        instructions,
    );
    instructions.push(abi::compare_registers(SCRATCH_RESIZE, width));
    instructions.push(abi::branch_ne(&changed));
    SurfaceDimension::Height.emit_load(
        SCRATCH_RESIZE,
        base,
        &format!("{symbol}{tag}_resize_old_height"),
        instructions,
    );
    instructions.push(abi::compare_registers(SCRATCH_RESIZE, height));
    instructions.push(abi::branch_eq(&unchanged));
    instructions.push(abi::label(&changed));
    instructions.push(abi::load_u64(SCRATCH_RESIZE, base, GRAPHICS_OFFSET_RESIZES));
    instructions.push(abi::add_immediate(SCRATCH_RESIZE, SCRATCH_RESIZE, 1));
    instructions.push(abi::store_u64(
        SCRATCH_RESIZE,
        base,
        GRAPHICS_OFFSET_RESIZES,
    ));
    instructions.push(abi::label(&unchanged));
    instructions.push(abi::store_u64(width, base, GRAPHICS_OFFSET_WIDTH));
    instructions.push(abi::store_u64(height, base, GRAPHICS_OFFSET_HEIGHT));
}

/// Publish the display's refresh rate, in millihertz, from the platform's main thread
/// (plan-155-A). `mhz` holds the value; `base` is a register the caller can spare.
///
/// One plain aligned store and nothing else — no lock, for `emit_publish_surface_size`'s
/// reason, and **no redraw**: a new rate changes no pixel, and `.ai/canvas-threading.md`
/// §4's triggers stay five. Readers want "some recent value", and a store of a word is
/// never torn.
///
/// Takes operands rather than register names so a caller can pass an ABI role token
/// (the Windows helper uses the Win64 argument registers, spelled `abi::mfb_arg(n)`):
/// rendering such a token to a string loses the role the selector realizes it from.
pub(crate) fn emit_publish_display_rate(
    symbol: &str,
    base: impl Into<abi::Reg>,
    mhz: impl Into<abi::Reg>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let base: abi::Reg = base.into();
    push_symbol_address(
        symbol,
        GRAPHICS_STATE_SYMBOL,
        &base,
        instructions,
        relocations,
    );
    instructions.push(abi::store_u64(mhz, &base, GRAPHICS_OFFSET_DISPLAY_MHZ));
}

/// `canvas::displayMilliHz() AS Integer` — the rate, override first; `0` when unknown.
pub(crate) fn emit_display_milli_hz(
    symbol: &str,
    scratch: &GraphicsScratch,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let chosen = format!("{symbol}_display_override");
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_DISPLAY_OVERRIDE,
    ));
    instructions.push(abi::compare_immediate(&scratch.scratch, "0"));
    instructions.push(abi::branch_ne(&chosen));
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_DISPLAY_MHZ,
    ));
    instructions.push(abi::label(&chosen));
    instructions.push(abi::move_register(RESULT_VALUE_REGISTER, &scratch.scratch));
}

/// `canvas::setDisplayOverride(mhz)` — record `MFB_CANVAS_DISPLAY_HZ`, read from
/// MFBASIC beside `setSyncMode` for the same reason: the environment is portably
/// readable there.
pub(crate) fn emit_set_display_override(
    symbol: &str,
    scratch: &GraphicsScratch,
    value: &Operand,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::store_u64(
        value,
        &scratch.base,
        GRAPHICS_OFFSET_DISPLAY_OVERRIDE,
    ));
}

/// The one scratch register `emit_publish_surface_size` needs for its compare.
///
/// Named here rather than taken as a parameter because every caller is a hand-written
/// platform resize callback that already spells its own registers, and adding a fifth
/// argument to all of them to pass a register they all have spare is churn. `SCRATCH[5]`
/// is above the four each caller stages its own values in.
const SCRATCH_RESIZE: abi::Reg = abi::SCRATCH[5];

/// `canvas::didResize()` — has the surface changed size since this was last asked?
///
/// Read-and-acknowledge: the worker compares the platform's resize counter against the
/// value it last reported and, if they differ, records the new one and answers TRUE. So
/// the answer is TRUE exactly once per resize however many resizes happened in between,
/// and no lock is involved — the two words have one writer each.
pub(crate) fn emit_did_resize(
    symbol: &str,
    scratch: &GraphicsScratch,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let same = format!("{symbol}_resize_seen");
    let done = format!("{symbol}_resize_done");
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_RESIZES,
    ));
    instructions.push(abi::load_u64(
        &scratch.arena,
        &scratch.base,
        GRAPHICS_OFFSET_RESIZES_SEEN,
    ));
    instructions.push(abi::compare_registers(&scratch.scratch, &scratch.arena));
    instructions.push(abi::branch_eq(&same));
    instructions.push(abi::store_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_RESIZES_SEEN,
    ));
    instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"));
    instructions.push(abi::branch(&done));
    instructions.push(abi::label(&same));
    instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
    instructions.push(abi::label(&done));
}

/// `canvas::setGpuMode(on)` — record whether a GPU renderer was asked for.
///
/// Read from MFBASIC at first present, next to `setSyncMode`, for the same reason:
/// the environment is portably readable there.
pub(crate) fn emit_set_gpu_mode(
    symbol: &str,
    scratch: &GraphicsScratch,
    value: &Operand,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::store_u64(value, &scratch.base, GRAPHICS_OFFSET_GPU));
}

/// `canvas::useGpu()` — did the program ask for a GPU renderer?
///
/// The flag and nothing else, on every target. It used to hard-return FALSE off
/// macOS, back when it was named `useMetal` and meant "is Metal selected"; by the
/// time a second backend existed that early return silently made `MFB_CANVAS_GPU=1`
/// a no-op on Linux (plan-98-F Correction 4). "Was a GPU asked for" and "is one
/// usable here" are different questions, and `canvas::metalReady` /
/// `canvas::vulkanReady` answer the second.
pub(crate) fn emit_use_gpu(
    symbol: &str,
    scratch: &GraphicsScratch,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_GPU,
    ));
    instructions.push(abi::move_register(RESULT_VALUE_REGISTER, &scratch.scratch));
}

#[cfg(test)]
mod tests {
    use super::{
        emit_publish_surface_size, DEFAULT_SURFACE_HEIGHT, DEFAULT_SURFACE_WIDTH,
        GRAPHICS_OFFSET_HEIGHT, GRAPHICS_OFFSET_RESIZES, GRAPHICS_OFFSET_WIDTH,
    };
    use super::{graphics_trampoline_saved, graphics_trampoline_shadow, thread_entry_frame};

    /// The graphics entry's frame: the shared frame arithmetic at its own save count.
    fn graphics_trampoline_frame(arch: &str, windows: bool) -> usize {
        thread_entry_frame(
            arch,
            windows,
            graphics_trampoline_saved(arch, windows).len(),
        )
    }
    use crate::arch::ops::CodeOp;
    use crate::target::shared::abi;
    use std::collections::HashMap;

    /// Run the size publisher's own instructions over a graphics state held as
    /// `offset -> word`, publishing `width` by `height`.
    ///
    /// The publisher is a straight run of loads, compares, branches and stores on the
    /// registers it was handed, the same at every platform's resize callback, so it can
    /// be executed here rather than read: what it leaves in the state is the contract.
    /// An instruction this does not know fails the test.
    fn publish(state: &mut HashMap<usize, u64>, width: u64, height: u64) {
        let mut instructions = Vec::new();
        let mut relocations = Vec::new();
        emit_publish_surface_size(
            "resize_test",
            abi::LOCAL[3],
            abi::LOCAL[1],
            abi::LOCAL[2],
            &mut instructions,
            &mut relocations,
        );
        let name = |reg: abi::Reg| {
            abi::move_register(reg, reg)
                .get("dst")
                .expect("a register renders")
        };
        let base = name(abi::LOCAL[3]);
        let mut registers: HashMap<String, u64> =
            HashMap::from([(name(abi::LOCAL[1]), width), (name(abi::LOCAL[2]), height)]);
        let field = |at: usize, key: &str| -> String {
            instructions[at]
                .get(key)
                .unwrap_or_else(|| panic!("instruction {at} has no `{key}`"))
        };
        let label = |target: &str| -> usize {
            instructions
                .iter()
                .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some(target))
                .unwrap_or_else(|| panic!("no label `{target}`"))
        };
        // The address of the state is the first thing emitted; everything after it
        // reaches the state through `base`.
        let mut at = instructions
            .iter()
            .position(|i| i.op == CodeOp::LdrU64)
            .expect("the publisher reads the old size");
        let mut flags = (0u64, 0u64);
        while at < instructions.len() {
            let op = instructions[at].op;
            let mut next = at + 1;
            match op {
                CodeOp::Label => {}
                CodeOp::LdrU64 => {
                    assert_eq!(field(at, "base"), base, "a load not from the state");
                    let offset: usize = field(at, "offset").parse().expect("an offset");
                    let value = state.get(&offset).copied().unwrap_or(0);
                    registers.insert(field(at, "dst"), value);
                }
                CodeOp::StrU64 => {
                    assert_eq!(field(at, "base"), base, "a store not to the state");
                    let offset: usize = field(at, "offset").parse().expect("an offset");
                    state.insert(offset, registers[&field(at, "src")]);
                }
                CodeOp::Cmp => flags = (registers[&field(at, "lhs")], registers[&field(at, "rhs")]),
                CodeOp::CmpImm => {
                    flags = (
                        registers[&field(at, "lhs")],
                        field(at, "rhs").parse().expect("an immediate"),
                    )
                }
                CodeOp::MovImm => {
                    registers.insert(
                        field(at, "dst"),
                        field(at, "value").parse().expect("an immediate"),
                    );
                }
                CodeOp::AddImm => {
                    let sum = registers[&field(at, "src")]
                        + field(at, "imm").parse::<u64>().expect("an immediate");
                    registers.insert(field(at, "dst"), sum);
                }
                CodeOp::BranchEq => {
                    if flags.0 == flags.1 {
                        next = label(&field(at, "target"));
                    }
                }
                CodeOp::BranchNe => {
                    if flags.0 != flags.1 {
                        next = label(&field(at, "target"));
                    }
                }
                other => panic!("the size publisher emitted `{other:?}`, which this does not run"),
            }
            at = next;
        }
    }

    /// bug-970. A size the program already sees is not a resize, and the first publish
    /// is not special.
    ///
    /// A zero size word reads as the startup size (`canvas::getSize` answers 900 by 640
    /// before any window exists), so a window that opens AT that size publishes no
    /// change, and `canvas::didResize` must not answer TRUE for it. A window that opens
    /// at another size (Windows' first client area is 384 by 261) did change what the
    /// program sees, and counts.
    #[test]
    fn publishing_the_size_the_program_already_sees_counts_no_resize() {
        let (w, h) = (DEFAULT_SURFACE_WIDTH as u64, DEFAULT_SURFACE_HEIGHT as u64);
        let resizes =
            |state: &HashMap<usize, u64>| state.get(&GRAPHICS_OFFSET_RESIZES).copied().unwrap_or(0);

        let mut state = HashMap::new();
        publish(&mut state, w, h);
        assert_eq!(
            resizes(&state),
            0,
            "the first publish of the startup size is the size `getSize` already reported"
        );
        assert_eq!(state[&GRAPHICS_OFFSET_WIDTH], w);
        assert_eq!(state[&GRAPHICS_OFFSET_HEIGHT], h);
        publish(&mut state, w, h);
        assert_eq!(resizes(&state), 0, "a repeat of the size it has");
        // Real resizes count once each, one dimension or both.
        publish(&mut state, 640, h);
        assert_eq!(resizes(&state), 1, "the width changed");
        publish(&mut state, 640, 480);
        assert_eq!(resizes(&state), 2, "the height changed");
        publish(&mut state, 640, 480);
        assert_eq!(resizes(&state), 2, "a repeat of the size it has");
        publish(&mut state, w, h);
        assert_eq!(resizes(&state), 3, "back to the startup size IS a change");

        // A first publish at any other size is a change from what the program saw.
        for (width, height) in [(384, 261), (w, 261), (384, h)] {
            let mut state = HashMap::new();
            publish(&mut state, width, height);
            assert_eq!(resizes(&state), 1, "first publish of {width} by {height}");
        }
    }

    /// bug-479. The graphics thread ran eight bytes out of alignment on Windows for as
    /// long as canvas mode existed there, because this frame skipped the x86-64 realign
    /// on the (false) premise that `BaseThreadInitThunk` enters a start routine already
    /// 16-aligned. Nothing noticed until `SleepConditionVariableSRW`, which builds its
    /// wait block on the caller's stack and tags the pointer in its low 4 bits — eight
    /// bytes out, ntdll walked a garbage list and faulted with every argument correct.
    ///
    /// The invariant is the same on every x86-64 OS, which is the point: a thread entry
    /// is reached by a `call`, so `rsp % 16 == 8` on arrival, and a frame of `8 (mod 16)`
    /// is what puts the next `call` back on an aligned boundary.
    #[test]
    fn every_x86_64_graphics_trampoline_frame_realigns_the_stack() {
        for windows in [false, true] {
            let frame = graphics_trampoline_frame("x86_64", windows);
            assert_eq!(
                frame % 16,
                8,
                "x86_64 (windows={windows}) trampoline frame {frame} leaves every call \
                 the render loop makes misaligned by 8"
            );
        }
    }

    /// AArch64 is entered with a 16-aligned `sp` and the return address in `lr`, so it
    /// takes no realign — asserted so a future "just make it uniform" does not silently
    /// skew the Mac and the Linux arm64 boxes to fix Windows.
    #[test]
    fn aarch64_takes_no_realign() {
        assert_eq!(graphics_trampoline_frame("aarch64", false) % 16, 0);
    }

    /// The saves must sit ABOVE the shadow space, or a callee spilling its register
    /// arguments overwrites the saved return address and registers. The emitter
    /// stores `lr` at `shadow` and each saved register in the slot after it, so the
    /// frame has to hold all of them beyond it.
    #[test]
    fn the_frame_holds_every_save_above_the_shadow_space() {
        for (arch, windows) in [("x86_64", true), ("x86_64", false), ("aarch64", false)] {
            let frame = graphics_trampoline_frame(arch, windows);
            let shadow = graphics_trampoline_shadow(windows);
            let saves = 8 * (1 + graphics_trampoline_saved(arch, windows).len());
            assert!(
                frame >= shadow + saves,
                "{arch} (windows={windows}): frame {frame} cannot hold {saves} bytes of \
                 saves above its {shadow}-byte shadow space"
            );
        }
    }

    /// bug-982 audit: the entry saves every callee-saved register the MFB render
    /// loop may leave changed — on Win64 that includes `rdi`/`rsi`, which are
    /// argument registers inside MFB code and callee-saved to the thread library.
    #[test]
    fn win64_saves_the_internal_argument_registers_it_owes_the_thread_library() {
        use crate::target::shared::abi;
        let saved = graphics_trampoline_saved("x86_64", true);
        for register in [abi::mfb_arg(4), abi::mfb_arg(5), abi::mfb_arg(7)] {
            assert!(
                saved.contains(&register),
                "{register} is not saved: {saved:?}"
            );
        }
        assert!(!graphics_trampoline_saved("x86_64", false).contains(&abi::mfb_arg(4)));
    }

    /// Windows owes its callees 32 bytes; nothing else does.
    #[test]
    fn only_windows_reserves_shadow_space() {
        assert_eq!(graphics_trampoline_shadow(true), 32);
        assert_eq!(graphics_trampoline_shadow(false), 0);
    }
}
