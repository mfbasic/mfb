//! Native code generation for the `os::` environment helpers (plan-31-A). Each
//! is a small runtime helper wrapping a libc primitive:
//!
//! - `os.getEnv` / `os.getEnvOr` / `os.hasEnv` — `getenv`.
//! - `os.setEnv` — `setenv(name, value, 1)`.
//! - `os.unsetEnv` — `unsetenv(name)`.
//! - `os.environ` — walk the live `char **environ` and build a `Map OF String`.
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
pub(crate) fn module_uses_env_lock(module: &NirModule) -> bool {
    OS_ENV_LOCK_CALLS
        .iter()
        .any(|call| module_uses_call(module, call))
}

/// The statically-initialized bytes of the env/pwd mutex for `target`, as a hex
/// string (two chars per byte), so no runtime initializer call is needed. Linux
/// `PTHREAD_MUTEX_INITIALIZER` is an all-zero `pthread_mutex_t`; macOS is
/// `{ _PTHREAD_MUTEX_SIG_init, {0} }`, i.e. the `0x32AAABA7` signature in the first
/// 8-byte `__sig` word with the rest zero, which libc lazily first-use-initializes
/// on the first `pthread_mutex_lock` (exactly as a static `PTHREAD_MUTEX_INITIALIZER`
/// does).
pub(crate) fn os_env_lock_init_hex(target: &str) -> String {
    let mut bytes = vec![0u8; OS_ENV_LOCK_SIZE];
    if target.starts_with("macos") {
        // `_PTHREAD_MUTEX_SIG_init` = 0x32AAABA7, little-endian in the `long __sig`.
        bytes[0..4].copy_from_slice(&0x32AA_ABA7u32.to_le_bytes());
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Acquire the env/pwd lock: `pthread_mutex_lock(&_mfb_rt_os_env_lock)`. Emitted at
/// helper entry, after incoming `String*` arguments have been saved into vregs (the
/// call clobbers all caller-saved registers).
fn emit_env_lock(ctx: &mut EmitCtx) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    push_symbol_address(
        symbol,
        OS_ENV_LOCK_SYMBOL,
        abi::ARG[0],
        ctx.instructions,
        ctx.relocations,
    );
    platform.emit_libc_call(
        "pthread_mutex_lock",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )
}

/// Release the env/pwd lock and return. The four result registers (tag/value/
/// message/source) are preserved across the `pthread_mutex_unlock` call — which
/// clobbers all caller-saved registers — through vregs the allocator keeps live.
/// Every helper routes all exit paths through a single `done` label so exactly one
/// balanced unlock runs per (matched) lock.
fn emit_env_unlock_return(ctx: &mut EmitCtx, vregs: &mut Vregs) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let saved_tag = vregs.next();
    let saved_value = vregs.next();
    let saved_message = vregs.next();
    let saved_source = vregs.next();
    ctx.instructions.extend([
        abi::move_register(&saved_tag, RESULT_TAG_REGISTER),
        abi::move_register(&saved_value, RESULT_VALUE_REGISTER),
        abi::move_register(&saved_message, RESULT_ERROR_MESSAGE_REGISTER),
        abi::move_register(&saved_source, RESULT_ERROR_SOURCE_REGISTER),
    ]);
    push_symbol_address(
        symbol,
        OS_ENV_LOCK_SYMBOL,
        abi::ARG[0],
        ctx.instructions,
        ctx.relocations,
    );
    platform.emit_libc_call(
        "pthread_mutex_unlock",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::move_register(RESULT_TAG_REGISTER, &saved_tag),
        abi::move_register(RESULT_VALUE_REGISTER, &saved_value),
        abi::move_register(RESULT_ERROR_MESSAGE_REGISTER, &saved_message),
        abi::move_register(RESULT_ERROR_SOURCE_REGISTER, &saved_source),
        abi::return_(),
    ]);
    Ok(())
}

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

/// Marshal a MFBASIC `String*` held in `src` into a fresh NUL-terminated arena
/// C-string, leaving its pointer in `out`. Both `src` and `out` are vregs so the
/// allocator preserves them across the `arena_alloc` call. Branches to
/// `alloc_fail` on OOM. `uniq` disambiguates the copy-loop labels.
#[allow(clippy::too_many_arguments)]
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
#[allow(clippy::too_many_arguments)]
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

fn lower_get_env(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    with_fallback: bool,
) -> HelperResult {
    let not_found = format!("{symbol}_not_found");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let name = vregs.next();
    let fallback = vregs.next();
    let cname = vregs.next();
    let value = vregs.next();
    let mut instructions = vec![abi::label("entry"), abi::move_register(&name, abi::ARG[0])];
    if with_fallback {
        instructions.push(abi::move_register(&fallback, abi::ARG[1]));
    }
    let mut relocations = Vec::new();
    // Serialize the whole `getenv` + marshal-into-arena against a concurrent
    // `os::setEnv` relocating/freeing `environ` (bug-64).
    emit_env_lock(&mut EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    })?;
    marshal_cstring(
        symbol,
        &name,
        &cname,
        &alloc_error,
        &format!("{symbol}_name"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::move_register(abi::ARG[0], &cname));
    platform.emit_libc_call(
        "getenv",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(&value, abi::return_register()),
        abi::compare_immediate(&value, "0"),
        abi::branch_eq(&not_found),
    ]);
    build_string_from_cstr(
        symbol,
        &value,
        &alloc_error,
        &format!("{symbol}_found"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&not_found)]);
    if with_fallback {
        // Return a fresh owned copy of `fallback` (by its stored length, so an
        // embedded NUL is preserved).
        let flen = vregs.next();
        let alloc_ok = format!("{symbol}_fb_ok");
        let copy_loop = format!("{symbol}_fb_copy_loop");
        let copy_done = format!("{symbol}_fb_copy_done");
        let block = vregs.next();
        let src = vregs.next();
        let dst = vregs.next();
        let index = vregs.next();
        let byte = vregs.next();
        instructions.extend([
            abi::load_u64(&flen, &fallback, 0),
            abi::add_immediate(abi::return_register(), &flen, 9),
            abi::move_immediate(abi::ARG[1], "Integer", "8"),
            abi::branch_link(ARENA_ALLOC_SYMBOL),
        ]);
        alloc_reloc(symbol, &mut relocations);
        instructions.extend([
            abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
            abi::branch_ne(&alloc_error),
            abi::label(&alloc_ok),
            abi::move_register(&block, abi::RET[1]),
            abi::load_u64(&flen, &fallback, 0),
            abi::store_u64(&flen, &block, 0),
            abi::add_immediate(&src, &fallback, 8),
            abi::add_immediate(&dst, &block, 8),
            abi::move_immediate(&index, "Integer", "0"),
            abi::label(&copy_loop),
            abi::compare_registers(&index, &flen),
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
            abi::branch(&done),
        ]);
    } else {
        instructions.extend([
            abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_NOT_FOUND_CODE),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
        ]);
        push_error_message_address(
            symbol,
            ERR_NOT_FOUND_SYMBOL,
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::branch(&done));
    }
    instructions.push(abi::label(&alloc_error));
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.push(abi::label(&done));
    emit_env_unlock_return(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
    )?;

    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

