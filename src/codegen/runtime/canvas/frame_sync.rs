//! plan-176-V: frame synchronisation — the frame ring, presents, frame counters and stats, the
//! lock and condition, pickup waits and the sync mode. The shared imports and `GraphicsScratch`
//! live in `runtime/canvas/mod.rs`.

use super::*;

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
pub(super) fn emit_frame_cond_broadcast(
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
pub(super) const NANOS_PER_SECOND: &str = "1000000000";

/// `now` = `CLOCK_REALTIME` in nanoseconds, through the timespec at `[sp + ts]`.
pub(super) fn emit_realtime_nanos(
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
