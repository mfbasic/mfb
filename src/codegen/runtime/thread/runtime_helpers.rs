// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::regalloc;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::runtime::thread::*;
use crate::target::shared::abi;
use std::collections::HashMap;
/// The control block's run state. `CLOSED` is written only by the parent view's drop:
/// the worker's exit then publishes no result, since nothing can read it.
pub(crate) const THREAD_STATE_RUNNING: &str = "0";
pub(crate) const THREAD_STATE_COMPLETED: &str = "1";
pub(crate) const THREAD_STATE_CLOSED: &str = "2";

// plan-156-A: a queue is embedded in the control block (`CONTROL_OFFSET_*_QUEUE`); these
// offsets are relative to the queue's base. It has no mutex of its own — every queue
// shares the block lock — only its two condvars (64 bytes each, which covers every
// target's `pthread_cond_t`).
pub(crate) const THREAD_QUEUE_NOT_EMPTY_OFFSET: usize = 0;
pub(crate) const THREAD_QUEUE_NOT_FULL_OFFSET: usize = 64;
pub(crate) const THREAD_QUEUE_CAPACITY_OFFSET: usize = 128;
pub(crate) const THREAD_QUEUE_COUNT_OFFSET: usize = 136;
pub(crate) const THREAD_QUEUE_HEAD_OFFSET: usize = 144;
pub(crate) const THREAD_QUEUE_TAIL_OFFSET: usize = 152;
pub(crate) const THREAD_QUEUE_CLOSED_OFFSET: usize = 160;
/// The ring: `capacity` entries of [`THREAD_QUEUE_ENTRY_SIZE`] bytes, allocated from the
/// block arena by `thread::start`. An entry is the queued value: the block-arena copy
/// the sender's helper made, or the word itself for a value that crosses as its word.
pub(crate) const THREAD_QUEUE_VALUES_OFFSET: usize = 168;
/// The block-arena message the last read on this queue handed out, still owned by
/// the queue (0 = none). The reader's call site copies it into its own arena AFTER the
/// read helper returns, so it cannot be dropped then; the NEXT read on the same queue
/// drops it (each queue has exactly one reader, so that read is strictly later on the
/// same thread). One left over when the thread is freed goes with the block arena.
pub(crate) const THREAD_QUEUE_LAST_READ_OFFSET: usize = 176;
/// The message type's drop function (0 = the values are words), recorded by every
/// write — one queue carries one message type. The block's final release runs it on
/// every message still queued and on the last read's, so an undelivered message is
/// dropped — and an undelivered resource CLOSED — exactly once.
pub(crate) const THREAD_QUEUE_DROP_OFFSET: usize = 184;
pub(crate) const THREAD_QUEUE_SIZE: usize = 192;
/// One ring entry: the queued value.
pub(crate) const THREAD_QUEUE_ENTRY_SIZE: usize = 8;
/// `log2(THREAD_QUEUE_ENTRY_SIZE)` — the shift that turns a ring index into a byte
/// offset.
pub(crate) const THREAD_QUEUE_ENTRY_SHIFT: u8 = 3;

pub(crate) fn thread_symbol(platform: &dyn CodegenPlatform, name: &str) -> String {
    match platform.family() {
        // Mach-O prefixes C symbols with a leading underscore; ELF does not.
        PlatformFamily::MacOS => format!("_{name}"),
        PlatformFamily::Linux => name.to_string(),
        // 47-H owns Windows threading (CreateThread + IAT), so no pthread symbol
        // is emitted for Windows.
        PlatformFamily::Windows => unreachable!("47-H owns Windows thread-symbol naming"),
    }
}

pub(crate) fn emit_thread_external_call(ctx: &mut EmitCtx, name: &str) -> Result<(), String> {
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    // Windows has no pthreads: translate each POSIX threading primitive to its
    // Win32 equivalent (plan-47-H). SRWLOCK and CONDITION_VARIABLE are pointer-
    // sized and zero-initialised (a zeroed value is a valid unlocked lock / fresh
    // condvar), so they fit the pthread-sized slots the shared code zeroes.
    if platform.family() == PlatformFamily::Windows {
        return emit_windows_thread_call(ctx, name);
    }

    // `ctx.symbol` is the emitting symbol, which is exactly what the old `from`
    // parameter carried — the two were always passed the same value.
    let symbol = thread_symbol(platform, name);
    ctx.instructions.push(abi::branch_link(&symbol));
    ctx.relocations
        .push(external_branch(ctx.symbol, &symbol, platform_imports)?);
    Ok(())
}