fn lower_has_env(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let present = format!("{symbol}_present");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let name = vregs.next();
    let cname = vregs.next();
    let mut instructions = vec![abi::label("entry"), abi::move_register(&name, abi::ARG[0])];
    let mut relocations = Vec::new();
    // Serialize the `getenv` probe against a concurrent `os::setEnv` relocating
    // `environ` (bug-64).
    emit_env_lock(&mut EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    })?;
    marshal_cstring(
        symbol,
        &name,
        &cname,
        &alloc_error,
        &format!("{symbol}_name"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::move_register(abi::ARG[0], &cname));
    platform.emit_libc_call(
        "getenv",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&present),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&present),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&alloc_error),
    ]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.push(abi::label(&done));
    emit_env_unlock_return(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
    )?;

    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

fn lower_set_env(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let ok = format!("{symbol}_ok");
    let fail = format!("{symbol}_fail");
    let oom = format!("{symbol}_oom");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let name = vregs.next();
    let value = vregs.next();
    let cname = vregs.next();
    let cvalue = vregs.next();
    let errno = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&name, abi::ARG[0]),
        abi::move_register(&value, abi::ARG[1]),
    ];
    let mut relocations = Vec::new();
    // Hold the lock across `setenv` so a concurrent env reader on another thread
    // never observes a half-relocated `environ` (bug-64).
    emit_env_lock(&mut EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    })?;
    marshal_cstring(
        symbol,
        &name,
        &cname,
        &alloc_error,
        &format!("{symbol}_name"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    marshal_cstring(
        symbol,
        &value,
        &cvalue,
        &alloc_error,
        &format!("{symbol}_value"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::move_register(abi::ARG[0], &cname),
        abi::move_register(abi::ARG[1], &cvalue),
        abi::move_immediate(abi::ARG[2], "Integer", "1"),
    ]);
    platform.emit_libc_call(
        "setenv",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&fail),
        abi::label(&ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&fail),
    ]);
    // Distinguish ENOMEM (→ ErrOutOfMemory) from every other errno (→
    // ErrInvalidArgument: empty name, or a name containing '=').
    platform.emit_errno(
        symbol,
        &errno,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&errno, ERRNO_ENOMEM),
        abi::branch_eq(&oom),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&oom)]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.push(abi::label(&done));
    emit_env_unlock_return(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
    )?;

    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

fn lower_unset_env(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let name = vregs.next();
    let cname = vregs.next();
    let mut instructions = vec![abi::label("entry"), abi::move_register(&name, abi::ARG[0])];
    let mut relocations = Vec::new();
    // Hold the lock across `unsetenv` so a concurrent env reader on another thread
    // never observes a half-relocated `environ` (bug-64).
    emit_env_lock(&mut EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    })?;
    marshal_cstring(
        symbol,
        &name,
        &cname,
        &alloc_error,
        &format!("{symbol}_name"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::move_register(abi::ARG[0], &cname));
    platform.emit_libc_call(
        "unsetenv",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // `unsetenv` is a no-op for an absent variable; treat any return as success.
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&alloc_error),
    ]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.push(abi::label(&done));
    emit_env_unlock_return(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
    )?;

    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

