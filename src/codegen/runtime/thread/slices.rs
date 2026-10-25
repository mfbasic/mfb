//! plan-183-B: each `thread::start` takes two slices of the arena reservation — one for
//! the worker's arena and one for the control block's arena — and the control block's
//! final release gives them back.
//!
//! The slice bitmap (`_mfb_rt_arena_slice_bits`) is guarded by the thread-table mutex,
//! which is taken at exactly the same two moments. The fixed slices (the entry
//! thread's, a canvas program's graphics thread's) are marked taken at startup and
//! never released. A start that finds fewer than two free slices raises
//! `ErrThreadLimit` before anything is created.
//!
//! A pair travels as one word: `(worker + 1) | (block + 1) << 32`, so 0 means none.

use crate::codegen::engine::types::*;
use crate::codegen::engine::util::{finalize_vreg_helper, Vregs};
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::runtime::arena_cap::{
    ARENA_BASE_SYMBOL, ARENA_SLICE_BITS_SYMBOL, ARENA_SLICE_SIZE_SYMBOL,
};
use crate::codegen::runtime::thread::table_lock;
use crate::target::shared::abi;
use std::collections::HashMap;

/// `_mfb_rt_thread_slices_acquire()`: the packed pair in the value register (tag
/// `Ok`), or `ErrThreadLimit`.
pub(crate) const THREAD_SLICES_ACQUIRE_SYMBOL: &str = "_mfb_rt_thread_slices_acquire";
/// `_mfb_rt_thread_slices_release(pair)`: clear both slices' bits (a zero half is
/// skipped). The arenas on them were decommitted first.
pub(crate) const THREAD_SLICES_RELEASE_SYMBOL: &str = "_mfb_rt_thread_slices_release";
/// `ErrThreadLimit`'s message, which names this build's `maxThreads`.
pub(crate) const THREAD_LIMIT_MESSAGE_SYMBOL: &str = "_mfb_rt_thread_limit_message";

/// The message `ErrThreadLimit` carries in a program built with `max_threads`.
pub(crate) fn thread_limit_message(max_threads: u64) -> String {
    format!(
        "{max_threads} threads are already running; raise options.maxThreads in project.json \
         to allow more"
    )
}

pub(crate) fn thread_limit_data_object(max_threads: u64) -> CodeDataObject {
    string_data_object(
        THREAD_LIMIT_MESSAGE_SYMBOL,
        thread_limit_message(max_threads),
    )
}

/// `_mfb_rt_thread_slices_acquire`: the two lowest free slices from `fixed` up to
/// `slices`, marked taken under the table lock.
pub(crate) fn lower_thread_slices_acquire(
    slices: u64,
    fixed: u64,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = THREAD_SLICES_ACQUIRE_SYMBOL;
    let mut vregs = Vregs::new();
    let bits = vregs.next();
    let index = vregs.next();
    let first = vregs.next();
    let second = vregs.next();
    let word_addr = vregs.next();
    let word = vregs.next();
    let bit = vregs.next();
    let scratch = vregs.next();
    let scan = format!("{symbol}_scan");
    let next = format!("{symbol}_next");
    let second_found = format!("{symbol}_take");
    let limit = format!("{symbol}_limit");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    table_lock(
        symbol,
        "pthread_mutex_lock",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    push_symbol_address(
        symbol,
        ARENA_SLICE_BITS_SYMBOL,
        &bits,
        &mut instructions,
        &mut relocations,
    );
    // word_addr = bits + (index >> 6) * 8; bit = 1 << (index & 63).
    let locate = |instructions: &mut Vec<CodeInstruction>,
                  of: &crate::codegen::engine::operand::VirtualRegister| {
        instructions.extend([
            abi::shift_right_immediate(&word_addr, of, 6),
            abi::shift_left_immediate(&word_addr, &word_addr, 3),
            abi::add_registers(&word_addr, &bits, &word_addr),
            abi::move_immediate(&scratch, "Integer", "63"),
            abi::and_registers(&scratch, of, &scratch),
            abi::move_immediate(&bit, "Integer", "1"),
            abi::shift_left_variable(&bit, &bit, &scratch),
        ]);
    };
    instructions.extend([
        abi::move_immediate(&first, "Integer", "0"),
        abi::move_immediate(&index, "Integer", &fixed.to_string()),
        abi::label(&scan),
        abi::compare_immediate(&index, &slices.to_string()),
        abi::branch_lo(&format!("{symbol}_in_range")),
        abi::branch(&limit),
        abi::label(&format!("{symbol}_in_range")),
    ]);
    locate(&mut instructions, &index);
    instructions.extend([
        abi::load_u64(&word, &word_addr, 0),
        abi::and_registers(&word, &word, &bit),
        abi::compare_immediate(&word, "0"),
        abi::branch_ne(&next),
        abi::compare_immediate(&first, "0"),
        abi::branch_ne(&second_found),
        abi::add_immediate(&first, &index, 1),
        abi::label(&next),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&scan),
        abi::label(&second_found),
        abi::add_immediate(&second, &index, 1),
    ]);
    // Take both: first - 1 and second - 1.
    for taken in [&first, &second] {
        instructions.push(abi::subtract_immediate(&index, taken, 1));
        locate(&mut instructions, &index);
        instructions.extend([
            abi::load_u64(&word, &word_addr, 0),
            abi::or_registers(&word, &word, &bit),
            abi::store_u64(&word, &word_addr, 0),
        ]);
    }
    table_lock(
        symbol,
        "pthread_mutex_unlock",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::shift_left_immediate(&second, &second, 32),
        abi::or_registers(RESULT_VALUE_REGISTER, &first, &second),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&limit),
    ]);
    table_lock(
        symbol,
        "pthread_mutex_unlock",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    let (code, _) = crate::codegen::registry::runtime_error_emission("ErrThreadLimit")
        .expect("ErrThreadLimit is an errorCode constant");
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", code),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        THREAD_LIMIT_MESSAGE_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok(finalize_vreg_helper(
        "runtime.thread_slices_acquire",
        symbol,
        "Integer",
        instructions,
        relocations,
    ))
}

