//! plan-176-U: process setup for the program entry — `/dev/null`, the standard-fd reservation,
//! the riscv hwcap probe and the arena start time. The program entry (`lower_program_entry`)
//! lives in `engine/function/entry/mod.rs`.

use super::*;

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
/// bug-902: the read-only `"/dev/null"` C string [`emit_reserve_standard_fds`] opens.
/// Emitted for every POSIX module with an entry.
pub(crate) const DEV_NULL_PATH_SYMBOL: &str = "_mfb_rt_dev_null_path";

/// The data object behind [`DEV_NULL_PATH_SYMBOL`].
pub(crate) fn dev_null_path_data_object() -> CodeDataObject {
    let text = "/dev/null";
    CodeDataObject {
        symbol: DEV_NULL_PATH_SYMBOL.to_string(),
        kind: "raw".to_string(),
        layout: "C string (NUL-terminated)".to_string(),
        align: 1,
        size: text.len() + 1,
        value: text
            .bytes()
            .chain(std::iter::once(0))
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    }
}

/// Writes the `/dev/null` path's address into the given register
/// ([`emit_reserve_standard_fds`]).
pub(crate) type AddressOfDevNull<'a> =
    dyn FnMut(&mut Vec<CodeInstruction>, &mut Vec<CodeRelocation>, abi::Reg) + 'a;

/// Emits a call to the libc function with the given base name
/// ([`emit_reserve_standard_fds`]).
pub(crate) type CallLibc<'a> =
    dyn FnMut(&mut Vec<CodeInstruction>, &mut Vec<CodeRelocation>, &str) -> Result<(), String> + 'a;

/// bug-902: reserve each standard descriptor (0, 1, 2) the process was started
/// without, by opening `/dev/null` onto it — `O_RDONLY` for 0, `O_WRONLY` for 1 and
/// 2 — as Rust std and Go do.
///
/// POSIX hands the lowest free number to the next `open`, so a program started with
/// `<&-` gave fd 0 to its first `fs::open`: `io::readLine` then read the program's own
/// file, and `io::print` (fd 1) / the error banners (fd 2) wrote into one it had
/// opened. With the numbers reserved, a closed stdin reads as end-of-input, a closed
/// stdout or stderr swallows writes, and no file the program opens can be 0, 1 or 2.
///
/// Checked in order 0, 1, 2 with `fcntl(fd, F_GETFD)`; a `-1` (`EBADF`) means closed.
/// Because every lower number is open by then, the `open` returns exactly `fd`. If
/// even `/dev/null` cannot be opened the process `abort`s rather than run with the
/// hazard, which is what Rust std does.
///
/// Shared by the console entry and the two app-mode bootstraps, which reach the
/// data symbol and their external calls differently: `address_of_dev_null` writes
/// the string's address into the given register, and `call` emits a call to the
/// libc function named by its base name (`fcntl`, `open`, `abort`). Clobbers the
/// caller-saved registers.
pub(crate) fn emit_reserve_standard_fds(
    label_prefix: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    address_of_dev_null: &mut AddressOfDevNull<'_>,
    call: &mut CallLibc<'_>,
) -> Result<(), String> {
    const F_GETFD: &str = "1";
    const O_RDONLY: &str = "0";
    const O_WRONLY: &str = "1";
    for (fd, flags) in [("0", O_RDONLY), ("1", O_WRONLY), ("2", O_WRONLY)] {
        let open = format!("{label_prefix}_std_fd_{fd}_open");
        instructions.extend([
            abi::move_immediate(abi::c_arg(0), "Integer", fd),
            abi::move_immediate(abi::c_arg(1), "Integer", F_GETFD),
        ]);
        call(instructions, relocations, "fcntl")?;
        // A C `int`: the upper half of the return register is unspecified.
        instructions.extend([
            abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_ge(&open),
        ]);
        address_of_dev_null(instructions, relocations, abi::c_arg(0));
        instructions.push(abi::move_immediate(abi::c_arg(1), "Integer", flags));
        call(instructions, relocations, "open")?;
        instructions.extend([
            abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_ge(&open),
        ]);
        call(instructions, relocations, "abort")?;
        instructions.push(abi::label(&open));
    }
    Ok(())
}

pub(super) fn emit_riscv_hwcap_probe(
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
