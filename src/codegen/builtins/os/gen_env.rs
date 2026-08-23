// --- codegen tier imports (migration) ---
use super::gen_shared::*;
use crate::codegen::engine::analysis::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::target::shared::abi;
use crate::target::shared::nir::NirModule;
use std::collections::HashMap;
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
pub(crate) fn os_env_lock_init_hex(family: PlatformFamily) -> String {
    let mut bytes = [0u8; OS_ENV_LOCK_SIZE];
    match family {
        // `_PTHREAD_MUTEX_SIG_init` = 0x32AAABA7, little-endian in the `long __sig`.
        PlatformFamily::MacOS => bytes[0..4].copy_from_slice(&0x32AA_ABA7u32.to_le_bytes()),
        // Linux `PTHREAD_MUTEX_INITIALIZER` is an all-zero `pthread_mutex_t`.
        PlatformFamily::Linux => {}
        // The Windows env/pwd lock is an SRWLOCK, whose valid initial value
        // (`SRWLOCK_INIT`) is all zero — exactly like PTHREAD_MUTEX_INITIALIZER; the
        // Acquire/Release SRW calls the lock/unlock arms emit need no separate init.
        PlatformFamily::Windows => {}
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The env/pwd lock acquire and release function names. POSIX uses the pthread
/// mutex; Windows uses an SRWLOCK (its all-zero `SRWLOCK_INIT` static already lands
/// via `os_env_lock_init_hex`'s Windows arm). plan-66-B.
fn env_lock_fns(family: PlatformFamily) -> (&'static str, &'static str) {
    match family {
        PlatformFamily::Windows => ("AcquireSRWLockExclusive", "ReleaseSRWLockExclusive"),
        _ => ("pthread_mutex_lock", "pthread_mutex_unlock"),
    }
}

/// Acquire the env/pwd lock: `pthread_mutex_lock(&_mfb_rt_os_env_lock)`. Emitted at
/// helper entry, after incoming `String*` arguments have been saved into vregs (the
/// call clobbers all caller-saved registers).
pub(crate) fn emit_env_lock(ctx: &mut EmitCtx) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    push_symbol_address(
        symbol,
        OS_ENV_LOCK_SYMBOL,
        abi::c_arg(0),
        ctx.instructions,
        ctx.relocations,
    );
    let (lock_fn, _) = env_lock_fns(platform.family());
    platform.emit_external_call(
        lock_fn,
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
pub(crate) fn emit_env_unlock_return(ctx: &mut EmitCtx, vregs: &mut Vregs) -> Result<(), String> {
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
        abi::c_arg(0),
        ctx.instructions,
        ctx.relocations,
    );
    let (_, unlock_fn) = env_lock_fns(platform.family());
    platform.emit_external_call(
        unlock_fn,
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

pub(crate) fn lower_get_env(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    with_fallback: bool,
) -> Result<OsBodyParts, String> {
    let not_found = format!("{symbol}_not_found");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let name = vregs.next();
    let fallback = vregs.next();
    let cname = vregs.next();
    let value = vregs.next();
    let mut instructions = vec![abi::move_register(&name, abi::c_arg(0))];
    if with_fallback {
        instructions.push(abi::move_register(&fallback, abi::c_arg(1)));
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
    instructions.push(abi::move_register(abi::c_arg(0), &cname));
    // Windows has no `getenv`: GetEnvironmentVariableW + UTF-16↔UTF-8 marshal,
    // leaving a UTF-8 value C-string pointer (0 = unset) in the return register —
    // the same contract the not-found/build-string tail below expects (plan-66-B).
    if platform.family() == PlatformFamily::Windows {
        platform.emit_env_get(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    } else {
        platform.emit_external_call(
            "getenv",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    }
    instructions.extend([
        // plan-85: getenv's char* return is a C result (`rax`, `%retC`).
        abi::move_register(&value, abi::c_return(0)),
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
            abi::move_immediate(abi::c_arg(1), "Integer", "8"),
            abi::branch_link(ARENA_ALLOC_SYMBOL),
        ]);
        alloc_reloc(symbol, &mut relocations);
        instructions.extend([
            abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
            abi::branch_ne(&alloc_error),
            abi::label(&alloc_ok),
            abi::move_register(&block, abi::mfb_return(1)),
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
        raise_error_into(symbol, "ErrNotFound", &mut instructions, &mut relocations);
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

    Ok((instructions, relocations, 0))
}
