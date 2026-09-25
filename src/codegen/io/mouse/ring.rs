//! The per-thread decoded-mouse-event ring (plan-94-B §3b).
//!
//! A fixed-size, timestamped, overwrite-on-full ring living in an arena block that
//! the mouse-state region points at ([`MOUSE_STATE_RING_PTR_OFFSET`]). Allocated
//! on `enableMouse(TRUE)`, freed on `enableMouse(FALSE)` and `term::off`.
//!
//! **Worker-local by construction.** Only the thread that owns the arena ever
//! writes it — the decoder runs on whichever thread is reading stdin, and that is
//! the same thread whose `pollMouse` drains it. In app mode the cross-thread
//! boundary is the window input pipe, not this block: a UI-thread mouse handler
//! writes *bytes* to the pipe and the worker decodes them (plan-94-A §4.3). That
//! is the whole reason overwrite needs no atomics, and it is a rule to keep: a
//! cross-thread writer here would be a lock-free SPSC-overwrite hazard.
//!
//! **Overwrite-on-full, not grow.** The backpressure a mouse wants is "newest
//! wins" — a program that stalls for a second wants where the mouse is now, not a
//! second of replay to catch up on. Overwriting gives that for free and bounds the
//! memory with no policy to tune.
//!
//! **Two cursors, monotonic, never wrapped.** `head` and `tail` are absolute event
//! counts, not indices; the slot is `cursor % capacity`. That makes "is it full?"
//! a subtraction (`head - tail >= capacity`) with no ambiguous full/empty state,
//! which the usual wrapped-index ring has to burn a slot or a flag on.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;

use super::clock::emit_monotonic_nanos;

/// The decoded event an enqueue writes, as registers the caller has already
/// filled. Each is a vreg name.
pub(crate) struct MouseEventRegs<'a> {
    /// `MouseKind` ordinal (`MOUSE_KIND_*`).
    pub(crate) kind: &'a str,
    /// `MouseButton` ordinal (`MOUSE_BUTTON_*`).
    pub(crate) button: &'a str,
    /// First coordinate — `row` in cells, `x` in pixels. The producer chose the
    /// unit; the ring does not care (plan-94-A §4.2).
    pub(crate) coord_a: &'a str,
    /// Second coordinate — `column` in cells, `y` in pixels.
    pub(crate) coord_b: &'a str,
    /// Packed `MOUSE_MOD_*` bits.
    pub(crate) mods: &'a str,
}

/// Address of the ring slot for absolute cursor `cursor`, into `dst`.
///
/// `slot = ring + (cursor % CAPACITY) * SLOT_BYTES`. The capacity is a power of
/// two, so the modulo is a mask — which matters because this runs inside the
/// per-byte stdin read path.
fn emit_slot_address(dst: &str, ring: &str, cursor: &str, ctx: &mut EmitCtx, vregs: &mut Vregs) {
    debug_assert!(
        MOUSE_RING_CAPACITY.is_power_of_two(),
        "the slot index masks rather than divides; a non-power-of-two capacity \
         would silently alias slots"
    );
    let index = vregs.next();
    let mask = vregs.next();
    let scale = vregs.next();
    ctx.instructions.extend([
        abi::move_immediate(&mask, "Integer", &(MOUSE_RING_CAPACITY - 1).to_string()),
        abi::and_registers(&index, cursor, &mask),
        abi::move_immediate(&scale, "Integer", &MOUSE_SLOT_BYTES.to_string()),
        abi::multiply_registers(&index, &index, &scale),
        abi::add_registers(dst, ring, &index),
    ]);
}

