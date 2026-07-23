//! Native code generation for the `os::` environment and introspection helpers
//! (plan-31-A/B). Most are a small runtime helper wrapping a libc primitive; the
//! exceptions are called out below. Every arm of `lower_os_call` is listed:
//!
//! - `os.getEnv` / `os.getEnvOr` / `os.hasEnv` — `getenv`.
//! - `os.setEnv` — `setenv(name, value, 1)`.
//! - `os.unsetEnv` — `unsetenv(name)`.
//! - `os.environ` — walk the live `char **environ` and build a `Map OF String`.
//! - `os.name` / `os.arch` — **no libc call**: compile-time constants folded from
//!   the target triple (`os_family`/`os_arch`) and emitted as a const `String`.
//! - `os.pid` — `getpid`.
//! - `os.cpuCount` — `sysconf(_SC_NPROCESSORS_ONLN)`.
//! - `os.hostName` — `gethostname`.
//! - `os.userName` — `getpwuid`/`getuid`.
//! - `os.executablePath` — the platform's own executable-path primitive.
//! - `os.resourcePath` — **build-mode dependent** (plan-55-B): resolves against
//!   the app bundle/AppDir layout or the build output directory, not a libc call.
//! - `os.args` — reads the `os::args` globals captured at entry (plan-31-B).
//!
//! String arguments are marshalled into NUL-terminated C buffers with the same
//! arena-copy idiom the `fs` path helpers use; results are the standard owned
//! `String`/`Boolean`/`Map OF String` values built directly in the arena.

use std::collections::HashMap;

use super::*;
use crate::target::shared::abi;

// `setenv`/`unsetenv` set `errno` on failure; ENOMEM/EINVAL are identical on
// Linux and macOS.
const ERRNO_ENOMEM: &str = "12";
pub(crate) const OS_ARGC_GLOBAL_SYMBOL: &str = "_mfb_rt_os_argc";
pub(crate) const OS_ARGV_GLOBAL_SYMBOL: &str = "_mfb_rt_os_argv";
const EXE_PATH_BUF: usize = 4096;
const EXE_PATH_FRAME_LOCALS: usize = EXE_PATH_BUF + 16;

/// Process-global mutex serializing `os::` env/pwd access against a concurrent
/// `os::setEnv`/`os::unsetEnv` from another MFBASIC thread (bug-64). The env
/// readers (`getEnv`/`getEnvOr`/`hasEnv`/`environ`/`userName`) hold it across the
/// libc call *and* the marshal-into-arena, and the writers (`setEnv`/`unsetEnv`)
/// hold it across `setenv`/`unsetenv`, so a reader never walks a `char **environ`
/// array a concurrent `setenv` is relocating/freeing, and `userName` never has its
/// static `getpwuid` buffer overwritten mid-copy. Single-threaded programs pay one
/// uncontended lock/unlock per call.
pub(crate) const OS_ENV_LOCK_SYMBOL: &str = "_mfb_rt_os_env_lock";

/// Storage size of the env/pwd mutex. 64 bytes covers the largest `pthread_mutex_t`
/// on every supported libc (glibc aarch64 = 48, glibc x86_64/riscv64 = 40,
/// musl = 40, macOS = 64), so one fixed-size, statically-initialized global works
/// on all targets.
pub(crate) const OS_ENV_LOCK_SIZE: usize = 64;

/// The frontend `os::` calls whose lowering takes the env/pwd lock. Kept in sync
/// with the plan-layer import gate (`module_uses_os_env_lock` in
/// `target::shared::plan::symbols`).
const OS_ENV_LOCK_CALLS: &[&str] = &[
    "os.getEnv",
    "os.getEnvOr",
    "os.hasEnv",
    "os.environ",
    "os.userName",
    "os.setEnv",
    "os.unsetEnv",
];

/// Whether `module` uses any `os::` helper that must serialize on the env/pwd
/// lock, so the writable mutex global is emitted (see `OS_ENV_LOCK_SYMBOL`).