/// Emit the Win32 realization of a POSIX threading primitive (plan-47-H). The
/// shared callers set arguments in the pthread convention (`x0..`); each arm
/// re-uses or lightly adjusts them for the matching kernel32 call and leaves the
/// POSIX return contract in the **C** return register (`c_return(0)`; 0 = success)
/// — the same register a real `pthread_*` would return in, so the shared callers
/// read one register on every family. It must NOT be `return_register()`
/// (`mfb_return(0)`): the two are both `x0` on AArch64 but differ on x86-64
/// (`rax` vs the aligned MFB bank's `rdi`/`rcx`), so an MFB-token read here
/// silently tests a clobbered caller-saved register.
///
/// **WIN64_DIV_CLOBBER (plan-99).** These arms name ABI tokens directly, so their
/// values land on the Win64 call bank: `c_arg(0)`=rcx, `c_arg(1)`=**rdx**,
/// `c_arg(2)`=r8, `c_arg(3)`=r9. The x86-64 `div` expansion writes the quotient to
/// rax and the REMAINDER to **rdx** (`div_seq` in `arch::x86_64::encode::emitter`),
/// and the allocator clobber model does not cover hand-written token code — that
/// expansion is sound for allocated code only because rax/rcx/rdx are never
/// allocatable. So a value parked in `c_arg(1)` across an
/// `unsigned_divide_registers` is silently replaced by the remainder. Three arms
/// here did exactly that (`nanosleep`, `clock_gettime`, and the deadline math in
/// `pthread_cond_timedwait`); keep every live intermediate in `c_arg(2)`/`c_arg(3)`
/// and re-materialize a constant after a divide rather than holding it in
/// `c_arg(1)`.
fn emit_windows_thread_call(ctx: &mut EmitCtx, name: &str) -> Result<(), String> {
    let from = ctx.symbol;
    let pi = ctx.platform_imports;
    // Emit a `bl <win32>` + external reloc.
    fn call(ctx: &mut EmitCtx, from: &str, sym: &str) -> Result<(), String> {
        ctx.instructions.push(abi::branch_link(sym));
        ctx.relocations
            .push(external_branch(from, sym, ctx.platform_imports)?);
        Ok(())
    }
    match name {
        // Mutex → SRWLOCK (exclusive). All take the lock pointer in x0 and return
        // void; the POSIX callers expect 0.
        "pthread_mutex_init" => {
            call(ctx, from, "InitializeSRWLock")?;
            ctx.instructions
                .push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
        }
        "pthread_mutex_lock" => {
            call(ctx, from, "AcquireSRWLockExclusive")?;
            ctx.instructions
                .push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
        }
        "pthread_mutex_unlock" => {
            call(ctx, from, "ReleaseSRWLockExclusive")?;
            ctx.instructions
                .push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
        }
        // Condition variable → CONDITION_VARIABLE.
        "pthread_cond_init" => {
            call(ctx, from, "InitializeConditionVariable")?;
            ctx.instructions
                .push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
        }
        "pthread_cond_signal" => {
            call(ctx, from, "WakeConditionVariable")?;
            ctx.instructions
                .push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
        }
        "pthread_cond_broadcast" => {
            call(ctx, from, "WakeAllConditionVariable")?;
            ctx.instructions
                .push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
        }
        // pthread_cond_wait(cond=x0, mutex=x1) → SleepConditionVariableSRW(cond,
        // lock, dwMilliseconds=INFINITE, Flags=0). Returns BOOL; POSIX wants 0.
        "pthread_cond_wait" => {
            ctx.instructions.extend([
                abi::move_immediate(abi::c_arg(2), "Integer", "0"),
                abi::subtract_immediate(abi::c_arg(2), abi::c_arg(2), 1), // INFINITE = (DWORD)-1
                abi::move_immediate(abi::c_arg(3), "Integer", "0"),       // Flags (exclusive)
            ]);
            call(ctx, from, "SleepConditionVariableSRW")?;
            ctx.instructions
                .push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
        }
        // pthread_cond_timedwait(cond=x0, mutex=x1, &abstime=x2) →
        // SleepConditionVariableSRW(cond, lock, dwMilliseconds, Flags=0), waiting
        // until the caller's ABSOLUTE deadline and returning ETIMEDOUT (110) only
        // when that deadline has actually passed, 0 on any earlier wake.
        //
        // plan-99: this arm used to ignore `abstime` entirely and poll a fixed
        // 20 ms, on the theory that "the shared callers loop on their own deadline,
        // re-checking the predicate after each wake". That theory is FALSE — every
        // caller (`emit_cancellable_sleep_wait`, `thread_queue_read_helper`,
        // `thread_queue_write_helper`) treats a NON-ZERO return as "the deadline
        // elapsed" and stops waiting. So on Windows every timed wait expired after
        // ~20 ms: a worker `thread::sleep(t, 200)` returned in ~20 ms (box 2230:
        // `os_sleep_worker_rt.exe` printed "slept short", and the 5000 ms
        // `*_worker_cancel_rt.exe` ran to completion before the parent could cancel,
        // printing "no interrupt 0"), and a bounded `thread::send`/`receive` gave up
        // ~20 ms into its timeout. Honoring `abstime` here fixes all three callers
        // at once and leaves every POSIX target byte-identical.
        //
        // `abstime` is in the epoch this file's `clock_gettime` shim produces
        // (`GetSystemTimePreciseAsFileTime`, 100 ns ticks since 1601, split into
        // sec/nsec), so the remaining wait is recovered by rebuilding the deadline's
        // tick count and subtracting `now`. The ms conversion rounds UP so a wait is
        // never cut short by sub-millisecond truncation; a spurious wake returns 0
        // and the caller re-enters, where the remaining time is recomputed.
        "pthread_cond_timedwait" => {
            let n = ctx.instructions.len();
            let timed = format!("{from}_ctw_timeout_{n}");
            let expired = format!("{from}_ctw_expired_{n}");
            let done = format!("{from}_ctw_done_{n}");
            // Hand-managed scratch frame, like the `clock_gettime` arm: 0x00..0x20
            // is the Win64 shadow space every call below needs, 0x20.. are locals.
            const COND: usize = 0x20;
            const LOCK: usize = 0x28;
            const ABSTIME: usize = 0x30;
            const NOW_FT: usize = 0x38;
            ctx.instructions.extend([
                abi::subtract_stack(0x50),
                abi::store_u64(abi::c_arg(0), abi::stack_pointer(), COND),
                abi::store_u64(abi::c_arg(1), abi::stack_pointer(), LOCK),
                abi::store_u64(abi::c_arg(2), abi::stack_pointer(), ABSTIME),
                abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), NOW_FT),
            ]);
            call(ctx, from, "GetSystemTimePreciseAsFileTime")?;
            ctx.instructions.extend([
                // deadline_ft = abstime.tv_sec * 1e7 + abstime.tv_nsec / 100.
                // Every value that must survive a divide is held in `c_arg(2)`/
                // `c_arg(3)` (r8/r9) — never `c_arg(1)` (rdx), see
                // WIN64_DIV_CLOBBER below.
                abi::load_u64(abi::c_arg(0), abi::stack_pointer(), ABSTIME),
                abi::load_u64(abi::c_arg(2), abi::c_arg(0), 0), // tv_sec
                abi::load_u64(abi::c_arg(3), abi::c_arg(0), 8), // tv_nsec
                abi::move_immediate(abi::c_arg(1), "Integer", "10000000"),
                abi::multiply_registers(abi::c_arg(2), abi::c_arg(2), abi::c_arg(1)),
                abi::move_immediate(abi::c_arg(1), "Integer", "100"),
                abi::unsigned_divide_registers(abi::c_arg(3), abi::c_arg(3), abi::c_arg(1)),
                abi::add_registers(abi::c_arg(2), abi::c_arg(2), abi::c_arg(3)),
                // remaining 100 ns ticks = deadline_ft - now_ft; <= 0 is already due.
                abi::load_u64(abi::c_arg(3), abi::stack_pointer(), NOW_FT),
                abi::subtract_registers(abi::c_arg(2), abi::c_arg(2), abi::c_arg(3)),
                abi::compare_immediate(abi::c_arg(2), "0"),
                abi::branch_le(&expired),
                // ms = ceil(ticks / 10000) so a wait never ends early. The quotient
                // lands in `c_arg(2)` (r8), which IS SleepConditionVariableSRW's
                // third argument, so no further staging is needed.
                abi::add_immediate(abi::c_arg(2), abi::c_arg(2), 9999),
                abi::move_immediate(abi::c_arg(3), "Integer", "10000"),
                abi::unsigned_divide_registers(abi::c_arg(2), abi::c_arg(2), abi::c_arg(3)),
                abi::load_u64(abi::c_arg(0), abi::stack_pointer(), COND),
                abi::load_u64(abi::c_arg(1), abi::stack_pointer(), LOCK),
                abi::move_immediate(abi::c_arg(3), "Integer", "0"), // Flags (exclusive)
            ]);
            call(ctx, from, "SleepConditionVariableSRW")?;
            ctx.instructions.extend([
                abi::compare_immediate(abi::c_return(0), "0"),
                abi::branch_eq(&timed), // BOOL 0 → timed out (or error)
                abi::move_immediate(abi::c_return(0), "Integer", "0"),
                abi::branch(&done),
                // The deadline had already passed: report ETIMEDOUT without waiting.
                // The caller still holds the mutex, exactly as after a real wait.
                abi::label(&expired),
                abi::label(&timed),
                abi::move_immediate(abi::c_return(0), "Integer", "110"), // ETIMEDOUT
                abi::label(&done),
                abi::add_stack(0x50),
            ]);
        }
        // nanosleep(&req, &rem): Win32 has no nanosleep. `req` (c_arg(0)) is a
        // RELATIVE timespec {tv_sec, tv_nsec} (plan-91-A's parent sleep helper);
        // convert it to whole milliseconds and call Sleep(dwMilliseconds). Sleep
        // is uninterruptible, so report POSIX success (0) — the shared EINTR-retry
        // loop then exits after this single Sleep.
        "nanosleep" => {
            // plan-99: the seconds term is staged in `c_arg(2)`/`c_arg(3)` (r8/r9),
            // NOT in `c_arg(1)` — see WIN64_DIV_CLOBBER below. It used to sit in
            // `c_arg(1)` (rdx) across the `nsec/1e6` divide, which overwrote it with
            // the division's REMAINDER: every sleep of a whole second or more lost
            // its seconds and slept only `ms % 1000` (box 2230, measured: a 1500 ms
            // `os::sleep` on the main thread returned in 505 ms). Sub-second sleeps
            // were accidentally correct — `tv_sec` is 0 and the remainder is 0 —
            // which is why the 50 ms fixtures never caught it.
            ctx.instructions.extend([
                abi::load_u64(abi::c_arg(2), abi::c_arg(0), 0), // tv_sec
                abi::load_u64(abi::c_arg(3), abi::c_arg(0), 8), // tv_nsec
                abi::move_immediate(abi::c_arg(1), "Integer", "1000"),
                abi::multiply_registers(abi::c_arg(2), abi::c_arg(2), abi::c_arg(1)), // sec*1000
                abi::move_immediate(abi::c_arg(1), "Integer", "1000000"),
                abi::unsigned_divide_registers(abi::c_arg(3), abi::c_arg(3), abi::c_arg(1)), // nsec/1e6
                abi::add_registers(abi::c_arg(0), abi::c_arg(2), abi::c_arg(3)), // total ms → dwMilliseconds
            ]);
            call(ctx, from, "Sleep")?;
            ctx.instructions
                .push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
        }
        // pthread_detach(handle=x0): release our reference; the thread runs on.
        "pthread_detach" => {
            call(ctx, from, "CloseHandle")?;
            ctx.instructions
                .push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
        }
        // pthread_join(handle=x0, NULL) → WaitForSingleObject(handle, INFINITE), then
        // CloseHandle(handle) — a joined POSIX thread holds no further reference either
        // (bug-622). The handle rides the frame across the first call: rcx is volatile.
        "pthread_join" => {
            ctx.instructions.extend([
                abi::subtract_stack(0x30),
                abi::store_u64(abi::c_arg(0), abi::stack_pointer(), 0x20),
                abi::move_immediate(abi::c_arg(1), "Integer", "0"),
                abi::subtract_immediate(abi::c_arg(1), abi::c_arg(1), 1), // INFINITE = (DWORD)-1
            ]);
            call(ctx, from, "WaitForSingleObject")?;
            ctx.instructions
                .push(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), 0x20));
            call(ctx, from, "CloseHandle")?;
            ctx.instructions.extend([
                abi::add_stack(0x30),
                abi::move_immediate(abi::c_return(0), "Integer", "0"),
            ]);
        }
        // Stack-size attr is set directly on CreateThread (see the spawn helper);
        // these are inert on Windows.
        "pthread_attr_init" | "pthread_attr_setstacksize" | "pthread_attr_destroy" => {
            ctx.instructions
                .push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
        }
        "sched_yield" => {
            call(ctx, from, "SwitchToThread")?;
            ctx.instructions
                .push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
        }
        // clock_gettime(clk=x0, &ts=x1): the deadline helpers compare a stored
        // `now` against `now + timeout`, so the value must be a real, consistent,
        // increasing clock even though the timedwait arm polls. Fill the timespec
        // from GetSystemTimePreciseAsFileTime (100 ns ticks since 1601):
        // tv_sec = ft / 1e7, tv_nsec = (ft % 1e7) * 100.
        "clock_gettime" => {
            ctx.instructions.extend([
                abi::subtract_stack(0x30),
                abi::store_u64(abi::c_arg(1), abi::stack_pointer(), 0x28), // save &ts
                abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), 0x20), // &ft
            ]);
            call(ctx, from, "GetSystemTimePreciseAsFileTime")?;
            // plan-99: the `1e7` scale is re-materialized in `c_arg(3)` (r9) and the
            // seconds live in `c_arg(2)` (r8) — see WIN64_DIV_CLOBBER below. The
            // scale used to sit in `c_arg(1)` (rdx) ACROSS the `ft / 1e7` divide,
            // which left the division's remainder there, so `sec * 1e7` computed
            // `sec * (ft % 1e7)` and `tv_nsec` came out garbage. `tv_sec` was right,
            // which is why the clock looked plausible: what broke was every ABSOLUTE
            // deadline built on it (`emit_thread_deadline`), and that only became
            // visible once `pthread_cond_timedwait` started honoring the deadline
            // instead of polling a fixed 20 ms.
            ctx.instructions.extend([
                abi::load_u64(abi::c_arg(0), abi::stack_pointer(), 0x20), // ft
                abi::move_immediate(abi::c_arg(3), "Integer", "10000000"),
                abi::unsigned_divide_registers(abi::c_arg(2), abi::c_arg(0), abi::c_arg(3)), // sec
                abi::move_immediate(abi::c_arg(3), "Integer", "10000000"),
                abi::multiply_registers(abi::c_arg(3), abi::c_arg(2), abi::c_arg(3)), // sec*1e7
                abi::subtract_registers(abi::c_arg(3), abi::c_arg(0), abi::c_arg(3)), // ft % 1e7
                abi::move_immediate(abi::c_arg(1), "Integer", "100"),
                abi::multiply_registers(abi::c_arg(3), abi::c_arg(3), abi::c_arg(1)), // nsec
                abi::load_u64(abi::c_arg(0), abi::stack_pointer(), 0x28),             // &ts
                abi::store_u64(abi::c_arg(2), abi::c_arg(0), 0),                      // tv_sec
                abi::store_u64(abi::c_arg(3), abi::c_arg(0), 8),                      // tv_nsec
                abi::move_immediate(abi::c_return(0), "Integer", "0"),
                abi::add_stack(0x30),
            ]);
        }
        other => {
            return Err(format!(
                "windows-x86_64: no Win32 translation for thread primitive '{other}'"
            ));
        }
    }
    let _ = pi;
    Ok(())
}