/// Append one decoded event, overwriting the oldest if the ring is full.
///
/// A no-op when no ring is allocated (mouse mode is off), so a caller need not
/// gate — which keeps the decoder's hot path one branch shorter.
pub(crate) fn emit_enqueue(
    event: &MouseEventRegs,
    mouse_state_offset: usize,
    clock_scratch: usize,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let done = format!("{symbol}_mouse_enqueue_done");
    let not_full = format!("{symbol}_mouse_enqueue_not_full");

    let ring = vregs.next();
    ctx.instructions.push(abi::load_u64(
        &ring,
        ARENA_STATE_REGISTER,
        mouse_state_offset + MOUSE_STATE_RING_PTR_OFFSET,
    ));
    ctx.instructions.push(abi::compare_immediate(&ring, "0"));
    ctx.instructions.push(abi::branch_eq(&done));

    let head = vregs.next();
    let tail = vregs.next();
    let span = vregs.next();
    let cap = vregs.next();
    ctx.instructions.extend([
        abi::load_u64(
            &head,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_HEAD_OFFSET,
        ),
        abi::load_u64(
            &tail,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_TAIL_OFFSET,
        ),
        // Full when head has run a whole capacity ahead of tail. Advancing tail
        // (rather than refusing the write) is the overwrite rule: the new event
        // displaces the oldest unread one.
        abi::subtract_registers(&span, &head, &tail),
        abi::move_immediate(&cap, "Integer", &MOUSE_RING_CAPACITY.to_string()),
        abi::compare_registers(&span, &cap),
        abi::branch_lo(&not_full),
        abi::add_immediate(&tail, &tail, 1),
        abi::store_u64(
            &tail,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_TAIL_OFFSET,
        ),
        abi::label(&not_full),
    ]);

    let slot = vregs.next();
    emit_slot_address(&slot, &ring, &head, ctx, vregs);

    // The stamp is read here rather than passed in, so every enqueue site gets a
    // consistent one and no caller can forget it.
    let stamp = vregs.next();
    emit_monotonic_nanos(&stamp, clock_scratch, ctx, vregs)?;

    ctx.instructions.extend([
        abi::store_u64(event.kind, &slot, MOUSE_SLOT_KIND_OFFSET),
        abi::store_u64(event.button, &slot, MOUSE_SLOT_BUTTON_OFFSET),
        abi::store_u64(event.coord_a, &slot, MOUSE_SLOT_COORD_A_OFFSET),
        abi::store_u64(event.coord_b, &slot, MOUSE_SLOT_COORD_B_OFFSET),
        abi::store_u64(event.mods, &slot, MOUSE_SLOT_MODS_OFFSET),
        abi::store_u64(&stamp, &slot, MOUSE_SLOT_STAMP_OFFSET),
    ]);

    // Publish by advancing head only after the slot is fully written. The ring is
    // worker-local so nothing can observe the half-written slot today, but the
    // ordering costs nothing and is the invariant a future reader would rely on.
    let next = vregs.next();
    ctx.instructions.extend([
        abi::load_u64(
            &next,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_HEAD_OFFSET,
        ),
        abi::add_immediate(&next, &next, 1),
        abi::store_u64(
            &next,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_HEAD_OFFSET,
        ),
        abi::label(&done),
    ]);
    Ok(())
}

/// Where a successful [`emit_dequeue`] leaves the event it took.
pub(crate) struct DequeuedRegs {
    pub(crate) kind: String,
    pub(crate) button: String,
    pub(crate) coord_a: String,
    pub(crate) coord_b: String,
    pub(crate) mods: String,
}

