//! Unix native backend for the `process` package: fork + exec + three pipes +
//! waitpid/kill, emitted as self-contained runtime helpers. Every libc call goes
//! through `platform.emit_external_call` so the four Unix targets (macOS-aarch64,
//! Linux x86_64/aarch64/riscv64) share one body. Values live in numeric virtual
//! registers (`Vregs`; the shared allocator spills them across each `bl`); only
//! genuine memory a syscall fills — the `pipe(int[2])` arrays, the `waitpid`
//! status int, the `read` errno buffer, the built `argv` array — uses the
//! explicit `sp`-relative frame from `finalize_vreg_body_with_locals`.

// --- codegen tier imports (migration) ---
use super::gen_shared::*;
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::emit_fail;
use crate::codegen::os::syscall::{SIGPIPE_SIGNO, SIG_DFL};
use crate::target::shared::abi;
use std::collections::HashMap;
// POSIX constants identical across macOS and Linux for the ops here.
pub(crate) const SIGKILL: &str = "9";
pub(crate) const WNOHANG: &str = "1";

// Collection layout (a `List OF String`): count/capacity in the header, each
// element's bytes stored inline in the data region and addressed by the entry's
// (valueOffset, valueLength). Data region base = coll + HEADER + capacity*ENTRY.
pub(crate) const LIST_COUNT: usize = COLLECTION_OFFSET_COUNT;
pub(crate) const LIST_CAP: usize = COLLECTION_OFFSET_CAPACITY;
pub(crate) const LIST_HEADER: usize = COLLECTION_HEADER_SIZE;
pub(crate) const LIST_ENTRY: usize = COLLECTION_ENTRY_SIZE;
pub(crate) const ENTRY_VOFF: usize = COLLECTION_ENTRY_OFFSET_VALUE_OFFSET;
pub(crate) const ENTRY_VLEN: usize = COLLECTION_ENTRY_OFFSET_VALUE_LENGTH;

/// Decode a raw `waitpid` status word (`status` vreg) into an exit code (`exit`
/// vreg): `WIFEXITED` → `(status >> 8) & 0xff`, otherwise signal-death → `-1`.
/// `s0`/`s1` are caller-supplied scratch vregs distinct from `status`/`exit`.
pub(crate) fn emit_decode_status(
    status: &str,
    exit: &str,
    s0: &str,
    s1: &str,
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let signaled = format!("{symbol}_signaled");
    let decoded = format!("{symbol}_decoded");
    instructions.extend([
        // termsig = status & 0x7f
        abi::move_immediate(s0, "Integer", "127"),
        abi::and_registers(s0, status, s0),
        abi::compare_immediate(s0, "0"),
        abi::branch_ne(&signaled),
        // WIFEXITED: exit = (status >> 8) & 0xff
        abi::shift_right_immediate(exit, status, 8),
        abi::move_immediate(s1, "Integer", "255"),
        abi::and_registers(exit, exit, s1),
        abi::branch(&decoded),
        abi::label(&signaled),
        abi::bitwise_not(exit, abi::ZERO), // WIFSIGNALED → -1
        abi::label(&decoded),
    ]);
}
// ---------------------------------------------------------------------------
// process.__drop — scope-drop: SIGKILL + reap a live child, close pipe fds, set
// the closed bit. Idempotent (a closed record is a no-op).
// ---------------------------------------------------------------------------
pub(crate) fn lower_process_drop_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const STATUS_SLOT: usize = 0;
    let mut v = Vregs::new();
    let file = v.next();
    let closed = v.next();
    let reaped = v.next();
    let fd = v.next();
    let one = v.next();
    let done = format!("{symbol}_done");
    let done_ok = format!("{symbol}_done_ok");
    let already_reaped = format!("{symbol}_already_reaped");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&closed, "0"),
        abi::branch_ne(&done_ok),
        abi::load_u64(&reaped, &file, PROC_REAPED),
        abi::compare_immediate(&reaped, "0"),
        abi::branch_ne(&already_reaped),
        abi::load_u64(abi::c_arg(0), &file, RESOURCE_OFFSET_HANDLE),
        abi::move_immediate(abi::c_arg(1), "Integer", SIGKILL),
    ];
    let mut relocations = Vec::new();
    platform.emit_external_call(
        "kill",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::c_arg(0), &file, RESOURCE_OFFSET_HANDLE),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), STATUS_SLOT),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    platform.emit_external_call(
        "waitpid",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&already_reaped));
    // Close the three retained pipe fds if still open (>= 0).
    for off in [PROC_STDIN_W, PROC_STDOUT_R, PROC_STDERR_R] {
        let skip = format!("{symbol}_skip_{off}");
        instructions.extend([
            abi::load_u64(&fd, &file, off),
            abi::compare_immediate(&fd, "0"),
            abi::branch_lt(&skip),
            abi::move_register(abi::c_arg(0), &fd),
        ]);
        platform.emit_external_call(
            "close",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.push(abi::label(&skip));
    }
    // Hand back the two spill blocks `waitFor` may have grown (bug-475). Each can
    // reach SPILL_MAX_CAPACITY, so leaving them parked on the arena for the rest of
    // the program would be a real retention on a program that supervises several
    // chatty children. The handle is closed on this path, so nothing can read them
    // afterwards.
    for off in [PROC_STDOUT_BUF, PROC_STDERR_BUF] {
        let skip = format!("{symbol}_skip_spill_{off}");
        instructions.extend([
            abi::load_u64(&fd, &file, off),
            abi::compare_immediate(&fd, "0"),
            abi::branch_eq(&skip),
            abi::load_u64(&one, &fd, SPILL_CAPACITY),
            abi::add_immediate(abi::c_arg(1), &one, SPILL_DATA),
            abi::move_register(abi::return_register(), &fd),
        ]);
        emit_arena_free(symbol, &mut instructions, &mut relocations);
        instructions.extend([abi::store_u64(abi::ZERO, &file, off), abi::label(&skip)]);
    }
    instructions.extend([
        abi::move_immediate(&one, "Integer", "1"),
        abi::store_u64(&one, &file, RESOURCE_OFFSET_CLOSED),
        abi::label(&done_ok),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], 16);
    Ok((frame, instructions, relocations, stack_slots))
}