/// `os::environ()` — walk `char **environ` twice: pass 1 counts entries and the
/// total key+value data bytes (the `=` separator is dropped); pass 2 allocates
/// the `Map OF String` (header + entry table + data + lazy bucket region) and
/// fills it. Each `KEY=VALUE` splits at the first `=`.
fn lower_environ(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let count_loop = format!("{symbol}_count_loop");
    let count_done = format!("{symbol}_count_done");
    let count_scan = format!("{symbol}_count_scan");
    let count_scan_done = format!("{symbol}_count_scan_done");
    let count_data = format!("{symbol}_count_data");
    let count_next = format!("{symbol}_count_next");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let fill_loop = format!("{symbol}_fill_loop");
    let fill_done = format!("{symbol}_fill_done");
    let key_scan = format!("{symbol}_key_scan");
    let key_scan_done = format!("{symbol}_key_scan_done");
    let key_copy_loop = format!("{symbol}_key_copy_loop");
    let key_copy_done = format!("{symbol}_key_copy_done");
    let val_len_loop = format!("{symbol}_val_len_loop");
    let val_store = format!("{symbol}_val_store");
    let val_copy_loop = format!("{symbol}_val_copy_loop");
    let val_copy_done = format!("{symbol}_val_copy_done");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let envp = vregs.next();
    let cursor = vregs.next();
    let entry_ptr = vregs.next();
    let count = vregs.next();
    let data_bytes = vregs.next();
    let scan = vregs.next();
    let byte = vregs.next();
    let collection = vregs.next();
    let entry_cursor = vregs.next();
    let data_cursor = vregs.next();
    let data_offset = vregs.next();
    let scratch = vregs.next();
    let key_len = vregs.next();
    let val_ptr = vregs.next();
    let val_len = vregs.next();
    let src = vregs.next();
    let eq_flag = vregs.next();

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // Hold the lock across the whole two-pass `environ` walk and the marshal into
    // the arena `Map`, so a concurrent `os::setEnv` cannot relocate/free the array
    // or its strings mid-walk (bug-64).
    emit_env_lock(&mut EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    })?;
    platform.emit_environ_pointer(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(&envp, abi::return_register()),
        // Pass 1: count entries and data bytes.
        abi::move_register(&cursor, &envp),
        abi::move_immediate(&count, "Integer", "0"),
        abi::move_immediate(&data_bytes, "Integer", "0"),
        abi::label(&count_loop),
        abi::load_u64(&entry_ptr, &cursor, 0),
        abi::compare_immediate(&entry_ptr, "0"),
        abi::branch_eq(&count_done),
        // Scan "KEY=VALUE": every byte before the NUL contributes to data, minus
        // exactly the FIRST '=' separator. A '=' inside the value (e.g.
        // `LS_COLORS`) is kept — pass 2 splits only at the first '=', so pass 1
        // must undercount by exactly one to keep the data region correctly sized.
        abi::move_register(&scan, &entry_ptr),
        abi::move_immediate(&eq_flag, "Integer", "0"),
        abi::label(&count_scan),
        abi::load_u8(&byte, &scan, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&count_scan_done),
        abi::compare_immediate(&byte, "61"), // '='
        abi::branch_ne(&count_data),
        abi::compare_immediate(&eq_flag, "0"),
        abi::branch_ne(&count_data), // a later '=' is value data
        abi::move_immediate(&eq_flag, "Integer", "1"), // first '=' is the separator
        abi::branch(&count_next),
        abi::label(&count_data),
        abi::add_immediate(&data_bytes, &data_bytes, 1),
        abi::label(&count_next),
        abi::add_immediate(&scan, &scan, 1),
        abi::branch(&count_scan),
        abi::label(&count_scan_done),
        abi::add_immediate(&count, &count, 1),
        abi::add_immediate(&cursor, &cursor, 8),
        abi::branch(&count_loop),
        abi::label(&count_done),
        // size = HEADER + count*ENTRY_SIZE + data_bytes + count*(2*MAP_BUCKET_SIZE)
        abi::move_immediate(
            &scratch,
            "Integer",
            &(COLLECTION_ENTRY_SIZE + 2 * MAP_BUCKET_SIZE).to_string(),
        ),
        abi::multiply_registers(&scratch, &count, &scratch),
        abi::add_registers(&scratch, &scratch, &data_bytes),
        abi::add_immediate(abi::return_register(), &scratch, COLLECTION_HEADER_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(symbol, &mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&collection, abi::RET[1]),
        // Header.
        abi::move_immediate(&scratch, "Byte", &COLLECTION_KIND_MAP.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KIND),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_STRING.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_STRING.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&scratch, "Byte", "1"),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_FLAGS_VERSION),
        abi::move_immediate(&scratch, "Byte", "0"),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_BUCKETS_READY),
        abi::store_u64(&count, &collection, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&count, &collection, COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(&data_bytes, &collection, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&data_bytes, &collection, COLLECTION_OFFSET_DATA_CAPACITY),
        // entry_cursor = base + HEADER; data_cursor = entry table end.
        abi::add_immediate(&entry_cursor, &collection, COLLECTION_HEADER_SIZE),
        abi::move_immediate(&scratch, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&scratch, &count, &scratch),
        abi::add_registers(&data_cursor, &entry_cursor, &scratch),
        abi::move_immediate(&data_offset, "Integer", "0"),
        // Pass 2: fill.
        abi::move_register(&cursor, &envp),
        abi::label(&fill_loop),
        abi::load_u64(&entry_ptr, &cursor, 0),
        abi::compare_immediate(&entry_ptr, "0"),
        abi::branch_eq(&fill_done),
        // key_len = index of first '=' (or full length if none).
        abi::move_register(&scan, &entry_ptr),
        abi::move_immediate(&key_len, "Integer", "0"),
        abi::label(&key_scan),
        abi::load_u8(&byte, &scan, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&key_scan_done),
        abi::compare_immediate(&byte, "61"), // '='
        abi::branch_eq(&key_scan_done),
        abi::add_immediate(&key_len, &key_len, 1),
        abi::add_immediate(&scan, &scan, 1),
        abi::branch(&key_scan),
        abi::label(&key_scan_done),
        // Entry: FLAGS=used, KEY_OFFSET=data_offset, KEY_LENGTH=key_len.
        abi::move_immediate(&scratch, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8(&scratch, &entry_cursor, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(
            &data_offset,
            &entry_cursor,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ),
        abi::store_u64(&key_len, &entry_cursor, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        // Copy key bytes [entry_ptr .. entry_ptr+key_len) into the data region.
        abi::move_register(&src, &entry_ptr),
        abi::move_immediate(&scratch, "Integer", "0"),
        abi::label(&key_copy_loop),
        abi::compare_registers(&scratch, &key_len),
        abi::branch_eq(&key_copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &data_cursor, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&data_cursor, &data_cursor, 1),
        abi::add_immediate(&scratch, &scratch, 1),
        abi::branch(&key_copy_loop),
        abi::label(&key_copy_done),
        abi::add_registers(&data_offset, &data_offset, &key_len),
        // val_ptr points at the '=' (or the NUL, for a key with no '=').
        abi::add_registers(&val_ptr, &entry_ptr, &key_len),
        abi::move_immediate(&val_len, "Integer", "0"),
        abi::load_u8(&byte, &val_ptr, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&val_store), // no '=': empty value (val_ptr at NUL, len 0)
        abi::add_immediate(&val_ptr, &val_ptr, 1), // skip '='
        // val_len = strlen(val_ptr).
        abi::move_register(&scan, &val_ptr),
        abi::label(&val_len_loop),
        abi::load_u8(&byte, &scan, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&val_store),
        abi::add_immediate(&val_len, &val_len, 1),
        abi::add_immediate(&scan, &scan, 1),
        abi::branch(&val_len_loop),
        abi::label(&val_store),
        // VALUE_OFFSET / VALUE_LENGTH.
        abi::store_u64(
            &data_offset,
            &entry_cursor,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ),
        abi::store_u64(
            &val_len,
            &entry_cursor,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ),
        abi::move_register(&src, &val_ptr),
        abi::move_immediate(&scratch, "Integer", "0"),
        abi::label(&val_copy_loop),
        abi::compare_registers(&scratch, &val_len),
        abi::branch_eq(&val_copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &data_cursor, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&data_cursor, &data_cursor, 1),
        abi::add_immediate(&scratch, &scratch, 1),
        abi::branch(&val_copy_loop),
        abi::label(&val_copy_done),
        abi::add_registers(&data_offset, &data_offset, &val_len),
        abi::add_immediate(&entry_cursor, &entry_cursor, COLLECTION_ENTRY_SIZE),
        abi::add_immediate(&cursor, &cursor, 8),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
        abi::move_register(RESULT_VALUE_REGISTER, &collection),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&alloc_error),
    ]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.push(abi::label(&done));
    emit_env_unlock_return(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
    )?;

    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

/// Program-entry-captured `argc`/`argv` globals (plan-31-B). Two writable
/// 8-byte words the entry fills before any user code, read by `os::args()`.
/// Emitted only when a module uses `os.args`.
pub(crate) const OS_ARGC_GLOBAL_SYMBOL: &str = "_mfb_rt_os_argc";
pub(crate) const OS_ARGV_GLOBAL_SYMBOL: &str = "_mfb_rt_os_argv";

/// Build an owned arena `String` of exactly `len` bytes copied from `src`
/// (which need NOT be NUL-terminated — used for `readlink`), landing it in the
/// result registers with the OK tag. Branches to `alloc_fail` on OOM.
#[allow(clippy::too_many_arguments)]
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

/// `os::name` / `os::arch` — return a fixed, target-selected `String` constant,
/// materialized directly into a fresh arena `String` (length header + bytes +
/// NUL) so the result is an ordinary owned value.
fn lower_const_string(symbol: &str, value: &str) -> HelperResult {
    let alloc_ok = format!("{symbol}_ok");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let bytes = value.as_bytes();
    let len = bytes.len();

    let mut vregs = Vregs::new();
    let block = vregs.next();
    let byte = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_immediate(abi::return_register(), "Integer", &(len + 9).to_string()),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = Vec::new();
    alloc_reloc(symbol, &mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&block, abi::RET[1]),
        abi::move_immediate(&byte, "Integer", &len.to_string()),
        abi::store_u64(&byte, &block, 0),
    ]);
    for (i, b) in bytes.iter().enumerate() {
        instructions.push(abi::move_immediate(&byte, "Byte", &b.to_string()));
        instructions.push(abi::store_u8(&byte, &block, 8 + i));
    }
    instructions.extend([
        abi::move_immediate(&byte, "Byte", "0"),
        abi::store_u8(&byte, &block, 8 + len),
        abi::move_register(RESULT_VALUE_REGISTER, &block),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&alloc_error),
    ]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);

    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

