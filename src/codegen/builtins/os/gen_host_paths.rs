//! Shared emitters for the plan-156 host-path family (`os::appResourcePath`,
//! `appDataPath`, `appCachePath`, `userHomePath`, `userDocumentsPath`). Every
//! member has the same contract: reject a `.`/`..` component of `relative`
//! ([`emit_validate_relative`]), resolve a host base, and return
//! `base ++ suffix ++ ("/" ++ relative, only when relative is non-empty)`
//! ([`emit_join_result`]), raising through the shared tails
//! ([`emit_path_error_tails`]).

use super::gen_env::{emit_env_lock, emit_env_unlock_return};
use super::gen_paths::emit_reject_dot_component;
use super::gen_shared::{
    alloc_reloc, emit_copy_counted, emit_store_byte_advance, push_alloc_error, void_result,
};
use crate::codegen::engine::builder::EmitCtx;
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;

/// The captured `relative` argument: the `String` block pointer, its byte length,
/// and its data pointer (`block + 8`). A `String` block is
/// `[8-byte length][bytes][NUL]`.
pub(crate) struct RelativeArg {
    pub(crate) len: String,
    pub(crate) data: String,
}

/// Capture the incoming `String` argument (ARG 0) into vregs. Call it FIRST: the
/// argument register dies at the first external call.
pub(crate) fn emit_capture_relative(
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
) -> RelativeArg {
    let ptr = vregs.next();
    let len = vregs.next();
    let data = vregs.next();
    instructions.extend([
        abi::move_register(&ptr, abi::c_arg(0)),
        abi::load_u64(&len, &ptr, 0),
        abi::add_immediate(&data, &ptr, 8),
    ]);
    RelativeArg { len, data }
}