// ---------------------------------------------------------------------------
// process.spawn — argv-only. Builds a NUL-terminated C `argv` from a
// `List OF String`, creates three stdio pipes + a close-on-exec self-pipe,
// forks, and execvp's in the child. The child reports an exec failure to the
// parent over the self-pipe (the parent's read returns >0 bytes = errno);
// a successful exec closes the O_CLOEXEC self-pipe, so the parent reads EOF (0).
// ---------------------------------------------------------------------------
// Frame for a spawn/shell helper: three stdio pipes + a self-pipe (each an
// `int[2]`) + a 4-byte errno readback buffer.
pub(crate) const STDIN_P: usize = 0;
pub(crate) const STDOUT_P: usize = 8;
pub(crate) const STDERR_P: usize = 16;
pub(crate) const ERR_P: usize = 24;
pub(crate) const ERRBUF: usize = 32;
pub(crate) const SPAWN_LOCAL: usize = 48;

/// Copy `len` bytes from `src` into a freshly arena-allocated NUL-terminated C
/// string (allocated `len + 1`), returning it in `mfb_return(1)`. Leaves the
/// result also in the `out` vreg. Runs in the fork child, so the allocation is
/// never freed (the child execs or _exits). `cnt`/`byte`/`sp`/`dp` are scratch
/// vregs. `src`/`len` must survive `emit_alloc` (vregs spill).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_copy_to_cstring(
    symbol: &str,
    src: &str,
    len: &str,
    out: &str,
    sp: &str,
    dp: &str,
    cnt: &str,
    byte: &str,
    loop_label: &str,
    done_label: &str,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.extend([
        abi::add_immediate(abi::return_register(), len, 1),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::move_register(out, abi::mfb_return(1)),
        abi::move_register(sp, src),
        abi::move_register(dp, out),
        abi::move_immediate(cnt, "Integer", "0"),
        abi::label(loop_label),
        abi::compare_registers(cnt, len),
        abi::branch_eq(done_label),
        abi::load_u8(byte, sp, 0),
        abi::store_u8(byte, dp, 0),
        abi::add_immediate(sp, sp, 1),
        abi::add_immediate(dp, dp, 1),
        abi::add_immediate(cnt, cnt, 1),
        abi::branch(loop_label),
        abi::label(done_label),
        abi::store_u8(abi::ZERO, dp, 0),
    ]);
}