pub(super) fn lower_os_helper(
    call: &str,
    symbol: &str,
    build_mode: crate::target::NativeBuildMode,
    module_name: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    match call {
        "os.getEnv" => lower_get_env(symbol, platform_imports, platform, false),
        "os.getEnvOr" => lower_get_env(symbol, platform_imports, platform, true),
        "os.hasEnv" => lower_has_env(symbol, platform_imports, platform),
        "os.setEnv" => lower_set_env(symbol, platform_imports, platform),
        "os.unsetEnv" => lower_unset_env(symbol, platform_imports, platform),
        "os.environ" => lower_environ(symbol, platform_imports, platform),
        "os.name" => lower_const_string(symbol, os_family(platform.target())),
        "os.arch" => lower_const_string(symbol, os_arch(platform.target())),
        "os.pid" => lower_pid(symbol, platform_imports, platform),
        "os.cpuCount" => lower_cpu_count(symbol, platform_imports, platform),
        "os.hostName" => lower_host_name(symbol, platform_imports, platform),
        "os.userName" => lower_user_name(symbol, platform_imports, platform),
        "os.executablePath" => lower_executable_path(symbol, platform_imports, platform),
        "os.resourcePath" => {
            lower_resource_path(symbol, build_mode, module_name, platform_imports, platform)
        }
        "os.args" => lower_args(symbol),
        other => Err(format!(
            "native os lowering does not support runtime call '{other}'"
        )),
    }
}

/// The OS family string for `os::name` — the part of the target triple before
/// the first `-` (`macos-aarch64` → `macos`).
fn os_family(target: &str) -> &'static str {
    if target.starts_with("macos") {
        "macos"
    } else {
        "linux"
    }
}

/// The CPU architecture string for `os::arch` — the part after the first `-`.
fn os_arch(target: &str) -> &'static str {
    if target.ends_with("x86_64") {
        "x86_64"
    } else if target.ends_with("riscv64") {
        "riscv64"
    } else {
        "aarch64"
    }
}

fn alloc_reloc(symbol: &str, relocations: &mut Vec<CodeRelocation>) {
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
}

#[allow(clippy::too_many_arguments)]
/// Marshal a MFBASIC `String*` held in `src` into a fresh NUL-terminated arena
/// C-string, leaving its pointer in `out`. Both `src` and `out` are vregs so the
/// allocator preserves them across the `arena_alloc` call. Branches to
/// `alloc_fail` on OOM. `uniq` disambiguates the copy-loop labels.
fn marshal_cstring(
    symbol: &str,
    src: &str,
    out: &str,
    alloc_fail: &str,
    uniq: &str,
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let alloc_ok = format!("{uniq}_alloc_ok");
    let copy_loop = format!("{uniq}_copy_loop");
    let copy_done = format!("{uniq}_copy_done");
    let len = vregs.next();
    let src_cursor = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    instructions.extend([
        abi::load_u64(&len, src, 0),
        abi::add_immediate(abi::return_register(), &len, 1),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(symbol, relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(alloc_fail),
        abi::label(&alloc_ok),
        abi::move_register(out, abi::RET[1]),
        abi::load_u64(&len, src, 0),
        abi::add_immediate(&src_cursor, src, 8),
        abi::move_register(&dst, out),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, &len),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &src_cursor, 0),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src_cursor, &src_cursor, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, &dst, 0),
    ]);
}