/// Take the oldest event that is still fresh, skipping any that have expired.
///
/// On return `kind` is either a real `MOUSE_KIND_*` or [`MOUSE_KIND_NONE`], and
/// `NONE` is the whole "nothing pending" answer — the other registers are zero
/// with it, so a caller can store all five unconditionally.
///
/// **Stale entries are skipped, not just ignored.** Stamps are monotonic in
/// enqueue order, so everything older than the first fresh entry is also stale;
/// the loop advances `tail` past them and they never cost anything again. Without
/// that, a program that stopped polling for a second would pay to re-walk the same
/// expired prefix on every subsequent poll.
pub(crate) fn emit_dequeue(
    mouse_state_offset: usize,
    clock_scratch: usize,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) -> Result<DequeuedRegs, String> {
    let symbol = ctx.symbol;
    let empty = format!("{symbol}_mouse_deq_empty");
    let scan = format!("{symbol}_mouse_deq_scan");
    let fresh = format!("{symbol}_mouse_deq_fresh");
    let done = format!("{symbol}_mouse_deq_done");

    let out = DequeuedRegs {
        kind: vregs.next(),
        button: vregs.next(),
        coord_a: vregs.next(),
        coord_b: vregs.next(),
        mods: vregs.next(),
    };
    // Default answer: nothing pending. Written first so every exit path is
    // complete without a second store site.
    ctx.instructions.extend([
        abi::move_immediate(&out.kind, "Integer", &MOUSE_KIND_NONE.to_string()),
        abi::move_immediate(&out.button, "Integer", &MOUSE_BUTTON_NONE.to_string()),
        abi::move_immediate(&out.coord_a, "Integer", "0"),
        abi::move_immediate(&out.coord_b, "Integer", "0"),
        abi::move_immediate(&out.mods, "Integer", "0"),
    ]);

    let ring = vregs.next();
    ctx.instructions.push(abi::load_u64(
        &ring,
        ARENA_STATE_REGISTER,
        mouse_state_offset + MOUSE_STATE_RING_PTR_OFFSET,
    ));
    ctx.instructions.push(abi::compare_immediate(&ring, "0"));
    ctx.instructions.push(abi::branch_eq(&done));

    // One clock read for the whole scan: every candidate is measured against the
    // same "now", so a long skip cannot make later entries look fresher.
    let now = vregs.next();
    emit_monotonic_nanos(&now, clock_scratch, ctx, vregs)?;
    let ttl = vregs.next();
    ctx.instructions.push(abi::move_immediate(
        &ttl,
        "Integer",
        &MOUSE_EVENT_TTL_NANOS.to_string(),
    ));

    let head = vregs.next();
    let tail = vregs.next();
    let slot = vregs.next();
    let age = vregs.next();
    let stamp = vregs.next();

    ctx.instructions.push(abi::label(&scan));
    ctx.instructions.extend([
        abi::load_u64(
            &head,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_HEAD_OFFSET,
        ),
        abi::load_u64(
            &tail,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_TAIL_OFFSET,
        ),
        // Empty when tail has caught head. Compared head-first so the test is
        // `head <= tail` (unsigned `ls`) — the available mnemonic — rather than the
        // `hs` the tail-first reading would want.
        abi::compare_registers(&head, &tail),
        abi::branch_ls(&empty),
    ]);
    emit_slot_address(&slot, &ring, &tail, ctx, vregs);
    ctx.instructions.extend([
        abi::load_u64(&stamp, &slot, MOUSE_SLOT_STAMP_OFFSET),
        // Unsigned wrapping subtraction: correct for any interval under 584 years,
        // which is why the clock emitter needs no overflow trap.
        abi::subtract_registers(&age, &now, &stamp),
        abi::compare_registers(&age, &ttl),
        abi::branch_ls(&fresh),
        // Expired: drop it and look at the next one.
        abi::add_immediate(&tail, &tail, 1),
        abi::store_u64(
            &tail,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_TAIL_OFFSET,
        ),
        abi::branch(&scan),
    ]);

    ctx.instructions.extend([
        abi::label(&fresh),
        abi::load_u64(&out.kind, &slot, MOUSE_SLOT_KIND_OFFSET),
        abi::load_u64(&out.button, &slot, MOUSE_SLOT_BUTTON_OFFSET),
        abi::load_u64(&out.coord_a, &slot, MOUSE_SLOT_COORD_A_OFFSET),
        abi::load_u64(&out.coord_b, &slot, MOUSE_SLOT_COORD_B_OFFSET),
        abi::load_u64(&out.mods, &slot, MOUSE_SLOT_MODS_OFFSET),
        abi::add_immediate(&tail, &tail, 1),
        abi::store_u64(
            &tail,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_TAIL_OFFSET,
        ),
        abi::branch(&done),
        abi::label(&empty),
        abi::label(&done),
    ]);
    Ok(out)
}

