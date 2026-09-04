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

pub(crate) mod metal;
pub(crate) mod vulkan;

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
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
    base: String,
    scratch: String,
    verdict: String,
    arena: String,
    cursor: String,
    end: String,
}

impl GraphicsScratch {
    pub(crate) fn new(vreg: &mut dyn FnMut() -> String) -> Self {
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
/// **Opt-in, not the default.** The software path is the *oracle* the GPU backends
/// are measured against (plan-98-A invariant 7), and its goldens are exact-match; a
/// GPU frame only matches within a tolerance. Making Metal the default would turn
/// every exact-match golden into a tolerance test and destroy the reference they are
/// compared to. `MFB_CANVAS_GPU=1` selects it; plan-98-E's own tests set it.
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
/// `VULKAN_READY` is tri-state (0 untried, 1 built, 2 failed) rather than "is the
/// device non-zero", so a machine with no ICD pays the `dlopen` and the enumeration
/// once instead of per frame.
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
/// `CANVAS_ITEM_BUFFER_BYTES` rather than by a device limit.
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
/// It carries **two regions**: the item blocks from byte 0, then the polygon edges
/// from `METAL_EDGE_BASE_WORDS`. Before this, Metal's edges rode a per-item
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
/// Total block size.
pub(crate) const GRAPHICS_STATE_SIZE: usize = 760;

/// The per-item parameter block both GPU backends push to their shaders.
///
/// **One contract, two emitters.** The block is byte-identical between the Metal
/// pipeline (an MSL `constant MfbItem&`) and the Vulkan one (a GLSL push-constant
/// block) — glslang's reflection reports exactly these offsets and this size — so
/// the shaders agree by construction rather than by two hand-maintained layouts
/// staying in step. Each backend emits the *stores* in its own IR flavour; the
/// *layout* is here, once.
///
/// Six `ivec4`s and a seventh holding the surface size. Every member is an `ivec4` so
/// the two shading languages' packing rules cannot disagree.
///
/// **The bound is the item buffer, not a push-constant range** (plan-116-A). Until
/// then the block rode `vkCmdPushConstants` / `setVertexBytes:` and 112 was chosen to
/// fit Vulkan's *guaranteed* 128-byte range — a hard ceiling, and the only guaranteed
/// one, so widening the block past it would have made the feature set depend on the
/// device. It now travels in `…_VULKAN_ITEM_BUFFER` (Metal:
/// `…_MTL_ITEM_BUFFER`), one record per drawn quad indexed by instance, so the only
/// limit left is capacity: `CANVAS_MAX_FRAME_ITEMS` records must fit
/// `CANVAS_ITEM_BUFFER_BYTES`, which is defined *from* this constant and therefore
/// cannot fall out of step.
///
/// What still constrains the value is **agreement between the two shading languages**,
/// and that is gated rather than trusted: glslang's reflection for the GLSL
/// `ItemBlock` reports `topLevelArrayStride 208` with members at
/// 0/16/32/48/64/80/96/112/128/144/160/176/192 (`glslangValidator -V -q
/// mfb_canvas.vert`, re-measured 2026-09-02 when plan-116-F added the gradient axis
/// `ivec4`), matching the
/// `ITEM_OFFSET_*` constants below one for one. `the_item_block_matches_the_std430_stride`
/// in `vulkan.rs` pins it. Widening the block means keeping every member `ivec4`-sized
/// so std430's stride stays equal to the size, then re-running that reflection.
pub(crate) const ITEM_BLOCK_SIZE: usize = 208;
/// Bounds `minX, minY, maxX, maxY`, 16.16 fixed point.
pub(crate) const ITEM_OFFSET_QUAD: usize = 0;
/// The shape parameters `p0..p3`, 16.16.
pub(crate) const ITEM_OFFSET_SHAPE: usize = 16;
/// Fill and stroke `RGBA`, whole 0–255 values.
pub(crate) const ITEM_OFFSET_FILL: usize = 32;
pub(crate) const ITEM_OFFSET_STROKE: usize = 48;
/// `kind`, `radius` (16.16), `strokeHalf` (16.16), `edgeCount`.
pub(crate) const ITEM_OFFSET_MISC: usize = 64;
/// The arc's `startAngle`, `endAngle` (16.16 radians), then the polygon's first-edge
/// index into the Vulkan edge buffer, then one unused word.
///
/// Slots 2 and 3 are per-kind the same way `HEADER_AUX0` is: an arc never reads the
/// edge base and a polygon never reads the angles, so one `ivec4` carries both rather
/// than each taking its own. Both backends carry the edge base since plan-116-A: each
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
/// Only a `Round`-capped arc reads them. Every item pays 16 bytes of buffer for it,
/// which is the trade: the alternative is a per-pixel `sin`/`cos` pair in three
/// renderers, and the deterministic series the oracle requires is far more expensive
/// than a fetch.
pub(crate) const ITEM_OFFSET_ARC_CAPS: usize = 160;
/// An ellipse's rotation, as `cos, sin` in 16.16, then two unused words (plan-116-E).
///
/// The twelfth `ivec4`. Two words would have fit in an existing block's spare, but
/// std430 rounds the array stride to a multiple of 16 anyway, so a half-used `ivec4`
/// costs exactly what a full one does and keeps every member `ivec4`-sized — which is
/// the property `the_item_block_matches_the_std430_stride` exists to hold.
///
/// **16.16 is enough here and would not be for the radii.** These are a cosine and a
/// sine, so `|v| <= 1` and the fixed-point step is 1/65536 — an angular error under
/// 1.6e-5 rad, which at radius 900 displaces the rim by 0.014 px, a fiftieth of a
/// coverage step. The CPU still evaluates the deterministic Taylor pair; this is only
/// how the answer travels.
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
/// The gradient's kind (0 linear, 1 radial) in `surface.w`, which was the block's last
/// spare word.
pub(crate) const ITEM_SURFACE_GRADIENT_KIND: usize = 12;
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

/// Slots of `__canvas_headerFor`'s fixed 47-float geometry header that the GPU
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
/// Only a `Round`-capped arc reads them, but they are written by every arc, because a
/// per-shape constant computed once at header-build time is the cheap half of this
/// letter: a cap disc centred on the endpoint would otherwise need a `sin`/`cos` pair
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
pub(crate) const HEADER_SLOTS: usize = 47;
/// Doubles per cached polygon edge: `x0, y0, dx, dy, invLenSq`.
pub(crate) const EDGE_SLOTS: usize = 5;
/// The most edges one polygon may carry on the **Metal** path.
///
/// `setFragmentBytes:` copies into the command buffer and is small, so Metal's edges
/// ride a bounded payload sized at compile time. `__canvas_metalRenderable` declines
/// a polygon past this rather than truncating it, because a truncated polygon
/// renders as a *different shape* and would read as a geometry bug.
pub(crate) const MAX_EDGES: usize = 256;

/// The most edges one **frame** may carry on the Vulkan path.
///
/// Vulkan has no per-item limit — the edges live in a storage buffer, not in the
/// push constants — so the real constraint is the buffer, and it is a whole-frame
/// one: every polygon in the scene takes a slice. `__canvas_vulkanRenderable` sums
/// the scene's polygon edges against this and declines the frame to software if the
/// total does not fit, so the emitter's own bound check is unreachable.
///
/// 16384 edges is 256 KiB. It is generous rather than tuned: the fragment shader
/// walks every edge of a polygon per covered pixel, so a scene anywhere near this
/// bound is already too slow to want, on either backend.
pub(crate) const VULKAN_MAX_FRAME_EDGES: usize = 16384;
/// Four 16.16 words per edge — the two endpoints.
pub(crate) const VULKAN_EDGE_BYTES: usize = VULKAN_MAX_FRAME_EDGES * 16;

/// The most **item blocks** one frame may carry — the capacity of the item buffer.
///
/// A *drawn quad*, not a scene item: every non-text item takes one block, and a glyph
/// run takes one per glyph, because each glyph is its own quad with its own block
/// (`GEO_KIND_TEXT`). So the count both predicates sum is "quads", and that is the
/// number this bounds.
///
/// 4096 blocks is 448 KiB at the current `ITEM_BLOCK_SIZE`, and it grows with the
/// block — which is the point: later letters widen the block, and the buffer absorbs
/// that where the 128-byte push-constant range could not.
///
/// Both `*Renderable` predicates sum a frame's quads against this and decline the
/// whole frame to software past it, the same honesty gate `VULKAN_MAX_FRAME_EDGES`
/// already has and for the same reason: a truncated scene is a *different scene*, and
/// software is the oracle, so declining is never worse than drawing.
pub(crate) const CANVAS_MAX_FRAME_ITEMS: usize = 4096;
/// The item buffer's size in bytes — one `ITEM_BLOCK_SIZE` record per quad.
pub(crate) const CANVAS_ITEM_BUFFER_BYTES: usize = CANVAS_MAX_FRAME_ITEMS * ITEM_BLOCK_SIZE;

/// The most edges one **frame** may carry on the Metal path, mirroring
/// `VULKAN_MAX_FRAME_EDGES` (plan-116-A).
///
/// Metal's edges used to ride a per-item `setFragmentBytes:` payload, which is copied
/// into the command buffer at record time — so the cap was per *item* (`MAX_EDGES`)
/// and there was no frame total at all. An instanced draw cannot rebind that payload
/// between instances, so the edges moved into a region of the frame buffer, exactly
/// where Vulkan has always kept them, and the cap became a frame total to match.
///
/// **This is the one scene class that newly declines to software**: a Metal scene
/// whose polygon edges sum past 16384. It previously rendered on the GPU through the
/// unbounded per-item payload. Software is the oracle, so the picture is at least as
/// correct. The per-item `MAX_EDGES` decline is deliberately kept beside this one —
/// unifying the two caps is later work, taken deliberately or not at all.
pub(crate) const METAL_MAX_FRAME_EDGES: usize = 16384;
/// Where Metal's edge region starts inside the frame buffer, in 32-bit words.
///
/// The item blocks come first, so this is simply past them. The shader adds it to each
/// polygon's `ITEM_ARC_EDGE_BASE` rather than reading a separately-offset binding,
/// which is the same shape `VULKAN_GLYPH_BASE_WORDS` already uses — and it sidesteps
/// `MTLBuffer` offset alignment entirely, since nothing is ever bound at a non-zero
/// offset. `the_metal_shader_region_bases_match_the_buffer_layout` pins the number
/// against the copy inside the MSL string.
pub(crate) const METAL_EDGE_BASE_WORDS: usize = CANVAS_ITEM_BUFFER_BYTES / 4;
/// The most gradient stops one **frame** may carry, on either backend (plan-116-F).
///
/// A starting value, as the plan says: raise it only against a measured scene. Five
/// words a stop, so 4096 stops is 80 KiB — noise beside the edge and glyph regions.
///
/// The same number on both backends deliberately. A scene that renders on Metal and
/// declines on Vulkan (or the reverse) would make "the GPU path" mean something
/// different per host, and the oracle comparison could not be read the same way on
/// both.
pub(crate) const MAX_FRAME_GRADIENT_STOPS: usize = 4096;
/// Five 32-bit words a stop: offset, then the four colour channels.
pub(crate) const GRADIENT_STOP_WORDS: usize = 5;
/// Where Metal's gradient region starts, in 32-bit words — after the items and edges.
pub(crate) const METAL_GRADIENT_BASE_WORDS: usize =
    CANVAS_ITEM_BUFFER_BYTES / 4 + METAL_MAX_FRAME_EDGES * 4;
/// The whole Metal frame buffer: item blocks, then edges (four 16.16 words each), then
/// gradient stops (five each).
pub(crate) const METAL_BUFFER_BYTES: usize = CANVAS_ITEM_BUFFER_BYTES
    + METAL_MAX_FRAME_EDGES * 16
    + MAX_FRAME_GRADIENT_STOPS * GRADIENT_STOP_WORDS * 4;

/// The most coverage samples one **frame**'s glyphs may carry on the Vulkan path.
///
/// Glyph bitmaps ride the same buffer as the edges, in a region after them, for the
/// reason a second buffer would have to be justified rather than assumed: it would need
/// its own allocation, its own memory-type search, its own descriptor binding and its
/// own upload, to hold data with exactly the edges' lifetime and exactly their access
/// pattern. One buffer, two regions, one binding.
///
/// One sample per 32-bit word rather than four packed per word. That wastes three
/// quarters of the region and buys a shader arm with no shifting and no masking, in the
/// only place where a packing mistake would be invisible — a wrongly unpacked coverage
/// byte still produces a glyph, just a wrong one. A megabyte of samples is 4 MiB of
/// buffer, which is nothing on any device that has a Vulkan driver at all.
pub(crate) const VULKAN_MAX_FRAME_GLYPH_SAMPLES: usize = 1 << 20;
/// Where the glyph region starts, in 32-bit words — i.e. immediately after the edges.
pub(crate) const VULKAN_GLYPH_BASE_WORDS: usize = VULKAN_EDGE_BYTES / 4;
/// Where the gradient region starts, in 32-bit words — after the edges and glyphs
/// (plan-116-F). A third region of the one buffer, for the reason the second one gave:
/// a separate buffer would need its own allocation, memory-type search, descriptor
/// binding and upload, for data with exactly the same lifetime and access pattern.
pub(crate) const VULKAN_GRADIENT_BASE_WORDS: usize =
    VULKAN_GLYPH_BASE_WORDS + VULKAN_MAX_FRAME_GLYPH_SAMPLES;
/// The whole shared buffer: edges, then glyph coverage, then gradient stops.
pub(crate) const VULKAN_BUFFER_BYTES: usize = VULKAN_EDGE_BYTES
    + VULKAN_MAX_FRAME_GLYPH_SAMPLES * 4
    + MAX_FRAME_GRADIENT_STOPS * GRADIENT_STOP_WORDS * 4;

/// The most coverage samples one glyph may carry on the **Metal** path.
///
/// Metal's glyph bitmap rides `setFragmentBytes:` exactly as its edges do, so the bound
/// is the same 4 KiB payload and it is per glyph rather than per frame. That is about a
/// 64x64 bitmap, which is a glyph at roughly 200 px. `__canvas_metalRenderable` declines
/// a scene containing a bigger one rather than clipping it, for the reason it declines
/// an over-long polygon: a clipped glyph is a *different glyph*, and would read as a
/// rasteriser bug rather than as a backend limit.
pub(crate) const METAL_MAX_GLYPH_SAMPLES: usize = 4096;

/// The Win64 shadow space this trampoline owes its callees: 32 bytes on Windows, none
/// elsewhere. It sits at the BOTTOM of the frame, so the saves above it are out of reach
/// of a callee that spills its register arguments into it.
pub(crate) fn graphics_trampoline_shadow(windows: bool) -> usize {
    usize::from(windows) * 32
}

/// The graphics trampoline's frame size, split out from the emitter so the alignment
/// invariant it exists to satisfy can be asserted directly (bug-479).
///
/// **Every x86-64 thread entry is reached by a `call`** — `_pthread_start` on Unix and
/// `BaseThreadInitThunk` on Windows alike — so the routine begins at `rsp % 16 == 8`.
/// A callee must in turn be entered at `rsp % 16 == 8`, which means `rsp % 16 == 0`
/// immediately before the `call`. This trampoline only subtracts (it pushes nothing), so
/// the frame itself has to carry the odd 8: `frame % 16 == 8` is the whole requirement.
///
/// AArch64 needs no such thing — the entry `sp` is 16-aligned and the return address
/// arrives in `lr` rather than on the stack.
pub(crate) fn graphics_trampoline_frame(arch: &str, windows: bool) -> usize {
    let realign = usize::from(arch == "x86_64") * 8;
    32 + realign + graphics_trampoline_shadow(windows)
}

/// The trampoline `pthread_create` starts: establishes the MFB context, then loops.
pub(crate) const GRAPHICS_TRAMPOLINE_SYMBOL: &str = "_mfb_rt_canvas_graphics_entry";

/// The graphics thread's shared-state data object.
pub(crate) fn graphics_state_data_object() -> CodeDataObject {
    CodeDataObject {
        symbol: GRAPHICS_STATE_SYMBOL.to_string(),
        kind: "raw".to_string(),
        layout: "mfb.runtime.canvas_graphics.v1 { u64 started, pending; mutex; cond; \
                 u64 arenaState }"
            .to_string(),
        align: 8,
        size: GRAPHICS_STATE_SIZE,
        value: "00".repeat(GRAPHICS_STATE_SIZE),
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
    let frame = graphics_trampoline_frame(platform.arch(), windows);
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
    instructions.push(abi::store_u64(
        ARENA_STATE_REGISTER,
        abi::stack_pointer(),
        shadow + 8,
    ));
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
    instructions.push(abi::store_u64(
        abi::mfb_arg(7),
        abi::stack_pointer(),
        shadow + 16,
    ));
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

    // Return, which is how a pthread entry exits. It used to park here on the
    // theory that a render loop returning was a bug — but shutdown asks it to
    // return, and parking made the shutdown join wait forever on a thread that
    // was spinning two instructions away from finishing.
    instructions.push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
    // The 8th argument register, then the arena register: on x86-64 these are `rbp`
    // and `r15`, and both must reach `start_thread` intact.
    instructions.push(abi::load_u64(
        abi::mfb_arg(7),
        abi::stack_pointer(),
        shadow + 16,
    ));
    instructions.push(abi::load_u64(
        ARENA_STATE_REGISTER,
        abi::stack_pointer(),
        shadow + 8,
    ));
    instructions.push(abi::load_u64(
        abi::link_register(),
        abi::stack_pointer(),
        shadow,
    ));
    instructions.push(abi::add_stack(frame));
    instructions.push(abi::return_());

    let _ = (platform_imports, platform);
    Ok(CodeFunction {
        name: "canvas.graphics_entry".to_string(),
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
    })
}

/// Load the graphics-state block's address into `dst`.
pub(crate) fn state_base(
    from: &str,
    dst: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    push_symbol_address(from, GRAPHICS_STATE_SYMBOL, dst, ins, rel);
}

/// `canvas::signalRedraw()` — a redraw is owed.
///
/// Sets the flag and wakes the loop. A **flag**, not a counter: two presents before a
/// frame must produce one frame, not two (`.ai/canvas-threading.md` §3), and a
/// counter would make the renderer draw the same scene twice.
pub(crate) fn emit_signal_redraw(
    symbol: &str,
    scratch: &GraphicsScratch,
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

    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::move_immediate(&scratch.scratch, "Integer", "1"));
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

/// `canvas::waitForRedraw()` — block until a redraw is owed, then take it.
///
/// The wait is a `while`, not an `if`: a condition variable may wake spuriously, and
/// a spurious wake that fell through would render a frame nobody asked for — which is
/// not a correctness bug but is exactly the "a static scene costs nothing" guarantee
/// (`.ai/canvas-threading.md` §4) quietly failing.
pub(crate) fn emit_wait_for_redraw(
    symbol: &str,
    scratch: &GraphicsScratch,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let retry = format!("{symbol}_wait");
    let have = format!("{symbol}_have");
    let stop = format!("{symbol}_stop");
    let done = format!("{symbol}_wait_done");

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

    instructions.push(abi::label(&have));
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::store_u64(
        abi::ZERO,
        &scratch.base,
        GRAPHICS_OFFSET_PENDING,
    ));
    instructions.push(abi::move_immediate(&scratch.verdict, "Integer", "1"));
    instructions.push(abi::branch(&done));

    instructions.push(abi::label(&stop));
    instructions.push(abi::move_immediate(&scratch.verdict, "Integer", "0"));

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

/// `canvas::frameDone()` — the render loop reports a completed frame.
///
/// Advances the frame counter and wakes anything waiting on it. Broadcast rather
/// than signal: the waiter is a different party from the render loop, and a signal
/// could wake the loop's own wait instead.
pub(crate) fn emit_frame_done(
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

/// Read one published surface dimension, falling back to the startup size.
pub(crate) fn emit_surface_dimension(
    symbol: &str,
    scratch: &GraphicsScratch,
    offset: usize,
    default: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let ready = format!("{symbol}_dim_ready_{offset}");
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::load_u64(&scratch.scratch, &scratch.base, offset));
    instructions.push(abi::compare_immediate(&scratch.scratch, "0"));
    instructions.push(abi::branch_ne(&ready));
    instructions.push(abi::move_immediate(
        &scratch.scratch,
        "Integer",
        &default.to_string(),
    ));
    instructions.push(abi::label(&ready));
    instructions.push(abi::move_register(RESULT_VALUE_REGISTER, &scratch.scratch));
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
    base: &str,
    width: &str,
    height: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let unchanged = format!("{symbol}_resize_same");
    state_base(symbol, base, instructions, relocations);
    // Count the resize only when the size actually changed. Every platform re-publishes
    // on events that are not resizes — AppKit sends `setFrameSize:` with the size it
    // already has, and the headless scripted resize does the same — and
    // `canvas::didResize` promises the program a *change*, not an event.
    //
    // The compare reads the old values before the stores overwrite them, and uses the
    // scratch the caller already gave us rather than a fifth register: this runs in a
    // platform resize callback, where the register budget is whatever the caller had.
    instructions.push(abi::load_u64(SCRATCH_RESIZE, base, GRAPHICS_OFFSET_WIDTH));
    instructions.push(abi::compare_registers(SCRATCH_RESIZE, width));
    instructions.push(abi::branch_ne(&format!("{symbol}_resize_changed")));
    instructions.push(abi::load_u64(SCRATCH_RESIZE, base, GRAPHICS_OFFSET_HEIGHT));
    instructions.push(abi::compare_registers(SCRATCH_RESIZE, height));
    instructions.push(abi::branch_eq(&unchanged));
    instructions.push(abi::label(&format!("{symbol}_resize_changed")));
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

/// The one scratch register `emit_publish_surface_size` needs for its compare.
///
/// Named here rather than taken as a parameter because every caller is a hand-written
/// platform resize callback that already spells its own registers, and adding a fifth
/// argument to all of them to pass a register they all have spare is churn. `SCRATCH[5]`
/// is above the four each caller stages its own values in.
const SCRATCH_RESIZE: &str = abi::SCRATCH[5];

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
    use super::{graphics_trampoline_frame, graphics_trampoline_shadow};

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
    /// arguments overwrites the saved return address and arena pointer. The emitter
    /// stores at `shadow` and `shadow + 8`, so the frame has to hold both beyond it.
    #[test]
    fn the_frame_holds_both_saves_above_the_shadow_space() {
        for (arch, windows) in [("x86_64", true), ("x86_64", false), ("aarch64", false)] {
            let frame = graphics_trampoline_frame(arch, windows);
            let shadow = graphics_trampoline_shadow(windows);
            assert!(
                frame >= shadow + 16,
                "{arch} (windows={windows}): frame {frame} cannot hold the two saves at \
                 {shadow} and {} above its {shadow}-byte shadow space",
                shadow + 8
            );
        }
    }

    /// Windows owes its callees 32 bytes; nothing else does.
    #[test]
    fn only_windows_reserves_shadow_space() {
        assert_eq!(graphics_trampoline_shadow(true), 32);
        assert_eq!(graphics_trampoline_shadow(false), 0);
    }
}