/// Initialize the queue embedded at `queue` in the control block held in the frame word
/// `block`: its two condvars, its capacity (the limit in the frame word `limit`), and its
/// ring — `capacity * THREAD_QUEUE_ENTRY_SIZE` bytes from the block arena (the arena
/// register pointed at the block arena in the frame word `arena` for the one
/// `arena_alloc`, parking the caller's arena in `saved_arena`). The block is already
/// zeroed, so the counters, the closed flag and the reclaim lists start empty. Failures
/// branch to `oom` / `init_error`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_thread_queue_init(
    ctx: &mut EmitCtx,
    r: &ThreadVregs,
    block: usize,
    arena: usize,
    saved_arena: usize,
    limit: usize,
    queue: usize,
    oom: &str,
    init_error: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    for cond in [THREAD_QUEUE_NOT_EMPTY_OFFSET, THREAD_QUEUE_NOT_FULL_OFFSET] {
        ctx.instructions.extend([
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), block),
            abi::add_immediate(abi::c_arg(0), abi::c_arg(0), queue + cond),
            abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        ]);
        emit_thread_external_call(ctx, "pthread_cond_init")?;
        ctx.instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_ne(init_error),
        ]);
    }
    ctx.instructions.extend([
        abi::load_u64(&r.v9, abi::stack_pointer(), block),
        abi::load_u64(&r.v10, abi::stack_pointer(), limit),
        abi::store_u64(&r.v10, &r.v9, queue + THREAD_QUEUE_CAPACITY_OFFSET),
        // size = capacity * THREAD_QUEUE_ENTRY_SIZE. The limit is upper-bounded by the
        // start helper so this cannot wrap; a wrap would size the ring tiny while the
        // stored capacity stays huge (bug-60).
        abi::shift_left_immediate(abi::c_arg(0), &r.v10, THREAD_QUEUE_ENTRY_SHIFT),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    crate::codegen::runtime::thread::emit_with_arena(
        ctx.instructions,
        arena,
        saved_arena,
        |instructions| instructions.push(abi::branch_link(ARENA_ALLOC_SYMBOL)),
    );
    ctx.relocations
        .push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    ctx.instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_ne(oom),
        abi::load_u64(&r.v9, abi::stack_pointer(), block),
        abi::store_u64(
            abi::mfb_return(1),
            &r.v9,
            queue + THREAD_QUEUE_VALUES_OFFSET,
        ),
    ]);
    Ok(())
}

/// `thread::openStdIn`/`closeStdIn` (plan-15 §4.5). A thin wrapper over the stdin
/// broadcast subscribe/unsubscribe helpers: `x0 = 0` (the padded no-arg self form)
/// subscribes the calling thread (its arena `x19`); a non-null parent `Thread`
/// handle subscribes the worker behind it (its arena at `CONTROL_OFFSET_WORKER_ARENA`).
/// Returns `Nothing`.
pub(crate) fn lower_thread_stdin_subscription_helper(
    symbol: &str,
    subscribe: bool,
) -> Result<ThreadBodyParts, String> {
    let target = if subscribe {
        STDIN_SUBSCRIBE_SYMBOL
    } else {
        STDIN_UNSUBSCRIBE_SYMBOL
    };
    let worker = format!("{symbol}_worker");
    let do_call = format!("{symbol}_call");
    let view_closed = format!("{symbol}_view_closed");
    let mut vregs = Vregs::new();
    let r = &ThreadVregs::mint(&mut vregs);
    let mut instructions = vec![
        abi::compare_immediate(abi::c_arg(0), "0"),
        abi::branch_ne(&worker),
        // Self form: subscribe the calling thread's own arena.
        abi::move_register(abi::c_arg(0), ARENA_STATE_REGISTER),
        abi::branch(&do_call),
        abi::label(&worker),
    ];
    // Worker form: the parent `Thread` view names the control block, which carries the
    // worker's arena state (plan-156-C).
    emit_control_block_of(&mut instructions, &view_closed, &r.v9);
    instructions.extend([
        abi::load_u64(abi::c_arg(0), abi::c_arg(0), CONTROL_OFFSET_WORKER_ARENA),
        abi::label(&do_call),
        abi::branch_link(target),
    ]);
    let mut relocations = vec![internal_branch(symbol, target)];
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    instructions.push(abi::return_());
    emit_view_closed_exit(symbol, &view_closed, &mut instructions, &mut relocations);
    Ok((instructions, relocations, 0))
}

