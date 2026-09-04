// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::regalloc;
use crate::codegen::engine::types::CodegenPlatform;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::os::syscall::{SIGPIPE_SIGNO, SIG_IGN};
use crate::target::shared::abi;
use crate::types::ParameterType;
use std::collections::HashMap;
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_program_entry(
    entry_symbol: &str,
    language_entry_symbol: &str,
    language_entry_returns: &ParameterType,
    language_entry_accepts_args: bool,
    global_initializer_symbol: Option<&str>,
    link_init_symbol: Option<&str>,
    closure_init_symbol: Option<&str>,
    entry_stack_size: usize,
    global_slot_count: usize,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    emit_cleanup_failure_audit: bool,
    seed_rng: bool,
    register_signal_handlers: bool,
    capture_args: bool,
    subscribe_stdin: bool,
    entry_called_as_function: bool,
    needs_winsock: bool,
    seed_presentation_mode_offset: Option<usize>,
) -> Result<CodeFunction, String> {
    // bug-175 I: the `entry_exit_range_error` handler (and its label) is emitted
    // only for an `Integer` entry, but the range-check branch to it is emitted for
    // every non-`Nothing` return. A future non-`Integer`/`Nothing` entry return
    // would branch to an undefined label → link failure. `validate_entry_point`
    // already forces Integer/Nothing; enforce that invariant here as a loud
    // plan-level error rather than a silent broken branch.
    if *language_entry_returns != ParameterType::Integer
        && *language_entry_returns != ParameterType::Nothing
    {
        return Err(format!(
            "program entry return type must be 'Integer' or 'Nothing', got \
             '{language_entry_returns}'"
        ));
    }
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // Realign the stack to the 16-byte ABI boundary the rest of the entry
    // assumes. A Windows PE entry is `call`-reached by the loader, so it arrives
    // at `sp % 16 == 8`; one `sub sp, 8` restores `sp % 16 == 0` before any
    // capture/frame/call runs. The Linux/macOS entries already arrive aligned
    // and return false, so their entry stays byte-identical. The entry never
    // returns (it `ExitProcess`/`exit`s), so the lost 8 bytes are never reclaimed
    // and no incoming stack argument is disturbed (Windows delivers none).
    if platform.entry_stack_misaligned_on_entry() {
        instructions.push(abi::subtract_stack(8));
    }
    // Where argc/argv live on arrival. The platform answer describes the RAW
    // process entry; an entry reached as a call gets them in registers on every
    // platform, from its caller (bug-240).
    let args_in_registers = platform.entry_args_in_registers() || entry_called_as_function;
    // plan-32-A: on a raw-ELF `linux-riscv64` entry, probe the aux vector for the
    // RVV "V" bit and record it in `_mfb_rt_has_rvv` while `sp` still points at
    // the kernel's initial stack (argc/argv/envp/auxv all reachable, no frame
    // carved yet). riscv64-only and reads only the initial stack + writes the
    // global, so every other target's entry — and the argc/argv this entry loads
    // below — stays byte-identical. Skipped for a called-as-function entry, whose
    // `sp` carries no kernel auxv (riscv64 has no such app-mode entry today).
    if platform.arch() == "riscv64" && !args_in_registers {
        emit_riscv_hwcap_probe(entry_symbol, &mut instructions, &mut relocations);
    }
    // Capture argc/argv into the `os::args` globals before the frame is carved
    // (plan-31-B), while the OS-supplied values are still at their entry
    // positions: macOS delivers them in `ARG[0]`/`ARG[1]`; a raw Linux ELF entry
    // has argc at `[sp,0]` and argv (the `char**`) at `sp+8`. Uses
    // `SCRATCH[0]`/`SCRATCH[1]` only, so `ARG[0]`/`ARG[1]` stay live for the
    // arg-materialization path below. Gated on os.args
    // usage, so a program that never calls it keeps a byte-identical entry.
    // Windows defers capture until after the arena is mapped (its argv is built +
    // marshaled into the arena from GetCommandLineW, not delivered at entry) —
    // plan-66-B. Every other platform captures the OS-supplied argc/argv here.
    if capture_args && !platform.defers_arg_capture() {
        if args_in_registers {
            push_symbol_address(
                entry_symbol,
                crate::codegen::builtins::os::OS_ARGC_GLOBAL_SYMBOL,
                abi::SCRATCH[0],
                &mut instructions,
                &mut relocations,
            );
            instructions.push(abi::store_u64(abi::c_arg(0), abi::SCRATCH[0], 0));
            push_symbol_address(
                entry_symbol,
                crate::codegen::builtins::os::OS_ARGV_GLOBAL_SYMBOL,
                abi::SCRATCH[0],
                &mut instructions,
                &mut relocations,
            );
            instructions.push(abi::store_u64(abi::c_arg(1), abi::SCRATCH[0], 0));
        } else {
            instructions.push(abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), 0));
            push_symbol_address(
                entry_symbol,
                crate::codegen::builtins::os::OS_ARGC_GLOBAL_SYMBOL,
                abi::SCRATCH[0],
                &mut instructions,
                &mut relocations,
            );
            instructions.push(abi::store_u64(abi::SCRATCH[1], abi::SCRATCH[0], 0));
            instructions.push(abi::add_immediate(abi::SCRATCH[1], abi::stack_pointer(), 8));
            push_symbol_address(
                entry_symbol,
                crate::codegen::builtins::os::OS_ARGV_GLOBAL_SYMBOL,
                abi::SCRATCH[0],
                &mut instructions,
                &mut relocations,
            );
            instructions.push(abi::store_u64(abi::SCRATCH[1], abi::SCRATCH[0], 0));
        }
    }
    // A raw Linux ELF entry is jumped to with `argc` at `[sp]` / `argv` at
    // `[sp+8]` and undefined argument registers; load them into the
    // `ARG[0]`/`ARG[1]` the rest of the entry expects BEFORE the frame is carved
    // (the entry does not pass through `finalize_frame`, so `[sp,0]` here is the
    // true initial stack). macOS delivers them in `ARG[0]`/`ARG[1]` already
    // (libSystem calls `main`).
    //
    // Skipped when the entry is CALLED (app mode): the worker thread's `[sp]`
    // holds no kernel argv layout, so this would load garbage over the real
    // argc/argv the caller passed in registers (bug-240).
    if language_entry_accepts_args && !args_in_registers {
        instructions.extend([
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), 0),
            abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), 8),
        ]);
    }
    // Park argc/argv into callee-saved SCRATCH[17]/SCRATCH[18] (AArch64 x27/x28,
    // x86-64 r12/r13) IMMEDIATELY, while they are still live in ARG[0]/ARG[1].
    // Everything below clobbers them — starting with the very next block, the
    // arena-state zero loop, whose end pointer is `SCRATCH[1]`.
    //
    // On AArch64 `SCRATCH[1]` is x10, distinct from ARG[1]=x1, so parking later
    // was harmless there. On x86-64 both tokens realize to **rsi**
    // (`map_scratch_register(10)` → index (10-9)%11 = 1 → rsi; `CALL_ARGS[1]` →
    // rsi), so the loop destroyed argv two instructions after it was loaded and
    // the entry then dereferenced the arena address as a `char**`: every
    // arg-accepting program SIGSEGV'd on linux-x86_64, glibc and musl alike,
    // console and app mode (bug-240). aarch64/riscv64 were unaffected. Parking
    // first makes the sequence correct on every ISA rather than relying on a
    // token-aliasing coincidence.
    //
    // Gated on `language_entry_accepts_args`, so a non-arg entry stays
    // byte-identical.
    if language_entry_accepts_args && !platform.defers_arg_capture() {
        instructions.extend([
            abi::move_register(abi::SCRATCH[17], abi::c_arg(0)),
            abi::move_register(abi::SCRATCH[18], abi::c_arg(1)),
        ]);
    }
    // Reserve the callee's outgoing shadow space at the very bottom of the entry
    // frame and place the arena state ABOVE it. The entry manages its own stack
    // (it never passes through `finalize_frame`), so without this the arena state
    // would sit at `sp` and every call the entry makes — the arena-start-time
    // seed, the RNG seed, the global/link initializers, `main` — would write its
    // 32-byte shadow space into `[sp, sp+32)`, corrupting the arena block-list
    // head at `[arena+0]` (the first field). Win64 requires 32 bytes of shadow;
    // Linux/macOS return 0 here, so their entry stays byte-identical. Everything
    // addressed `[arena + X]` keeps its absolute position (arena == sp + shadow);
    // the sp-relative args region below shifts up by the same `shadow`.
    let shadow = platform.backend().shadow_space_bytes();
    // The x86-64 call-parity bias, when this entry is *called* rather than entered
    // by the kernel.
    //
    // SysV puts `rsp` 16-aligned at a `call` and therefore `rsp % 16 == 8` on entry,
    // so a frame that is a multiple of 16 leaves every call this entry makes
    // misaligned. `finalize_frame` adds `frame_call_padding()` for exactly this and
    // the entry never passes through it, so the bias has to be added here.
    //
    // It is conditional because the two ways in are genuinely different. The kernel
    // enters a process at `_start` with `rsp` already 16-aligned and **no return
    // address pushed**, so a 16-multiple frame is correct there — which is why every
    // ordinary program has always been fine. App mode is the other case: the worker
    // shim *calls* this entry, so it arrives at `rsp % 16 == 8` and inherits the
    // wrong parity for the whole program beneath it.
    //
    // The symptom that found this is worth recording, because it points nowhere near
    // here: an app-mode program on Linux x86-64 died in `__libc_calloc` under
    // `g_idle_add` on the first `app::setMode` — in Console mode as much as Canvas —
    // and the faulting instruction was `movaps %xmm0,(%rsp)`, an *aligned* store. It
    // reads as heap corruption and is a stack-parity bug six frames up.
    let call_parity = if entry_called_as_function {
        platform.backend().frame_call_padding()
    } else {
        0
    };
    instructions.extend([
        abi::subtract_stack(entry_stack_size + shadow + call_parity),
        abi::add_immediate(ARENA_STATE_REGISTER, abi::stack_pointer(), shadow),
        // Zero the whole arena state with a loop (allocator-04): the entry
        // frame is live stack, NOT zero-filled, and this initializer must stay
        // in lockstep with the thread-spawn child-state zeroing
        // (`runtime_helpers.rs` `lower_thread_start_helper`) — both zero
        // exactly `ARENA_STATE_SIZE`, so growing the state (e.g. quick bins)
        // can never leave a field as garbage in one path but not the other.
        // `SCRATCH[0]`/`SCRATCH[1]` are free scratch here; `ARG[0]`/`ARG[1]`
        // (argc/argv) are live.
        abi::move_register(abi::SCRATCH[0], ARENA_STATE_REGISTER),
        abi::add_immediate(abi::SCRATCH[1], ARENA_STATE_REGISTER, ARENA_STATE_SIZE),
        abi::label("entry_arena_state_zero"),
        abi::store_u64(abi::ZERO, abi::SCRATCH[0], 0),
        abi::add_immediate(abi::SCRATCH[0], abi::SCRATCH[0], 8),
        abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]),
        abi::branch_lo("entry_arena_state_zero"),
    ]);
    for index in 0..global_slot_count {
        instructions.push(abi::store_u64(
            abi::ZERO,
            ARENA_STATE_REGISTER,
            ENTRY_GLOBALS_OFFSET + index * 8,
        ));
    }
    let error_label = "entry_error";
    let exit_label = "entry_exit";
    // Publish this thread's arena-state address to the writable global so the
    // signal handler and `_mfb_shutdown` can find the arena without `x19`. `x9`
    // is a scratch temporary here; `x0`/`x1` (argc/argv) are left untouched.
    push_symbol_address(
        entry_symbol,
        MAIN_ARENA_GLOBAL_SYMBOL,
        abi::SCRATCH[0],
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::store_u64(ARENA_STATE_REGISTER, abi::SCRATCH[0], 0));
    // plan-67-B: start runtime perf tracking. `perf_init` mmaps its own system
    // region (arena-free) and stores the base in `_mfb_rt_perf_state`; the matching
    // `perf_done` prints the table at exit. macOS-only and `--cfg perf`-only: a
    // perf-free compiler (`perf_injection_enabled()` false) and the Linux/Windows
    // entry paths emit nothing here, staying byte-identical. The call clobbers x0–x17, but
    // argc/argv are already parked in the callee-saved SCRATCH[17]/SCRATCH[18] (or
    // irrelevant for a non-arg entry) and are re-materialized by the blocks below,
    // and `perf_init` preserves the callee-saved arena register. The symbol is
    // derived, never hard-coded — `symbol_for_call` doubles the family into the name
    // (`_mfb_rt_perf_perf_init`), matching the emitted body.
    if perf_injection_enabled() && platform.family() == PlatformFamily::MacOS {
        let perf_init = crate::target::shared::runtime::symbol_for_call(
            crate::target::shared::runtime::RuntimeHelper::Perf,
            "perf.init",
        );
        instructions.push(abi::branch_link(&perf_init));
        relocations.push(internal_branch(entry_symbol, &perf_init));
        // plan-67-C: open the whole-program span immediately after the region is
        // live. Load the "program" name object's address into the arg register and
        // call perf_start; the matching perf_end("program") is injected before
        // perf_done in plan-67-D. Both `bl`s clobber x0–x17, but argc/argv are in
        // the callee-saved SCRATCH[17]/SCRATCH[18] and are re-materialized below.
        let perf_start = crate::target::shared::runtime::symbol_for_call(
            crate::target::shared::runtime::RuntimeHelper::Perf,
            "perf.start",
        );
        push_symbol_address(
            entry_symbol,
            PERF_NAME_PROGRAM_SYMBOL,
            abi::c_arg(0),
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::branch_link(&perf_start));
        relocations.push(internal_branch(entry_symbol, &perf_start));
    }
    // Deferred (Windows) `os::args` capture: now that the arena is mapped and
    // `ARENA_STATE_REGISTER` is pinned, build the UTF-8 argv and store it into the
    // os::args globals (plan-66-B). `emit_build_argv_utf8` leaves argc in `ARG[0]`
    // and argv in `ARG[1]`; the stores mirror the pre-arena path exactly, so a
    // program that never calls `os::args` (capture_args=false) keeps a
    // byte-identical entry.
    if platform.defers_arg_capture() && (capture_args || language_entry_accepts_args) {
        platform.emit_build_argv_utf8(
            entry_symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        if language_entry_accepts_args {
            instructions.extend([
                abi::move_register(abi::SCRATCH[17], abi::c_arg(0)),
                abi::move_register(abi::SCRATCH[18], abi::c_arg(1)),
            ]);
        }
        if capture_args {
            push_symbol_address(
                entry_symbol,
                crate::codegen::builtins::os::OS_ARGC_GLOBAL_SYMBOL,
                abi::SCRATCH[0],
                &mut instructions,
                &mut relocations,
            );
            instructions.push(abi::store_u64(abi::c_arg(0), abi::SCRATCH[0], 0));
            push_symbol_address(
                entry_symbol,
                crate::codegen::builtins::os::OS_ARGV_GLOBAL_SYMBOL,
                abi::SCRATCH[0],
                &mut instructions,
                &mut relocations,
            );
            instructions.push(abi::store_u64(abi::c_arg(1), abi::SCRATCH[0], 0));
        }
    }
    // Install SIGINT/SIGTERM handlers (console programs) and, on every POSIX
    // target, take SIGPIPE out of the picture (bug-467). `signal()` clobbers
    // `x0`/`x1`, so argc/argv are parked below the frame across the calls; `x19`
    // pins the entry frame, so temporarily lowering `sp` is safe.
    //
    // bug-467: SIGPIPE's default disposition TERMINATES the process, and POSIX
    // delivers it to any `write` to a socket whose peer has sent an RST. That
    // made a remote peer able to end an MFBASIC server by connecting and
    // disconnecting -- the second `tcp::write` after the close killed the process
    // before `write` could return `EPIPE`, so no `TRAP` and no scope drop ever
    // ran. A per-call `MSG_NOSIGNAL` / per-socket `SO_NOSIGPIPE` fixes only the
    // writes this compiler emits itself; the Linux TLS backend writes inside
    // libssl (`SSL_write` -> the socket BIO's plain `write(2)`), which no call
    // site can reach, and that path was measured to die the same way. The
    // process-wide disposition is the only mechanism that covers it, so it is
    // installed here, unconditionally, for every program on every POSIX target --
    // app mode included, since an app-mode program owns sockets too.
    //
    // The one behaviour this must NOT change is `prog | head`: a CLI whose stdout
    // pipe closes is *supposed* to die. The `io::` stdout write path therefore
    // classifies its own `EPIPE` and restores `SIG_DFL` before re-raising (see
    // `emit_sigpipe_restore_and_raise` in `codegen::os::syscall`), so the pipeline
    // convention survives while sockets stop being lethal.
    //
    // Windows has no SIGPIPE (and this build links only kernel32, no CRT), so the
    // whole block stays off there and its entry is unchanged.
    let ignore_sigpipe = platform.family() != PlatformFamily::Windows;
    if register_signal_handlers || ignore_sigpipe {
        instructions.extend([
            abi::subtract_stack(16),
            abi::store_u64(abi::c_arg(0), abi::stack_pointer(), 0),
            abi::store_u64(abi::c_arg(1), abi::stack_pointer(), 8),
        ]);
        if register_signal_handlers {
            for signo in ["2", "15"] {
                instructions.push(abi::move_immediate(abi::c_arg(0), "Integer", signo));
                push_symbol_address(
                    entry_symbol,
                    SIGNAL_HANDLER_SYMBOL,
                    abi::c_arg(1),
                    &mut instructions,
                    &mut relocations,
                );
                platform.emit_external_call(
                    "signal",
                    entry_symbol,
                    platform_imports,
                    &mut instructions,
                    &mut relocations,
                )?;
            }
        }
        if ignore_sigpipe {
            // signal(SIGPIPE, SIG_IGN). `SIG_IGN` is the constant `1` cast to a
            // handler pointer on every POSIX system; it must be materialized as an
            // immediate rather than moved from `abi::ZERO`-style operands because
            // x86-64 has no hardware zero register (`.ai/arch-abi.md`).
            instructions.extend([
                abi::move_immediate(abi::c_arg(0), "Integer", SIGPIPE_SIGNO),
                abi::move_immediate(abi::c_arg(1), "Integer", SIG_IGN),
            ]);
            platform.emit_external_call(
                "signal",
                entry_symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
        }
        instructions.extend([
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), 0),
            abi::load_u64(abi::c_arg(1), abi::stack_pointer(), 8),
            abi::add_stack(16),
        ]);
    }
    // Seed this thread's PCG64 generator from the OS entropy pool before any
    // user code (including global initializers, which may call `math::rand`).
    // The seed scratch lives in the as-yet-unused args slot; pre-fill it with
    // the arena address so a `getentropy` failure still yields a varying seed.
    //
    // bug-512: the scratch is `ENTRY_SEED_SCRATCH_OFFSET` past the ARENA, and the
    // arena sits at `sp + shadow` — so the sp-relative slot is `shadow` further up
    // too. Addressing it as a bare `sp + ENTRY_SEED_SCRATCH_OFFSET` put it at
    // `arena + ARENA_STATE_SIZE - 32` on Win64, which is `ARENA_STDIN_LOCAL_BUF_OFFSET`:
    // the eight CSPRNG bytes became the "already allocated" stdin buffer pointer
    // that `_mfb_rt_stdin_next_byte` copies stdin bytes through. `shadow == 0` on
    // Linux/macOS, so those entries are byte-identical either way; the invariant
    // is pinned by `tests/codegen_win64_entry_seed_scratch.rs`.
    let seed_scratch_sp_offset = ENTRY_SEED_SCRATCH_OFFSET + shadow;
    // argc/argv were parked in SCRATCH[17]/SCRATCH[18] at the top of the entry:
    // everything here — the seed_rng `getentropy`/`RNG_SEED` calls, the always-on
    // fill block's `clock_gettime`/`getentropy`/`ARENA_FILL_SEED`, and the later
    // `LINK`/global-initializer calls — clobbers ARG[0]/ARG[1], so the args region
    // is read back from those callee-saved registers below.
    if seed_rng {
        instructions.extend([
            abi::store_u64(
                ARENA_STATE_REGISTER,
                abi::stack_pointer(),
                seed_scratch_sp_offset,
            ),
            abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), seed_scratch_sp_offset),
            abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        ]);
        platform.emit_random_bytes(
            entry_symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::load_u64(abi::c_arg(1), abi::stack_pointer(), seed_scratch_sp_offset),
            abi::move_register(abi::c_arg(0), ARENA_STATE_REGISTER),
            abi::branch_link(RNG_SEED_SYMBOL),
        ]);
        relocations.push(internal_branch(entry_symbol, RNG_SEED_SYMBOL));
    }
    // plan-62-B §3.3: seed the per-arena presentation-mode word to `None` (`1`)
    // when that is the program's static default (it references `app::setMode`).
    // `Console`-default programs pass `None` here and rely on the zero-init region,
    // so their entry is byte-identical. ARG[0] is free scratch at this point (the
    // seed_rng block above and the LINK/global-init calls below both treat it as
    // clobbered), and ARENA_STATE_REGISTER is already pinned.
    if let Some(offset) = seed_presentation_mode_offset {
        instructions.push(abi::move_immediate(abi::c_arg(0), "Integer", "1"));
        instructions.push(abi::store_u64(abi::c_arg(0), ARENA_STATE_REGISTER, offset));
    }
    // Capture the arena start time (offset 40) and seed the dedicated memory-fill
    // RNG (offsets 16/24). Always on — entropy fill is a requirement (plan-01 §6),
    // so this runs for every program before the first allocation. The seed is OS
    // entropy XORed with the arena address and start time, so a `getentropy`
    // failure or two arenas seeding in the same instant still yield distinct
    // poison streams. This is a separate stream from `math::rand` (offsets 88/96),
    // so it never perturbs the reproducible language RNG.
    //
    // `argc`/`argv` (x0/x1) may still be live here (for a non-arg entry they are
    // irrelevant), and this block clobbers x0–x16, so park them in callee-saved
    // x27/x28 — preserved by the libc calls and the fill helpers — and restore
    // them afterward. A local 16-byte stack buffer holds first the `timespec`
    // and then the entropy bytes, so no entry-stack slot is touched. An
    // arg-accepting entry has already parked argc/argv into x27/x28 above (before
    // the seed_rng block clobbered x0/x1), so it must NOT re-park garbage here.
    if !language_entry_accepts_args {
        instructions.extend([
            abi::move_register(abi::SCRATCH[17], abi::c_arg(0)),
            abi::move_register(abi::SCRATCH[18], abi::c_arg(1)),
        ]);
    }
    // Capture the arena start time into ARENA_START_TIME_OFFSET, using a 16-byte
    // stack buffer at [sp] that the entropy block below reuses (freed by the
    // matching `add_stack(16)`). The clock source is platform-abstracted
    // (`clock_gettime` on POSIX; `GetSystemTimePreciseAsFileTime` on Windows,
    // plan-47-D §3.1) — `emit_arena_start_time`'s default reproduces the POSIX
    // sequence, so every existing target's entry is byte-identical.
    platform.emit_arena_start_time(
        entry_symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // Set the console text code page to UTF-8 so verbatim UTF-8 output decodes as
    // glyphs, not OEM-code-page mojibake (bug-392). No-op on every POSIX platform
    // (the default trait impl emits nothing), so their entry stays byte-identical;
    // Windows emits SetConsoleOutputCP/SetConsoleCP(65001) with a self-contained,
    // balanced frame. Placed after the arena is mapped: the call clobbers only the
    // caller-saved ARG registers, and any incoming argc/argv are already consumed
    // (POSIX captures pre-arena) or irrelevant (Windows receives none and rebuilds
    // argv later from GetCommandLineW), so overwriting ARG here is safe.
    platform.emit_console_utf8(
        entry_symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // Initialize the platform network stack before any initializer or the language
    // entry can issue a socket call (plan-47-I §3.2). No-op on POSIX; Windows emits
    // WSAStartup. Gated on `net.*` usage, so a socket-free program is byte-identical.
    if needs_winsock {
        platform.emit_net_startup(
            entry_symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    }
    instructions.extend([
        // Pre-fill the seed scratch with the arena address (getentropy fallback).
        abi::store_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), 0),
        abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), 0),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    platform.emit_random_bytes(
        entry_symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), 0), // entropy (or arena addr)
        abi::add_stack(16),
        abi::load_u64(
            abi::SCRATCH[0],
            ARENA_STATE_REGISTER,
            ARENA_START_TIME_OFFSET,
        ),
        abi::exclusive_or_registers(abi::c_arg(1), abi::c_arg(1), abi::SCRATCH[0]), // mix start time
        abi::exclusive_or_registers(abi::c_arg(1), abi::c_arg(1), ARENA_STATE_REGISTER), // mix arena address
        abi::move_register(abi::c_arg(0), ARENA_STATE_REGISTER),
        abi::branch_link(ARENA_FILL_SEED_SYMBOL),
        // Restore argc/argv for the arg-materialization path below.
        abi::move_register(abi::c_arg(0), abi::SCRATCH[17]),
        abi::move_register(abi::c_arg(1), abi::SCRATCH[18]),
    ]);
    relocations.push(internal_branch(entry_symbol, ARENA_FILL_SEED_SYMBOL));
    // Populate the static closure descriptors (bug-78): write each no-capture
    // function value's `code` word with `&func`. Runs once, before `main` and
    // before any thread is spawned, and cannot fail — no tag check.
    if let Some(symbol) = closure_init_symbol {
        instructions.push(abi::branch_link(symbol));
        relocations.push(CodeRelocation {
            from: entry_symbol.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
    }
    // Resolve native `LINK` bindings (dlopen/dlsym) before anything runs; a load
    // failure aborts before `main` through the standard error path
    // (plan-linker.md §12.1).
    if let Some(symbol) = link_init_symbol {
        instructions.push(abi::branch_link(symbol));
        relocations.push(CodeRelocation {
            from: entry_symbol.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        instructions.extend([
            abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
            abi::branch_ne(error_label),
        ]);
    }
    if let Some(symbol) = global_initializer_symbol {
        instructions.push(abi::branch_link(symbol));
        relocations.push(CodeRelocation {
            from: entry_symbol.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        instructions.extend([
            abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_PROGRAM_EXIT_TAG),
            abi::branch_ne("global_initializer_not_program_exit"),
            abi::move_register(abi::return_register(), RESULT_VALUE_REGISTER),
            abi::branch(exit_label),
            abi::label("global_initializer_not_program_exit"),
        ]);
        instructions.extend([
            abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
            abi::branch_ne(error_label),
        ]);
    }
    // plan-15 §4.5: subscribe the main thread to the stdin broadcast log at the
    // current frontier (fill == 0 here), so a single-threaded program reads stdin
    // with no `thread::openStdIn` call and stays byte-identical to a direct reader.
    // Runs after the global/link initializers (so the log's lazy setup is
    // single-threaded) and before the entry call; clobbers x0–x17, but the args
    // materialization below re-derives x0 from the frame. Workers subscribe
    // explicitly via `thread::openStdIn(worker)`.
    if subscribe_stdin {
        instructions.push(abi::move_register(abi::c_arg(0), ARENA_STATE_REGISTER));
        instructions.push(abi::branch_link(STDIN_SUBSCRIBE_SYMBOL));
        relocations.push(internal_branch(entry_symbol, STDIN_SUBSCRIBE_SYMBOL));
    }
    if language_entry_accepts_args {
        // The args region sits at the top of the entry frame (above the
        // globals); `entry_stack_size` includes ENTRY_ARGS_REGION_SIZE for an
        // arg-accepting entry (see the mod.rs sizing).
        let args_base = entry_stack_size - ENTRY_ARGS_REGION_SIZE + shadow;
        // Source argc/argv from the preserved callee-saved registers rather than
        // x0/x1: a `LINK` initializer or global initializer runs between here and
        // the top-of-entry parking, and those `bl`s clobber x0/x1 (but preserve
        // x27/x28).
        instructions.extend([
            abi::store_u64(abi::SCRATCH[17], abi::stack_pointer(), args_base),
            abi::store_u64(abi::SCRATCH[18], abi::stack_pointer(), args_base + 8),
        ]);
        emit_entry_args_list_materialization(
            entry_symbol,
            error_label,
            args_base,
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            args_base + 16,
        ));
    }
    instructions.push(abi::branch_link(language_entry_symbol));
    relocations.push(CodeRelocation {
        from: entry_symbol.to_string(),
        to: language_entry_symbol.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_PROGRAM_EXIT_TAG),
        abi::branch_ne("entry_not_program_exit"),
        abi::move_register(abi::return_register(), RESULT_VALUE_REGISTER),
        abi::branch(exit_label),
        abi::label("entry_not_program_exit"),
    ]);
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_ne(error_label),
    ]);
    if *language_entry_returns == ParameterType::Nothing {
        instructions.push(abi::move_immediate(abi::return_register(), "Integer", "0"));
    } else {
        instructions.push(abi::move_register(
            abi::return_register(),
            RESULT_VALUE_REGISTER,
        ));
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "255"),
            abi::branch_hi("entry_exit_range_error"),
        ]);
    }
    instructions.push(abi::branch(exit_label));
    if *language_entry_returns == ParameterType::Integer {
        instructions.push(abi::label("entry_exit_range_error"));
        raise_error_into(
            entry_symbol,
            "ErrOverflow",
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::branch(error_label));
    }
    instructions.extend([
        abi::label(error_label),
        abi::store_u64(RESULT_VALUE_REGISTER, ARENA_STATE_REGISTER, 32),
        abi::move_register(abi::SCRATCH[10], RESULT_ERROR_MESSAGE_REGISTER),
    ]);
    emit_write_string_object(
        &mut EmitCtx {
            symbol: entry_symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        ENTRY_ERROR_PREFIX_SYMBOL,
    )?;
    emit_write_integer_to_stderr(&mut EmitCtx {
        symbol: entry_symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    })?;
    // The untrapped-error banner is `Error:  <G-SSS-EEEE>\n<message>\n`, so the
    // code (printed above in canonical hyphenated form) is followed by a newline,
    // not the legacy ` Message: ` label. The cleanup-failure audit keeps its own
    // ` Message: ` separator (a distinct call below), so this only reformats the
    // program-ending untrapped-error output.
    emit_write_string_object(
        &mut EmitCtx {
            symbol: entry_symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        ENTRY_ERROR_NEWLINE_SYMBOL,
    )?;
    instructions.extend([
        abi::load_u64(abi::string_length_register(), abi::SCRATCH[10], 0),
        abi::add_immediate(abi::string_data_register(), abi::SCRATCH[10], 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(
        entry_symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_write_string_object(
        &mut EmitCtx {
            symbol: entry_symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        ENTRY_ERROR_NEWLINE_SYMBOL,
    )?;
    if emit_cleanup_failure_audit {
        emit_cleanup_failure_audit_report(&mut EmitCtx {
            symbol: entry_symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        })?;
    }
    instructions.push(abi::move_immediate(
        abi::return_register(),
        "Integer",
        "255",
    ));
    instructions.push(abi::label(exit_label));
    // Run the shared teardown (terminal restore + arena free), then exit. The
    // exit code is parked in the arena-state scratch slot across the call: that
    // slot lives in this stack-resident entry frame (not in the freed mmap
    // blocks), and `_mfb_shutdown` preserves `x19`, so it is valid on return.
    // `_mfb_shutdown` is internally gated and idempotent, so the SIGINT/SIGTERM
    // handler racing this path cannot double-free.
    instructions.push(abi::store_u64(
        abi::return_register(),
        ARENA_STATE_REGISTER,
        32,
    ));
    // Tear down the network stack (WSACleanup on Windows; no-op on POSIX) after the
    // exit code is safely parked in the arena scratch slot — WSACleanup clobbers the
    // return register but preserves the callee-saved arena register.
    if needs_winsock {
        platform.emit_net_shutdown(
            entry_symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    }
    instructions.push(abi::branch_link(SHUTDOWN_SYMBOL));
    relocations.push(internal_branch(entry_symbol, SHUTDOWN_SYMBOL));
    // plan-67-B: print the perf table as the very last thing before exit — after
    // `_mfb_shutdown` (terminal reset + arena free), before the exit code is
    // reloaded. `perf_done` is arena-free (it reads only `_mfb_rt_perf_state`), so
    // the arena having just been freed is irrelevant; it preserves the callee-saved
    // arena register and never touches the parked exit code at `[arena+32]` (which
    // lives in the stack-resident entry frame, not the freed mmap blocks). macOS +
    // `--cfg perf` only — perf-free / Linux / Windows emit nothing and stay byte-identical.
    if perf_injection_enabled() && platform.family() == PlatformFamily::MacOS {
        // plan-67-D: close the whole-program span (records one duration), then
        // print the table. perf_end reads the "program" name pointer in the arg
        // register; both `bl`s clobber x0–x17 but preserve the callee-saved arena
        // register and never touch the parked exit code at `[arena+32]`.
        let perf_end = crate::target::shared::runtime::symbol_for_call(
            crate::target::shared::runtime::RuntimeHelper::Perf,
            "perf.end",
        );
        push_symbol_address(
            entry_symbol,
            PERF_NAME_PROGRAM_SYMBOL,
            abi::c_arg(0),
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::branch_link(&perf_end));
        relocations.push(internal_branch(entry_symbol, &perf_end));
        let perf_done = crate::target::shared::runtime::symbol_for_call(
            crate::target::shared::runtime::RuntimeHelper::Perf,
            "perf.done",
        );
        instructions.push(abi::branch_link(&perf_done));
        relocations.push(internal_branch(entry_symbol, &perf_done));
    }
    instructions.push(abi::load_u64(
        abi::return_register(),
        ARENA_STATE_REGISTER,
        32,
    ));
    platform.emit_program_exit(entry_symbol, &mut instructions, &mut relocations)?;
    // plan-34-D: the entry stub is machine-floor shared lowering that bypasses
    // the allocator — its stream must still name no physical register (scratch
    // is the neutral `abi::SCRATCH` pool, realized during selection).
    if let Some(offense) = regalloc::find_physical_operand(&instructions) {
        return Err(format!(
            "entry-stub lowering violated the zero-physical-register invariant \
             (plan-34-D): {offense}"
        ));
    }
    Ok(CodeFunction {
        name: if entry_symbol == "_main" {
            "program.entry".to_string()
        } else {
            "program.entry.macapp".to_string()
        },
        symbol: entry_symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    })
}

/// plan-32-A: probe the ELF aux vector for the RISC-V "V" ISA bit and record it
/// (0/1) in the process-global `_mfb_rt_has_rvv` byte, once at program start. The
/// caller emits this only for the raw-ELF `linux-riscv64` entry, where the initial
/// process stack is intact at `sp`: `argc` at `[sp]`, then the `argv` vector + a
/// NULL, the `envp` vector + a NULL, then the `AT_*` key/value `u64` pairs
/// (terminated by `AT_NULL`=0). This walks to `AT_HWCAP` (key 16) and tests bit 21
/// (`V` = `1 << ('V' - 'A')`). Register-neutral (`abi::SCRATCH` temporaries, no
/// physical registers, all dead afterward) and touches only the initial stack +
/// the global, so it does not disturb the argc/argv the rest of the entry loads
/// from `[sp]`. A syscall-free scan — no libc `getauxval`.
fn emit_riscv_hwcap_probe(
    entry_symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    const AT_HWCAP: &str = "16"; // auxv key for the HWCAP bitmask
                                 // Four temporaries — kept to the caller-saved scratch pool so no callee-saved
                                 // register is disturbed at the top of the entry.
    let cursor = abi::SCRATCH[0]; // walking stack pointer, live across the scan
    let a = abi::SCRATCH[1]; // argc / auxv key / (later) the flag byte `has`
    let b = abi::SCRATCH[2]; // skip amount / auxv value / (later) the global address
    let one = abi::SCRATCH[3];
    instructions.extend([
        // cursor = sp; argc = [cursor]; step past the argc word to argv[0].
        abi::add_immediate(cursor, abi::stack_pointer(), 0),
        abi::load_u64(a, cursor, 0),
        abi::add_immediate(cursor, cursor, 8),
        // Skip the argv vector and its NULL terminator: cursor += (argc + 1) * 8.
        abi::add_immediate(a, a, 1),
        abi::shift_left_immediate(b, a, 3),
        abi::add_registers(cursor, cursor, b),
        // Scan the envp vector to its NULL terminator.
        abi::label("entry_hwcap_envp_loop"),
        abi::load_u64(b, cursor, 0),
        abi::compare_immediate(b, "0"),
        abi::branch_eq("entry_hwcap_envp_done"),
        abi::add_immediate(cursor, cursor, 8),
        abi::branch("entry_hwcap_envp_loop"),
        abi::label("entry_hwcap_envp_done"),
        // Step past the envp NULL → cursor at the first auxv pair.
        abi::add_immediate(cursor, cursor, 8),
        // Loop the auxv key/value pairs.
        abi::label("entry_hwcap_auxv_loop"),
        abi::load_u64(a, cursor, 0),    // key
        abi::compare_immediate(a, "0"), // AT_NULL → not found, leave the default 0
        abi::branch_eq("entry_hwcap_done"),
        abi::compare_immediate(a, AT_HWCAP),
        abi::branch_ne("entry_hwcap_auxv_next"),
        // AT_HWCAP found: has = (value >> 21) & 1. The key in `a` is dead after the
        // compare, so `has` reuses it; `b`/`one` are dead after the `and`.
        abi::load_u64(b, cursor, 8),
        abi::shift_right_immediate(b, b, 21),
        abi::move_immediate(one, "Integer", "1"),
        abi::and_registers(a, b, one),
    ]);
    // Store the flag byte into `_mfb_rt_has_rvv` (its address goes in `b`).
    push_symbol_address(
        entry_symbol,
        HAS_RVV_GLOBAL_SYMBOL,
        b,
        instructions,
        relocations,
    );
    instructions.extend([
        abi::store_u8(a, b, 0),
        abi::branch("entry_hwcap_done"),
        abi::label("entry_hwcap_auxv_next"),
        abi::add_immediate(cursor, cursor, 16),
        abi::branch("entry_hwcap_auxv_loop"),
        abi::label("entry_hwcap_done"),
    ]);
}

/// The POSIX arena-start-time capture (plan-47-D §3.1), extracted so a non-POSIX
/// OS can override it. `clock_gettime(CLOCK_REALTIME)` into a freshly-allocated
/// 16-byte stack buffer at `[sp]`, reduced to nanoseconds and stored at
/// `ARENA_START_TIME_OFFSET`. The buffer is deliberately left allocated: the
/// entry's entropy block immediately below reuses it and frees it with the
/// matching `add_stack(16)`. This is `CodegenPlatform::emit_arena_start_time`'s
/// default, so every existing target's entry is byte-identical; Windows overrides
/// it (`GetSystemTimePreciseAsFileTime`).
pub(crate) fn emit_default_arena_start_time<
    P: crate::codegen::engine::types::CodegenPlatform + ?Sized,
>(
    platform: &P,
    entry_symbol: &str,
    platform_imports: &std::collections::HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    instructions.extend([
        abi::subtract_stack(16),
        abi::move_immediate(abi::c_arg(0), "Integer", "0"), // CLOCK_REALTIME
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), 0),
    ]);
    platform.emit_external_call(
        "clock_gettime",
        entry_symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), 0), // tv_sec
        abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), 8), // tv_nsec
        abi::move_immediate(abi::SCRATCH[2], "Integer", "1000000000"),
        abi::multiply_registers(abi::SCRATCH[0], abi::SCRATCH[0], abi::SCRATCH[2]),
        abi::add_registers(abi::SCRATCH[0], abi::SCRATCH[0], abi::SCRATCH[1]), // ns = sec*1e9 + nsec
        abi::store_u64(
            abi::SCRATCH[0],
            ARENA_STATE_REGISTER,
            ARENA_START_TIME_OFFSET,
        ),
    ]);
    Ok(())
}