/// `os::pid` — `getpid()` as an `Integer` (a small positive value; the int
/// return is zero-extended by the W-register write, so no widening is needed).
fn lower_pid(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    platform.emit_libc_call(
        "getpid",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, abi::return_register()),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

/// `os::cpuCount` — `sysconf(_SC_NPROCESSORS_ONLN)` as an `Integer`, clamped to
/// at least 1. `_SC_NPROCESSORS_ONLN` is 58 on Darwin and 84 on Linux.
fn lower_cpu_count(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let sc_nprocessors_onln = if platform.target().starts_with("macos") {
        "58"
    } else {
        "84"
    };
    let positive = format!("{symbol}_positive");
    let mut vregs = Vregs::new();
    let count = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_immediate(abi::ARG[0], "Integer", sc_nprocessors_onln),
    ];
    let mut relocations = Vec::new();
    platform.emit_libc_call(
        "sysconf",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(&count, abi::return_register()),
        // sysconf returns -1 (or 0) on failure or an indeterminate answer: clamp
        // to a minimum of 1 so callers always get a usable count.
        abi::compare_immediate(&count, "1"),
        abi::branch_ge(&positive),
        abi::move_immediate(&count, "Integer", "1"),
        abi::label(&positive),
        abi::move_register(RESULT_VALUE_REGISTER, &count),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

/// `os::hostName` — `gethostname(buf, 256)` into an on-frame buffer, then a
/// `String` copy. HOST_NAME_MAX is 64 (Linux) / 255 (macOS), so 256 always
/// holds a NUL-terminated name.
fn lower_host_name(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const BUF: usize = 256;
    let ok = format!("{symbol}_ok");
    let fail = format!("{symbol}_fail");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let buf = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::add_immediate(abi::ARG[0], abi::stack_pointer(), 0),
        abi::move_immediate(abi::ARG[1], "Integer", &BUF.to_string()),
    ];
    let mut relocations = Vec::new();
    platform.emit_libc_call(
        "gethostname",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&ok),
        abi::branch(&fail),
        abi::label(&ok),
        // Defensive NUL at the last byte, then build the String from the buffer.
        abi::add_immediate(&buf, abi::stack_pointer(), 0),
        abi::store_u8(abi::ZERO, &buf, BUF - 1),
    ]);
    build_string_from_cstr(
        symbol,
        &buf,
        &alloc_error,
        &format!("{symbol}_str"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&fail)]);
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_UNSUPPORTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_UNSUPPORTED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], BUF);
    Ok((frame, instructions, relocations, stack_slots))
}