/// Apply an environment `Map OF String TO String` in the fork child before
/// `execvp`. When `envreplace` is nonzero, first clear the inherited environment
/// (portably: `unsetenv` each current `environ` entry's name — `unsetenv` exists
/// on macOS and Linux, unlike `clearenv`). Then `setenv(name, value, 1)` for each
/// USED map entry. All C strings are arena-allocated and never freed (the child
/// execs immediately).
///
/// The clear walks `environ` by INDEX (bug-500). The kernel does not validate
/// `envp`, so a launcher can hand the process an entry `unsetenv` cannot remove:
/// one with no `=` at all (the "name" is the whole string and matches nothing)
/// or one with a leading `=` (`unsetenv("")` fails `EINVAL`). Restarting from
/// `environ[0]` after every call would spin forever on such an entry — and, since
/// the fork child never frees, arena-allocate a name buffer per spin. Instead an
/// entry with nothing to unset is stepped over, and after `unsetenv` the index
/// only stays put when `environ[idx]` actually changed; every iteration therefore
/// either removes an entry or advances, so the loop (and the total name-buffer
/// allocation) is bounded by the length of `environ`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_child_apply_env(
    symbol: &str,
    v: &mut Vregs,
    map: &str,
    envreplace: &str,
    alloc_fail: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let map0 = v.next();
    let ep = v.next();
    let estr = v.next();
    let nlen = v.next();
    let sp = v.next();
    let dp = v.next();
    let cnt = v.next();
    let byte = v.next();
    let namebuf = v.next();
    let cap = v.next();
    let i = v.next();
    let entry = v.next();
    let dbase = v.next();
    let off = v.next();
    let klen = v.next();
    let vlen = v.next();
    let keyc = v.next();
    let valc = v.next();
    let flags = v.next();
    let idx = v.next();
    let epi = v.next();
    let cur = v.next();

    // Preserve the map pointer across the environ/unsetenv/setenv libc calls.
    instructions.push(abi::move_register(&map0, map));

    // --- optional clear ---
    let no_clear = format!("{symbol}_env_noclear");
    let clear_loop = format!("{symbol}_env_clear");
    let clear_done = format!("{symbol}_env_clear_done");
    let clear_skip = format!("{symbol}_env_clear_skip");
    let scan_loop = format!("{symbol}_env_scan");
    let scan_done = format!("{symbol}_env_scan_done");
    let name_copy = format!("{symbol}_env_name_copy");
    let name_copy_done = format!("{symbol}_env_name_copy_done");
    instructions.extend([
        abi::compare_immediate(envreplace, "0"),
        abi::branch_eq(&no_clear),
    ]);
    platform.emit_environ_pointer(symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::move_register(&ep, abi::return_register()),
        abi::move_immediate(&idx, "Integer", "0"),
        abi::label(&clear_loop),
        // estr = environ[idx]
        abi::move_immediate(&off, "Integer", "8"),
        abi::multiply_registers(&epi, &idx, &off),
        abi::add_registers(&epi, &ep, &epi),
        abi::load_u64(&estr, &epi, 0),
        abi::compare_immediate(&estr, "0"),
        abi::branch_eq(&clear_done),
        // nlen = index of '=' (or NUL) in estr
        abi::move_register(&sp, &estr),
        abi::move_immediate(&nlen, "Integer", "0"),
        abi::label(&scan_loop),
        abi::load_u8(&byte, &sp, 0),
        abi::compare_immediate(&byte, "61"), // '='
        abi::branch_eq(&scan_done),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&scan_done),
        abi::add_immediate(&sp, &sp, 1),
        abi::add_immediate(&nlen, &nlen, 1),
        abi::branch(&scan_loop),
        abi::label(&scan_done),
        // No '=' (scan ended at NUL): not a variable, nothing to unset — step over.
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&clear_skip),
        // Leading '=' (empty name): unsetenv("") is EINVAL and removes nothing.
        abi::compare_immediate(&nlen, "0"),
        abi::branch_eq(&clear_skip),
    ]);
    emit_copy_to_cstring(
        symbol,
        &estr,
        &nlen,
        &namebuf,
        &sp,
        &dp,
        &cnt,
        &byte,
        &name_copy,
        &name_copy_done,
        alloc_fail,
        instructions,
        relocations,
    );
    instructions.push(abi::move_register(abi::c_arg(0), &namebuf));
    platform.emit_external_call(
        "unsetenv",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    // unsetenv shifts the array down in place, so on success environ[idx] is now
    // the NEXT entry and idx stays put. Reload ep (the accessor may hand back a
    // fresh array — Darwin copies environ on first modification) and re-read
    // environ[idx]: if it is still the same string, unsetenv removed nothing and
    // the entry must be stepped over, or the loop would never terminate.
    platform.emit_environ_pointer(symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::move_register(&ep, abi::return_register()),
        abi::move_immediate(&off, "Integer", "8"),
        abi::multiply_registers(&epi, &idx, &off),
        abi::add_registers(&epi, &ep, &epi),
        abi::load_u64(&cur, &epi, 0),
        abi::compare_registers(&cur, &estr),
        abi::branch_ne(&clear_loop),
        abi::label(&clear_skip),
        abi::add_immediate(&idx, &idx, 1),
        abi::branch(&clear_loop),
        abi::label(&clear_done),
        abi::label(&no_clear),
    ]);

    // --- setenv each USED map entry ---
    let ent_loop = format!("{symbol}_env_ent");
    let ent_done = format!("{symbol}_env_ent_done");
    let ent_skip = format!("{symbol}_env_ent_skip");
    let key_copy = format!("{symbol}_env_key_copy");
    let key_copy_done = format!("{symbol}_env_key_copy_done");
    let val_copy = format!("{symbol}_env_val_copy");
    let val_copy_done = format!("{symbol}_env_val_copy_done");
    instructions.extend([
        abi::load_u64(&cap, &map0, LIST_CAP),
        // dbase = map + HEADER + cap*ENTRY
        abi::move_immediate(&off, "Integer", &LIST_ENTRY.to_string()),
        abi::multiply_registers(&dbase, &cap, &off),
        abi::add_immediate(&dbase, &dbase, LIST_HEADER),
        abi::add_registers(&dbase, &map0, &dbase),
        abi::move_immediate(&i, "Integer", "0"),
        abi::label(&ent_loop),
        abi::compare_registers(&i, &cap),
        abi::branch_eq(&ent_done),
        // entry = map + HEADER + i*ENTRY
        abi::move_immediate(&off, "Integer", &LIST_ENTRY.to_string()),
        abi::multiply_registers(&entry, &i, &off),
        abi::add_immediate(&entry, &entry, LIST_HEADER),
        abi::add_registers(&entry, &map0, &entry),
        // skip unused slots (tombstones)
        abi::load_u8(&flags, &entry, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::move_immediate(&byte, "Integer", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::and_registers(&flags, &flags, &byte),
        abi::compare_immediate(&flags, "0"),
        abi::branch_eq(&ent_skip),
        // key cstr
        abi::load_u64(&off, &entry, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::add_registers(&sp, &dbase, &off),
        abi::load_u64(&klen, &entry, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
    ]);
    emit_copy_to_cstring(
        symbol,
        &sp,
        &klen,
        &keyc,
        &dp,
        &namebuf,
        &cnt,
        &byte,
        &key_copy,
        &key_copy_done,
        alloc_fail,
        instructions,
        relocations,
    );
    instructions.extend([
        // value cstr — recompute entry (vregs survive, but reload offsets fresh)
        abi::move_immediate(&off, "Integer", &LIST_ENTRY.to_string()),
        abi::multiply_registers(&entry, &i, &off),
        abi::add_immediate(&entry, &entry, LIST_HEADER),
        abi::add_registers(&entry, &map0, &entry),
        abi::load_u64(&off, &entry, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::add_registers(&sp, &dbase, &off),
        abi::load_u64(&vlen, &entry, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
    ]);
    emit_copy_to_cstring(
        symbol,
        &sp,
        &vlen,
        &valc,
        &dp,
        &namebuf,
        &cnt,
        &byte,
        &val_copy,
        &val_copy_done,
        alloc_fail,
        instructions,
        relocations,
    );
    // setenv(key, value, 1)
    instructions.extend([
        abi::move_register(abi::c_arg(0), &keyc),
        abi::move_register(abi::c_arg(1), &valc),
        abi::move_immediate(abi::c_arg(2), "Integer", "1"),
    ]);
    platform.emit_external_call(
        "setenv",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::label(&ent_skip),
        abi::add_immediate(&i, &i, 1),
        abi::branch(&ent_loop),
        abi::label(&ent_done),
    ]);
    Ok(())
}

/// Shared spawn tail: given a fully-built NUL-terminated C `argv` (in the `argv`
/// vreg), create the three stdio pipes + an O_CLOEXEC self-pipe, fork, and
/// `execvp` in the child (reporting an exec failure to the parent over the
/// self-pipe), then in the parent allocate + stamp the `Process` record. Branches
/// to `fork_fail` on a pipe/fork failure, `alloc_fail` on OOM, `done` on success.
/// Emits its own `child`/`spawn_fail` labels; the caller emits the
/// `fork_fail`/`alloc_fail`/`done` labels. Draws its scratch vregs from the
/// caller's `Vregs` so nothing collides.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_spawn_tail(
    symbol: &str,
    v: &mut Vregs,
    argv: &str,
    // Optional child-side setup applied AFTER the dup2/close dance and BEFORE
    // `execvp`: a working-directory C-string ptr (skipped when its first byte is
    // NUL, i.e. an empty cwd) and an environment `(map vreg, envReplace vreg)`.
    cwd: Option<&str>,
    env: Option<(&str, &str)>,
    alloc_fail: &str,
    fork_fail: &str,
    done: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    const F_SETFD: &str = "2";
    const FD_CLOEXEC: &str = "1";
    let pid = v.next();
    let rec = v.next();
    let tmp = v.next();
    let errno = v.next();
    let child = format!("{symbol}_child");
    let spawn_fail = format!("{symbol}_spawn_fail");
    // Create the three stdio pipes and the self-pipe.
    for off in [STDIN_P, STDOUT_P, STDERR_P, ERR_P] {
        instructions.push(abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), off));
        platform.emit_external_call("pipe", symbol, platform_imports, instructions, relocations)?;
        instructions.extend([
            abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_lt(fork_fail),
        ]);
    }
    // Self-pipe write end O_CLOEXEC: closed automatically on a successful exec, so
    // the parent's read returns EOF; left open on exec failure to carry errno.
    instructions.extend([
        abi::load_u32(abi::c_arg(0), abi::stack_pointer(), ERR_P + 4),
        abi::move_immediate(abi::c_arg(1), "Integer", F_SETFD),
        abi::move_immediate(abi::c_arg(2), "Integer", FD_CLOEXEC),
    ]);
    platform.emit_variadic_external_call(
        "fcntl",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    // fork()
    platform.emit_external_call("fork", symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::sign_extend_word(&pid, abi::c_return(0)),
        abi::compare_immediate(&pid, "0"),
        abi::branch_eq(&child),
        abi::branch_lt(fork_fail),
    ]);
    // ---- parent ----
    for slot in [STDIN_P, STDOUT_P + 4, STDERR_P + 4, ERR_P + 4] {
        instructions.push(abi::load_u32(abi::c_arg(0), abi::stack_pointer(), slot));
        platform.emit_external_call(
            "close",
            symbol,
            platform_imports,
            instructions,
            relocations,
        )?;
    }
    instructions.extend([
        abi::load_u32(abi::c_arg(0), abi::stack_pointer(), ERR_P),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), ERRBUF),
        abi::move_immediate(abi::c_arg(2), "Integer", "4"),
    ]);
    platform.emit_external_call("read", symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_gt(&spawn_fail),
        abi::load_u32(abi::c_arg(0), abi::stack_pointer(), ERR_P),
    ]);
    platform.emit_external_call("close", symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::move_register(&rec, abi::mfb_return(1)),
        abi::move_immediate(&tmp, "Integer", RESOURCE_TAG_PROCESS),
        abi::store_u64(&tmp, &rec, RESOURCE_OFFSET_TAG),
        abi::store_u64(&pid, &rec, RESOURCE_OFFSET_HANDLE),
        abi::store_u64(abi::ZERO, &rec, RESOURCE_OFFSET_CLOSED),
        abi::store_u64(abi::ZERO, &rec, RESOURCE_OFFSET_STATE),
        abi::load_u32(&tmp, abi::stack_pointer(), STDIN_P + 4),
        abi::store_u64(&tmp, &rec, PROC_STDIN_W),
        abi::load_u32(&tmp, abi::stack_pointer(), STDOUT_P),
        abi::store_u64(&tmp, &rec, PROC_STDOUT_R),
        abi::load_u32(&tmp, abi::stack_pointer(), STDERR_P),
        abi::store_u64(&tmp, &rec, PROC_STDERR_R),
        abi::store_u64(abi::ZERO, &rec, PROC_REAPED),
        abi::store_u64(abi::ZERO, &rec, PROC_STATUS),
        abi::store_u64(abi::ZERO, &rec, PROC_EXITCODE),
        abi::store_u64(abi::ZERO, &rec, 80),
        abi::store_u64(abi::ZERO, &rec, 88),
        abi::move_register(RESULT_VALUE_REGISTER, &rec),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);
    // ---- child ----
    instructions.push(abi::label(&child));
    for (off, hi, target) in [
        (STDIN_P, false, "0"),
        (STDOUT_P, true, "1"),
        (STDERR_P, true, "2"),
    ] {
        let slot = if hi { off + 4 } else { off };
        instructions.extend([
            abi::load_u32(abi::c_arg(0), abi::stack_pointer(), slot),
            abi::move_immediate(abi::c_arg(1), "Integer", target),
        ]);
        platform.emit_external_call("dup2", symbol, platform_imports, instructions, relocations)?;
    }
    for slot in [
        STDIN_P,
        STDIN_P + 4,
        STDOUT_P,
        STDOUT_P + 4,
        STDERR_P,
        STDERR_P + 4,
        ERR_P,
    ] {
        instructions.push(abi::load_u32(abi::c_arg(0), abi::stack_pointer(), slot));
        platform.emit_external_call(
            "close",
            symbol,
            platform_imports,
            instructions,
            relocations,
        )?;
    }
    // Child-side working directory + environment, applied before execvp (safe in
    // the single-threaded fork child). A chdir/env failure is best-effort: the
    // subsequent execvp still runs and any real failure surfaces via the
    // self-pipe as ErrSpawnFailed.
    if let Some(cwd) = cwd {
        let skip_chdir = format!("{symbol}_skip_chdir");
        let byte = v.next();
        instructions.extend([
            abi::load_u8(&byte, cwd, 0),
            abi::compare_immediate(&byte, "0"),
            abi::branch_eq(&skip_chdir),
            abi::move_register(abi::c_arg(0), cwd),
        ]);
        platform.emit_external_call(
            "chdir",
            symbol,
            platform_imports,
            instructions,
            relocations,
        )?;
        instructions.push(abi::label(&skip_chdir));
    }
    if let Some((map, envreplace)) = env {
        emit_child_apply_env(
            symbol,
            v,
            map,
            envreplace,
            alloc_fail,
            platform,
            platform_imports,
            instructions,
            relocations,
        )?;
    }
    // bug-467: the parent installs a process-wide `signal(SIGPIPE, SIG_IGN)` so a
    // socket peer cannot kill it, and POSIX carries an IGNORED disposition across
    // `exec` (only *caught* signals are reset). Handing a spawned child an
    // inherited `SIG_IGN` would silently change that program's own behaviour --
    // `mfbprog` running `sh -c 'yes | head'` would leave `yes` running forever.
    // Restore the default in the child, after the fork and before the exec, where
    // it affects nobody else.
    instructions.extend([
        abi::move_immediate(abi::c_arg(0), "Integer", SIGPIPE_SIGNO),
        abi::move_immediate(abi::c_arg(1), "Integer", SIG_DFL),
    ]);
    platform.emit_external_call(
        "signal",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::c_arg(0), argv, 0),
        abi::move_register(abi::c_arg(1), argv),
    ]);
    platform.emit_external_call(
        "execvp",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    platform.emit_errno(
        symbol,
        errno.as_str().into(),
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::store_u32(&errno, abi::stack_pointer(), ERRBUF),
        abi::load_u32(abi::c_arg(0), abi::stack_pointer(), ERR_P + 4),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), ERRBUF),
        abi::move_immediate(abi::c_arg(2), "Integer", "4"),
    ]);
    platform.emit_external_call("write", symbol, platform_imports, instructions, relocations)?;
    instructions.push(abi::move_immediate(abi::c_arg(0), "Integer", "127"));
    platform.emit_external_call("_exit", symbol, platform_imports, instructions, relocations)?;
    // ---- exec-failure reap (no zombie) ----
    instructions.push(abi::label(&spawn_fail));
    instructions.extend([
        abi::move_register(abi::c_arg(0), &pid),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    platform.emit_external_call(
        "waitpid",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.push(abi::branch(fork_fail));
    Ok(())
}

/// Emit `store_u8` bytes materializing a NUL-terminated ASCII literal at
/// `dst + 0..`, using `byte` as a scratch vreg. The buffer must already be
/// allocated with room for `text.len() + 1` bytes.
pub(crate) fn emit_cstring_literal(
    text: &str,
    dst: &str,
    byte: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    for (offset, ch) in text.bytes().enumerate() {
        instructions.push(abi::move_immediate(byte, "Integer", &ch.to_string()));
        instructions.push(abi::store_u8(byte, dst, offset));
    }
    instructions.push(abi::store_u8(abi::ZERO, dst, text.len()));
}
// ---------------------------------------------------------------------------
// process.send / process.sendBytes — write to the child's stdin (the parent's
// write end). `send` writes the String bytes then a trailing '\n'; `sendBytes`
// writes the raw List OF Byte with no newline. Blocking (partial-write loop with
// EINTR retry); a broken pipe (child stdin gone) raises ErrResourceClosed.
// ---------------------------------------------------------------------------
pub(crate) fn lower_process_send_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    is_bytes: bool,
    with_timeout: bool,
) -> Result<ProcBodyParts, String> {
    const NL_SLOT: usize = 0;
    const POLLFD_SLOT: usize = 8; // { int fd; short events; short revents }
    const EINTR: &str = "4";
    let mut v = Vregs::new();
    let file = v.next();
    let val = v.next();
    let fd = v.next();
    let srcp = v.next();
    let rem = v.next();
    let n = v.next();
    let errno = v.next();
    let timeout = v.next();
    let s0 = v.next();
    let s1 = v.next();
    let s2 = v.next();
    let closed_l = format!("{symbol}_closed");
    let write_loop = format!("{symbol}_write_loop");
    let write_fail = format!("{symbol}_write_fail");
    let payload_done = format!("{symbol}_payload_done");
    let nl_loop = format!("{symbol}_nl_loop");
    let nl_fail = format!("{symbol}_nl_fail");
    let nl_done = format!("{symbol}_nl_done");
    let timeout_l = format!("{symbol}_timeout");
    let done = format!("{symbol}_done");

    let mut instructions = vec![
        abi::move_register(&file, abi::return_register()),
        abi::move_register(&val, abi::c_arg(1)),
    ];
    if with_timeout {
        instructions.push(abi::move_register(&timeout, abi::c_arg(2)));
    }
    instructions.extend([
        abi::load_u64(&n, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&n, "0"),
        abi::branch_ne(&closed_l),
        abi::load_u64(&fd, &file, PROC_STDIN_W),
        abi::compare_immediate(&fd, "0"),
        abi::branch_lt(&closed_l),
    ]);
    let mut relocations = Vec::new();
    if is_bytes {
        instructions.push(abi::load_u64(&rem, &val, COLLECTION_OFFSET_COUNT));
        push_collection_data_base_from_capacity(&mut instructions, &srcp, &val, &s0, &s1, &s2);
    } else {
        instructions.extend([
            abi::load_u64(&rem, &val, 0),
            abi::add_immediate(&srcp, &val, 8),
        ]);
    }
    instructions.extend([
        abi::label(&write_loop),
        abi::compare_immediate(&rem, "0"),
        abi::branch_eq(&payload_done),
    ]);
    if with_timeout {
        emit_poll_wait(
            symbol,
            &fd,
            &timeout,
            &n,
            POLLFD_SLOT,
            POLLOUT,
            &timeout_l,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    }
    instructions.extend([
        abi::move_register(abi::c_arg(0), &fd),
        abi::move_register(abi::c_arg(1), &srcp),
        abi::move_register(abi::c_arg(2), &rem),
    ]);
    platform.emit_external_call(
        "write",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(&n, abi::c_return(0)),
        abi::compare_immediate(&n, "0"),
        abi::branch_le(&write_fail),
        abi::add_registers(&srcp, &srcp, &n),
        abi::subtract_registers(&rem, &rem, &n),
        abi::branch(&write_loop),
        abi::label(&write_fail),
    ]);
    platform.emit_errno(
        symbol,
        errno.as_str().into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&errno, EINTR),
        abi::branch_eq(&write_loop),
        abi::branch(&closed_l),
        abi::label(&payload_done),
    ]);
    if !is_bytes {
        instructions.extend([
            abi::move_immediate(&n, "Integer", "10"),
            abi::store_u8(&n, abi::stack_pointer(), NL_SLOT),
            abi::label(&nl_loop),
        ]);
        if with_timeout {
            emit_poll_wait(
                symbol,
                &fd,
                &timeout,
                &n,
                POLLFD_SLOT,
                POLLOUT,
                &timeout_l,
                platform,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
        }
        instructions.extend([
            abi::move_register(abi::c_arg(0), &fd),
            abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), NL_SLOT),
            abi::move_immediate(abi::c_arg(2), "Integer", "1"),
        ]);
        platform.emit_external_call(
            "write",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::sign_extend_word(&n, abi::c_return(0)),
            abi::compare_immediate(&n, "0"),
            abi::branch_gt(&nl_done),
            abi::label(&nl_fail),
        ]);
        platform.emit_errno(
            symbol,
            errno.as_str().into(),
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(&errno, EINTR),
            abi::branch_eq(&nl_loop),
            abi::branch(&closed_l),
            abi::label(&nl_done),
        ]);
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed_l),
    ]);
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    if with_timeout {
        instructions.push(abi::label(&timeout_l));
        emit_fail(
            symbol,
            "ErrTimeout",
            &mut instructions,
            &mut relocations,
            &done,
        );
    }
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, 32))
}