/// `_mfb_rt_thread_slices_release(pair)`: clear the pair's bits under the table lock.
pub(crate) fn lower_thread_slices_release(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = THREAD_SLICES_RELEASE_SYMBOL;
    let mut vregs = Vregs::new();
    let pair = vregs.next();
    let bits = vregs.next();
    let index = vregs.next();
    let word_addr = vregs.next();
    let word = vregs.next();
    let bit = vregs.next();
    let scratch = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&pair, abi::c_arg(0)),
    ];
    let mut relocations = Vec::new();
    table_lock(
        symbol,
        "pthread_mutex_lock",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    push_symbol_address(
        symbol,
        ARENA_SLICE_BITS_SYMBOL,
        &bits,
        &mut instructions,
        &mut relocations,
    );
    for (half, skip) in [(0u8, "low"), (32u8, "high")] {
        let skip = format!("{symbol}_{skip}_none");
        instructions.extend([
            abi::shift_right_immediate(&index, &pair, half),
            abi::move_immediate(&scratch, "Integer", "4294967295"),
            abi::and_registers(&index, &index, &scratch),
            abi::compare_immediate(&index, "0"),
            abi::branch_eq(&skip),
            abi::subtract_immediate(&index, &index, 1),
            abi::shift_right_immediate(&word_addr, &index, 6),
            abi::shift_left_immediate(&word_addr, &word_addr, 3),
            abi::add_registers(&word_addr, &bits, &word_addr),
            abi::move_immediate(&scratch, "Integer", "63"),
            abi::and_registers(&scratch, &index, &scratch),
            abi::move_immediate(&bit, "Integer", "1"),
            abi::shift_left_variable(&bit, &bit, &scratch),
            abi::bitwise_not(&bit, &bit),
            abi::load_u64(&word, &word_addr, 0),
            abi::and_registers(&word, &word, &bit),
            abi::store_u64(&word, &word_addr, 0),
            abi::label(&skip),
        ]);
    }
    table_lock(
        symbol,
        "pthread_mutex_unlock",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::return_());
    Ok(finalize_vreg_helper(
        "runtime.thread_slices_release",
        symbol,
        "Nothing",
        instructions,
        relocations,
    ))
}

/// Point the arena state at `state` (a vreg) at slice `index` (a vreg holding the
/// index, not the packed form): base `reservation + index × sliceSize`, nothing
/// committed. `a`/`b` are scratch vregs. Call-free.
pub(crate) fn emit_assign_slice(
    from: &str,
    state: &crate::codegen::engine::operand::VirtualRegister,
    index: &crate::codegen::engine::operand::VirtualRegister,
    a: &crate::codegen::engine::operand::VirtualRegister,
    b: &crate::codegen::engine::operand::VirtualRegister,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    push_symbol_address(from, ARENA_SLICE_SIZE_SYMBOL, a, instructions, relocations);
    push_symbol_address(from, ARENA_BASE_SYMBOL, b, instructions, relocations);
    instructions.extend([
        abi::load_u64(a, a, 0),
        abi::load_u64(b, b, 0),
        abi::multiply_registers(a, a, index),
        abi::add_registers(b, b, a),
        abi::store_u64(b, state, ARENA_SLICE_BASE_OFFSET),
    ]);
}

/// Point the arena state `state` at one half of the packed pair in the frame word
/// `pair_slot`: the high (control-block) slice or the low (worker) one. `scratch` are
/// three vregs the call clobbers. Call-free.
pub(crate) fn emit_assign_pair_slice(
    from: &str,
    state: &crate::codegen::engine::operand::VirtualRegister,
    pair_slot: usize,
    high: bool,
    scratch: [&crate::codegen::engine::operand::VirtualRegister; 3],
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let [index, a, b] = scratch;
    instructions.push(abi::load_u64(index, abi::stack_pointer(), pair_slot));
    if high {
        instructions.push(abi::shift_right_immediate(index, index, 32));
    } else {
        instructions.extend([
            abi::move_immediate(a, "Integer", "4294967295"),
            abi::and_registers(index, index, a),
        ]);
    }
    instructions.push(abi::subtract_immediate(index, index, 1));
    emit_assign_slice(from, state, index, a, b, instructions, relocations);
}
