//! Native code generation for the `crypto::` runtime helpers
//! (plan-04-crypto.md §A.6). Only `crypto::randomBytes` is lowered here today:
//! it fills a fresh `List OF Byte` from OS entropy via `getentropy`, which is
//! present and non-deprecated on both macOS and Linux (glibc and musl). The
//! remaining `crypto` primitives are portable software cores in
//! `crypto_package.mfb`; the NIST-EC public-key operations bind the platform key
//! APIs (a later phase).
//!
//! `getentropy(buf, len)` accepts at most 256 bytes per call, so the fill runs
//! in <=256-byte chunks. A negative `count` fails `ErrInvalidArgument`; `count`
//! of 0 returns the empty list.

use std::collections::HashMap;

use super::*;
use crate::target::shared::abi;

const GETENTROPY_MAX: usize = 256;

/// Upper bound on `crypto::randomBytes(count)`. Far above any real key-material
/// request (16 MiB), it caps the `count * ENTRY + HEADER + count` collection-size
/// arithmetic well below a u64 overflow and rejects an absurd allocation before it
/// is attempted (bug-177 D). A larger ask is reported as an invalid argument.
const RANDOM_BYTES_MAX_COUNT: usize = 16 * 1024 * 1024;

pub(super) fn lower_crypto_random_bytes_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    // Frame slots (sp-relative locals below the spill area).
    const COUNT_OFFSET: usize = 0; // requested byte count
    const BUF_OFFSET: usize = 8; // scratch entropy buffer pointer
    const OFF_OFFSET: usize = 16; // fill cursor
    const COLLECTION_OFFSET: usize = 24; // the List OF Byte being built
    const LOCAL_SIZE: usize = 32;

    let invalid = format!("{symbol}_invalid");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let entropy_fail = format!("{symbol}_entropy_fail");
    let fill_loop = format!("{symbol}_fill_loop");
    let fill_done = format!("{symbol}_fill_done");
    let chunk_ok = format!("{symbol}_chunk_ok");
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();

    // Validate 0 <= count <= RANDOM_BYTES_MAX_COUNT and stash it. The upper bound
    // rejects an absurd request before the count*ENTRY + HEADER + count size
    // arithmetic below can overflow (bug-177 D); the cap is materialized into a
    // vreg so the compare is size-safe on every backend.
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&invalid),
        abi::move_immediate("%v9", "Integer", &RANDOM_BYTES_MAX_COUNT.to_string()),
        abi::compare_registers(abi::return_register(), "%v9"),
        abi::branch_gt(&invalid),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), COUNT_OFFSET),
        // Allocate a scratch buffer of `count` bytes (arena_alloc rounds up, so a
        // zero request still yields a valid pointer we simply never read). `count`
        // already sits in return_register() == ARG[0] for the alloc (bug-138
        // removed a dead x0<-x0 self-move here).
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    emit_arena_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::RET[1], abi::stack_pointer(), BUF_OFFSET),
        // Fill the buffer from OS entropy in <=256-byte chunks.
        abi::move_immediate("%v9", "Integer", "0"),
        abi::store_u64("%v9", abi::stack_pointer(), OFF_OFFSET),
        abi::label(&fill_loop),
        abi::load_u64("%v9", abi::stack_pointer(), OFF_OFFSET),
        abi::load_u64("%v10", abi::stack_pointer(), COUNT_OFFSET),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_ge(&fill_done),
        // chunk = min(count - off, 256)
        abi::subtract_registers("%v11", "%v10", "%v9"),
        abi::move_immediate("%v12", "Integer", &GETENTROPY_MAX.to_string()),
        abi::compare_registers("%v11", "%v12"),
        abi::branch_le(&chunk_ok),
        abi::move_register("%v11", "%v12"),
        abi::label(&chunk_ok),
        // getentropy(buf + off, chunk)
        abi::load_u64("%v13", abi::stack_pointer(), BUF_OFFSET),
        abi::add_registers(abi::return_register(), "%v13", "%v9"),
        abi::move_register(abi::ARG[1], "%v11"),
    ]);
    platform.emit_libc_call(
        "getentropy",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&entropy_fail),
        abi::load_u64("%v9", abi::stack_pointer(), OFF_OFFSET),
        // recompute chunk from off/count for the cursor advance
        abi::load_u64("%v10", abi::stack_pointer(), COUNT_OFFSET),
        abi::subtract_registers("%v11", "%v10", "%v9"),
        abi::move_immediate("%v12", "Integer", &GETENTROPY_MAX.to_string()),
        abi::compare_registers("%v11", "%v12"),
        abi::branch_le(&format!("{symbol}_adv_ok")),
        abi::move_register("%v11", "%v12"),
        abi::label(&format!("{symbol}_adv_ok")),
        abi::add_registers("%v9", "%v9", "%v11"),
        abi::store_u64("%v9", abi::stack_pointer(), OFF_OFFSET),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
    ]);

    // Build the List OF Byte with `count` elements, copying from the buffer.
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), COUNT_OFFSET),
        abi::move_immediate("%v11", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v12", "%v10", "%v11"),
        abi::add_immediate("%v12", "%v12", COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), "%v12", "%v10"),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_arena_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::RET[1], abi::stack_pointer(), COLLECTION_OFFSET),
        abi::move_immediate("%v9", "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8("%v9", abi::RET[1], COLLECTION_OFFSET_KIND),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("%v9", abi::RET[1], COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8("%v9", abi::RET[1], COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("%v9", "Byte", "1"),
        abi::store_u8("%v9", abi::RET[1], COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64("%v10", abi::stack_pointer(), COUNT_OFFSET),
        abi::store_u64("%v10", abi::RET[1], COLLECTION_OFFSET_COUNT),
        abi::store_u64("%v10", abi::RET[1], COLLECTION_OFFSET_CAPACITY),
        abi::store_u64("%v10", abi::RET[1], COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("%v10", abi::RET[1], COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate("%v11", abi::RET[1], COLLECTION_HEADER_SIZE),
        abi::move_immediate("%v12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v13", "%v10", "%v12"),
        abi::add_registers("%v14", "%v11", "%v13"),
        abi::load_u64("%v15", abi::stack_pointer(), BUF_OFFSET),
        abi::move_immediate("%v9", "Integer", "0"),
        abi::label(&entry_loop),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_eq(&entry_done),
        abi::move_immediate("%v12", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8("%v12", "%v11", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::store_u64("%v9", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::move_immediate("%v12", "Integer", "1"),
        abi::store_u64("%v12", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_registers("%v12", "%v14", "%v9"),
        abi::load_u8("%v13", "%v15", 0),
        abi::store_u8("%v13", "%v12", 0),
        abi::add_immediate("%v15", "%v15", 1),
        abi::add_immediate("%v11", "%v11", COLLECTION_ENTRY_SIZE),
        abi::add_immediate("%v9", "%v9", 1),
        abi::branch(&entry_loop),
        abi::label(&entry_done),
    ]);

    // Wipe the entropy scratch buffer now that its bytes have been copied into the
    // returned List OF Byte, so a later same-program arena allocation cannot be
    // handed a block still holding the generated random bytes (bug-177 D). Call-free
    // guarded zero loop mirroring the EC helpers' zero_scratch_guarded; %v9 = cursor,
    // %v10 = count, %v11 = index.
    let zero_skip = format!("{symbol}_zero_skip");
    let zero_loop = format!("{symbol}_zero_loop");
    let zero_end = format!("{symbol}_zero_end");
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), BUF_OFFSET),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&zero_skip),
        abi::load_u64("%v10", abi::stack_pointer(), COUNT_OFFSET),
        abi::move_immediate("%v11", "Integer", "0"),
        abi::label(&zero_loop),
        abi::compare_registers("%v11", "%v10"),
        abi::branch_eq(&zero_end),
        abi::store_u8(abi::ZERO, "%v9", 0),
        abi::add_immediate("%v9", "%v9", 1),
        abi::add_immediate("%v11", "%v11", 1),
        abi::branch(&zero_loop),
        abi::label(&zero_end),
        abi::label(&zero_skip),
    ]);

    instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, abi::RET[1]),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Error exits.
    instructions.push(abi::label(&invalid));
    emit_fail_result(
        symbol,
        ERR_INVALID_ARGUMENT_CODE,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&entropy_fail));
    emit_fail_result(
        symbol,
        ERR_UNKNOWN_CODE,
        ERR_UNKNOWN_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail_result(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );

    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], LOCAL_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

/// `bl _mfb_arena_alloc` with size in `x0`, alignment in `x1`; the block pointer
/// is left in `x1`. Branches to `fail` on allocation failure.
fn emit_arena_alloc(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    fail: &str,
) {
    instructions.push(abi::branch_link(ARENA_ALLOC_SYMBOL));
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(fail),
    ]);
}

fn emit_fail_result(
    symbol: &str,
    code: &str,
    message_symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    done: &str,
) {
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", code),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, message_symbol, instructions, relocations);
    instructions.push(abi::branch(done));
}