pub(crate) const POLLOUT: &str = "4";

/// Emit a `poll(&{fd, events, 0}, 1, timeout)` on the pollfd staged at
/// `sp + pollfd_slot`; branch to `timeout_l` on a `0` (timed-out) return. `events`
/// is `POLLIN`/`POLLOUT`. `scratch` is a caller vreg for the sign-extended return.
/// A `< 0` poll error (e.g. EINTR) falls through and the following blocking op
/// re-polls — acceptable since a spurious wakeup just retries.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_poll_wait(
    symbol: &str,
    fd: &str,
    timeout: &str,
    scratch: &str,
    pollfd_slot: usize,
    events: &str,
    timeout_l: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    instructions.extend([
        abi::store_u32(fd, abi::stack_pointer(), pollfd_slot),
        abi::move_immediate(scratch, "Integer", events),
        abi::store_u16(scratch, abi::stack_pointer(), pollfd_slot + 4),
        abi::store_u16(abi::ZERO, abi::stack_pointer(), pollfd_slot + 6),
        abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), pollfd_slot),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::move_register(abi::c_arg(2), timeout),
    ]);
    platform.emit_external_call("poll", symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::sign_extend_word(scratch, abi::c_return(0)),
        abi::compare_immediate(scratch, "0"),
        abi::branch_eq(timeout_l),
    ]);
    Ok(())
}

