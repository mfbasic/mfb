//! plan-176-U: the entry-args list materialization (`emit_entry_args_list_materialization`). The program entry (`lower_program_entry`) lives in
//! `engine/function/entry/mod.rs`.

use super::*;

pub(super) fn emit_entry_args_list_materialization(
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
    instructions.push(abi::move_immediate(
        RESULT_ERROR_SOURCE_REGISTER,
        "Integer",
        "0",
    ));
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