fn emit_entry_args_list_materialization(
    entry_symbol: &str,
    error_label: &str,
    args_base: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.extend([
        abi::load_u64(abi::SCRATCH[10], abi::stack_pointer(), args_base),
        abi::load_u64(abi::SCRATCH[11], abi::stack_pointer(), args_base + 8),
        abi::move_immediate(abi::SCRATCH[12], "Integer", "0"),
        abi::move_immediate(abi::SCRATCH[13], "Integer", "0"),
        abi::label("entry_args_count_loop"),
        abi::compare_registers(abi::SCRATCH[13], abi::SCRATCH[10]),
        abi::branch_eq("entry_args_count_done"),
        abi::load_u64(abi::SCRATCH[14], abi::SCRATCH[11], 0),
        abi::move_register(abi::SCRATCH[15], abi::SCRATCH[14]),
        abi::move_immediate(abi::SCRATCH[16], "Integer", "0"),
        abi::label("entry_args_count_len_loop"),
        abi::load_u8(abi::SCRATCH[17], abi::SCRATCH[15], 0),
        abi::compare_immediate(abi::SCRATCH[17], "0"),
        abi::branch_eq("entry_args_count_len_done"),
        abi::add_immediate(abi::SCRATCH[16], abi::SCRATCH[16], 1),
        abi::add_immediate(abi::SCRATCH[15], abi::SCRATCH[15], 1),
        abi::branch("entry_args_count_len_loop"),
        abi::label("entry_args_count_len_done"),
        abi::add_registers(abi::SCRATCH[12], abi::SCRATCH[12], abi::SCRATCH[16]),
        abi::add_immediate(abi::SCRATCH[11], abi::SCRATCH[11], 8),
        abi::add_immediate(abi::SCRATCH[13], abi::SCRATCH[13], 1),
        abi::branch("entry_args_count_loop"),
        abi::label("entry_args_count_done"),
        abi::move_immediate(
            abi::SCRATCH[14],
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ),
        abi::multiply_registers(abi::SCRATCH[15], abi::SCRATCH[10], abi::SCRATCH[14]),
        abi::add_registers(abi::SCRATCH[15], abi::SCRATCH[15], abi::SCRATCH[12]),
        abi::store_u64(abi::SCRATCH[12], abi::stack_pointer(), args_base + 24),
        abi::store_u64(abi::SCRATCH[10], abi::stack_pointer(), args_base + 32),
        abi::add_immediate(
            abi::return_register(),
            abi::SCRATCH[15],
            COLLECTION_HEADER_SIZE,
        ),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(entry_symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq("entry_args_alloc_ok"),
    ]);
    raise_error_into(entry_symbol, "ErrOutOfMemory", instructions, relocations);
    // The fill phase below uses ONLY `x9`–`x17`: the x86 residual-scratch pool
    // has 11 distinct registers, so `map_scratch_register` wraps at `xN+11` —
    // `x9`/`x20` share rbx, `x10`/`x21` share rsi, and so on. Mixing the low
    // scratch with `x20`–`x28` in one live range is fine on AArch64 (all
    // distinct) but self-clobbering on x86 (the original fill loop's `x10` byte
    // load destroyed the `x21` argv cursor). `x9`–`x17` map to nine distinct
    // GPRs on both ISAs. Everything is reloaded from the entry-frame slots
    // after the allocation call, so nothing needs to survive it in a register.
    instructions.extend([
        abi::branch(error_label),
        abi::label("entry_args_alloc_ok"),
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), args_base + 16),
        abi::load_u64(abi::SCRATCH[7], abi::stack_pointer(), args_base + 24),
        abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), args_base + 32),
        abi::move_immediate(abi::SCRATCH[8], "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8(abi::SCRATCH[8], abi::mfb_return(1), COLLECTION_OFFSET_KIND),
        abi::move_immediate(abi::SCRATCH[8], "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(
            abi::SCRATCH[8],
            abi::mfb_return(1),
            COLLECTION_OFFSET_KEY_TYPE,
        ),
        abi::move_immediate(abi::SCRATCH[8], "Byte", &COLLECTION_TYPE_STRING.to_string()),
        abi::store_u8(
            abi::SCRATCH[8],
            abi::mfb_return(1),
            COLLECTION_OFFSET_VALUE_TYPE,
        ),
        abi::move_immediate(abi::SCRATCH[8], "Byte", "1"),
        abi::store_u8(
            abi::SCRATCH[8],
            abi::mfb_return(1),
            COLLECTION_OFFSET_FLAGS_VERSION,
        ),
        abi::store_u64(abi::SCRATCH[0], abi::mfb_return(1), COLLECTION_OFFSET_COUNT),
        abi::store_u64(
            abi::SCRATCH[0],
            abi::mfb_return(1),
            COLLECTION_OFFSET_CAPACITY,
        ),
        abi::store_u64(
            abi::SCRATCH[7],
            abi::mfb_return(1),
            COLLECTION_OFFSET_DATA_LENGTH,
        ),
        abi::store_u64(
            abi::SCRATCH[7],
            abi::mfb_return(1),
            COLLECTION_OFFSET_DATA_CAPACITY,
        ),
        // SCRATCH[2] = entry cursor, SCRATCH[3] = data write cursor (= entries end).
        abi::add_immediate(abi::SCRATCH[2], abi::mfb_return(1), COLLECTION_HEADER_SIZE),
        abi::move_immediate(
            abi::SCRATCH[8],
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ),
        abi::multiply_registers(abi::SCRATCH[3], abi::SCRATCH[0], abi::SCRATCH[8]),
        abi::add_registers(abi::SCRATCH[3], abi::SCRATCH[2], abi::SCRATCH[3]),
        // SCRATCH[4] = value-offset accumulator, SCRATCH[5] = index,
        // SCRATCH[1] = argv cursor.
        abi::move_immediate(abi::SCRATCH[4], "Integer", "0"),
        abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), args_base + 8),
        abi::move_immediate(abi::SCRATCH[5], "Integer", "0"),
        abi::label("entry_args_fill_loop"),
        abi::compare_registers(abi::SCRATCH[5], abi::SCRATCH[0]),
        abi::branch_eq("entry_args_fill_done"),
        abi::load_u64(abi::SCRATCH[6], abi::SCRATCH[1], 0), // SCRATCH[6] = argv[i] (NUL-terminated source)
        abi::move_immediate(
            abi::SCRATCH[8],
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ),
        abi::store_u8(
            abi::SCRATCH[8],
            abi::SCRATCH[2],
            COLLECTION_ENTRY_OFFSET_FLAGS,
        ),
        abi::store_u64(
            abi::ZERO,
            abi::SCRATCH[2],
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ),
        abi::store_u64(
            abi::ZERO,
            abi::SCRATCH[2],
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ),
        abi::store_u64(
            abi::SCRATCH[4],
            abi::SCRATCH[2],
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ),
        // Copy bytes until the NUL, counting the length in x16 as we go (one
        // pass replaces the original separate strlen + copy loops).
        abi::move_immediate(abi::SCRATCH[7], "Integer", "0"),
        abi::label("entry_args_copy_loop"),
        abi::load_u8(abi::SCRATCH[8], abi::SCRATCH[6], 0),
        abi::compare_immediate(abi::SCRATCH[8], "0"),
        abi::branch_eq("entry_args_copy_done"),
        abi::store_u8(abi::SCRATCH[8], abi::SCRATCH[3], 0),
        abi::add_immediate(abi::SCRATCH[6], abi::SCRATCH[6], 1),
        abi::add_immediate(abi::SCRATCH[3], abi::SCRATCH[3], 1),
        abi::add_immediate(abi::SCRATCH[7], abi::SCRATCH[7], 1),
        abi::branch("entry_args_copy_loop"),
        abi::label("entry_args_copy_done"),
        abi::store_u64(
            abi::SCRATCH[7],
            abi::SCRATCH[2],
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ),
        abi::add_registers(abi::SCRATCH[4], abi::SCRATCH[4], abi::SCRATCH[7]),
        abi::add_immediate(abi::SCRATCH[2], abi::SCRATCH[2], COLLECTION_ENTRY_SIZE),
        abi::add_immediate(abi::SCRATCH[1], abi::SCRATCH[1], 8),
        abi::add_immediate(abi::SCRATCH[5], abi::SCRATCH[5], 1),
        abi::branch("entry_args_fill_loop"),
        abi::label("entry_args_fill_done"),
    ]);
}

