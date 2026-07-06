use super::*;

/// `_mfb_rt_fs_file_drain` (plan-14-B): flush one `File`'s per-handle output buffer
/// to its fd. `x0 = File*`. No-op when the handle is unbuffered (`BUF_ENABLED == 0`)
/// or nothing is pending; otherwise a `write(fd, BUF_PTR, BUF_FILLED)` loop that
/// empties the buffer and resets `BUF_FILLED = 0`. Returns `x0 = 0` on success
/// (including the no-op cases) and `x0 = 1` on a write failure — on failure the
/// buffer is left intact so a later flush can retry. Shared by `fs::flush`, the
/// buffered-write overflow path, `fs::setBuffered(FALSE)`, and flush-on-close.
pub(super) fn lower_fs_file_drain(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = FILE_DRAIN_SYMBOL;
    let ok = format!("{symbol}_ok");
    let drain_loop = format!("{symbol}_loop");
    let err = format!("{symbol}_err");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register("%v0", abi::return_register()), // File* survives the write call
        abi::load_u64("%v1", "%v0", FILE_OFFSET_BUF_ENABLED),
        abi::compare_immediate("%v1", "0"),
        abi::branch_eq(&ok),
        abi::load_u64("%v2", "%v0", FILE_OFFSET_BUF_FILLED),
        abi::compare_immediate("%v2", "0"),
        abi::branch_eq(&ok),
        abi::load_u64("%v3", "%v0", FILE_OFFSET_FD),
        abi::load_u64("%v4", "%v0", FILE_OFFSET_BUF_PTR),
        abi::label(&drain_loop),
        abi::move_register(abi::return_register(), "%v3"),
        abi::move_register(abi::string_data_register(), "%v4"),
        abi::move_register(abi::string_length_register(), "%v2"),
    ];
    let mut relocations = Vec::new();
    platform.emit_write(symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::move_register("%v5", abi::return_register()),
        abi::compare_immediate("%v5", "0"),
        abi::branch_lt(&err),
        abi::add_registers("%v4", "%v4", "%v5"),
        abi::subtract_registers("%v2", "%v2", "%v5"),
        abi::compare_immediate("%v2", "0"),
        abi::branch_ne(&drain_loop),
        abi::store_u64("x31", "%v0", FILE_OFFSET_BUF_FILLED),
        abi::label(&ok),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::return_(),
        abi::label(&err),
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::return_(),
    ]);
    Ok(finalize_vreg_helper(
        "runtime.fs.file_drain",
        symbol,
        "Integer",
        instructions,
        relocations,
    ))
}