/// Branch to `bad_arg` when `relative` holds a component that is exactly `.` or
/// `..`. A component ends at `/` on every target and also at `\` on Windows
/// (`windows`), where `\` separates directories to every Win32 path API — so
/// `..\secret` navigates out of the base exactly as `../secret` does (bug-454).
/// A dot inside a filename (`..foo`, `a..b`) is fine.
pub(crate) fn emit_validate_relative(
    symbol: &str,
    arg: &RelativeArg,
    windows: bool,
    bad_arg: &str,
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
) {
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
        abi::compare_registers(&scan_index, &arg.len),
        abi::branch_ge(&validate_end),
        abi::label(&validate_body),
        abi::add_registers(&scan_byte, &arg.data, &scan_index),
        abi::load_u8(&scan_byte, &scan_byte, 0),
        abi::compare_immediate(&scan_byte, "47"), // '/'
        abi::branch_eq(&validate_slash),
    ]);
    if windows {
        instructions.extend([
            abi::compare_immediate(&scan_byte, "92"), // '\' — also a separator on Windows
            abi::branch_eq(&validate_slash),
        ]);
    }
    instructions.extend([abi::branch(&validate_char), abi::label(&validate_slash)]);
    emit_reject_dot_component(
        &comp_len,
        &comp_all_dots,
        bad_arg,
        &check_boundary_ok,
        instructions,
    );
    instructions.extend([
        abi::label(&check_boundary_ok),
        abi::move_immediate(&comp_len, "Integer", "0"),
        abi::move_immediate(&comp_all_dots, "Integer", "1"),
        abi::branch(&validate_next),
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
    let validate_done = format!("{symbol}_validate_done");
    emit_reject_dot_component(
        &comp_len,
        &comp_all_dots,
        bad_arg,
        &validate_done,
        instructions,
    );
    instructions.push(abi::label(&validate_done));
}

/// Build the result `String` `base[..base_len] ++ suffix ++ ("/" ++ relative)`
/// in a fresh arena block and set the OK result, then branch to `done`. The
/// joining `/` exists only for a non-empty `relative`, so an empty one yields the
/// bare base with no trailing `/` (plan-156-A §4.2). `suffix` is compile-time
/// bytes (a build-mode resource suffix, a per-OS directory, the app name) and
/// carries its own leading `/`. An allocation failure branches to `alloc_error`.
///
/// `label_prefix` names this join's labels (a member with two join sites passes a
/// distinct prefix to each); `symbol` is the helper symbol relocations bind to.
///
/// `base_ptr`/`base_len` and the `relative` vregs may be live across the arena
/// call: they are vregs, which the allocator spills across every `bl _mfb_*`
/// (`.ai/compiler.md`, register lifetimes).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_join_result(
    symbol: &str,
    label_prefix: &str,
    base_ptr: &str,
    base_len: &str,
    suffix: &[u8],
    arg: &RelativeArg,
    alloc_error: &str,
    done: &str,
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let base_len = &if suffix.is_empty() {
        emit_root_elided_len(label_prefix, base_ptr, base_len, Some(&arg.len), vregs, instructions)
    } else {
        emit_root_elided_len(label_prefix, base_ptr, base_len, None, vregs, instructions)
    };
    let total_len = vregs.next();
    let join_counted = format!("{label_prefix}_join_counted");
    instructions.extend([
        abi::add_registers(&total_len, base_len, &arg.len),
        abi::add_immediate(&total_len, &total_len, suffix.len()),
        abi::compare_immediate(&arg.len, "0"),
        abi::branch_eq(&join_counted),
        abi::add_immediate(&total_len, &total_len, 1),
        abi::label(&join_counted),
        abi::add_immediate(abi::return_register(), &total_len, 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(symbol, relocations);
    let block = vregs.next();
    let dst = vregs.next();
    let copy_index = vregs.next();
    let copy_byte = vregs.next();
    let copy_src = vregs.next();
    let alloc_ok = format!("{label_prefix}_alloc_ok");
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&block, abi::mfb_return(1)),
        abi::store_u64(&total_len, &block, 0),
        abi::add_immediate(&dst, &block, 8),
    ]);
    emit_copy_counted(
        base_ptr,
        base_len,
        &dst,
        &copy_src,
        &copy_index,
        &copy_byte,
        &format!("{label_prefix}_copy_prefix"),
        instructions,
    );
    for &b in suffix {
        emit_store_byte_advance(b, &dst, &copy_byte, instructions);
    }
    let join_written = format!("{label_prefix}_join_written");
    instructions.extend([
        abi::compare_immediate(&arg.len, "0"),
        abi::branch_eq(&join_written),
    ]);
    emit_store_byte_advance(b'/', &dst, &copy_byte, instructions);
    instructions.push(abi::label(&join_written));
    emit_copy_counted(
        &arg.data,
        &arg.len,
        &dst,
        &copy_src,
        &copy_index,
        &copy_byte,
        &format!("{label_prefix}_copy_arg"),
        instructions,
    );
    instructions.extend([
        abi::store_u8(abi::ZERO, &dst, 0),
        abi::move_register(RESULT_VALUE_REGISTER, &block),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);
}

/// The root base `/` followed by something (a suffix, which starts with `/`, or
/// a joined `relative`): drop its own `/` so the result reads `/x`, not `//x`
/// (a leading `//` is implementation-defined in POSIX). A base can end in `/`
/// only when it IS `/` — every environment- and `passwd`-derived base is trimmed
/// by [`emit_trim_trailing_slashes`], which keeps a lone `/`. Returns the
/// effective length in a fresh vreg; `follows` is a runtime flag vreg (non-zero
/// when something follows), or `None` when a non-empty `suffix` always does.
fn emit_root_elided_len(
    label_prefix: &str,
    base_ptr: &str,
    base_len: &str,
    follows: Option<&str>,
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
) -> String {
    let eff = vregs.next();
    let byte = vregs.next();
    let keep = format!("{label_prefix}_root_keep");
    instructions.push(abi::move_register(&eff, base_len));
    if let Some(follows) = follows {
        instructions.extend([abi::compare_immediate(follows, "0"), abi::branch_eq(&keep)]);
    }
    instructions.extend([
        abi::compare_immediate(&eff, "0"),
        abi::branch_eq(&keep),
        abi::add_registers(&byte, base_ptr, &eff),
        abi::subtract_immediate(&byte, &byte, 1),
        abi::load_u8(&byte, &byte, 0),
        abi::compare_immediate(&byte, "47"), // '/'
        abi::branch_ne(&keep),
        abi::subtract_immediate(&eff, &eff, 1),
        abi::label(&keep),
    ]);
    eff
}

/// The three raise tails every family member shares — `fail` raises
/// `ErrUnsupported` (the host lookup failed), `bad_arg` raises `ErrInvalidPath`
/// (a `.`/`..` component), `alloc_error` the arena failure — each ending at
/// `done`, whose label this emits last. The caller emits what follows `done`: a
/// plain return, or the POSIX env-lock release.
pub(crate) fn emit_path_error_tails(
    symbol: &str,
    fail: &str,
    bad_arg: &str,
    alloc_error: &str,
    done: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::label(fail));
    raise_error_into(symbol, "ErrUnsupported", instructions, relocations);
    instructions.extend([abi::branch(done), abi::label(bad_arg)]);
    raise_error_into(symbol, "ErrInvalidPath", instructions, relocations);
    instructions.extend([abi::branch(done), abi::label(alloc_error)]);
    push_alloc_error(symbol, instructions, relocations);
    instructions.push(abi::label(done));
}