/// Build an owned arena `String` from the NUL-terminated C-string in `cstr`,
/// landing it in the result registers with the OK tag. Branches to `alloc_fail`
/// on OOM. `cstr` is a vreg (preserved across `arena_alloc`).
fn build_string_from_cstr(
    symbol: &str,
    cstr: &str,
    alloc_fail: &str,
    uniq: &str,
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let count_loop = format!("{uniq}_len_loop");
    let count_done = format!("{uniq}_len_done");
    let alloc_ok = format!("{uniq}_str_ok");
    let copy_loop = format!("{uniq}_str_copy_loop");
    let copy_done = format!("{uniq}_str_copy_done");
    let cursor = vregs.next();
    let length = vregs.next();
    let byte = vregs.next();
    let block = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    instructions.extend([
        abi::move_register(&cursor, cstr),
        abi::move_immediate(&length, "Integer", "0"),
        abi::label(&count_loop),
        abi::load_u8(&byte, &cursor, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&count_done),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::add_immediate(&length, &length, 1),
        abi::branch(&count_loop),
        abi::label(&count_done),
        // 8-byte length header + bytes + NUL terminator.
        abi::add_immediate(abi::return_register(), &length, 9),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(symbol, relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(alloc_fail),
        abi::label(&alloc_ok),
        abi::move_register(&block, abi::RET[1]),
        abi::store_u64(&length, &block, 0),
        abi::move_register(&src, cstr),
        abi::add_immediate(&dst, &block, 8),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, &length),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, &dst, 0),
        abi::move_register(RESULT_VALUE_REGISTER, &block),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
    ]);
}

fn push_alloc_error(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ALLOCATION_SYMBOL, instructions, relocations);
}

#[allow(clippy::too_many_arguments)]
/// Build an owned arena `String` of exactly `len` bytes copied from `src`
/// (which need NOT be NUL-terminated — used for `readlink`), landing it in the
/// result registers with the OK tag. Branches to `alloc_fail` on OOM.
fn build_string_from_len(
    symbol: &str,
    src: &str,
    len: &str,
    alloc_fail: &str,
    uniq: &str,
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let alloc_ok = format!("{uniq}_ok");
    let copy_loop = format!("{uniq}_copy_loop");
    let copy_done = format!("{uniq}_copy_done");
    let block = vregs.next();
    let cursor = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    instructions.extend([
        abi::add_immediate(abi::return_register(), len, 9),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(symbol, relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(alloc_fail),
        abi::label(&alloc_ok),
        abi::move_register(&block, abi::RET[1]),
        abi::store_u64(len, &block, 0),
        abi::move_register(&cursor, src),
        abi::add_immediate(&dst, &block, 8),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, len),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &cursor, 0),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, &dst, 0),
        abi::move_register(RESULT_VALUE_REGISTER, &block),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
    ]);
}

#[allow(clippy::too_many_arguments)]
/// Copy `len` bytes from `src_base[0..len]` to `dst`, advancing `dst` past the
/// copied bytes (plan-55-B §4.4). `src_cursor`/`index`/`byte` are caller-owned
/// scratch vregs; `uniq` disambiguates the loop labels.
fn emit_copy_counted(
    src_base: &str,
    len: &str,
    dst: &str,
    src_cursor: &str,
    index: &str,
    byte: &str,
    uniq: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let loop_label = format!("{uniq}_loop");
    let done_label = format!("{uniq}_done");
    instructions.extend([
        abi::move_register(src_cursor, src_base),
        abi::move_immediate(index, "Integer", "0"),
        abi::label(&loop_label),
        abi::compare_registers(index, len),
        abi::branch_eq(&done_label),
        abi::load_u8(byte, src_cursor, 0),
        abi::store_u8(byte, dst, 0),
        abi::add_immediate(src_cursor, src_cursor, 1),
        abi::add_immediate(dst, dst, 1),
        abi::add_immediate(index, index, 1),
        abi::branch(&loop_label),
        abi::label(&done_label),
    ]);
}

/// Store the constant byte `value` at `dst` and advance `dst` by one
/// (plan-55-B §4.4). `scratch` is a caller-owned vreg.
fn emit_store_byte_advance(
    value: u8,
    dst: &str,
    scratch: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    instructions.extend([
        abi::move_immediate(scratch, "Byte", &value.to_string()),
        abi::store_u8(scratch, dst, 0),
        abi::add_immediate(dst, dst, 1),
    ]);
}

mod env;
mod introspect;
mod paths;

use env::*;
use introspect::*;
use paths::*;

pub(crate) use env::{module_uses_env_lock, os_env_lock_init_hex};