/// Allocate the ring block and reset both cursors, unless one already exists.
///
/// Idempotent, because `enableMouse(TRUE)` twice is a thing a program does and
/// allocating a second block would leak the first.
pub(crate) fn emit_ring_alloc(
    mouse_state_offset: usize,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let done = format!("{symbol}_mouse_ring_alloc_done");
    let ok = format!("{symbol}_mouse_ring_alloc_ok");

    let existing = vregs.next();
    ctx.instructions.push(abi::load_u64(
        &existing,
        ARENA_STATE_REGISTER,
        mouse_state_offset + MOUSE_STATE_RING_PTR_OFFSET,
    ));
    ctx.instructions
        .push(abi::compare_immediate(&existing, "0"));
    ctx.instructions.push(abi::branch_ne(&done));

    ctx.instructions.extend([
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &MOUSE_RING_BYTES.to_string(),
        ),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    ctx.relocations
        .push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&ok),
        // Allocation failed. `enableMouse` is best-effort and returns `Nothing`
        // (plan-94-A, Open Decisions), so there is no error to report: leave the
        // pointer null and every poll keeps answering `None`. Reporting a failure
        // a program cannot act on would be worse than degrading to "no events".
        abi::branch(&done),
        abi::label(&ok),
        abi::store_u64(
            abi::mfb_return(1),
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_RING_PTR_OFFSET,
        ),
    ]);
    // Reset the cursors and any half-parsed escape sequence. A re-enable must not
    // inherit a prefix from the previous session.
    let zero = vregs.next();
    ctx.instructions
        .push(abi::move_immediate(&zero, "Integer", "0"));
    for offset in [
        MOUSE_STATE_HEAD_OFFSET,
        MOUSE_STATE_TAIL_OFFSET,
        MOUSE_STATE_PARSE_LEN_OFFSET,
    ] {
        ctx.instructions.push(abi::store_u64(
            &zero,
            ARENA_STATE_REGISTER,
            mouse_state_offset + offset,
        ));
    }
    ctx.instructions.push(abi::label(&done));
    Ok(())
}

/// Free the ring block and null the pointer, if one is allocated.
///
/// Idempotent for the same reason the allocation is: `term::off` after
/// `enableMouse(FALSE)` must not double-free.
pub(crate) fn emit_ring_free(
    mouse_state_offset: usize,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let done = format!("{symbol}_mouse_ring_free_done");

    let ring = vregs.next();
    ctx.instructions.push(abi::load_u64(
        &ring,
        ARENA_STATE_REGISTER,
        mouse_state_offset + MOUSE_STATE_RING_PTR_OFFSET,
    ));
    ctx.instructions.push(abi::compare_immediate(&ring, "0"));
    ctx.instructions.push(abi::branch_eq(&done));
    // Null the pointer BEFORE the free: everything that touches the ring tests
    // this pointer first, so the window in which it names freed memory is zero.
    let zero = vregs.next();
    ctx.instructions
        .push(abi::move_immediate(&zero, "Integer", "0"));
    for offset in [
        MOUSE_STATE_RING_PTR_OFFSET,
        MOUSE_STATE_HEAD_OFFSET,
        MOUSE_STATE_TAIL_OFFSET,
        MOUSE_STATE_PARSE_LEN_OFFSET,
    ] {
        ctx.instructions.push(abi::store_u64(
            &zero,
            ARENA_STATE_REGISTER,
            mouse_state_offset + offset,
        ));
    }
    ctx.instructions.extend([
        abi::move_register(abi::return_register(), &ring),
        abi::move_immediate(abi::c_arg(1), "Integer", &MOUSE_RING_BYTES.to_string()),
        abi::branch_link(ARENA_FREE_SYMBOL),
    ]);
    ctx.relocations
        .push(internal_branch(symbol, ARENA_FREE_SYMBOL));
    ctx.instructions.push(abi::label(&done));
    Ok(())
}