/// plan-99: a plain, uninterruptible RELATIVE delay of `ms_reg` milliseconds via
/// libc `nanosleep` (Win32 `Sleep`), re-entered on an EINTR wake so a signal can
/// never truncate it. Control leaves this block ONLY by branching to `ok_label`,
/// once the full delay has elapsed — the caller owns the `ok_label` epilogue and
/// every error path.
///
/// `req_offset`/`rem_offset` name two 16-byte relative `timespec { tv_sec; tv_nsec }`
/// slots in the caller's frame: `req` is the requested remaining sleep, `rem` the
/// kernel's leftover on an EINTR wake. `ms_reg` must hold a validated positive `ms`
/// (the caller rejects a negative and short-circuits zero); it is read once, before
/// the C-argument registers are staged, so passing `c_arg(0)`/`c_arg(1)` is safe.
///
/// Shared by the parent `thread::sleep` (`ms` in `c_arg(1)`, after its handle
/// argument) and the main-thread branch of `os::sleep` (`ms` in `c_arg(0)`).
pub(crate) fn emit_relative_sleep(
    ctx: &mut EmitCtx,
    r: &ThreadVregs,
    ms_reg: impl Into<abi::Reg>,
    req_offset: usize,
    rem_offset: usize,
    ok_label: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let retry = format!("{symbol}_retry");
    ctx.instructions.extend([
        // ms -> relative {tv_sec, tv_nsec}: sec = ms/1000, nsec = (ms%1000)*1e6.
        // Same split as `emit_thread_deadline`, but RELATIVE (no clock_gettime add).
        abi::move_register(&r.v9, ms_reg),
        abi::move_immediate(&r.v10, "Integer", "1000"),
        abi::signed_divide_registers(&r.v11, &r.v9, &r.v10),
        abi::multiply_subtract_registers(&r.v12, &r.v11, &r.v10, &r.v9),
        abi::move_immediate(&r.v13, "Integer", "1000000"),
        abi::multiply_registers(&r.v12, &r.v12, &r.v13),
        abi::store_u64(&r.v11, abi::stack_pointer(), req_offset),
        abi::store_u64(&r.v12, abi::stack_pointer(), req_offset + 8),
        abi::label(&retry),
        abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), req_offset), // &req
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), rem_offset), // &rem
    ]);
    emit_thread_external_call(ctx, "nanosleep")?;
    ctx.instructions.extend([
        // 0 = the full sleep elapsed. Non-zero = EINTR (a signal cut it short);
        // the kernel wrote the leftover into `rem`, so copy rem->req and re-enter
        // so a signal cannot truncate the sleep. nsec/sec are always in range, so
        // EINVAL is impossible — any non-zero return is an interrupt.
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(ok_label),
        abi::load_u64(&r.v9, abi::stack_pointer(), rem_offset),
        abi::store_u64(&r.v9, abi::stack_pointer(), req_offset),
        abi::load_u64(&r.v9, abi::stack_pointer(), rem_offset + 8),
        abi::store_u64(&r.v9, abi::stack_pointer(), req_offset + 8),
        abi::branch(&retry),
    ]);
    Ok(())
}