/// `os::userName` — `getpwuid(getuid())->pw_name` (`pw_name` is the first field
/// of `struct passwd` on every supported libc). Raises `ErrUnsupported` if the
/// uid has no passwd entry (e.g. a bare container uid).
fn lower_user_name(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let have_pwd = format!("{symbol}_have_pwd");
    let have_name = format!("{symbol}_have_name");
    let fail = format!("{symbol}_fail");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let pwname = vregs.next();
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // Hold the lock across `getpwuid` and the copy of its static `passwd`/`pw_name`
    // buffer, so a concurrent `getpwuid`/`getpwnam` cannot overwrite it mid-copy.
    // The env lock doubles as the process-global pwd lock (bug-64).
    emit_env_lock(&mut EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    })?;
    platform.emit_libc_call(
        "getuid",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    platform.emit_libc_call(
        "getpwuid",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&have_pwd),
        abi::branch(&fail),
        abi::label(&have_pwd),
        abi::load_u64(&pwname, abi::return_register(), 0), // pw_name @ offset 0
        abi::compare_immediate(&pwname, "0"),
        abi::branch_ne(&have_name),
        abi::branch(&fail),
        abi::label(&have_name),
    ]);
    build_string_from_cstr(
        symbol,
        &pwname,
        &alloc_error,
        &format!("{symbol}_str"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&fail)]);
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_UNSUPPORTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_UNSUPPORTED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.push(abi::label(&done));
    emit_env_unlock_return(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
    )?;
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

/// Bytes the executable-path acquisition reserves as function-frame locals: the
/// path buffer plus, on Linux, 16 bytes holding the `"/proc/self/exe\0"` path.
/// Shared by `lower_executable_path` and `lower_resource_path` (plan-55-B §4.1).
const EXE_PATH_BUF: usize = 4096;
const EXE_PATH_FRAME_LOCALS: usize = EXE_PATH_BUF + 16;

/// Emit the platform acquisition of the running executable's absolute path into
/// the function frame (plan-55-B §4.1). macOS uses `_NSGetExecutablePath(buf,
/// &size)`; Linux reads the `/proc/self/exe` symlink with `readlink`. Returns the
/// buffer pointer in a fresh vreg, plus — on Linux only — the byte count
/// `readlink` reported (the buffer is not NUL-terminated). macOS leaves the buffer
/// NUL-terminated and reports no count (callers needing a length scan for the NUL).
/// Branches to `fail` on acquisition error.
///
/// Callers must reserve at least `EXE_PATH_FRAME_LOCALS` frame locals and invoke
/// this FIRST, before allocating any other vreg, so `os::executablePath` keeps the
/// exact vreg-allocation order — and therefore the byte-identical output — it had
/// before this factoring.
fn emit_executable_path_into(
    ctx: &mut EmitCtx,
    fail: &str,
    vregs: &mut Vregs,
) -> Result<(String, Option<String>), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let ok = format!("{symbol}_ok");
    let buf = vregs.next();
    if platform.target().starts_with("macos") {
        // Frame: [0..BUF) path buffer, [BUF..BUF+8) uint32 size word (=BUF).
        let size_word = vregs.next();
        ctx.instructions.extend([
            abi::move_immediate(&size_word, "Integer", &EXE_PATH_BUF.to_string()),
            abi::store_u32(&size_word, abi::stack_pointer(), EXE_PATH_BUF),
            abi::add_immediate(abi::ARG[0], abi::stack_pointer(), 0),
            abi::add_immediate(abi::ARG[1], abi::stack_pointer(), EXE_PATH_BUF),
        ]);
        platform.emit_libc_call(
            "_NSGetExecutablePath",
            symbol,
            platform_imports,
            ctx.instructions,
            ctx.relocations,
        )?;
        ctx.instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&ok),
            abi::branch(fail),
            abi::label(&ok),
            abi::add_immediate(&buf, abi::stack_pointer(), 0),
        ]);
        Ok((buf, None))
    } else {
        // Frame: [0..16) "/proc/self/exe\0" path, [16..16+BUF) readlink buffer.
        let path = b"/proc/self/exe\0";
        for (i, b) in path.iter().enumerate() {
            let byte = vregs.next();
            ctx.instructions
                .push(abi::move_immediate(&byte, "Byte", &b.to_string()));
            ctx.instructions
                .push(abi::store_u8(&byte, abi::stack_pointer(), i));
        }
        let count = vregs.next();
        ctx.instructions.extend([
            abi::add_immediate(abi::ARG[0], abi::stack_pointer(), 0),
            abi::add_immediate(abi::ARG[1], abi::stack_pointer(), 16),
            abi::move_immediate(abi::ARG[2], "Integer", &EXE_PATH_BUF.to_string()),
        ]);
        platform.emit_libc_call(
            "readlink",
            symbol,
            platform_imports,
            ctx.instructions,
            ctx.relocations,
        )?;
        ctx.instructions.extend([
            abi::move_register(&count, abi::return_register()),
            abi::compare_immediate(&count, "0"),
            abi::branch_gt(&ok),
            abi::branch(fail),
            abi::label(&ok),
            abi::add_immediate(&buf, abi::stack_pointer(), 16),
        ]);
        Ok((buf, Some(count)))
    }
}

/// `os::executablePath` — the absolute path of the running binary. Acquires the
/// path via `emit_executable_path_into` (plan-55-B §4.1) and builds an owned
/// `String` from it: NUL-terminated on macOS, byte-counted on Linux.
fn lower_executable_path(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let fail = format!("{symbol}_fail");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();

    let (buf, count) = emit_executable_path_into(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &fail,
        &mut vregs,
    )?;
    match count {
        // Linux: `readlink` reported the byte count; the buffer has no NUL.
        Some(count) => build_string_from_len(
            symbol,
            &buf,
            &count,
            &alloc_error,
            &format!("{symbol}_str"),
            &mut vregs,
            &mut instructions,
            &mut relocations,
        ),
        // macOS: the buffer is NUL-terminated.
        None => build_string_from_cstr(
            symbol,
            &buf,
            &alloc_error,
            &format!("{symbol}_str"),
            &mut vregs,
            &mut instructions,
            &mut relocations,
        ),
    }
    instructions.extend([abi::branch(&done), abi::label(&fail)]);
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_UNSUPPORTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_UNSUPPORTED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) =
        finalize_vreg_body_with_locals(&mut instructions, &[], EXE_PATH_FRAME_LOCALS);
    Ok((frame, instructions, relocations, stack_slots))
}

