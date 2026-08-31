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
//! its module globals — including `__CANVAS_SRGB`, whose absence would silently
//! render every antialiased pixel black. It gets both the same way a `thread::start`
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
/// compared to. `MFB_CANVAS_METAL=1` selects it; plan-98-E's own tests set it.
pub(crate) const GRAPHICS_OFFSET_METAL: usize = 296;
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
/// Total block size.
pub(crate) const GRAPHICS_STATE_SIZE: usize = 504;

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
/// the loop its own geometry cache and, critically, a populated `__CANVAS_SRGB` —
/// and then enters `__canvas_renderLoop`, which never returns.
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
    let realign = usize::from(platform.arch() == "x86_64") * 8;
    let frame = 32 + realign;
    instructions.push(abi::label("entry"));
    instructions.push(abi::subtract_stack(frame));
    instructions.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
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
        8,
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
    instructions.push(abi::load_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), 8));
    instructions.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
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
fn state_base(
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
        // CreateThread(NULL, 0, entry, arg, 0, NULL) — six arguments, the last two on
        // the stack. The handle is dropped: nothing joins the graphics thread, and the
        // process exit tears it down.
        instructions.push(abi::move_immediate(abi::c_arg(0), "Integer", "0"));
        instructions.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
        push_symbol_address(
            symbol,
            GRAPHICS_TRAMPOLINE_SYMBOL,
            abi::c_arg(2),
            instructions,
            relocations,
        );
        instructions.push(abi::move_register(abi::c_arg(3), &scratch.arena));
        instructions.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20));
        instructions.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x28));
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
    state_base(symbol, base, instructions, relocations);
    instructions.push(abi::store_u64(width, base, GRAPHICS_OFFSET_WIDTH));
    instructions.push(abi::store_u64(height, base, GRAPHICS_OFFSET_HEIGHT));
}

/// `canvas::setMetalMode(on)` — record whether the Metal renderer is selected.
///
/// Read from MFBASIC at first present, next to `setSyncMode`, for the same reason:
/// the environment is portably readable there.
pub(crate) fn emit_set_metal_mode(
    symbol: &str,
    scratch: &GraphicsScratch,
    value: &Operand,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::store_u64(value, &scratch.base, GRAPHICS_OFFSET_METAL));
}

/// `canvas::useMetal()` — is the Metal renderer selected?
///
/// Always FALSE on a non-macOS target: the flag exists everywhere so the renderer
/// seam has one shape, but only macOS has a Metal path behind it.
pub(crate) fn emit_use_metal(
    symbol: &str,
    scratch: &GraphicsScratch,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    if platform.family() != PlatformFamily::MacOS {
        instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
        return;
    }
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_METAL,
    ));
    instructions.push(abi::move_register(RESULT_VALUE_REGISTER, &scratch.scratch));
}