pub(crate) fn lower_thread_start_helper(
    symbol: &str,
    uses_rng: bool,
    arena_global_slots: usize,
    debug_arena_registry: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ThreadBodyParts, String> {
    // Vreg-allocated (plan-00-G Phase 2): the explicit sp-relative local region holds
    // everything that must survive a call. Runs in the parent (x20 is not the worker's
    // control block here), so no reservation.
    const FRAME_SIZE: usize = 176;
    const ENTRY_OFFSET: usize = 8;
    const DATA_OFFSET: usize = 16;
    const IN_LIMIT_OFFSET: usize = 24;
    const OUT_LIMIT_OFFSET: usize = 32;
    const CB_OFFSET: usize = 40;
    // The block arena, parked for `emit_with_arena`, and the caller's arena across it.
    const ARENA_OFFSET: usize = 48;
    // pthread_attr_t scratch: 64 bytes covers musl/glibc (56) and macOS (64).
    const ATTR_OFFSET: usize = 56;
    // plan-156-A: args 4 and 5, the seed type's and the result type's copy functions
    // (0 for a value that crosses as its word), parked before the calls below clobber
    // the argument registers.
    const SEED_COPY_OFFSET: usize = 120;
    const SAVED_ARENA_OFFSET: usize = 128;
    // The worker's arena-state block, until it is stored in the control block.
    const WORKER_OFFSET: usize = 136;
    const RESULT_COPY_OFFSET: usize = 144;
    // plan-156-C: the parent view this call returns, in the caller's arena.
    const VIEW_OFFSET: usize = 152;
    // Largest queue limit whose ring size (`capacity * THREAD_QUEUE_ENTRY_SIZE`) still
    // fits in 64 bits.
    const MAX_QUEUE_LIMIT: u64 = u64::MAX / THREAD_QUEUE_ENTRY_SIZE as u64;

    // bug-369: the writable globals region (program globals + `LINK`/`FREE` pointer
    // slots + `term::` state) is addressed off the pinned arena-state register at
    // `ENTRY_GLOBALS_OFFSET + slot * 8`. The main thread carves it out of the entry
    // frame; a worker's arena state is allocated HERE, so it must reserve the same
    // slots or every global access in a worker lands past the end of the block. Sized
    // to `ENTRY_GLOBALS_OFFSET` (not `ARENA_STATE_SIZE`) so the entry's one seed-scratch
    // word between the state and the globals is present too.
    let worker_arena_size = worker_arena_state_size(arena_global_slots);

    let invalid_limit = format!("{symbol}_invalid_limit");
    let oom = format!("{symbol}_oom");
    let oom_unslotted = format!("{symbol}_oom_unslotted");
    let init_error = format!("{symbol}_init_error");
    let spawn_error = format!("{symbol}_spawn_error");
    let parent_done = format!("{symbol}_parent_done");
    let view_oom = format!("{symbol}_view_oom");
    let spawned = format!("{symbol}_spawned");
    let block_zero = format!("{symbol}_block_zero");
    let mut vregs = Vregs::new();
    let r = &ThreadVregs::mint(&mut vregs);
    let mut instructions = Vec::new();
    let mut relocations = Vec::new();

    instructions.extend([
        abi::store_u64(abi::c_arg(0), abi::stack_pointer(), ENTRY_OFFSET),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64(abi::c_arg(2), abi::stack_pointer(), IN_LIMIT_OFFSET),
        abi::store_u64(abi::c_arg(3), abi::stack_pointer(), OUT_LIMIT_OFFSET),
        abi::store_u64(abi::c_arg(4), abi::stack_pointer(), SEED_COPY_OFFSET),
        abi::store_u64(abi::c_arg(5), abi::stack_pointer(), RESULT_COPY_OFFSET),
        abi::compare_immediate(abi::c_arg(2), "1"),
        abi::branch_lt(&invalid_limit),
        abi::compare_immediate(abi::c_arg(3), "1"),
        abi::branch_lt(&invalid_limit),
        // Upper-bound the queue limit so the ring size cannot wrap 64 bits and
        // under-allocate; an out-of-range limit is an invalid argument (bug-60).
        abi::move_immediate(&r.v12, "Integer", &MAX_QUEUE_LIMIT.to_string()),
        abi::compare_registers(abi::c_arg(2), &r.v12),
        abi::branch_hi(&invalid_limit),
        abi::compare_registers(abi::c_arg(3), &r.v12),
        abi::branch_hi(&invalid_limit),
        // plan-156-C: the parent view — the `Thread` value this call returns — is a
        // resource record in the CALLER's arena, like any other resource the caller
        // opens, so the caller's scope drop frees it. Allocated before anything else so
        // every later failure only has to free it.
        abi::move_immediate(abi::c_arg(0), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_ne(&view_oom),
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), VIEW_OFFSET),
        // plan-156-A: the control block comes from the process heap — it belongs to no
        // thread, so whichever side lets go of it last can free it.
        abi::move_immediate(abi::c_arg(0), "Integer", &CONTROL_BLOCK_SIZE.to_string()),
    ]);
    platform.emit_heap_alloc(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&oom_unslotted),
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), CB_OFFSET),
        // Zero the whole block: RUNNING, not cancelled, no result, empty queues, no
        // reclaim lists, and every pointer a later failure path tests reads 0.
        abi::move_register(&r.v9, abi::c_return(0)),
        abi::add_immediate(&r.v10, &r.v9, CONTROL_BLOCK_SIZE),
        abi::label(&block_zero),
        abi::store_u64(abi::ZERO, &r.v9, 0),
        abi::add_immediate(&r.v9, &r.v9, 8),
        abi::compare_registers(&r.v9, &r.v10),
        abi::branch_lo(&block_zero),
        abi::load_u64(&r.v9, abi::stack_pointer(), CB_OFFSET),
        // plan-156-A §4.4: one reference for the handle side, one for the worker.
        abi::move_immediate(&r.v10, "Integer", "2"),
        abi::store_u64(&r.v10, &r.v9, CONTROL_OFFSET_REFS),
        abi::load_u64(&r.v10, abi::stack_pointer(), ENTRY_OFFSET),
        abi::store_u64(&r.v10, &r.v9, CONTROL_OFFSET_ENTRY),
        abi::load_u64(&r.v10, abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64(&r.v10, &r.v9, CONTROL_OFFSET_DATA),
        abi::load_u64(&r.v10, abi::stack_pointer(), RESULT_COPY_OFFSET),
        abi::store_u64(&r.v10, &r.v9, CONTROL_OFFSET_RESULT_COPY),
        // plan-156-C: the worker's view, embedded in the (zeroed) block: open, no STATE.
        abi::move_immediate(&r.v10, "Integer", RESOURCE_TAG_THREAD_WORKER),
        abi::store_u64(
            &r.v10,
            &r.v9,
            CONTROL_OFFSET_WORKER_VIEW + RESOURCE_OFFSET_TAG,
        ),
        abi::store_u64(
            &r.v9,
            &r.v9,
            CONTROL_OFFSET_WORKER_VIEW + RESOURCE_OFFSET_HANDLE,
        ),
        // The table slot. From here on every failure releases the block through
        // `_mfb_rt_thread_block_release`, which frees whatever the block holds so far.
        abi::move_register(abi::c_arg(0), &r.v9),
        abi::branch_link(THREAD_TABLE_ACQUIRE_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, THREAD_TABLE_ACQUIRE_SYMBOL));
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_ne(&oom_unslotted),
        abi::branch_link(THREAD_BLOCK_ARENA_CREATE_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, THREAD_BLOCK_ARENA_CREATE_SYMBOL));
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_ne(&oom),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), ARENA_OFFSET),
        abi::load_u64(&r.v9, abi::stack_pointer(), CB_OFFSET),
        abi::store_u64(RESULT_VALUE_REGISTER, &r.v9, CONTROL_OFFSET_ARENA),
        abi::add_immediate(abi::c_arg(0), &r.v9, CONTROL_OFFSET_LOCK),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    };
    emit_thread_external_call(&mut ctx, "pthread_mutex_init")?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_ne(&init_error),
    ]);
    // plan-156-A: the worker's seed is its own copy, made in the block arena (bug-655's
    // leak came from handing over the caller's block uncopied; "`data` is copied into
    // the thread" is the documented contract). No lock is needed: the worker does not
    // exist yet, so nothing else can reach the block. A word-sized seed is stored as is.
    let seed_stored = format!("{symbol}_seed_stored");
    ctx.instructions.extend([
        abi::load_u64(&r.v9, abi::stack_pointer(), SEED_COPY_OFFSET),
        abi::compare_immediate(&r.v9, "0"),
        abi::branch_eq(&seed_stored),
    ]);
    emit_call_in_block_arena(
        ctx.instructions,
        &r.v15,
        ARENA_OFFSET,
        SAVED_ARENA_OFFSET,
        SEED_COPY_OFFSET,
        DATA_OFFSET,
    );
    ctx.instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_ne(&oom),
        abi::load_u64(&r.v9, abi::stack_pointer(), CB_OFFSET),
        abi::store_u64(RESULT_VALUE_REGISTER, &r.v9, CONTROL_OFFSET_DATA),
        abi::label(&seed_stored),
    ]);
    // The four queues: the data queues bounded by the two limits, and the resource
    // plane's (§7) inbound (parent→worker) bounded like the inbound data queue and
    // outbound (worker→parent) like the outbound one.
    for (limit, queue) in [
        (IN_LIMIT_OFFSET, CONTROL_OFFSET_INBOUND_QUEUE),
        (OUT_LIMIT_OFFSET, CONTROL_OFFSET_OUTBOUND_QUEUE),
        (IN_LIMIT_OFFSET, CONTROL_OFFSET_RESOURCE_INBOUND_QUEUE),
        (OUT_LIMIT_OFFSET, CONTROL_OFFSET_RESOURCE_OUTBOUND_QUEUE),
    ] {
        emit_thread_queue_init(
            &mut ctx,
            r,
            CB_OFFSET,
            ARENA_OFFSET,
            SAVED_ARENA_OFFSET,
            limit,
            queue,
            &oom,
            &init_error,
        )?;
    }
    // The worker's arena-state block, from the heap like the block itself.
    ctx.instructions.push(abi::move_immediate(
        abi::c_arg(0),
        "Integer",
        &worker_arena_size.to_string(),
    ));
    platform.emit_heap_alloc(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    // Zero the whole worker arena block (allocator-04): this initializer must stay in
    // lockstep with the program-entry zeroing (`lower_program_entry`), which zeroes
    // `ARENA_STATE_SIZE` and then every global slot — exactly the words zeroed here.
    let child_zero_loop = format!("{symbol}_child_arena_zero");
    ctx.instructions.extend([
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&oom),
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), WORKER_OFFSET),
        abi::move_register(&r.v11, abi::c_return(0)),
        abi::add_immediate(&r.v12, &r.v11, worker_arena_size),
        abi::label(&child_zero_loop),
        abi::store_u64(abi::ZERO, &r.v11, 0),
        abi::add_immediate(&r.v11, &r.v11, 8),
        abi::compare_registers(&r.v11, &r.v12),
        abi::branch_lo(&child_zero_loop),
        abi::load_u64(&r.v11, abi::stack_pointer(), WORKER_OFFSET),
        abi::load_u64(&r.v9, abi::stack_pointer(), CB_OFFSET),
        abi::store_u64(&r.v11, &r.v9, CONTROL_OFFSET_WORKER_ARENA),
        // Inherit the parent's Money rounding mode (plan-29-D): the child arena was just
        // zeroed (= Commercial), so copy the spawning thread's mode field (`x19` is the
        // parent arena here) into the child, which then diverges independently.
        abi::load_u64(&r.v12, abi::ARENA, ARENA_ROUNDING_MODE_OFFSET),
        abi::store_u64(&r.v12, &r.v11, ARENA_ROUNDING_MODE_OFFSET),
    ]);
    // plan-130-C: register the worker's arena with the `--debug` arena registry. The
    // parent does it, synchronously, before the worker exists, so workers register in
    // start order.
    if debug_arena_registry {
        ctx.instructions.extend([
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), WORKER_OFFSET),
            abi::move_immediate(
                abi::c_arg(1),
                "Integer",
                crate::codegen::debug::ARENA_KIND_WORKER,
            ),
            abi::branch_link(crate::codegen::debug::DEBUG_ARENA_REGISTER_SYMBOL),
        ]);
        ctx.relocations.push(internal_branch(
            symbol,
            crate::codegen::debug::DEBUG_ARENA_REGISTER_SYMBOL,
        ));
    }
    if uses_rng {
        // Give the new thread its own PCG64 stream by drawing a 64-bit seed from the
        // spawning thread's generator (runs in the parent, so `x19` is the parent
        // arena and the draw is race-free).
        ctx.instructions.push(abi::branch_link(RNG_NEXT_SYMBOL));
        ctx.relocations
            .push(internal_branch(symbol, RNG_NEXT_SYMBOL));
        ctx.instructions.extend([
            abi::move_register(abi::c_arg(1), abi::return_register()),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), WORKER_OFFSET),
        ]);
        ctx.instructions.push(abi::branch_link(RNG_SEED_SYMBOL));
        ctx.relocations
            .push(internal_branch(symbol, RNG_SEED_SYMBOL));
    }
    // Seed the worker's dedicated memory-fill RNG (always on, plan-01 §6): a value from
    // the parent's fill stream XOR the worker arena address, so each worker poisons
    // with a distinct stream, separate from its `math::rand` stream.
    ctx.instructions
        .push(abi::branch_link(ARENA_FILL_NEXT_SYMBOL));
    ctx.relocations
        .push(internal_branch(symbol, ARENA_FILL_NEXT_SYMBOL));
    ctx.instructions.extend([
        abi::move_register(abi::c_arg(1), abi::return_register()),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), WORKER_OFFSET),
        abi::exclusive_or_registers(abi::c_arg(1), abi::c_arg(1), abi::return_register()),
    ]);
    ctx.instructions
        .push(abi::branch_link(ARENA_FILL_SEED_SYMBOL));
    ctx.relocations
        .push(internal_branch(symbol, ARENA_FILL_SEED_SYMBOL));

    if platform.family() == PlatformFamily::Windows {
        // CreateThread(NULL, 8 MiB, trampoline, cb, 0, NULL). lpStartAddress and
        // lpParameter land in r8/r9 exactly where the POSIX ARG[2]/ARG[3] would; the two
        // remaining stack args (dwCreationFlags, lpThreadId) plus the 32-byte shadow ride
        // a self-contained 0x40 frame. CreateThread returns the OS HANDLE (NULL on
        // failure). Load lpParameter (cb) BEFORE the stack adjustment — CB_OFFSET is
        // relative to the function frame.
        ctx.instructions.push(abi::load_u64(
            abi::c_arg(3),
            abi::stack_pointer(),
            CB_OFFSET,
        ));
        ctx.instructions.push(abi::subtract_stack(0x40));
        ctx.instructions.push(abi::load_page_address(
            abi::c_arg(2),
            THREAD_TRAMPOLINE_SYMBOL,
        ));
        ctx.relocations.push(CodeRelocation {
            from: symbol.to_string(),
            to: THREAD_TRAMPOLINE_SYMBOL.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        });
        ctx.instructions.push(abi::add_page_offset(
            abi::c_arg(2),
            abi::c_arg(2),
            THREAD_TRAMPOLINE_SYMBOL,
        ));
        ctx.relocations.push(CodeRelocation {
            from: symbol.to_string(),
            to: THREAD_TRAMPOLINE_SYMBOL.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        });
        ctx.instructions.extend([
            abi::move_immediate(abi::c_arg(0), "Integer", "0"), // lpThreadAttributes
            abi::move_immediate(abi::c_arg(1), "Integer", &THREAD_STACK_BYTES.to_string()),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20), // dwCreationFlags = 0
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x28), // lpThreadId = NULL
            abi::branch_link("CreateThread"),
        ]);
        ctx.relocations
            .push(external_branch(symbol, "CreateThread", platform_imports)?);
        ctx.instructions.extend([
            abi::add_stack(0x40),
            abi::load_u64(&r.v9, abi::stack_pointer(), CB_OFFSET),
            abi::store_u64(abi::c_return(0), &r.v9, CONTROL_OFFSET_OS_HANDLE),
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_eq(&spawn_error), // NULL = failure
            abi::branch(&spawned),
        ]);
    } else {
        let (pthread_create_symbol, attr_init_symbol, attr_setstacksize_symbol) =
            match platform.family() {
                PlatformFamily::MacOS => (
                    "_pthread_create",
                    "_pthread_attr_init",
                    "_pthread_attr_setstacksize",
                ),
                _ => (
                    "pthread_create",
                    "pthread_attr_init",
                    "pthread_attr_setstacksize",
                ),
            };
        // Give the worker an explicit 8 MiB stack. musl's default pthread stack is
        // 128 KiB — far below what the main thread gets — so worker code with large
        // frames (the regex engine has a ~230 KiB frame) overflowed on Linux/musl. The
        // memory is reserved lazily (virtual). The attr is stack scratch;
        // pthread_attr_destroy is a no-op for a stacksize-only attr, so it is not called.
        ctx.instructions.push(abi::add_immediate(
            abi::c_arg(0),
            abi::stack_pointer(),
            ATTR_OFFSET,
        ));
        ctx.instructions.push(abi::branch_link(attr_init_symbol));
        ctx.relocations
            .push(external_branch(symbol, attr_init_symbol, platform_imports)?);
        ctx.instructions.extend([
            abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), ATTR_OFFSET),
            abi::move_immediate(abi::c_arg(1), "Integer", &THREAD_STACK_BYTES.to_string()),
        ]);
        ctx.instructions
            .push(abi::branch_link(attr_setstacksize_symbol));
        ctx.relocations.push(external_branch(
            symbol,
            attr_setstacksize_symbol,
            platform_imports,
        )?);
        ctx.instructions.extend([
            abi::load_u64(&r.v9, abi::stack_pointer(), CB_OFFSET),
            abi::add_immediate(abi::c_arg(0), &r.v9, CONTROL_OFFSET_OS_HANDLE),
            abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), ATTR_OFFSET),
        ]);
        ctx.instructions.push(abi::load_page_address(
            abi::c_arg(2),
            THREAD_TRAMPOLINE_SYMBOL,
        ));
        ctx.relocations.push(CodeRelocation {
            from: symbol.to_string(),
            to: THREAD_TRAMPOLINE_SYMBOL.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        });
        ctx.instructions.push(abi::add_page_offset(
            abi::c_arg(2),
            abi::c_arg(2),
            THREAD_TRAMPOLINE_SYMBOL,
        ));
        ctx.relocations.push(CodeRelocation {
            from: symbol.to_string(),
            to: THREAD_TRAMPOLINE_SYMBOL.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        });
        ctx.instructions.extend([
            abi::move_register(abi::c_arg(3), &r.v9),
            abi::branch_link(pthread_create_symbol),
        ]);
        ctx.relocations.push(external_branch(
            symbol,
            pthread_create_symbol,
            platform_imports,
        )?);
        ctx.instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_ne(&spawn_error),
            abi::load_u64(&r.v9, abi::stack_pointer(), CB_OFFSET),
            abi::branch(&spawned),
        ]);
    }

    // plan-156-C: the thread runs; fill in the parent view and return it. Every word is
    // written — the record came from the fill-poisoned arena — and it is open, with no
    // STATE (plan-156-E gives it one).
    ctx.instructions.extend([
        abi::label(&spawned),
        abi::load_u64(&r.v9, abi::stack_pointer(), CB_OFFSET),
        abi::load_u64(&r.v10, abi::stack_pointer(), VIEW_OFFSET),
    ]);
    for offset in (0..RESOURCE_RECORD_SIZE_BYTES).step_by(8) {
        ctx.instructions
            .push(abi::store_u64(abi::ZERO, &r.v10, offset));
    }
    ctx.instructions.extend([
        abi::move_immediate(&r.v11, "Integer", RESOURCE_TAG_THREAD),
        abi::store_u64(&r.v11, &r.v10, RESOURCE_OFFSET_TAG),
        abi::store_u64(&r.v9, &r.v10, RESOURCE_OFFSET_HANDLE),
        abi::move_register(RESULT_VALUE_REGISTER, &r.v10),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&parent_done),
    ]);

    // Failure paths. A failed start frees everything it built: before the table slot
    // there is only the heap block (freed directly); after it the block release frees
    // the slot, the block arena and the worker arena block, whichever exist (zero words
    // are skipped). The error registers are parked across the release.
    let free_view = |ctx: &mut EmitCtx| {
        ctx.instructions.extend([
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), VIEW_OFFSET),
            abi::move_immediate(abi::c_arg(1), "Integer", RESOURCE_RECORD_SIZE),
            abi::branch_link(ARENA_FREE_SYMBOL),
        ]);
        ctx.relocations
            .push(internal_branch(symbol, ARENA_FREE_SYMBOL));
    };
    let release_and_fail = |ctx: &mut EmitCtx, error: &str| -> Result<(), String> {
        ctx.instructions.extend([
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), CB_OFFSET),
            abi::branch_link(THREAD_BLOCK_RELEASE_SYMBOL),
        ]);
        ctx.relocations
            .push(internal_branch(symbol, THREAD_BLOCK_RELEASE_SYMBOL));
        free_view(ctx);
        raise_error_into(symbol, error, ctx.instructions, ctx.relocations);
        ctx.instructions.push(abi::branch(&parent_done));
        Ok(())
    };
    ctx.instructions.push(abi::label(&invalid_limit));
    raise_error_into(
        symbol,
        "ErrInvalidArgument",
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions
        .extend([abi::branch(&parent_done), abi::label(&oom_unslotted)]);
    ctx.instructions.push(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        CB_OFFSET,
    ));
    let skip_free = format!("{symbol}_oom_no_block");
    ctx.instructions.extend([
        abi::compare_immediate(abi::c_arg(0), "0"),
        abi::branch_eq(&skip_free),
    ]);
    platform.emit_heap_free(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    ctx.instructions.push(abi::label(&skip_free));
    free_view(&mut ctx);
    ctx.instructions.push(abi::label(&view_oom));
    raise_error_into(symbol, "ErrOutOfMemory", ctx.instructions, ctx.relocations);
    ctx.instructions
        .extend([abi::branch(&parent_done), abi::label(&oom)]);
    release_and_fail(&mut ctx, "ErrOutOfMemory")?;
    ctx.instructions.push(abi::label(&init_error));
    release_and_fail(&mut ctx, "ErrInterrupted")?;
    ctx.instructions.push(abi::label(&spawn_error));
    release_and_fail(&mut ctx, "ErrInterrupted")?;
    ctx.instructions
        .extend([abi::label(&parent_done), abi::return_()]);

    Ok((instructions, relocations, FRAME_SIZE))
}