/// A host base as bytes: a pointer vreg and a byte-length vreg. The bytes are not
/// owned — they point into `environ`, a `passwd` record, or a host buffer — so the
/// caller copies them (via [`emit_join_result`]) while still holding whatever keeps
/// them valid (the env/pwd lock on POSIX).
pub(crate) struct BaseBytes {
    pub(crate) ptr: String,
    pub(crate) len: String,
}

/// Frame locals the POSIX lookups use: a NUL-terminated variable name
/// (`XDG_CONFIG_HOME` is the longest at 16 bytes with its NUL) at `sp + 0`.
pub(crate) const HOST_PATH_NAME_SLOT_SIZE: usize = 32;

/// `struct passwd.pw_dir`'s byte offset. macOS (`<pwd.h>`): `pw_name`,
/// `pw_passwd`, `uid_t`+`gid_t`, `time_t pw_change`, `pw_class`, `pw_gecos`, then
/// `pw_dir` at 48. glibc and musl on LP64: `pw_name`, `pw_passwd`, `uid`+`gid`,
/// `pw_gecos`, then `pw_dir` at 32.
fn pw_dir_offset(family: PlatformFamily) -> usize {
    match family {
        PlatformFamily::MacOS => 48,
        _ => 32,
    }
}

/// Write `name` and a NUL into the frame name slot at `sp + 0` and point ARG 0 at it.
fn emit_name_arg(name: &str, vregs: &mut Vregs, instructions: &mut Vec<CodeInstruction>) {
    debug_assert!(name.len() < HOST_PATH_NAME_SLOT_SIZE);
    let byte = vregs.next();
    for (i, b) in name.bytes().chain(std::iter::once(0)).enumerate() {
        instructions.push(abi::move_immediate(&byte, "Byte", &b.to_string()));
        instructions.push(abi::store_u8(&byte, abi::stack_pointer(), i));
    }
    instructions.push(abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), 0));
}

/// Measure a NUL-terminated C string at `ptr` into a fresh length vreg.
fn emit_cstr_len(
    label_prefix: &str,
    ptr: &str,
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
) -> String {
    let len = vregs.next();
    let cursor = vregs.next();
    let byte = vregs.next();
    let lp = format!("{label_prefix}_strlen_loop");
    let done = format!("{label_prefix}_strlen_done");
    instructions.extend([
        abi::move_immediate(&len, "Integer", "0"),
        abi::move_register(&cursor, ptr),
        abi::label(&lp),
        abi::load_u8(&byte, &cursor, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&done),
        abi::add_immediate(&len, &len, 1),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::branch(&lp),
        abi::label(&done),
    ]);
    len
}

/// Drop trailing `/` bytes from an environment- or `passwd`-derived base, keeping
/// a lone `/`: `HOME=/tmp/h/` joins as `/tmp/h/...`, not `/tmp/h//...`.
pub(crate) fn emit_trim_trailing_slashes(
    label_prefix: &str,
    base: &BaseBytes,
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
) {
    let byte = vregs.next();
    let lp = format!("{label_prefix}_trim_loop");
    let done = format!("{label_prefix}_trim_done");
    instructions.extend([
        abi::label(&lp),
        abi::compare_immediate(&base.len, "1"),
        abi::branch_le(&done),
        abi::add_registers(&byte, &base.ptr, &base.len),
        abi::subtract_immediate(&byte, &byte, 1),
        abi::load_u8(&byte, &byte, 0),
        abi::compare_immediate(&byte, "47"), // '/'
        abi::branch_ne(&done),
        abi::subtract_immediate(&base.len, &base.len, 1),
        abi::branch(&lp),
        abi::label(&done),
    ]);
}