/// Emit the instructions that append the `len`-byte chunk at `src` to the `File`
/// handle's per-handle output buffer (plan-14-B §4.5), assuming buffering is
/// enabled. `file`/`src`/`len` are vreg names; all are preserved across the
/// internal calls. The buffer is lazily allocated on first use; on overflow it is
/// drained first, and a chunk larger than the whole buffer is written directly to
/// the fd after the drain. Any underlying `write` failure branches to
/// `write_error`. `tag` disambiguates the emitted labels. Uses vregs `%v30`..`%v39`.
fn emit_append_to_file_buffer(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    file: &str,
    src: &str,
    len: &str,
    tag: &str,
    write_error: &str,
) -> Result<(), String> {
    let cap = FILE_BUFFER_CAPACITY.to_string();
    let have_buf = format!("{symbol}_fbuf_{tag}_have");
    let alloc_failed = format!("{symbol}_fbuf_{tag}_alloc_failed");
    let fits = format!("{symbol}_fbuf_{tag}_fits");
    let copy_loop = format!("{symbol}_fbuf_{tag}_copy_loop");
    let copy_done = format!("{symbol}_fbuf_{tag}_copy_done");
    let appended = format!("{symbol}_fbuf_{tag}_appended");
    instructions.extend([
        abi::load_u64("%v30", file, FILE_OFFSET_BUF_PTR),
        abi::compare_immediate("%v30", "0"),
        abi::branch_ne(&have_buf),
        // Lazily allocate the buffer on first buffered write.
        abi::move_immediate(abi::return_register(), "Integer", &cap),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_failed),
        abi::store_u64("x1", file, FILE_OFFSET_BUF_PTR),
        abi::move_register("%v30", "x1"),
        abi::branch(&have_buf),
        // Allocation failed: write this chunk directly to the fd so no data is lost.
        abi::label(&alloc_failed),
        abi::load_u64("%v31", file, FILE_OFFSET_FD),
        abi::move_register(abi::return_register(), "%v31"),
        abi::move_register(abi::string_data_register(), src),
        abi::move_register(abi::string_length_register(), len),
    ]);
    platform.emit_write(symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(write_error),
        abi::branch(&appended),
        abi::label(&have_buf),
        abi::load_u64("%v32", file, FILE_OFFSET_BUF_FILLED),
        abi::add_registers("%v33", "%v32", len),
        abi::move_immediate("%v34", "Integer", &cap),
        abi::compare_registers("%v33", "%v34"),
        abi::branch_ls(&fits),
        // filled + len would overflow: drain this handle first.
        abi::move_register(abi::return_register(), file),
        abi::branch_link(FILE_DRAIN_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, FILE_DRAIN_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(write_error),
        abi::move_immediate("%v32", "Integer", "0"),
        abi::move_immediate("%v34", "Integer", &cap),
        abi::compare_registers(len, "%v34"),
        abi::branch_ls(&fits),
        // The chunk is larger than the whole buffer: write it directly to the fd.
        abi::load_u64("%v31", file, FILE_OFFSET_FD),
        abi::move_register(abi::return_register(), "%v31"),
        abi::move_register(abi::string_data_register(), src),
        abi::move_register(abi::string_length_register(), len),
    ]);
    platform.emit_write(symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(write_error),
        abi::branch(&appended),
        abi::label(&fits),
        abi::load_u64("%v30", file, FILE_OFFSET_BUF_PTR),
        abi::add_registers("%v35", "%v30", "%v32"),
        abi::move_register("%v36", src),
        abi::move_register("%v37", len),
        abi::label(&copy_loop),
        abi::compare_immediate("%v37", "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8("%v38", "%v36", 0),
        abi::store_u8("%v38", "%v35", 0),
        abi::add_immediate("%v35", "%v35", 1),
        abi::add_immediate("%v36", "%v36", 1),
        abi::subtract_immediate("%v37", "%v37", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::add_registers("%v39", "%v32", len),
        abi::store_u64("%v39", file, FILE_OFFSET_BUF_FILLED),
        abi::label(&appended),
    ]);
    Ok(())
}

/// `fs::isBuffered(file)` (plan-14-B §4.5): report whether this handle is buffered.
pub(super) fn lower_fs_is_buffered_helper(
    symbol: &str,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    let yes = format!("{symbol}_yes");
    let done = format!("{symbol}_done");
    let mut instructions = vec![
        abi::label("entry"),
        abi::load_u64("%v0", abi::return_register(), FILE_OFFSET_BUF_ENABLED),
        abi::compare_immediate("%v0", "0"),
        abi::branch_ne(&yes),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&yes),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ];
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, Vec::new(), stack_slots))
}

