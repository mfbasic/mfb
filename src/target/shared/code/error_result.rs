use super::*;

/// `_mfb_build_error_loc(x0 = filename String*, x1 = line, x2 = char)` — allocate
/// an `ErrorLoc` record `{filename(offset)@0, line@8, char@16}` with the filename
/// String block inlined at offset `ERROR_LOC_OBJECT_SIZE`, and return its pointer
/// in `x0` (`x0 = 0` on OOM). This is the out-of-line form of the block formerly
/// emitted inline at every trap site (`emit_build_error_loc`, plan-16); one shared
/// copy replaces ~48 instructions per site. `filename` is never null (the caller
/// passes an empty-String constant when the source file is unknown); the length
/// word it points at drives the inlined block size and the byte copy. Mirrors
/// `simd_alloc_list`: a framed function that calls `_mfb_arena_alloc` and returns
/// null rather than propagating an error (it runs *during* error handling).
pub(super) fn lower_build_error_loc() -> CodeFunction {
    let mut vregs = Vregs::new();
    // filename/line/char and the computed len are live across the arena_alloc
    // call; held in vregs, the allocator spills them (arena_alloc tramples every
    // integer register — `call_clobber_mask` returns ALL_INT — so nothing
    // survives in a register, exactly as the old hand frame spilled them).
    let filename = vregs.next();
    let line = vregs.next();
    let char_pos = vregs.next();
    let len = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&filename, abi::ARG[0]),
        abi::move_register(&line, abi::ARG[1]),
        abi::move_register(&char_pos, abi::ARG[2]),
        // len = *filename; size = ERROR_LOC_OBJECT_SIZE + len + 9 (inlined String).
        abi::load_u64(&len, &filename, 0),
        abi::add_immediate(abi::return_register(), &len, ERROR_LOC_OBJECT_SIZE + 9),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
        // x0 = result tag, x1 = pointer. On OOM (tag != ok) return a null pointer.
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq("build_error_loc_ok"),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::branch("build_error_loc_ret"),
        abi::label("build_error_loc_ok"),
    ];
    // x1 = ErrorLoc pointer (physical, preserved across the leaf copy below).
    let obj_off = vregs.next();
    let dst = vregs.next();
    let src = vregs.next();
    let remaining = vregs.next();
    let scratch = vregs.next();
    instructions.extend([
        // Fixed slots: filename block-relative offset @0 = OBJECT_SIZE, line @8, char @16.
        abi::move_immediate(&obj_off, "Integer", &ERROR_LOC_OBJECT_SIZE.to_string()),
        abi::store_u64(&obj_off, abi::RET[1], 0),
        abi::store_u64(&line, abi::RET[1], 8),
        abi::store_u64(&char_pos, abi::RET[1], 16),
        // Inline the filename String block (len + 9 bytes) at offset OBJECT_SIZE.
        abi::add_immediate(&dst, abi::RET[1], ERROR_LOC_OBJECT_SIZE),
        abi::move_register(&src, &filename),
        abi::add_immediate(&remaining, &len, 9),
        abi::label("build_error_loc_wloop"),
        abi::compare_immediate(&remaining, "8"),
        abi::branch_lo("build_error_loc_btail"),
        abi::load_u64(&scratch, &src, 0),
        abi::store_u64(&scratch, &dst, 0),
        abi::add_immediate(&src, &src, 8),
        abi::add_immediate(&dst, &dst, 8),
        abi::subtract_immediate(&remaining, &remaining, 8),
        abi::branch("build_error_loc_wloop"),
        abi::label("build_error_loc_btail"),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq("build_error_loc_copy_done"),
        abi::load_u8(&scratch, &src, 0),
        abi::store_u8(&scratch, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::subtract_immediate(&remaining, &remaining, 1),
        abi::branch("build_error_loc_btail"),
        abi::label("build_error_loc_copy_done"),
        abi::move_register(abi::return_register(), abi::RET[1]),
        abi::label("build_error_loc_ret"),
        abi::return_(),
    ]);
    let relocations = vec![internal_branch(BUILD_ERROR_LOC_SYMBOL, ARENA_ALLOC_SYMBOL)];
    finalize_vreg_helper(
        "runtime.build_error_loc",
        BUILD_ERROR_LOC_SYMBOL,
        "Pointer",
        instructions,
        relocations,
    )
}

/// `_mfb_make_error_result(x0=filename, x1=line, x2=char, x3=code, x4=message*)` —
/// build an `ErrorLoc` for the source location and land the standard error
/// `Result` in the return registers: `x0 = RESULT_ERR_TAG`, `x1 = code`,
/// `x2 = message*`, `x3 = ErrorLoc*` (null on OOM). The out-of-line form of the
/// per-trap-site register shuffle (`emit_error_register_return`, plan-16); each
/// site now just loads these five inputs and calls here. Mirrors
/// `lower_build_error_loc`: a framed function that preserves the code/message
/// across the `_mfb_build_error_loc` call.
pub(super) fn lower_make_error_result() -> CodeFunction {
    let mut vregs = Vregs::new();
    // Preserve code (x3) and message (x4) across the ErrorLoc allocation: holding
    // them in vregs makes the allocator keep them in callee-saved registers (it
    // spills/saves them automatically — no manual frame slots). x0/x1/x2 are
    // already positioned as `_mfb_build_error_loc`'s filename/line/char args.
    let code = vregs.next();
    let message = vregs.next();
    let instructions = vec![
        abi::label("entry"),
        abi::move_register(&code, abi::ARG[3]),
        abi::move_register(&message, abi::ARG[4]),
        abi::branch_link(BUILD_ERROR_LOC_SYMBOL),
        // Land the Result: tag=ERR, value=code, message=message, source=ErrorLoc.
        // Set source (x3) from the call result (x0) before x0 is reused for the tag.
        abi::move_register(RESULT_ERROR_SOURCE_REGISTER, abi::return_register()),
        abi::move_register(RESULT_VALUE_REGISTER, &code),
        abi::move_register(RESULT_ERROR_MESSAGE_REGISTER, &message),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
        abi::return_(),
    ];
    let relocations = vec![internal_branch(
        MAKE_ERROR_RESULT_SYMBOL,
        BUILD_ERROR_LOC_SYMBOL,
    )];
    finalize_vreg_helper(
        "runtime.make_error_result",
        MAKE_ERROR_RESULT_SYMBOL,
        "Pointer",
        instructions,
        relocations,
    )
}