/// The POSIX home directory (plan-156-B §4.1): `$HOME` when set and non-empty,
/// else `getpwuid(getuid())->pw_dir`; `fail` when neither yields a non-empty
/// path. Trailing `/` trimmed. The caller holds the env/pwd lock across this AND
/// the copy of the returned bytes: `getenv`'s pointer is into `environ` (which a
/// concurrent `os::setEnv` may relocate, bug-64) and `getpwuid`'s into a static
/// record a concurrent `getpwuid` overwrites. Needs [`HOST_PATH_NAME_SLOT_SIZE`]
/// frame locals.
pub(crate) fn emit_posix_home_base(
    ctx: &mut EmitCtx,
    label_prefix: &str,
    fail: &str,
    vregs: &mut Vregs,
) -> Result<BaseBytes, String> {
    let ptr = vregs.next();
    let byte = vregs.next();
    let use_pw = format!("{label_prefix}_home_pw");
    let have = format!("{label_prefix}_home_have");
    emit_name_arg("HOME", vregs, ctx.instructions);
    ctx.platform.emit_external_call(
        "getenv",
        ctx.symbol,
        ctx.platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        // getenv's char* is a C result (`%retC`, plan-85).
        abi::move_register(&ptr, abi::c_return(0)),
        abi::compare_immediate(&ptr, "0"),
        abi::branch_eq(&use_pw),
        abi::load_u8(&byte, &ptr, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_ne(&have),
        abi::label(&use_pw),
    ]);
    ctx.platform.emit_external_call(
        "getuid",
        ctx.symbol,
        ctx.platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions
        .push(abi::move_register(abi::c_arg(0), abi::c_return(0)));
    ctx.platform.emit_external_call(
        "getpwuid",
        ctx.symbol,
        ctx.platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    let pw_ok = format!("{label_prefix}_home_pw_ok");
    ctx.instructions.extend([
        abi::move_register(&ptr, abi::c_return(0)),
        abi::compare_immediate(&ptr, "0"),
        abi::branch_ne(&pw_ok),
        abi::branch(fail),
        abi::label(&pw_ok),
        abi::load_u64(&ptr, &ptr, pw_dir_offset(ctx.platform.family())),
        abi::compare_immediate(&ptr, "0"),
        abi::branch_eq(fail),
        abi::load_u8(&byte, &ptr, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(fail),
        abi::label(&have),
    ]);
    let len = emit_cstr_len(
        &format!("{label_prefix}_home"),
        &ptr,
        vregs,
        ctx.instructions,
    );
    let base = BaseBytes { ptr, len };
    emit_trim_trailing_slashes(
        &format!("{label_prefix}_home"),
        &base,
        vregs,
        ctx.instructions,
    );
    Ok(base)
}

/// An XDG-style environment variable used only when set, non-empty and absolute
/// (it starts with `/`); otherwise branch to `not_found`. The XDG Base Directory
/// spec says a relative value "should [be considered] invalid and ignore[d]".
/// Trailing `/` trimmed. Same lock and frame requirements as
/// [`emit_posix_home_base`].
pub(crate) fn emit_posix_env_abs_base(
    ctx: &mut EmitCtx,
    label_prefix: &str,
    name: &str,
    not_found: &str,
    vregs: &mut Vregs,
) -> Result<BaseBytes, String> {
    let ptr = vregs.next();
    let byte = vregs.next();
    emit_name_arg(name, vregs, ctx.instructions);
    ctx.platform.emit_external_call(
        "getenv",
        ctx.symbol,
        ctx.platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::move_register(&ptr, abi::c_return(0)),
        abi::compare_immediate(&ptr, "0"),
        abi::branch_eq(not_found),
        abi::load_u8(&byte, &ptr, 0),
        abi::compare_immediate(&byte, "47"), // '/': absolute (also rules out "")
        abi::branch_ne(not_found),
    ]);
    let len = emit_cstr_len(
        &format!("{label_prefix}_env"),
        &ptr,
        vregs,
        ctx.instructions,
    );
    let base = BaseBytes { ptr, len };
    emit_trim_trailing_slashes(
        &format!("{label_prefix}_env"),
        &base,
        vregs,
        ctx.instructions,
    );
    Ok(base)
}

/// A per-user directory the host designates, resolved per OS by
/// [`lower_host_dir`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostDir {
    /// `os::appDataPath`: this app's data, `/<module name>` appended.
    AppData,
    /// `os::appCachePath`: this app's regenerable cache, `/<module name>` appended.
    AppCache,
    /// `os::userHomePath`: the user's home folder (plan-156-C).
    UserHome,
    /// `os::userDocumentsPath`: the user's Documents folder (plan-156-C).
    UserDocuments,
}

impl HostDir {
    /// The `emit_os_wide_string` / `emit_known_folder_into` query naming this
    /// directory's Windows known folder.
    fn windows_query(self) -> &'static str {
        match self {
            HostDir::AppData => "appData",
            HostDir::AppCache => "appCache",
            HostDir::UserHome => "userHome",
            HostDir::UserDocuments => "userDocuments",
        }
    }

    /// Frame locals the POSIX body needs: the name slot, plus — for the Linux
    /// Documents lookup — the `user-dirs.dirs` parser's buffers.
    fn posix_frame_locals(self, family: PlatformFamily) -> usize {
        if self == HostDir::UserDocuments && family == PlatformFamily::Linux {
            super::gen_user_dirs::USER_DIRS_FRAME_LOCALS
        } else {
            HOST_PATH_NAME_SLOT_SIZE
        }
    }
}

