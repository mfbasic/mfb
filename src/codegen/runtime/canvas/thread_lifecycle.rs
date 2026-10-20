//! plan-176-V: the graphics thread's lifecycle — `state_base`, redraw signal and wait, start,
//! stop and spawn. The shared imports and `GraphicsScratch` live in `runtime/canvas/mod.rs`.

use super::*;

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