pub(crate) fn lower_thread_trampoline(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    uses_stdin: bool,
    arena_init: ArenaInitSymbols,
) -> Result<CodeFunction, String> {
    // Machine-floor code: hand-managed frame + pinned registers across the worker
    // call and the exit helper, so the allocator cannot run here. Scratch is
    // the neutral `abi::SCRATCH` pool (realized in `abi.rs`), confined to the low
    // indices `SCRATCH[4]`/`SCRATCH[5]` (AArch64 `x13`/`x14`) — NOT `SCRATCH[0]`
    // (`x9`): the x86 residual-scratch pool wraps at xN+11, so `x9` would alias the
    // parked control-block register (`abi::CURRENT_THREAD` = `x20`, both `rbx`) and
    // `load SCRATCH[0],[%thread,…]` would destroy the block pointer it dereferences.
    // `x13`/`x14` (`r9`/`r10`) are distinct from the current-thread register on both
    // ISAs.
    const FRAME_SIZE: usize = 80;
    const LR_OFFSET: usize = 0;
    const ARENA_OFFSET: usize = 8;
    const X20_OFFSET: usize = 16;
    const CLOSURE_OFFSET: usize = 24;
    const CB_OFFSET: usize = 32;
    const TAG_OFFSET: usize = 40;
    const VALUE_OFFSET: usize = 48;
    const ERROR_OFFSET: usize = 56;
    const SOURCE_OFFSET: usize = 64;
    // bug-957: the last free slot of the 80-byte frame. RISC-V realizes
    // `SCRATCH[4]` (AArch64 `x13`) as `s0` (`map_scratch_register` in
    // `arch/riscv64/select.rs`), which is CALLEE-saved under lp64d — and this
    // routine's caller is libc. glibc's `start_thread` keeps its `struct pthread *`
    // in `s0` across `pd->start_routine(pd->arg)` and stores the result through it
    // the moment this returns, so a clobbered `s0` SIGSEGVed every `thread::start`
    // on riscv64 glibc (musl's start path happens not to read `s0` afterwards).
    // On AArch64 (`x13`) and x86-64 (`r9`) the register is caller-saved, so only
    // riscv64 parks it: their trampoline bytes and frames are unchanged.
    const SCRATCH_OFFSET: usize = 72;
    let scratch_is_callee_saved = platform.arch() == "riscv64";
    // bug-982 audit: the worker is compiled MFB code, which keeps the 8th internal
    // argument in `rbp` on x86-64 (bug-296) and, on Win64, the 5th and 6th in
    // `rdi`/`rsi` — all three callee-saved to the thread library that called this.
    // (`rdi` is also where this routine itself stages `_mfb_rt_thread_finish`'s
    // fifth argument.) An `rbp` or `rdi` handed back changed reaches
    // `start_thread` / `BaseThreadInitThunk`, which may keep its own state there.
    // AArch64 and RISC-V pass those arguments in caller-saved registers, so they
    // save nothing extra and keep their frames. The slots start where RISC-V's
    // `SCRATCH_OFFSET` sits, which x86-64 never uses.
    let x86_argument_saves: Vec<abi::Reg> = if platform.arch() == "x86_64" {
        let mut saves = vec![abi::mfb_arg(7)];
        if platform.family() == PlatformFamily::Windows {
            saves.extend([abi::mfb_arg(4), abi::mfb_arg(5)]);
        }
        saves
    } else {
        Vec::new()
    };
    const ARGUMENT_SAVE_OFFSET: usize = SCRATCH_OFFSET;
    let slots_end = ARGUMENT_SAVE_OFFSET + 8 * x86_argument_saves.len();
    let worker_result = format!("{THREAD_TRAMPOLINE_SYMBOL}_worker_result");

    // Stack-alignment fixup, mirroring the program entry's
    // `entry_stack_misaligned_on_entry` (`entry.rs`): the hand-managed frame
    // below (like the entry) assumes `sp % 16 == 0` before it issues any call,
    // and `FRAME_SIZE` is a 16-multiple, so it preserves whatever alignment the
    // trampoline is entered with. Whether the entry alignment is `sp % 16 == 0`
    // or `sp % 16 == 8` depends on HOW the OS/thread library reaches this
    // start-routine, which differs per ISA — so the fixup is gated per-arch:
    //
    //   - EVERY x86-64 thread library reaches the start-routine with a `call`
    //     (glibc `start_thread` → `pd->start_routine(...)`; musl's pthread
    //     dispatch; Windows `BaseThreadInitThunk`). The `call` pushes an 8-byte
    //     return address onto an otherwise 16-aligned stack, so the trampoline is
    //     always entered at `sp % 16 == 8`. Without the +8 bias every downstream
    //     `call` lands at `sp % 16 == 8` and the first SSE store to a stack local
    //     (`movaps`/`movdqa` in `fstatat`/`pthread_create`/ntdll `SwitchBack`)
    //     faults — a hard SIGSEGV. So x86-64 ALWAYS needs the +8, independent of
    //     libc (bug-408; box-proven 2026-08-01: on glibc 2228 the 88-byte frame
    //     runs and the 80-byte frame SIGSEGVs 5/5; on musl 2227 the 88-byte frame
    //     runs and the 96-byte double-realign SIGSEGVs 5/5; Windows 47-H).
    //
    // This corrects bug-385, which wrongly believed glibc `start_thread` entered
    // the routine already 16-aligned and gated glibc OUT of the realign. That was
    // masked because `linux_x86_64` carried a SECOND, unconditional +8 override
    // that silently restored glibc's frame to 88 (and pushed musl to a broken 96);
    // bug-408 removes that override and lets this single per-arch realign stand.
    //
    // aarch64/riscv64 are unaffected on every OS: `bl`/`jal` write the return
    // address to a link register and leave `sp` unchanged, so the trampoline is
    // entered at `sp % 16 == 0` already and keeps its byte-identical 80-byte
    // frame. macOS (aarch64) likewise takes no realign.
    let needs_realign = platform.arch() == "x86_64";
    let x86_realign = if needs_realign { 8 } else { 0 };
    // The x86 realign's 8 bytes sit at the top of the frame, unused, so SysV's one
    // extra slot fits inside the 88-byte frame; Win64's three grow it by 16.
    let frame =
        FRAME_SIZE.max(slots_end.saturating_sub(x86_realign).div_ceil(16) * 16) + x86_realign;

    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(frame),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), ARENA_OFFSET),
        abi::store_u64(abi::CURRENT_THREAD, abi::stack_pointer(), X20_OFFSET),
        abi::store_u64(CLOSURE_ENV_REGISTER, abi::stack_pointer(), CLOSURE_OFFSET),
    ];
    if scratch_is_callee_saved {
        instructions.push(abi::store_u64(
            abi::SCRATCH[4],
            abi::stack_pointer(),
            SCRATCH_OFFSET,
        ));
    }
    for (index, register) in x86_argument_saves.iter().enumerate() {
        instructions.push(abi::store_u64(
            *register,
            abi::stack_pointer(),
            ARGUMENT_SAVE_OFFSET + 8 * index,
        ));
    }
    instructions.extend([
        abi::move_register(abi::CURRENT_THREAD, abi::c_arg(0)),
        abi::store_u64(abi::CURRENT_THREAD, abi::stack_pointer(), CB_OFFSET),
        abi::load_u64(
            ARENA_STATE_REGISTER,
            abi::CURRENT_THREAD,
            CONTROL_OFFSET_WORKER_ARENA,
        ),
        // plan-99: publish the TCB back into this worker's arena state, now that
        // both pinned registers are live. `os::sleep` reads `[arena+8]` to tell a
        // worker from the main thread (which leaves the zero-init `0` there) and
        // uses the value as the handle its cancellation-aware wait needs. One
        // store, once per worker; the main thread's entry path writes nothing.
        abi::store_u64(
            abi::CURRENT_THREAD,
            ARENA_STATE_REGISTER,
            ARENA_WORKER_THREAD_OFFSET,
        ),
    ]);
    let mut relocations = Vec::new();
    // bug-789: the worker's stack limit, before the worker runs. `thread::start`
    // gave it a `THREAD_STACK_BYTES` stack. The calls preserve the callee-saved
    // arena and current-thread registers.
    platform.emit_stack_limit(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        crate::codegen::engine::types::ThreadStack::Created(THREAD_STACK_BYTES),
        &mut instructions,
        &mut relocations,
    )?;
    // bug-369: everything in the writable globals region is PER-ARENA, and this
    // worker's arena was allocated fresh and zeroed by `thread::start`. Re-run the
    // same initializers the program entry runs, in the same order, so a worker
    // sees the region the main thread sees instead of a region of zeros:
    //
    //   - `_mfb_linker_init` resolves each `LINK`/`FREE` symbol into its pointer
    //     slot. Without it a `LINK` call from a worker jumps through a null slot
    //     (SIGSEGV). Re-running it per worker is idempotent in effect: `dlopen` is
    //     refcounted and never closed here, and `dlsym` returns the same address.
    //   - the module's global initializer writes each declared global. It runs
    //     after the linker init because a global's initializer may call a `LINK`
    //     function.
    //
    // The static closure descriptors are deliberately NOT re-run: they live in
    // process-global BSS, not the arena, so the entry's one-time pass covers every
    // thread.
    //
    // This is safe despite the machine-floor comment above: the allocator
    // restriction covers the pinned-register region AROUND the worker call, and
    // this sits at exactly the same point in the frame as that call — the arena
    // register is installed, the control block is parked on the stack, and no
    // scratch register is live yet (SCRATCH[4]/CLOSURE_ENV are loaded below,
    // after these return). Each initializer is an ordinary lowered function, so it
    // preserves the callee-saved arena and current-thread registers.
    //
    // A failing initializer must not run the worker body: its result registers
    // become the thread's result, exactly as if the body itself had failed, so
    // `thread::waitFor` reports the initializer's error to the parent.
    for (index, symbol) in arena_init.in_run_order().enumerate() {
        let ready = format!("{THREAD_TRAMPOLINE_SYMBOL}_arena_init_{index}_ok");
        instructions.push(abi::branch_link(symbol));
        relocations.push(internal_branch(THREAD_TRAMPOLINE_SYMBOL, symbol));
        instructions.extend([
            abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
            abi::branch_eq(&ready),
            abi::branch(&worker_result),
            abi::label(&ready),
            // Each `bl` clobbers the caller-saved registers, so re-derive the
            // control block from its parked stack slot before the next one.
            abi::load_u64(abi::CURRENT_THREAD, abi::stack_pointer(), CB_OFFSET),
        ]);
    }
    instructions.extend([
        abi::load_u64(abi::SCRATCH[4], abi::CURRENT_THREAD, CONTROL_OFFSET_ENTRY),
        abi::load_u64(CLOSURE_ENV_REGISTER, abi::SCRATCH[4], CLOSURE_OFFSET_ENV),
        abi::load_u64(abi::SCRATCH[4], abi::SCRATCH[4], CLOSURE_OFFSET_CODE),
        abi::load_u64(abi::c_arg(1), abi::CURRENT_THREAD, CONTROL_OFFSET_DATA),
        // plan-156-C: the entry's `ThreadWorker` is the view embedded in the block.
        abi::add_immediate(
            abi::c_arg(0),
            abi::CURRENT_THREAD,
            CONTROL_OFFSET_WORKER_VIEW,
        ),
        abi::branch_link_register(abi::SCRATCH[4]),
        abi::label(&worker_result),
        abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            ERROR_OFFSET,
        ),
        abi::store_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            SOURCE_OFFSET,
        ),
    ]);
    // plan-15 §4.5: auto-unsubscribe the worker from the stdin broadcast log at
    // teardown so an early-exiting worker never permanently pins the log's
    // reclamation point (which would eventually block other readers at the cap).
    // `x19` still holds the worker arena here; the result registers are already
    // parked on the stack, and `unsubscribe` preserves the callee-saved arena base.
    if uses_stdin {
        instructions.push(abi::move_register(abi::c_arg(0), ARENA_STATE_REGISTER));
        instructions.push(abi::branch_link(STDIN_UNSUBSCRIBE_SYMBOL));
        relocations.push(internal_branch(
            THREAD_TRAMPOLINE_SYMBOL,
            STDIN_UNSUBSCRIBE_SYMBOL,
        ));
    }
    // bug-806: the worker's own instance of the project ends here. Close the
    // resources its module-level bindings own before the result is published, so
    // a parent that waits for the worker observes them closed. The result is parked
    // on the stack above; the teardown preserves the callee-saved arena register.
    if let Some(symbol) = arena_init.teardown {
        instructions.push(abi::branch_link(symbol));
        relocations.push(internal_branch(THREAD_TRAMPOLINE_SYMBOL, symbol));
    }
    // plan-156-A: everything the exit does to the control block — publish the result
    // and `COMPLETED`, close the queues and wake every waiter, drop the worker's
    // reference and, when it was the last, free the block — is one vreg-allocated
    // helper, so this machine-floor tail only marshals the parked result into it.
    instructions.extend([
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), CB_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), TAG_OFFSET),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), VALUE_OFFSET),
        abi::load_u64(abi::c_arg(3), abi::stack_pointer(), ERROR_OFFSET),
        abi::load_u64(abi::c_arg(4), abi::stack_pointer(), SOURCE_OFFSET),
        abi::branch_link(THREAD_FINISH_SYMBOL),
    ]);
    relocations.push(internal_branch(
        THREAD_TRAMPOLINE_SYMBOL,
        THREAD_FINISH_SYMBOL,
    ));
    if scratch_is_callee_saved {
        instructions.push(abi::load_u64(
            abi::SCRATCH[4],
            abi::stack_pointer(),
            SCRATCH_OFFSET,
        ));
    }
    for (index, register) in x86_argument_saves.iter().enumerate() {
        instructions.push(abi::load_u64(
            *register,
            abi::stack_pointer(),
            ARGUMENT_SAVE_OFFSET + 8 * index,
        ));
    }
    instructions.extend([
        abi::move_immediate(abi::c_return(0), "Integer", "0"),
        abi::load_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), ARENA_OFFSET),
        abi::load_u64(CLOSURE_ENV_REGISTER, abi::stack_pointer(), CLOSURE_OFFSET),
        abi::load_u64(abi::CURRENT_THREAD, abi::stack_pointer(), X20_OFFSET),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(frame),
        abi::return_(),
    ]);
    // plan-34-D: the trampoline is machine-floor shared lowering that bypasses
    // the allocator — its stream must still name no physical register (pinned
    // registers and scratch are `abi` tokens, realized during selection).
    if let Some(offense) = regalloc::find_physical_operand(&instructions) {
        return Err(format!(
            "thread-trampoline lowering violated the zero-physical-register \
             invariant (plan-34-D): {offense}"
        ));
    }
    Ok(CodeFunction {
        name: "runtime.thread.trampoline".to_string(),
        symbol: THREAD_TRAMPOLINE_SYMBOL.to_string(),
        params: vec![CodeParam {
            name: "controlBlock".to_string(),
            type_: "ThreadControlBlock".to_string(),
            location: abi::c_arg(0).into(),
        }],
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: frame,
            callee_saved: [abi::link_register(), abi::CURRENT_THREAD]
                .into_iter()
                .chain(scratch_is_callee_saved.then_some(abi::SCRATCH[4]))
                .map(|register| register.to_string())
                .collect(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    })
}