/// Where a resolved base goes: the public call joins `relative` onto it in a
/// fresh `String` ([`emit_join_result`]); the internal `*Base` helper the in-place
/// arm calls writes it into a caller buffer ([`emit_into_result`]).
enum Sink {
    Join { arg: RelativeArg },
    Into { dst: String, cap: String },
}

/// The `*Base` helper's result (plan-156-B §4.4): `base ++ suffix` written to
/// `dst` when it fits `cap`, and its length returned either way — the arm reserves
/// that much scratch and asks again. Writes no NUL; allocates nothing.
#[allow(clippy::too_many_arguments)]
fn emit_into_result(
    label_prefix: &str,
    base_ptr: &str,
    base_len: &str,
    suffix: &[u8],
    dst: &str,
    cap: &str,
    done: &str,
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
) {
    // With nothing after it the root stays `/`; the arm then joins a value onto
    // it without a second `/` (`GrowKind::HostPath`), matching the copying call.
    let base_len = &if suffix.is_empty() {
        base_len.to_string()
    } else {
        emit_root_elided_len(label_prefix, base_ptr, base_len, None, vregs, instructions)
    };
    let total = vregs.next();
    let out = vregs.next();
    let copy_src = vregs.next();
    let copy_index = vregs.next();
    let copy_byte = vregs.next();
    let skip = format!("{label_prefix}_into_skip");
    instructions.extend([
        abi::add_immediate(&total, base_len, suffix.len()),
        abi::compare_registers(&total, cap),
        abi::branch_gt(&skip),
        abi::move_register(&out, dst),
    ]);
    emit_copy_counted(
        base_ptr,
        base_len,
        &out,
        &copy_src,
        &copy_index,
        &copy_byte,
        &format!("{label_prefix}_into_copy"),
        instructions,
    );
    for &b in suffix {
        emit_store_byte_advance(b, &out, &copy_byte, instructions);
    }
    instructions.extend([
        abi::label(&skip),
        abi::move_register(RESULT_VALUE_REGISTER, &total),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);
}

/// Route a resolved base to `sink`.
#[allow(clippy::too_many_arguments)]
fn emit_sink(
    symbol: &str,
    label_prefix: &str,
    base: &BaseBytes,
    suffix: &[u8],
    sink: &Sink,
    alloc_error: &str,
    done: &str,
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    match sink {
        Sink::Join { arg } => emit_join_result(
            symbol,
            label_prefix,
            &base.ptr,
            &base.len,
            suffix,
            arg,
            alloc_error,
            done,
            vregs,
            instructions,
            relocations,
        ),
        Sink::Into { dst, cap } => emit_into_result(
            label_prefix,
            &base.ptr,
            &base.len,
            suffix,
            dst,
            cap,
            done,
            vregs,
            instructions,
        ),
    }
}

/// The `*Base` helper's one failure: the host could not answer. It returns -1
/// (never raises); the arm raises `ErrUnsupported` under the public member's name.
fn emit_into_fail_tail(fail: &str, done: &str, instructions: &mut Vec<CodeInstruction>) {
    instructions.extend([
        abi::label(fail),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::subtract_immediate(RESULT_VALUE_REGISTER, RESULT_VALUE_REGISTER, 1),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(done),
    ]);
}

/// Resolve `dir` on macOS / Linux into `sink` (plan-156-B §4.3, plan-156-C).
/// The caller holds the env/pwd lock.
///
/// - `AppData` / `AppCache`: macOS `<home>/Library/Application Support/<name>` /
///   `<home>/Library/Caches/<name>`; Linux `$XDG_DATA_HOME/<name>` /
///   `$XDG_CACHE_HOME/<name>` when the variable is set, non-empty and absolute,
///   else `<home>/.local/share/<name>` / `<home>/.cache/<name>`.
/// - `UserHome`: `<home>`.
/// - `UserDocuments`: macOS `<home>/Documents`; Linux the `user-dirs.dirs`
///   folder (`gen_user_dirs.rs`), else `<home>/Documents`.
#[allow(clippy::too_many_arguments)]
fn emit_posix_host_dir(
    ctx: &AbiCtx,
    symbol: &str,
    dir: HostDir,
    sink: &Sink,
    fail: &str,
    alloc_error: &str,
    done: &str,
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let name = ctx.module_name;
    let linux = ctx.platform.family() == PlatformFamily::Linux;
    if linux && matches!(dir, HostDir::AppData | HostDir::AppCache) {
        let var = match dir {
            HostDir::AppData => "XDG_DATA_HOME",
            _ => "XDG_CACHE_HOME",
        };
        let xdg_prefix = format!("{symbol}_xdg");
        let xdg_missing = format!("{symbol}_xdg_missing");
        let xdg = emit_posix_env_abs_base(
            &mut EmitCtx {
                symbol,
                platform_imports: ctx.platform_imports,
                platform: ctx.platform,
                instructions,
                relocations,
            },
            &xdg_prefix,
            var,
            &xdg_missing,
            vregs,
        )?;
        emit_sink(
            symbol,
            &xdg_prefix,
            &xdg,
            format!("/{name}").as_bytes(),
            sink,
            alloc_error,
            done,
            vregs,
            instructions,
            relocations,
        );
        instructions.push(abi::label(&xdg_missing));
    }
    let home_prefix = format!("{symbol}_home");
    let home = emit_posix_home_base(
        &mut EmitCtx {
            symbol,
            platform_imports: ctx.platform_imports,
            platform: ctx.platform,
            instructions,
            relocations,
        },
        &home_prefix,
        fail,
        vregs,
    )?;
    if linux && dir == HostDir::UserDocuments {
        let docs_prefix = format!("{symbol}_docs");
        let found = format!("{symbol}_docs_found");
        let fallback = format!("{symbol}_docs_fallback");
        let docs = super::gen_user_dirs::emit_linux_user_dirs_documents(
            &mut EmitCtx {
                symbol,
                platform_imports: ctx.platform_imports,
                platform: ctx.platform,
                instructions,
                relocations,
            },
            &docs_prefix,
            &home,
            &found,
            &fallback,
            vregs,
        )?;
        instructions.push(abi::label(&found));
        emit_sink(
            symbol,
            &docs_prefix,
            &docs,
            b"",
            sink,
            alloc_error,
            done,
            vregs,
            instructions,
            relocations,
        );
        instructions.push(abi::label(&fallback));
    }
    let home_suffix = match (dir, linux) {
        (HostDir::AppData, true) => format!("/.local/share/{name}"),
        (HostDir::AppCache, true) => format!("/.cache/{name}"),
        (HostDir::AppData, false) => format!("/Library/Application Support/{name}"),
        (HostDir::AppCache, false) => format!("/Library/Caches/{name}"),
        (HostDir::UserHome, _) => String::new(),
        (HostDir::UserDocuments, _) => "/Documents".to_string(),
    };
    emit_sink(
        symbol,
        &home_prefix,
        &home,
        home_suffix.as_bytes(),
        sink,
        alloc_error,
        done,
        vregs,
        instructions,
        relocations,
    );
    Ok(())
}

/// `os::appDataPath` / `os::appCachePath` (`into == false`, the public call:
/// `relative` joined on, a fresh `String`, `ErrInvalidPath` / `ErrUnsupported`)
/// and their internal `*Base` helpers (`into == true`: `(dst, cap) -> length`,
/// the base written to `dst` when it fits, -1 on failure, nothing allocated —
/// what the in-place `s = os::appDataPath(s)` arm calls, plan-156-B §4.4).
///
/// Windows reads the known folder (`FOLDERID_RoamingAppData` /
/// `FOLDERID_LocalAppData`) and appends `/<name>`. On POSIX the env/pwd lock is
/// taken right after the argument capture — BEFORE validation — so every path to
/// `done`, the `ErrInvalidPath` one included, releases a lock it holds. Nothing
/// is created and nothing is checked for existence.
pub(crate) fn lower_host_dir(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
    dir: HostDir,
    into: bool,
    call: &str,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let symbol = symbol.as_str();
    let family = ctx.platform.family();
    let name = ctx.module_name;
    let fail = format!("{symbol}_fail");
    let bad_arg = format!("{symbol}_bad_arg");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let mut instructions = Vec::new();
    let mut relocations = Vec::new();
    let sink = if into {
        let dst = vregs.next();
        let cap = vregs.next();
        instructions.extend([
            abi::move_register(&dst, abi::c_arg(0)),
            abi::move_register(&cap, abi::c_arg(1)),
        ]);
        Sink::Into { dst, cap }
    } else {
        Sink::Join {
            arg: emit_capture_relative(&mut vregs, &mut instructions),
        }
    };
    let stack_size = match family {
        PlatformFamily::MacOS | PlatformFamily::Linux => {
            emit_env_lock(&mut EmitCtx {
                symbol,
                platform_imports: ctx.platform_imports,
                platform: ctx.platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            })?;
            if let Sink::Join { arg } = &sink {
                emit_validate_relative(symbol, arg, false, &bad_arg, &mut vregs, &mut instructions);
            }
            emit_posix_host_dir(
                ctx,
                symbol,
                dir,
                &sink,
                &fail,
                &alloc_error,
                &done,
                &mut vregs,
                &mut instructions,
                &mut relocations,
            )?;
            match &sink {
                Sink::Join { .. } => emit_path_error_tails(
                    symbol,
                    &fail,
                    &bad_arg,
                    &alloc_error,
                    &done,
                    &mut instructions,
                    &mut relocations,
                ),
                Sink::Into { .. } => emit_into_fail_tail(&fail, &done, &mut instructions),
            }
            emit_env_unlock_return(
                &mut EmitCtx {
                    symbol,
                    platform_imports: ctx.platform_imports,
                    platform: ctx.platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                &mut vregs,
            )?;
            dir.posix_frame_locals(family)
        }
        PlatformFamily::Windows => {
            // The app directories append the module name; the user folders are
            // the known folder itself.
            let suffix = match dir {
                HostDir::AppData | HostDir::AppCache => format!("/{name}"),
                HostDir::UserHome | HostDir::UserDocuments => String::new(),
            };
            match &sink {
                Sink::Join { arg } => {
                    emit_validate_relative(
                        symbol,
                        arg,
                        true,
                        &bad_arg,
                        &mut vregs,
                        &mut instructions,
                    );
                    ctx.platform.emit_os_wide_string(
                        dir.windows_query(),
                        symbol,
                        ctx.platform_imports,
                        &mut instructions,
                        &mut relocations,
                    )?;
                    let ptr = vregs.next();
                    instructions.extend([
                        abi::move_register(&ptr, abi::return_register()),
                        abi::compare_immediate(&ptr, "0"),
                        abi::branch_eq(&fail),
                    ]);
                    let len = emit_cstr_len(symbol, &ptr, &mut vregs, &mut instructions);
                    emit_sink(
                        symbol,
                        symbol,
                        &BaseBytes { ptr, len },
                        suffix.as_bytes(),
                        &sink,
                        &alloc_error,
                        &done,
                        &mut vregs,
                        &mut instructions,
                        &mut relocations,
                    );
                    emit_path_error_tails(
                        symbol,
                        &fail,
                        &bad_arg,
                        &alloc_error,
                        &done,
                        &mut instructions,
                        &mut relocations,
                    );
                }
                Sink::Into { dst, cap } => {
                    emit_windows_known_folder_into(
                        ctx,
                        symbol,
                        dir.windows_query(),
                        suffix.as_bytes(),
                        dst,
                        cap,
                        &fail,
                        &done,
                        &mut vregs,
                        &mut instructions,
                        &mut relocations,
                    )?;
                    emit_into_fail_tail(&fail, &done, &mut instructions);
                }
            }
            instructions.push(abi::return_());
            0
        }
    };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(void_result(call))
}

/// The Windows `*Base` body: the known folder written straight into `dst`
/// through `CodegenPlatform::emit_known_folder_into` (no arena buffer), then
/// `suffix` after it. The `*Base` contract (shared with POSIX,
/// [`emit_into_result`]) is "the result was written exactly when the returned
/// length is <= `cap`", so the in-place arm's reserve-and-retry loop can trust it.
///
/// The hook writes the folder and a NUL only when `folder_len + 1 <= cap'`:
/// - with a non-empty `suffix`, `cap' = cap + 1 - suffix.len()` (0 when that is
///   negative), so the hook writes exactly when `folder ++ suffix` fits `cap` —
///   the NUL lands where `suffix`'s first byte then goes;
/// - with an empty `suffix` (the user folders) the NUL needs a byte of its own,
///   so `cap' = cap`, and when it did not fit the helper returns `folder_len + 1`
///   (> `cap`) so the caller asks again with room for it.
#[allow(clippy::too_many_arguments)]
fn emit_windows_known_folder_into(
    ctx: &AbiCtx,
    symbol: &str,
    query: &str,
    suffix: &[u8],
    dst: &str,
    cap: &str,
    fail: &str,
    done: &str,
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let hook_cap = vregs.next();
    let folder_len = vregs.next();
    let total = vregs.next();
    let out = vregs.next();
    let byte = vregs.next();
    let cap_ready = format!("{symbol}_kf_cap_ready");
    let skip = format!("{symbol}_kf_into_skip");
    let report = format!("{symbol}_kf_into_report");
    if suffix.is_empty() {
        instructions.push(abi::move_register(&hook_cap, cap));
    } else {
        instructions.extend([
            abi::add_immediate(&hook_cap, cap, 1),
            abi::subtract_immediate(&hook_cap, &hook_cap, suffix.len()),
            abi::compare_immediate(&hook_cap, "0"),
            abi::branch_ge(&cap_ready),
            abi::move_immediate(&hook_cap, "Integer", "0"),
            abi::label(&cap_ready),
        ]);
    }
    instructions.extend([
        abi::move_register(abi::c_arg(0), dst),
        abi::move_register(abi::c_arg(1), &hook_cap),
    ]);
    ctx.platform.emit_known_folder_into(
        query,
        symbol,
        ctx.platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::move_register(&folder_len, abi::return_register()),
        abi::compare_immediate(&folder_len, "0"),
        abi::branch_lt(fail),
    ]);
    if suffix.is_empty() {
        // Written iff `folder_len + 1 <= cap`; otherwise report one more than
        // was needed-without-NUL, which is > `cap`.
        instructions.extend([
            abi::move_register(&total, &folder_len),
            abi::add_immediate(&out, &folder_len, 1),
            abi::compare_registers(&out, cap),
            abi::branch_le(&report),
            abi::move_register(&total, &out),
            abi::label(&report),
            abi::label(&skip),
        ]);
    } else {
        instructions.extend([
            abi::add_immediate(&total, &folder_len, suffix.len()),
            abi::compare_registers(&total, cap),
            abi::branch_gt(&skip),
            abi::add_registers(&out, dst, &folder_len),
        ]);
        for &b in suffix {
            emit_store_byte_advance(b, &out, &byte, instructions);
        }
        instructions.push(abi::label(&skip));
    }
    instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, &total),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);
    Ok(())
}