// ---------------------------------------------------------------------------
// `_mfb_rt_process_reaper` (bug-474) — the pthread start routine `process::detach`
// spawns to reap exactly ONE detached child.
// ---------------------------------------------------------------------------

/// `void *_mfb_rt_process_reaper(void *pid)` — `waitpid(pid, NULL, 0); return NULL;`.
///
/// `detach` used to arrange the child's cleanup by flipping the **process-wide**
/// `SIGCHLD` disposition to `SIG_IGN`, which tells the kernel to auto-reap *every*
/// child of the program. Any later `waitpid` then failed with `ECHILD`, and
/// `process::waitFor` reads `ECHILD` as "already reaped" and returns the handle's
/// cached exit code — `0` for a child nobody had waited on. One `detach` therefore
/// silently zeroed the exit status of every other child (bug-474). Reaping on a
/// per-pid thread keeps the disposition untouched, so `waitFor` on a handle that was
/// never detached still reports the child's real status.
///
/// The child pid arrives **by value** in the C first-argument register, never a
/// pointer to the `Process` record: the record's arena block may be reclaimed at the
/// detaching scope's exit while this thread is still blocked in `waitpid`.
///
/// **Callee-saved registers.** pthread IS the caller of a start routine and keeps its
/// own live state in the callee-saved bank, so clobbering one aborts at *thread exit*
/// (`_pthread_terminate` PAC failure / libmalloc in `_pthread_tsd_cleanup`) with none
/// of this code in the backtrace. The body DOES use one: `pid` is live across
/// `waitpid`, so the allocator colors it callee-saved (`x21` on macos/linux-aarch64,
/// `r12` on linux-x86_64) — and `finalize_vreg_helper`'s frame builder saves and
/// restores exactly the registers the allocator used (`calleeSaved: ["x21","lr"]` in
/// the dump), which is what makes that safe. The one register it must NOT touch is
/// `ARENA_STATE_REGISTER` (`x19`), and that is reserved from allocation, so it cannot.
/// This is the difference from `lower_thread_trampoline`, which hand-manages its frame
/// (and therefore has to save `x19`/`x20`/the closure register itself).
///
/// **Stack alignment.** A start routine is entered by a foreign `call` on x86-64
/// (glibc `start_thread`, musl's dispatch), i.e. at `sp % 16 == 8`, and a downstream
/// call from a frame that assumed `sp % 16 == 0` faults on libc's first `movaps` to a
/// stack local. This body needs no manual bias because `finalize_frame` already adds
/// the per-arch one — `frame_call_padding()` is 8 on x86-64 and 0 on AArch64/RISC-V
/// (`vreg_frame.rs`), which is why the emitted `stackSize` is 24 on linux-x86_64 and
/// 16 elsewhere. The trampoline hand-rolls the same `+8` only because it bypasses this
/// frame builder.
///
/// **Arena.** Arena state is per-thread and a spawned thread gets its own zeroed copy,
/// so a reaper must not allocate, free, or read through `x19`. It does not: the whole
/// body is register moves, `waitpid`, the errno accessor, and a return.
///
/// **Stack size.** `detach` passes a NULL `pthread_attr_t` rather than the explicit
/// 8 MiB `thread::start` and the graphics thread set, because the reason those need it
/// does not apply here: they run arbitrary MFB code, whose frames are routinely
/// hundreds of KiB. This thread runs no MFB code at all. Its own frame is 16 bytes (24
/// on linux-x86_64, the `stackSize` in the dump) plus libc's `waitpid`/errno frames,
/// against a 512 KiB macOS / 128 KiB musl default — four orders of magnitude of
/// headroom, and fixed, since the body cannot grow without editing this function.
/// Taking the default also keeps the per-detach cost small, which matters because the
/// thread count scales with the number of *live* detached children (`func_detach.rs`).
pub(crate) fn lower_process_reaper_helper(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    const EINTR: &str = "4";
    let symbol = PROCESS_REAPER_SYMBOL;
    let mut v = Vregs::new();
    let pid = v.next();
    let ret = v.next();
    let errno = v.next();
    let wait_loop = format!("{symbol}_wait");
    let interrupted = format!("{symbol}_interrupted");
    let done = format!("{symbol}_done");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&pid, abi::c_arg(0)),
        // waitpid(pid, NULL, 0) — block until this one child exits, then reap it.
        // The status is discarded: nothing can observe a detached child's exit
        // (its handle is closed), so there is nowhere to cache it.
        abi::label(&wait_loop),
        abi::move_register(abi::c_arg(0), &pid),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ];
    let mut relocations = Vec::new();
    platform.emit_external_call(
        "waitpid",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // Retry on EINTR. Signal dispositions and delivery are process-wide, so ANY
    // signal the program takes while this thread sits in `waitpid` returns
    // `-1`/`EINTR` here — and a `waitpid` that returns without reaping leaves
    // exactly the zombie this thread exists to prevent, intermittently. Any other
    // failure (`ECHILD`: something else already reaped the child) is terminal, so
    // the loop exits rather than spinning.
    instructions.extend([
        abi::sign_extend_word(&ret, abi::c_return(0)),
        abi::compare_immediate(&ret, "0"),
        abi::branch_lt(&interrupted),
        abi::branch(&done),
        abi::label(&interrupted),
    ]);
    platform.emit_errno(
        symbol,
        errno.as_str().into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&errno, EINTR),
        abi::branch_eq(&wait_loop),
        abi::label(&done),
        abi::move_immediate(abi::c_return(0), "Integer", "0"),
        abi::return_(),
    ]);
    Ok(finalize_vreg_helper(
        "process.reaper",
        symbol,
        "Integer",
        instructions,
        relocations,
    ))
}