/// The `(components-to-strip, suffix-to-append)` base offset for
/// `os::resourcePath`, per build mode (plan-55-B §4.2). `strip` drops that many
/// trailing `/`-delimited components of the absolute executable path (the filename
/// is component 1); `suffix` is appended after. Must stay in lockstep with
/// plan-55-A's `resource_output_dir`.
///
/// | build         | exe path                  | strip | suffix         | base                   |
/// | ---           | ---                       | ---   | ---            | ---                    |
/// | console       | `…/build/<name>`          | 1     | ``             | `…/build`              |
/// | macos `--app` | `…/Contents/MacOS/<name>` | 2     | `Resources`    | `…/Contents/Resources` |
/// | linux `--app` | `…/usr/bin/<name>`        | 2     | `share/<name>` | `…/usr/share/<name>`   |
fn resource_base_offset(
    build_mode: crate::target::NativeBuildMode,
    module_name: &str,
) -> (u32, String) {
    match build_mode {
        crate::target::NativeBuildMode::Console => (1, String::new()),
        crate::target::NativeBuildMode::MacApp => (2, "Resources".to_string()),
        crate::target::NativeBuildMode::LinuxApp => (2, format!("share/{module_name}")),
    }
}

/// `os::resourcePath(relative)` — the absolute on-disk path of a build resource
/// (plan-55-B §4.4). Validates that `relative` has no `.`/`..` path component
/// (else `ErrInvalidPath`), acquires the executable path, strips `strip` trailing
/// components and appends the mode `suffix` to form the base, and concatenates
/// `base + "/" + relative` into an owned arena `String`. The acquisition-failure
/// path returns `ErrUnsupported`, matching `os::executablePath`.
fn lower_resource_path(
    symbol: &str,
    build_mode: crate::target::NativeBuildMode,
    module_name: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let (strip, suffix) = resource_base_offset(build_mode, module_name);
    let suffix_bytes = suffix.into_bytes();

    let fail = format!("{symbol}_fail");
    let bad_arg = format!("{symbol}_bad_arg");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    // Capture the incoming `String` argument (pointer + length) before the exe-path
    // acquisition clobbers the ARG registers. A `String` block is
    // `[8-byte length][bytes][NUL]`; its data starts at pointer + 8.
    let arg_ptr = vregs.next();
    let arg_len = vregs.next();
    let arg_data = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&arg_ptr, abi::ARG[0]),
        abi::load_u64(&arg_len, &arg_ptr, 0),
        abi::add_immediate(&arg_data, &arg_ptr, 8),
    ];
    let mut relocations = Vec::new();

    // Step 1 (§4.4): reject a `.` or `..` path component. Forward scan tracking the
    // current component's length and whether every byte so far is a dot; at each
    // component boundary (`/` or string end) a component of length 1 or 2 that is
    // all dots is the rejection. Empty components (from `//` or a leading/trailing
    // `/`) are length 0, so never rejected.
    let scan_index = vregs.next();
    let comp_len = vregs.next();
    let comp_all_dots = vregs.next();
    let scan_byte = vregs.next();
    let validate_loop = format!("{symbol}_validate_loop");
    let validate_body = format!("{symbol}_validate_body");
    let validate_slash = format!("{symbol}_validate_slash");
    let validate_char = format!("{symbol}_validate_char");
    let validate_not_dot = format!("{symbol}_validate_not_dot");
    let validate_next = format!("{symbol}_validate_next");
    let validate_end = format!("{symbol}_validate_end");
    let check_boundary_ok = format!("{symbol}_boundary_ok");
    instructions.extend([
        abi::move_immediate(&scan_index, "Integer", "0"),
        abi::move_immediate(&comp_len, "Integer", "0"),
        abi::move_immediate(&comp_all_dots, "Integer", "1"),
        abi::label(&validate_loop),
        abi::compare_registers(&scan_index, &arg_len),
        abi::branch_ge(&validate_end),
        abi::label(&validate_body),
        // load byte = arg_data[scan_index]
        abi::add_registers(&scan_byte, &arg_data, &scan_index),
        abi::load_u8(&scan_byte, &scan_byte, 0),
        abi::compare_immediate(&scan_byte, "47"), // '/'
        abi::branch_eq(&validate_slash),
        abi::branch(&validate_char),
        // Component boundary at a slash: check then reset.
        abi::label(&validate_slash),
    ]);
    // Reject if the just-ended component was all dots and length 1 or 2.
    emit_reject_dot_component(
        &comp_len,
        &comp_all_dots,
        &bad_arg,
        &check_boundary_ok,
        &mut instructions,
    );
    instructions.extend([
        abi::label(&check_boundary_ok),
        abi::move_immediate(&comp_len, "Integer", "0"),
        abi::move_immediate(&comp_all_dots, "Integer", "1"),
        abi::branch(&validate_next),
        // A normal character: grow the component, clear all-dots unless it is '.'.
        abi::label(&validate_char),
        abi::add_immediate(&comp_len, &comp_len, 1),
        abi::compare_immediate(&scan_byte, "46"), // '.'
        abi::branch_eq(&validate_not_dot),
        abi::move_immediate(&comp_all_dots, "Integer", "0"),
        abi::label(&validate_not_dot),
        abi::branch(&validate_next),
        abi::label(&validate_next),
        abi::add_immediate(&scan_index, &scan_index, 1),
        abi::branch(&validate_loop),
        abi::label(&validate_end),
    ]);
    // Final component (string end is also a boundary).
    emit_reject_dot_component(
        &comp_len,
        &comp_all_dots,
        &bad_arg,
        &format!("{symbol}_validate_done"),
        &mut instructions,
    );
    instructions.push(abi::label(&format!("{symbol}_validate_done")));

    // Step 2 (§4.4): acquire the executable path, then compute its byte length `n`.
    let (buf, count) = emit_executable_path_into(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &fail,
        &mut vregs,
    )?;
    let n = vregs.next();
    match count {
        Some(count) => instructions.push(abi::move_register(&n, &count)),
        None => {
            // macOS: NUL-terminated buffer — scan for the NUL to get the length.
            let strlen_loop = format!("{symbol}_strlen_loop");
            let strlen_done = format!("{symbol}_strlen_done");
            let strlen_byte = vregs.next();
            let strlen_ptr = vregs.next();
            instructions.extend([
                abi::move_immediate(&n, "Integer", "0"),
                abi::move_register(&strlen_ptr, &buf),
                abi::label(&strlen_loop),
                abi::load_u8(&strlen_byte, &strlen_ptr, 0),
                abi::compare_immediate(&strlen_byte, "0"),
                abi::branch_eq(&strlen_done),
                abi::add_immediate(&n, &n, 1),
                abi::add_immediate(&strlen_ptr, &strlen_ptr, 1),
                abi::branch(&strlen_loop),
                abi::label(&strlen_done),
            ]);
        }
    }

    // Step 3 (§4.4): backward scan `buf[0..n]` for the `strip`-th slash from the end;
    // `prefix_len` is that slash's index (prefix = `buf[0..prefix_len]`, no trailing
    // slash). Fewer than `strip` slashes → a malformed path → `fail` (defensive).
    let prefix_len = vregs.next();
    let slash_scan = vregs.next();
    let slashes_left = vregs.next();
    let slash_byte = vregs.next();
    let slash_loop = format!("{symbol}_slash_loop");
    let slash_found = format!("{symbol}_slash_found");
    let prefix_ready = format!("{symbol}_prefix_ready");
    instructions.extend([
        abi::move_register(&slash_scan, &n),
        abi::move_immediate(&slashes_left, "Integer", &strip.to_string()),
        abi::label(&slash_loop),
        // No bytes left but slashes still needed → malformed path.
        abi::compare_immediate(&slash_scan, "0"),
        abi::branch_eq(&fail),
        abi::subtract_immediate(&slash_scan, &slash_scan, 1),
        abi::add_registers(&slash_byte, &buf, &slash_scan),
        abi::load_u8(&slash_byte, &slash_byte, 0),
        abi::compare_immediate(&slash_byte, "47"), // '/'
        abi::branch_eq(&slash_found),
        abi::branch(&slash_loop),
        abi::label(&slash_found),
        abi::subtract_immediate(&slashes_left, &slashes_left, 1),
        abi::compare_immediate(&slashes_left, "0"),
        abi::branch_eq(&prefix_ready),
        abi::branch(&slash_loop),
        abi::label(&prefix_ready),
        abi::move_register(&prefix_len, &slash_scan),
    ]);

    // Step 4 (§4.4): total result length =
    //   prefix_len + 1 ('/') + [suffix.len() + 1 ('/')] + arg_len.
    let extra = if suffix_bytes.is_empty() {
        1
    } else {
        suffix_bytes.len() + 2
    };
    let total_len = vregs.next();
    instructions.extend([
        abi::add_registers(&total_len, &prefix_len, &arg_len),
        abi::add_immediate(&total_len, &total_len, extra),
        // Arena block: 8-byte length header + bytes + NUL.
        abi::add_immediate(abi::return_register(), &total_len, 9),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(symbol, &mut relocations);
    let block = vregs.next();
    let dst = vregs.next();
    let copy_index = vregs.next();
    let copy_byte = vregs.next();
    let copy_src = vregs.next();
    let alloc_ok = format!("{symbol}_alloc_ok");
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&block, abi::RET[1]),
        abi::store_u64(&total_len, &block, 0),
        abi::add_immediate(&dst, &block, 8),
    ]);
    // Copy the prefix (`buf[0..prefix_len]`).
    emit_copy_counted(
        &buf,
        &prefix_len,
        &dst,
        &copy_src,
        &copy_index,
        &copy_byte,
        &format!("{symbol}_copy_prefix"),
        &mut instructions,
    );
    // '/'
    emit_store_byte_advance(b'/', &dst, &copy_byte, &mut instructions);
    // Optional suffix + '/'.
    if !suffix_bytes.is_empty() {
        for &b in &suffix_bytes {
            emit_store_byte_advance(b, &dst, &copy_byte, &mut instructions);
        }
        emit_store_byte_advance(b'/', &dst, &copy_byte, &mut instructions);
    }
    // Copy the argument bytes (`arg_data[0..arg_len]`).
    emit_copy_counted(
        &arg_data,
        &arg_len,
        &dst,
        &copy_src,
        &copy_index,
        &copy_byte,
        &format!("{symbol}_copy_arg"),
        &mut instructions,
    );
    instructions.extend([
        abi::store_u8(abi::ZERO, &dst, 0),
        abi::move_register(RESULT_VALUE_REGISTER, &block),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Error tails: acquisition failure → ErrUnsupported; bad component → ErrInvalidPath.
    instructions.extend([
        abi::label(&fail),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_UNSUPPORTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_UNSUPPORTED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&bad_arg),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_PATH_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_PATH_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) =
        finalize_vreg_body_with_locals(&mut instructions, &[], EXE_PATH_FRAME_LOCALS);
    Ok((frame, instructions, relocations, stack_slots))
}

/// Branch to `bad_arg` when the just-ended path component is exactly `.` or `..`
/// (all dots, length 1 or 2), else to `ok` (plan-55-B §4.4 step 1).
fn emit_reject_dot_component(
    comp_len: &str,
    comp_all_dots: &str,
    bad_arg: &str,
    ok: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    instructions.extend([
        // Not all-dots → fine.
        abi::compare_immediate(comp_all_dots, "0"),
        abi::branch_eq(ok),
        // All dots: reject length 1 (".") or 2 ("..").
        abi::compare_immediate(comp_len, "1"),
        abi::branch_eq(bad_arg),
        abi::compare_immediate(comp_len, "2"),
        abi::branch_eq(bad_arg),
        abi::branch(ok),
    ]);
}

/// Copy `len` bytes from `src_base[0..len]` to `dst`, advancing `dst` past the
/// copied bytes (plan-55-B §4.4). `src_cursor`/`index`/`byte` are caller-owned
/// scratch vregs; `uniq` disambiguates the loop labels.
#[allow(clippy::too_many_arguments)]
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

/// `os::args` — build a `List OF String` from the entry-captured `argv`,
/// excluding `argv[0]` (the program name; D1). Reads the `_mfb_rt_os_argc` /
/// `_mfb_rt_os_argv` globals the program entry fills at startup.
fn lower_args(symbol: &str) -> HelperResult {
    let count_loop = format!("{symbol}_count_loop");
    let count_done = format!("{symbol}_count_done");
    let count_str = format!("{symbol}_count_str");
    let count_str_done = format!("{symbol}_count_str_done");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let fill_loop = format!("{symbol}_fill_loop");
    let fill_done = format!("{symbol}_fill_done");
    let str_len = format!("{symbol}_str_len");
    let str_len_done = format!("{symbol}_str_len_done");
    let str_copy = format!("{symbol}_str_copy");
    let str_copy_done = format!("{symbol}_str_copy_done");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let argc = vregs.next();
    let argv = vregs.next();
    let index = vregs.next();
    let count = vregs.next();
    let data_bytes = vregs.next();
    let arg_ptr = vregs.next();
    let scan = vregs.next();
    let byte = vregs.next();
    let collection = vregs.next();
    let entry_cursor = vregs.next();
    let data_cursor = vregs.next();
    let data_offset = vregs.next();
    let arg_len = vregs.next();
    let scratch = vregs.next();
    let src = vregs.next();

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    push_symbol_address(
        symbol,
        OS_ARGC_GLOBAL_SYMBOL,
        &argc,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::load_u64(&argc, &argc, 0));
    push_symbol_address(
        symbol,
        OS_ARGV_GLOBAL_SYMBOL,
        &argv,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::load_u64(&argv, &argv, 0));
    instructions.extend([
        // Pass 1: count args (from index 1) and their total byte length.
        abi::move_immediate(&count, "Integer", "0"),
        abi::move_immediate(&data_bytes, "Integer", "0"),
        abi::move_immediate(&index, "Integer", "1"),
        abi::label(&count_loop),
        abi::compare_registers(&index, &argc),
        abi::branch_ge(&count_done),
        abi::shift_left_immediate(&scratch, &index, 3),
        abi::add_registers(&scratch, &argv, &scratch),
        abi::load_u64(&arg_ptr, &scratch, 0),
        abi::move_register(&scan, &arg_ptr),
        abi::label(&count_str),
        abi::load_u8(&byte, &scan, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&count_str_done),
        abi::add_immediate(&data_bytes, &data_bytes, 1),
        abi::add_immediate(&scan, &scan, 1),
        abi::branch(&count_str),
        abi::label(&count_str_done),
        abi::add_immediate(&count, &count, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&count_loop),
        abi::label(&count_done),
        // size = HEADER + count*ENTRY_SIZE + data_bytes (a List has no buckets).
        abi::move_immediate(&scratch, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&scratch, &count, &scratch),
        abi::add_registers(&scratch, &scratch, &data_bytes),
        abi::add_immediate(abi::return_register(), &scratch, COLLECTION_HEADER_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(symbol, &mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&collection, abi::RET[1]),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KIND),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_STRING.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&scratch, "Byte", "1"),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_FLAGS_VERSION),
        abi::store_u64(&count, &collection, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&count, &collection, COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(&data_bytes, &collection, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&data_bytes, &collection, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate(&entry_cursor, &collection, COLLECTION_HEADER_SIZE),
        abi::move_immediate(&scratch, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&scratch, &count, &scratch),
        abi::add_registers(&data_cursor, &entry_cursor, &scratch),
        abi::move_immediate(&data_offset, "Integer", "0"),
        // Pass 2: fill from index 1.
        abi::move_immediate(&index, "Integer", "1"),
        abi::label(&fill_loop),
        abi::compare_registers(&index, &argc),
        abi::branch_ge(&fill_done),
        abi::shift_left_immediate(&scratch, &index, 3),
        abi::add_registers(&scratch, &argv, &scratch),
        abi::load_u64(&arg_ptr, &scratch, 0),
        abi::move_register(&scan, &arg_ptr),
        abi::move_immediate(&arg_len, "Integer", "0"),
        abi::label(&str_len),
        abi::load_u8(&byte, &scan, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&str_len_done),
        abi::add_immediate(&arg_len, &arg_len, 1),
        abi::add_immediate(&scan, &scan, 1),
        abi::branch(&str_len),
        abi::label(&str_len_done),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8(&scratch, &entry_cursor, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(abi::ZERO, &entry_cursor, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64(abi::ZERO, &entry_cursor, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::store_u64(
            &data_offset,
            &entry_cursor,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ),
        abi::store_u64(
            &arg_len,
            &entry_cursor,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ),
        abi::move_register(&src, &arg_ptr),
        abi::move_immediate(&scratch, "Integer", "0"),
        abi::label(&str_copy),
        abi::compare_registers(&scratch, &arg_len),
        abi::branch_eq(&str_copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &data_cursor, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&data_cursor, &data_cursor, 1),
        abi::add_immediate(&scratch, &scratch, 1),
        abi::branch(&str_copy),
        abi::label(&str_copy_done),
        abi::add_registers(&data_offset, &data_offset, &arg_len),
        abi::add_immediate(&entry_cursor, &entry_cursor, COLLECTION_ENTRY_SIZE),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
        abi::move_register(RESULT_VALUE_REGISTER, &collection),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&alloc_error),
    ]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

#[cfg(test)]
mod resource_path_tests {
    use super::resource_base_offset;
    use crate::target::NativeBuildMode;

    #[test]
    fn base_offset_per_build_mode() {
        // plan-55-B §4.2: kept in lockstep with plan-55-A's resource_output_dir.
        assert_eq!(
            resource_base_offset(NativeBuildMode::Console, "app"),
            (1, String::new())
        );
        assert_eq!(
            resource_base_offset(NativeBuildMode::MacApp, "app"),
            (2, "Resources".to_string())
        );
        assert_eq!(
            resource_base_offset(NativeBuildMode::LinuxApp, "myprog"),
            (2, "share/myprog".to_string())
        );
    }
}