/// `fs::setBuffered(file, enabled)` (plan-14-B §4.5): turn per-handle buffering on
/// or off. Disabling drains any pending bytes first, then clears the flag.
pub(super) fn lower_fs_set_buffered_helper(
    symbol: &str,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    let enable = format!("{symbol}_enable");
    let done = format!("{symbol}_done");
    // x0 = File*, x1 = enabled (Boolean).
    let mut instructions = vec![
        abi::label("entry"),
        abi::compare_immediate("x1", "0"),
        abi::branch_ne(&enable),
        // Disable: drain first (best-effort — setBuffered returns Nothing), then
        // clear the flag. File* is already in x0 for the drain; park it for the store.
        abi::move_register("%v0", abi::return_register()),
        abi::branch_link(FILE_DRAIN_SYMBOL),
    ];
    let mut relocations = vec![internal_branch(symbol, FILE_DRAIN_SYMBOL)];
    instructions.extend([
        abi::store_u64("x31", "%v0", FILE_OFFSET_BUF_ENABLED),
        abi::branch(&done),
        abi::label(&enable),
        abi::move_immediate("%v1", "Integer", "1"),
        abi::store_u64("%v1", abi::return_register(), FILE_OFFSET_BUF_ENABLED),
        abi::label(&done),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

/// `fs::flush(file)` (plan-14-B §4.5): drain this handle's buffer now. Raises the
/// write-path ErrOutput on a failing final write; a no-op when the handle is
/// unbuffered.
pub(super) fn lower_fs_flush_helper(
    symbol: &str,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    let flush_error = format!("{symbol}_flush_error");
    let done = format!("{symbol}_done");
    // x0 = File*.
    let mut instructions = vec![abi::label("entry"), abi::branch_link(FILE_DRAIN_SYMBOL)];
    let mut relocations = vec![internal_branch(symbol, FILE_DRAIN_SYMBOL)];
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&flush_error),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&flush_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_OUTPUT_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_fs_open_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    no_follow: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    // Vreg-allocated (plan-00-G Phase 2). path/mode (held across the first alloc),
    // and the open fd (held across the file-record alloc) become spilled vregs; the
    // C-string and flags are consumed before the next call. The mode-literal matcher
    // (`emit_branch_if_ascii_literal`) takes the mode-String ptr/len vregs and uses
    // `x12` as its own scratch.
    let alloc_ok = format!("{symbol}_path_alloc_ok");
    let copy_loop = format!("{symbol}_path_copy_loop");
    let copy_done = format!("{symbol}_path_copy_done");
    let invalid = format!("{symbol}_invalid");
    let read = format!("{symbol}_mode_read");
    let write = format!("{symbol}_mode_write");
    let read_write = format!("{symbol}_mode_read_write");
    let append = format!("{symbol}_mode_append");
    let flags_done = format!("{symbol}_flags_done");
    let open_ok = format!("{symbol}_open_ok");
    let file_alloc_ok = format!("{symbol}_file_alloc_ok");
    let open_error = format!("{symbol}_open_error");
    let done = format!("{symbol}_done");

    let flags = open_flag_set(platform.target(), no_follow);
    let mut vregs = Vregs::new();
    let path = vregs.next();
    let mode = vregs.next();
    let c_path = vregs.next();
    let flag_val = vregs.next();
    let fd = vregs.next();
    let len0 = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&path, abi::return_register()),
        abi::move_register(&mode, "x1"),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), &len0, 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }];
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    let len = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    let mode_len = vregs.next();
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_ok),
        abi::move_register(&c_path, "x1"),
        abi::load_u64(&len, &path, 0),
        abi::add_immediate(&src, &path, 8),
        abi::move_register(&dst, &c_path),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, &len),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&invalid),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", &dst, 0),
        abi::load_u64(&mode_len, &mode, 0),
    ]);
    emit_branch_if_ascii_literal(&mut instructions, &mode, &mode_len, b"r", &read, symbol);
    emit_branch_if_ascii_literal(&mut instructions, &mode, &mode_len, b"read", &read, symbol);
    emit_branch_if_ascii_literal(&mut instructions, &mode, &mode_len, b"w", &write, symbol);
    emit_branch_if_ascii_literal(&mut instructions, &mode, &mode_len, b"write", &write, symbol);
    emit_branch_if_ascii_literal(&mut instructions, &mode, &mode_len, b"rw", &read_write, symbol);
    emit_branch_if_ascii_literal(
        &mut instructions,
        &mode,
        &mode_len,
        b"readWrite",
        &read_write,
        symbol,
    );
    emit_branch_if_ascii_literal(&mut instructions, &mode, &mode_len, b"a", &append, symbol);
    emit_branch_if_ascii_literal(&mut instructions, &mode, &mode_len, b"append", &append, symbol);
    instructions.extend([
        abi::branch(&invalid),
        abi::label(&read),
        abi::move_immediate(&flag_val, "Integer", flags.read),
        abi::branch(&flags_done),
        abi::label(&write),
        abi::move_immediate(&flag_val, "Integer", flags.write),
        abi::branch(&flags_done),
        abi::label(&read_write),
        abi::move_immediate(&flag_val, "Integer", flags.read_write),
        abi::branch(&flags_done),
        abi::label(&append),
        abi::move_immediate(&flag_val, "Integer", flags.append),
        abi::label(&flags_done),
        abi::move_register(abi::return_register(), &c_path),
        abi::move_register("x1", &flag_val),
        abi::move_immediate("x2", "Integer", "438"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&open_ok),
        abi::branch(&open_error),
        abi::label(&open_ok),
        abi::move_register(&fd, abi::return_register()),
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&file_alloc_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&file_alloc_ok),
        abi::store_u64(&fd, "x1", FILE_OFFSET_FD),
        abi::store_u64("x31", "x1", FILE_OFFSET_CLOSED),
        abi::store_u64("x31", "x1", FILE_OFFSET_STATE),
        // Opt-in per-File output buffer (plan-14-B): a fresh handle is unbuffered.
        // Arena memory is poisoned, so zero the buffer fields explicitly.
        abi::store_u64("x31", "x1", FILE_OFFSET_BUF_PTR),
        abi::store_u64("x31", "x1", FILE_OFFSET_BUF_FILLED),
        abi::store_u64("x31", "x1", FILE_OFFSET_BUF_ENABLED),
        abi::move_register(RESULT_VALUE_REGISTER, "x1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&open_error)]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fs_path_errno_error_mapping(
        symbol,
        platform.target(),
        no_follow,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);

    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_fs_close_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    flush_on_close: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    // Vreg-allocated (plan-00-G Phase 2). The file-record pointer is held across the
    // `close` call (read again afterward to mark CLOSED), so it spills.
    // `flush_on_close` is true for `fs::close` (which honors the per-File output
    // buffer, plan-14-B §4.5) and false for `net.close`, whose socket/listener
    // handles share the record layout but never carry an `fs::` output buffer — so
    // net closes must not reference the file-drain helper.
    let already_closed = format!("{symbol}_already_closed");
    let close_error = format!("{symbol}_close_error");
    let flush_failed = format!("{symbol}_flush_failed");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let file = vregs.next();
    let closed = vregs.next();
    let flag = vregs.next();
    let drain_result = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed, &file, FILE_OFFSET_CLOSED),
        abi::compare_immediate(&closed, "0"),
        abi::branch_ne(&already_closed),
    ];
    let mut relocations = Vec::new();
    if flush_on_close {
        // Mandatory flush-on-close (plan-14-B §4.5): drain the handle's output
        // buffer to the fd BEFORE releasing it, so buffered on-disk data is never
        // stranded. A no-op when unbuffered. The fd is still valid here. The drain
        // result is carried across the close so a failing final flush surfaces
        // ErrOutput even though the fd is still released.
        instructions.extend([
            abi::move_register(abi::return_register(), &file),
            abi::branch_link(FILE_DRAIN_SYMBOL),
            abi::move_register(&drain_result, abi::return_register()),
        ]);
        relocations.push(internal_branch(symbol, FILE_DRAIN_SYMBOL));
    }
    instructions.push(abi::load_u64(abi::return_register(), &file, FILE_OFFSET_FD));
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&close_error),
        abi::move_immediate(&flag, "Integer", "1"),
        abi::store_u64(&flag, &file, FILE_OFFSET_CLOSED),
    ]);
    if flush_on_close {
        // The fd is released; if the pre-close flush failed, report ErrOutput.
        instructions.extend([
            abi::compare_immediate(&drain_result, "0"),
            abi::branch_ne(&flush_failed),
        ]);
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&already_closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&close_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_CLOSE_FAILED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_CLOSE_FAILED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    if flush_on_close {
        instructions.extend([
            abi::branch(&done),
            abi::label(&flush_failed),
            abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
        ]);
        push_error_message_address(symbol, ERR_OUTPUT_SYMBOL, &mut instructions, &mut relocations);
    }
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_fs_write_all_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    // Vreg-allocated (plan-00-G Phase 2). fd / remaining / cursor are loop-carried
    // across the `write` syscall, so the allocator spills them.
    let loop_label = format!("{symbol}_write_loop");
    let done_write = format!("{symbol}_write_done");
    let closed = format!("{symbol}_closed");
    let write_error = format!("{symbol}_write_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let file = vregs.next();
    let data = vregs.next();
    let fd = vregs.next();
    let remaining = vregs.next();
    let cursor = vregs.next();
    let closed_flag = vregs.next();
    let buf_enabled = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::move_register(&data, "x1"),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
        abi::compare_immediate(&closed_flag, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        abi::load_u64(&remaining, &data, 0),
        abi::add_immediate(&cursor, &data, 8),
        // Opt-in per-File buffering (plan-14-B): when enabled, append the incoming
        // bytes into the handle's buffer instead of writing them straight through.
        // Off (the default) falls into today's unbuffered direct-write loop.
        abi::load_u64(&buf_enabled, &file, FILE_OFFSET_BUF_ENABLED),
        abi::compare_immediate(&buf_enabled, "0"),
        abi::branch_eq(&loop_label),
    ];
    let mut relocations = Vec::new();
    emit_append_to_file_buffer(
        symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
        &file,
        &cursor,
        &remaining,
        "wa",
        &write_error,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&loop_label),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&done_write),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register("x1", &cursor),
        abi::move_register("x2", &remaining),
    ]);
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&write_error),
        abi::add_registers(&cursor, &cursor, abi::return_register()),
        abi::subtract_registers(&remaining, &remaining, abi::return_register()),
        abi::branch(&loop_label),
        abi::label(&done_write),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&write_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_fs_read_all_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    // Vreg-allocated (plan-00-G Phase 2). fd (across the seeks + read loop), the
    // seek positions/length (across the alloc), and the result string (across the
    // read loop + UTF-8 validation) are vregs the allocator spills.
    let closed = format!("{symbol}_closed");
    let seek_error = format!("{symbol}_seek_error");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let alloc_error = format!("{symbol}_alloc_error");
    let read_loop = format!("{symbol}_read_loop");
    let read_done = format!("{symbol}_read_done");
    let read_error = format!("{symbol}_read_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let file = vregs.next();
    let fd = vregs.next();
    let start = vregs.next();
    let end = vregs.next();
    let length = vregs.next();
    let string = vregs.next();
    let remaining = vregs.next();
    let cursor = vregs.next();
    let closed_flag = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
        abi::compare_immediate(&closed_flag, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "1"),
    ];
    let mut relocations = Vec::new();
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::move_register(&start, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "2"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::move_register(&end, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register("x1", &start),
        abi::move_immediate("x2", "Integer", "0"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::compare_registers(&end, &start),
        abi::branch_lt(&seek_error),
        abi::subtract_registers(&length, &end, &start),
        abi::add_immediate(abi::return_register(), &length, 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&string, "x1"),
        abi::store_u64(&length, &string, 0),
        abi::move_register(&remaining, &length),
        abi::add_immediate(&cursor, &string, 8),
        abi::label(&read_loop),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&read_done),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register("x1", &cursor),
        abi::move_register("x2", &remaining),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&read_error),
        abi::add_registers(&cursor, &cursor, abi::return_register()),
        abi::subtract_registers(&remaining, &remaining, abi::return_register()),
        abi::branch(&read_loop),
        abi::label(&read_done),
        abi::store_u8("x31", &cursor, 0),
        abi::load_u64("x1", &string, 0),
        abi::add_immediate("x0", &string, 8),
    ]);
    let encoding_error = format!("{symbol}_encoding_error");
    emit_call_validate_utf8(symbol, &encoding_error, &mut instructions, &mut relocations);
    instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, &string),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&encoding_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ENCODING_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ENCODING_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&seek_error),
        abi::label(&read_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_fs_write_all_bytes_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    // Vreg-allocated (plan-00-G Phase 2). Writes the byte-List's data region;
    // fd/remaining/cursor are loop-carried across the `write` syscall (spilled).
    let loop_label = format!("{symbol}_write_loop");
    let done_write = format!("{symbol}_write_done");
    let closed = format!("{symbol}_closed");
    let write_error = format!("{symbol}_write_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let file = vregs.next();
    let bytes = vregs.next();
    let fd = vregs.next();
    let remaining = vregs.next();
    let cursor = vregs.next();
    let closed_flag = vregs.next();
    let scratch = vregs.next();
    let buf_enabled = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::move_register(&bytes, "x1"),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
        abi::compare_immediate(&closed_flag, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        abi::load_u64(&remaining, &bytes, COLLECTION_OFFSET_DATA_LENGTH),
        abi::add_immediate(&cursor, &bytes, COLLECTION_HEADER_SIZE),
        abi::load_u64(&scratch, &bytes, COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("x9", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&scratch, &scratch, "x9"),
        abi::add_registers(&cursor, &cursor, &scratch),
        // Opt-in per-File buffering (plan-14-B): append into the handle's buffer
        // when enabled; off falls into today's unbuffered direct-write loop.
        abi::load_u64(&buf_enabled, &file, FILE_OFFSET_BUF_ENABLED),
        abi::compare_immediate(&buf_enabled, "0"),
        abi::branch_eq(&loop_label),
    ];
    let mut relocations = Vec::new();
    emit_append_to_file_buffer(
        symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
        &file,
        &cursor,
        &remaining,
        "wab",
        &write_error,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&loop_label),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&done_write),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register("x1", &cursor),
        abi::move_register("x2", &remaining),
    ]);
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&write_error),
        abi::add_registers(&cursor, &cursor, abi::return_register()),
        abi::subtract_registers(&remaining, &remaining, abi::return_register()),
        abi::branch(&loop_label),
        abi::label(&done_write),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&write_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_fs_read_all_bytes_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    // Vreg-allocated (plan-00-G Phase 2). fd (across seeks + read loop), seek
    // positions/length (across the alloc), the collection and its data-region base
    // (across the read loop) are spilled vregs; the entry-init loop makes no call.
    let closed = format!("{symbol}_closed");
    let seek_error = format!("{symbol}_seek_error");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let read_loop = format!("{symbol}_read_loop");
    let read_done = format!("{symbol}_read_done");
    let read_error = format!("{symbol}_read_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let file = vregs.next();
    let fd = vregs.next();
    let start = vregs.next();
    let end = vregs.next();
    let length = vregs.next();
    let collection = vregs.next();
    let data_base = vregs.next();
    let entry_cursor = vregs.next();
    let idx = vregs.next();
    let remaining = vregs.next();
    let cursor = vregs.next();
    let scratch = vregs.next();
    let closed_flag = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
        abi::compare_immediate(&closed_flag, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "1"),
    ];
    let mut relocations = Vec::new();
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::move_register(&start, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "2"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::move_register(&end, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register("x1", &start),
        abi::move_immediate("x2", "Integer", "0"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::compare_registers(&end, &start),
        abi::branch_lt(&seek_error),
        abi::subtract_registers(&length, &end, &start),
        abi::move_immediate(&scratch, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&scratch, &length, &scratch),
        abi::add_immediate(&scratch, &scratch, COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), &scratch, &length),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&collection, "x1"),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KIND),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&scratch, "Byte", "1"),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_FLAGS_VERSION),
        abi::store_u64(&length, &collection, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&length, &collection, COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(&length, &collection, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&length, &collection, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate(&entry_cursor, &collection, COLLECTION_HEADER_SIZE),
        abi::move_immediate(&scratch, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&scratch, &length, &scratch),
        abi::add_registers(&data_base, &entry_cursor, &scratch),
        abi::move_immediate(&idx, "Integer", "0"),
        abi::label(&entry_loop),
        abi::compare_registers(&idx, &length),
        abi::branch_eq(&entry_done),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8(&scratch, &entry_cursor, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64("x31", &entry_cursor, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64("x31", &entry_cursor, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::store_u64(&idx, &entry_cursor, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::move_immediate(&scratch, "Integer", "1"),
        abi::store_u64(&scratch, &entry_cursor, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_immediate(&entry_cursor, &entry_cursor, COLLECTION_ENTRY_SIZE),
        abi::add_immediate(&idx, &idx, 1),
        abi::branch(&entry_loop),
        abi::label(&entry_done),
        abi::move_register(&remaining, &length),
        abi::move_register(&cursor, &data_base),
        abi::label(&read_loop),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&read_done),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register("x1", &cursor),
        abi::move_register("x2", &remaining),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&read_error),
        abi::add_registers(&cursor, &cursor, abi::return_register()),
        abi::subtract_registers(&remaining, &remaining, abi::return_register()),
        abi::branch(&read_loop),
        abi::label(&read_done),
        abi::move_register(RESULT_VALUE_REGISTER, &collection),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&seek_error),
        abi::label(&read_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_fs_eof_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    // Vreg-allocated (plan-00-G Phase 2). fd is held across the three seeks, the
    // start position across the second/third — both spilled vregs.
    let closed = format!("{symbol}_closed");
    let seek_error = format!("{symbol}_seek_error");
    let at_eof = format!("{symbol}_at_eof");
    let not_eof = format!("{symbol}_not_eof");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let file = vregs.next();
    let fd = vregs.next();
    let start = vregs.next();
    let end = vregs.next();
    let closed_flag = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
        abi::compare_immediate(&closed_flag, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "1"),
    ];
    let mut relocations = Vec::new();
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::move_register(&start, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "2"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::move_register(&end, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register("x1", &start),
        abi::move_immediate("x2", "Integer", "0"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::compare_registers(&start, &end),
        abi::branch_ge(&at_eof),
        abi::branch(&not_eof),
        abi::label(&at_eof),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&not_eof),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&seek_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}


pub(super) fn lower_fs_read_line_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    // Vreg-allocated (plan-00-G Phase 2). fd / start / the temp buffer / line_len /
    // the result string are held across various seek/alloc/read/validate calls and
    // become spilled vregs; the in-memory newline scan and the byte copy make no call.
    let closed = format!("{symbol}_closed");
    let seek_error = format!("{symbol}_seek_error");
    let eof_error = format!("{symbol}_eof_error");
    let temp_alloc_ok = format!("{symbol}_temp_alloc_ok");
    let read_loop = format!("{symbol}_read_loop");
    let read_done = format!("{symbol}_read_done");
    let read_error = format!("{symbol}_read_error");
    let scan_loop = format!("{symbol}_scan_loop");
    let scan_no_newline = format!("{symbol}_scan_no_newline");
    let scan_newline = format!("{symbol}_scan_newline");
    let trim_done = format!("{symbol}_trim_done");
    let result_alloc_ok = format!("{symbol}_result_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let file = vregs.next();
    let fd = vregs.next();
    let start = vregs.next();
    let end = vregs.next();
    let length = vregs.next();
    let temp = vregs.next();
    let line_len = vregs.next();
    let consumed = vregs.next();
    let result = vregs.next();
    let remaining = vregs.next();
    let cursor = vregs.next();
    let scan_ptr = vregs.next();
    let scan_rem = vregs.next();
    let byte = vregs.next();
    let scratch = vregs.next();
    let closed_flag = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
        abi::compare_immediate(&closed_flag, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "1"),
    ];
    let mut relocations = Vec::new();
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::move_register(&start, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "2"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::move_register(&end, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register("x1", &start),
        abi::move_immediate("x2", "Integer", "0"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::compare_registers(&end, &start),
        abi::branch_le(&eof_error),
        abi::subtract_registers(&length, &end, &start),
        abi::add_immediate(abi::return_register(), &length, 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&temp_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&temp_alloc_ok),
        abi::move_register(&temp, "x1"),
        abi::store_u64(&length, &temp, 0),
        abi::move_register(&remaining, &length),
        abi::add_immediate(&cursor, &temp, 8),
        abi::label(&read_loop),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&read_done),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register("x1", &cursor),
        abi::move_register("x2", &remaining),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&read_error),
        abi::add_registers(&cursor, &cursor, abi::return_register()),
        abi::subtract_registers(&remaining, &remaining, abi::return_register()),
        abi::branch(&read_loop),
        abi::label(&read_done),
        abi::add_immediate(&scan_ptr, &temp, 8),
        abi::move_register(&scan_rem, &length),
        abi::move_immediate(&line_len, "Integer", "0"),
        abi::move_immediate(&consumed, "Integer", "0"),
        abi::label(&scan_loop),
        abi::compare_immediate(&scan_rem, "0"),
        abi::branch_eq(&scan_no_newline),
        abi::load_u8(&byte, &scan_ptr, 0),
        abi::add_immediate(&consumed, &consumed, 1),
        abi::compare_immediate(&byte, "10"),
        abi::branch_eq(&scan_newline),
        abi::add_immediate(&line_len, &line_len, 1),
        abi::add_immediate(&scan_ptr, &scan_ptr, 1),
        abi::subtract_immediate(&scan_rem, &scan_rem, 1),
        abi::branch(&scan_loop),
        abi::label(&scan_no_newline),
        abi::move_register(&consumed, &line_len),
        abi::branch(&trim_done),
        abi::label(&scan_newline),
        abi::compare_immediate(&line_len, "0"),
        abi::branch_eq(&trim_done),
        abi::subtract_immediate(&scratch, &scan_ptr, 1),
        abi::load_u8(&byte, &scratch, 0),
        abi::compare_immediate(&byte, "13"),
        abi::branch_ne(&trim_done),
        abi::subtract_immediate(&line_len, &line_len, 1),
        abi::label(&trim_done),
        abi::add_registers("x1", &start, &consumed),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate("x2", "Integer", "0"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::add_immediate(abi::return_register(), &line_len, 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    let dst = vregs.next();
    let src = vregs.next();
    let remaining2 = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&result_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&result_alloc_ok),
        abi::move_register(&result, "x1"),
        abi::store_u64(&line_len, &result, 0),
        abi::add_immediate(&dst, &result, 8),
        abi::add_immediate(&src, &temp, 8),
        abi::move_register(&remaining2, &line_len),
        abi::label(&copy_loop),
        abi::compare_immediate(&remaining2, "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::subtract_immediate(&remaining2, &remaining2, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", &dst, 0),
        abi::load_u64("x1", &result, 0),
        abi::add_immediate("x0", &result, 8),
    ]);
    let encoding_error = format!("{symbol}_encoding_error");
    emit_call_validate_utf8(symbol, &encoding_error, &mut instructions, &mut relocations);
    instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, &result),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&encoding_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ENCODING_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ENCODING_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&eof_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_EOF_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_EOF_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&seek_error),
        abi::label(&read_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) struct OpenFlagSet {
    pub(super) read: &'static str,
    pub(super) write: &'static str,
    pub(super) read_write: &'static str,
    pub(super) append: &'static str,
}

pub(super) fn open_flag_set(target: &str, no_follow: bool) -> OpenFlagSet {
    // Linux (any arch) shares one set of O_* bit values; macOS differs. Keying only
    // on "linux-aarch64" gave linux-x86_64 the macOS bits — on Linux those decode
    // WITHOUT O_CREAT (write 1537 = O_WRONLY|O_APPEND|O_TRUNC → ENOENT "path not
    // found"; append 521 → EINVAL "invalid argument"), breaking openFile "w" /
    // appendText / createTempFile. Match the OS, not the arch.
    match (target.starts_with("linux"), no_follow) {
        (true, false) => OpenFlagSet {
            read: "0",
            write: "577",
            read_write: "66",
            append: "1089",
        },
        (true, true) => OpenFlagSet {
            read: "32768",
            write: "33345",
            read_write: "32834",
            append: "33857",
        },
        (false, false) => OpenFlagSet {
            read: "0",
            write: "1537",
            read_write: "514",
            append: "521",
        },
        (false, true) => OpenFlagSet {
            read: "256",
            write: "1793",
            read_write: "770",
            append: "777",
        },
    }
}

fn emit_branch_if_ascii_literal(
    instructions: &mut Vec<CodeInstruction>,
    ptr: &str,
    len: &str,
    literal: &[u8],
    target: &str,
    symbol: &str,
) {
    let next = format!(
        "{symbol}_literal_{}_{}",
        target.rsplit('_').next().unwrap_or("next"),
        literal.len()
    );
    instructions.extend([
        abi::compare_immediate(len, &literal.len().to_string()),
        abi::branch_ne(&next),
    ]);
    for (index, byte) in literal.iter().enumerate() {
        instructions.extend([
            abi::load_u8("x12", ptr, 8 + index),
            abi::compare_immediate("x12", &byte.to_string()),
            abi::branch_ne(&next),
        ]);
    }
    instructions.extend([abi::branch(target), abi::label(&next)]);
}