fn emit_cleanup_failure_audit_report(ctx: &mut EmitCtx) -> Result<(), String> {
    // `from` is gone: it named the emitting symbol, which is `ctx.symbol`.
    let from = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let done = "entry_cleanup_failure_audit_done";
    ctx.instructions.extend([
        abi::load_u64(
            abi::SCRATCH[0],
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_COUNT_OFFSET,
        ),
        abi::compare_immediate(abi::SCRATCH[0], "0"),
        abi::branch_eq(done),
    ]);
    emit_write_string_object(
        &mut EmitCtx {
            symbol: from,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        CLEANUP_FAILURE_PREFIX_SYMBOL,
    )?;
    ctx.instructions.push(abi::load_u64(
        abi::SCRATCH[0],
        ARENA_STATE_REGISTER,
        ARENA_CLEANUP_FAILURE_CODE_OFFSET,
    ));
    ctx.instructions
        .push(abi::store_u64(abi::SCRATCH[0], ARENA_STATE_REGISTER, 32));
    emit_write_integer_to_stderr_with_labels(
        &mut EmitCtx {
            symbol: from,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "entry_cleanup_failure_code",
        true,
    )?;
    // Same banner shape as the untrapped-error path: `Cleanup failure: <code>\n`
    // with the code in canonical hyphenated form, then the message on its own
    // line.
    emit_write_string_object(
        &mut EmitCtx {
            symbol: from,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        ENTRY_ERROR_NEWLINE_SYMBOL,
    )?;
    ctx.instructions.extend([
        abi::load_u64(
            abi::SCRATCH[10],
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET,
        ),
        abi::load_u64(abi::string_length_register(), abi::SCRATCH[10], 0),
        abi::add_immediate(abi::string_data_register(), abi::SCRATCH[10], 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(from, platform_imports, ctx.instructions, ctx.relocations)?;
    emit_write_string_object(
        &mut EmitCtx {
            symbol: from,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        ENTRY_ERROR_NEWLINE_SYMBOL,
    )?;
    ctx.instructions.push(abi::label(done));
    Ok(())
}

fn emit_write_string_object(ctx: &mut EmitCtx, data_symbol: &str) -> Result<(), String> {
    // `ctx.symbol` is the emitting symbol (each relocation's `from`);
    // `data_symbol` is the string object being addressed (its `to`).
    let from = ctx.symbol;
    let symbol = data_symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    ctx.instructions.extend([
        abi::load_page_address(abi::SCRATCH[11], symbol),
        abi::add_page_offset(abi::SCRATCH[11], abi::SCRATCH[11], symbol),
        abi::load_u64(abi::string_length_register(), abi::SCRATCH[11], 0),
        abi::add_immediate(abi::string_data_register(), abi::SCRATCH[11], 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    ctx.relocations.extend([
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
    platform.emit_write(from, platform_imports, ctx.instructions, ctx.relocations)
}

fn emit_write_integer_to_stderr(ctx: &mut EmitCtx) -> Result<(), String> {
    // `from` is gone: it named the emitting symbol, which is `ctx.symbol`.
    let from = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    emit_write_integer_to_stderr_with_labels(
        &mut EmitCtx {
            symbol: from,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "entry_error_code",
        true,
    )
}

fn emit_write_integer_to_stderr_with_labels(
    ctx: &mut EmitCtx,
    label_prefix: &str,
    hyphenate: bool,
) -> Result<(), String> {
    let from = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let absolute_ready_label = format!("{label_prefix}_absolute_ready");
    let digit_loop_label = format!("{label_prefix}_digit_loop");
    let digits_done_label = format!("{label_prefix}_digits_done");
    let write_label = format!("{label_prefix}_write");
    let hyphen_label = format!("{label_prefix}_hyphen");
    ctx.instructions.extend([
        abi::subtract_stack(64),
        abi::load_u64(abi::SCRATCH[11], ARENA_STATE_REGISTER, 32),
        // Record the value's original sign in x28 *before* negating: x21 is
        // overwritten (absolute value, then consumed by the digit loop), so the
        // minus test below must read this saved flag, not the value register
        // (bug-70: the test read x19/arena_base — always non-negative — so the
        // minus branch was dead). x28 is untouched by the digit loop.
        abi::move_immediate(abi::SCRATCH[18], "Integer", "0"),
        abi::compare_immediate(abi::SCRATCH[11], "0"),
        abi::branch_ge(&absolute_ready_label),
        abi::move_immediate(abi::SCRATCH[12], "Integer", "0"),
        abi::subtract_registers(abi::SCRATCH[11], abi::SCRATCH[12], abi::SCRATCH[11]),
        abi::move_immediate(abi::SCRATCH[18], "Integer", "1"),
        abi::label(&absolute_ready_label),
        abi::add_immediate(abi::SCRATCH[13], abi::stack_pointer(), 64),
        abi::move_immediate(abi::SCRATCH[14], "Integer", "10"),
    ]);
    // Canonical error-code formatting (doc `diagnostics 02_error-codes.md`): the
    // untrapped-error banner prints the code in its hyphenated `G-SSS-EEEE` form,
    // so the digit loop keeps a running digit count and injects a `-` after the
    // 4th and 7th digit-from-the-right — but only while more digits remain, so an
    // arbitrary short `FAIL` code degrades to a plain number rather than gaining a
    // stray leading/trailing hyphen. The cleanup-failure audit passes
    // `hyphenate = false` and stays on the bare integer (byte-identical to the
    // pre-change lowering). The counter lives in SCRATCH[12] (AArch64 x22 / x86
    // rdi / riscv scratch): the loop body never touches that register and the
    // divide's implicit rax:rdx clobber does not reach it, so it persists across
    // every iteration. The hyphen store below reuses SCRATCH[16] (the just-freed
    // digit register) for the `-` byte so it does not disturb the counter.
    if hyphenate {
        ctx.instructions
            .push(abi::move_immediate(abi::SCRATCH[12], "Integer", "0"));
    }
    ctx.instructions.extend([
        abi::compare_immediate(abi::SCRATCH[11], "0"),
        abi::branch_ne(&digit_loop_label),
        abi::subtract_immediate(abi::SCRATCH[13], abi::SCRATCH[13], 1),
        abi::move_immediate(abi::SCRATCH[12], "Integer", "48"),
        abi::store_u8(abi::SCRATCH[12], abi::SCRATCH[13], 0),
        abi::branch(&digits_done_label),
        abi::label(&digit_loop_label),
        abi::unsigned_divide_registers(abi::SCRATCH[15], abi::SCRATCH[11], abi::SCRATCH[14]),
        abi::multiply_subtract_registers(
            abi::SCRATCH[16],
            abi::SCRATCH[15],
            abi::SCRATCH[14],
            abi::SCRATCH[11],
        ),
        abi::add_immediate(abi::SCRATCH[16], abi::SCRATCH[16], 48),
        abi::subtract_immediate(abi::SCRATCH[13], abi::SCRATCH[13], 1),
        abi::store_u8(abi::SCRATCH[16], abi::SCRATCH[13], 0),
        abi::move_register(abi::SCRATCH[11], abi::SCRATCH[15]),
    ]);
    if hyphenate {
        ctx.instructions.extend([
            abi::add_immediate(abi::SCRATCH[12], abi::SCRATCH[12], 1),
            abi::compare_immediate(abi::SCRATCH[11], "0"),
            abi::branch_eq(&digits_done_label),
            abi::compare_immediate(abi::SCRATCH[12], "4"),
            abi::branch_eq(&hyphen_label),
            abi::compare_immediate(abi::SCRATCH[12], "7"),
            abi::branch_eq(&hyphen_label),
            abi::branch(&digit_loop_label),
            abi::label(&hyphen_label),
            abi::subtract_immediate(abi::SCRATCH[13], abi::SCRATCH[13], 1),
            abi::move_immediate(abi::SCRATCH[16], "Integer", "45"),
            abi::store_u8(abi::SCRATCH[16], abi::SCRATCH[13], 0),
            abi::branch(&digit_loop_label),
            abi::label(&digits_done_label),
        ]);
    } else {
        ctx.instructions.extend([
            abi::compare_immediate(abi::SCRATCH[11], "0"),
            abi::branch_ne(&digit_loop_label),
            abi::label(&digits_done_label),
        ]);
    }
    ctx.instructions.extend([
        abi::compare_immediate(abi::SCRATCH[18], "0"),
        abi::branch_eq(&write_label),
        abi::subtract_immediate(abi::SCRATCH[13], abi::SCRATCH[13], 1),
        abi::move_immediate(abi::SCRATCH[12], "Integer", "45"),
        abi::store_u8(abi::SCRATCH[12], abi::SCRATCH[13], 0),
        abi::label(&write_label),
        abi::add_immediate(abi::SCRATCH[17], abi::stack_pointer(), 64),
        // plan-99: capture the buffer cursor as the data pointer BEFORE computing
        // the length. The length's destination and the cursor can be the SAME
        // machine register: `string_length_register()` is MFB argument 2, which the
        // Win64 bank realizes as `r8`, and `SCRATCH[13]` (`x23`) realizes as `r8`
        // too (`map_scratch_register`, whose plan-47-B note predicted exactly this
        // "low scratch overlaps the Win64 argument registers" hazard). Computing
        // the length first overwrote the cursor, so the write was handed the digit
        // COUNT as its buffer address: on box 2230 an uncaught runtime error printed
        // `Error: ` with an empty code line — the message line, which uses a
        // different staging path, was fine. Reading the cursor first makes the order
        // safe on every ABI; the length's `dst == rhs` case is already encoded
        // correctly (`alu3` negates and adds).
        abi::move_register(abi::string_data_register(), abi::SCRATCH[13]),
        abi::subtract_registers(
            abi::string_length_register(),
            abi::SCRATCH[17],
            abi::SCRATCH[13],
        ),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(from, platform_imports, ctx.instructions, ctx.relocations)?;
    ctx.instructions.push(abi::add_stack(64));
    Ok(())
}
