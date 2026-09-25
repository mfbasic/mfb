//! Shared emitters for the plan-156 host-path family (`os::appResourcePath`,
//! `appDataPath`, `appCachePath`, `userHomePath`, `userDocumentsPath`). Every
//! member has the same contract: reject a `.`/`..` component of `relative`
//! ([`emit_validate_relative`]), resolve a host base, and return
//! `base ++ suffix ++ ("/" ++ relative, only when relative is non-empty)`
//! ([`emit_join_result`]), raising through the shared tails
//! ([`emit_path_error_tails`]).

use super::gen_paths::emit_reject_dot_component;
use super::gen_shared::{alloc_reloc, emit_copy_counted, emit_store_byte_advance, push_alloc_error};
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
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
/// `base_ptr`/`base_len` and the `relative` vregs may be live across the arena
/// call: they are vregs, which the allocator spills across every `bl _mfb_*`
/// (`.ai/compiler.md`, register lifetimes).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_join_result(
    symbol: &str,
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
    let total_len = vregs.next();
    let join_counted = format!("{symbol}_join_counted");
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
    let alloc_ok = format!("{symbol}_alloc_ok");
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
        &format!("{symbol}_copy_prefix"),
        instructions,
    );
    for &b in suffix {
        emit_store_byte_advance(b, &dst, &copy_byte, instructions);
    }
    let join_written = format!("{symbol}_join_written");
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
        &format!("{symbol}_copy_arg"),
        instructions,
    );
    instructions.extend([
        abi::store_u8(abi::ZERO, &dst, 0),
        abi::move_register(RESULT_VALUE_REGISTER, &block),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
    ]);
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
