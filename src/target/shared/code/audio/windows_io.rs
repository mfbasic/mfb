// write / read / readTimeout / poll / pollTimeout / available / xruns.

/// Copy `n_reg` bytes from `[src_reg]` to `[dst_reg]` (both consumed). Uses
/// `%v16`/`%v17`/`%v18` scratch, uniquely labelled by `tag`.
fn emit_copy_bytes(src_reg: &str, dst_reg: &str, n_reg: &str, tag: &str, ins: &mut Vec<CodeInstruction>) {
    let loop_l = format!("{tag}_cp");
    let done_l = format!("{tag}_cpd");
    ins.extend([
        abi::move_immediate("%v16", "Integer", "0"),
        abi::label(&loop_l),
        abi::compare_registers("%v16", n_reg),
        abi::branch_ge(&done_l),
        abi::add_registers("%v17", src_reg, "%v16"),
        abi::load_u8("%v18", "%v17", 0),
        abi::add_registers("%v17", dst_reg, "%v16"),
        abi::store_u8("%v18", "%v17", 0),
        abi::add_immediate("%v16", "%v16", 1),
        abi::branch(&loop_l),
        abi::label(&done_l),
    ]);
}

/// `WaitForSingleObject(state->W_EVENT, ms_reg_or_infinite)`. If `ms_off` is
/// `Some`, the timeout is loaded from that frame slot; else INFINITE. Leaves the
/// DWORD result in the return register.
fn emit_wait_event(
    symbol: &str,
    ms_off: Option<usize>,
    ms_reg: Option<&str>,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), "%v9", W_EVENT),
    ]);
    match (ms_off, ms_reg) {
        (Some(off), _) => ins.push(abi::load_u64(abi::ARG[1], abi::stack_pointer(), off)),
        (None, Some(reg)) => ins.push(abi::move_register(abi::ARG[1], reg)),
        (None, None) => ins.push(abi::bitwise_not(abi::ARG[1], abi::ZERO)), // INFINITE
    }
    ole_call(symbol, "WaitForSingleObject", 2, platform_imports, platform, ins, rel)
}

fn lower_write(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let invalid = format!("{symbol}_invalid");
    let dev_fail = format!("{symbol}_dev_fail");
    let loop_top = format!("{symbol}_loop");
    let loop_done = format!("{symbol}_loop_done");
    let cap_ok = format!("{symbol}_cap_ok");
    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), OFFSET_OFF), // byteList (temp)
        abi::load_u64("%v9", abi::return_register(), H_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&dev_fail),
        abi::load_u64("%v10", abi::return_register(), H_STATE),
        abi::store_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v10", abi::return_register(), H_BYTES_PER_FRAME),
        abi::store_u64("%v10", abi::stack_pointer(), BPF_OFF),
        // total bytes, frame-alignment
        abi::load_u64("%v13", abi::ARG[1], COLLECTION_OFFSET_COUNT),
        abi::compare_immediate("%v13", "0"),
        abi::branch_eq(&invalid),
        abi::subtract_immediate("%v11", "%v10", 1),
        abi::and_registers("%v12", "%v13", "%v11"),
        abi::compare_immediate("%v12", "0"),
        abi::branch_ne(&invalid),
    ]);
    push_collection_data_base_from_capacity(&mut ins, "%v14", abi::ARG[1], "%v12", "%v14", "%v14");
    ins.extend([
        abi::store_u64("%v14", abi::stack_pointer(), SRC_OFF),
        abi::unsigned_divide_registers("%v13", "%v13", "%v10"),
        abi::store_u64("%v13", abi::stack_pointer(), TOTAL_OFF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), OFFSET_OFF),
        abi::label(&loop_top),
        abi::load_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), TOTAL_OFF),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_ge(&loop_done),
    ]);
    emit_wait_event(symbol, None, None, platform_imports, platform, &mut ins, &mut rel)?;
    // padding = GetCurrentPadding()
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, "%v9", W_OUT0),
    ]);
    spill_obj(W_CLIENT, &mut ins);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::ARG[1], "%v9", W_OUT0),
    ]);
    com_call(SLOT_AC_GET_CURRENT_PADDING, 2, &mut ins);
    // avail = W_BUFFER - padding ; if 0 wait again
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::load_u32("%v10", "%v9", W_BUFFER),
        abi::load_u32("%v11", "%v9", W_OUT0),
        abi::subtract_registers("%v10", "%v10", "%v11"), // avail frames
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&loop_top),
        // toWrite = min(total-offset, avail)
        abi::load_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64("%v12", abi::stack_pointer(), TOTAL_OFF),
        abi::subtract_registers("%v12", "%v12", "%v9"), // remaining
        abi::compare_registers("%v12", "%v10"),
        abi::branch_le(&cap_ok),
        abi::move_register("%v12", "%v10"),
        abi::label(&cap_ok),
        abi::store_u64("%v12", abi::stack_pointer(), FRAMES_GOT_OFF), // toWrite
    ]);
    // render->GetBuffer(toWrite, &pData)
    spill_obj(W_SERVICE, &mut ins);
    ins.extend([
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::ARG[2], "%v9", W_OUT1),
    ]);
    com_call(SLOT_RENDER_GET_BUFFER, 3, &mut ins);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
    ]);
    // copy toWrite*bpf bytes: src = payload + offset*bpf, dst = pData
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), BPF_OFF),
        abi::multiply_registers("%v11", "%v9", "%v10"), // byteOffset
        abi::load_u64("%v12", abi::stack_pointer(), SRC_OFF),
        abi::add_registers("%v12", "%v12", "%v11"), // src
        abi::load_u64("%v13", abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::multiply_registers("%v14", "%v13", "%v10"), // n bytes
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v15", "%v9", W_OUT1), // dst = pData
    ]);
    emit_copy_bytes("%v12", "%v15", "%v14", &format!("{symbol}_w"), &mut ins);
    // render->ReleaseBuffer(toWrite, 0)
    spill_obj(W_SERVICE, &mut ins);
    ins.extend([
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
    ]);
    com_call(SLOT_RENDER_RELEASE_BUFFER, 3, &mut ins);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
        // offset += toWrite
        abi::load_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::add_registers("%v9", "%v9", "%v10"),
        abi::store_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::branch(&loop_top),
        abi::label(&loop_done),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&format!("{symbol}_done")),
        abi::label(&invalid),
    ]);
    emit_fail(symbol, ERR_INVALID_ARGUMENT_CODE, ERR_INVALID_ARGUMENT_SYMBOL, &mut ins, &mut rel, &format!("{symbol}_done"));
    ins.push(abi::label(&dev_fail));
    emit_fail(symbol, ERR_AUDIO_DEVICE_CODE, ERR_AUDIO_DEVICE_SYMBOL, &mut ins, &mut rel, &format!("{symbol}_done"));
    ins.push(abi::label(&format!("{symbol}_done")));
    ins.push(abi::return_());
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME);
    Ok((frame, ins, rel, slots))
}

fn lower_read(
    symbol: &str,
    timeout: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let invalid = format!("{symbol}_invalid");
    let dev_fail = format!("{symbol}_dev_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let loop_top = format!("{symbol}_loop");
    let loop_done = format!("{symbol}_loop_done");
    let cap_ok = format!("{symbol}_cap_ok");
    let done = format!("{symbol}_done");
    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), FRAMES_OFF),
    ]);
    if timeout {
        ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), TIMEOUT_OFF));
    }
    ins.extend([
        abi::load_u64("%v9", abi::return_register(), H_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&dev_fail),
        abi::load_u64("%v10", abi::return_register(), H_STATE),
        abi::store_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v10", abi::return_register(), H_BYTES_PER_FRAME),
        abi::store_u64("%v10", abi::stack_pointer(), BPF_OFF),
        // frames validation
        abi::load_u64("%v9", abi::stack_pointer(), FRAMES_OFF),
        abi::compare_immediate("%v9", "1"),
        abi::branch_lt(&invalid),
        abi::move_immediate("%v11", "Integer", READ_FRAMES_MAX),
        abi::compare_registers("%v9", "%v11"),
        abi::branch_gt(&invalid),
        abi::multiply_registers("%v12", "%v9", "%v10"),
        abi::store_u64("%v12", abi::stack_pointer(), NEED_OFF),
    ]);
    if timeout {
        ins.extend([
            abi::load_u64("%v9", abi::stack_pointer(), TIMEOUT_OFF),
            abi::move_immediate("%v11", "Integer", TIMEOUT_MAX),
            abi::compare_registers("%v9", "%v11"),
            abi::branch_gt(&invalid),
        ]);
    }
    emit_alloc_byte_list(symbol, "main", NEED_OFF, LIST_OFF, &alloc_fail, &mut ins, &mut rel);
    // payload base = list + HEADER + need*stride
    ins.extend([
        abi::load_u64("%v11", abi::stack_pointer(), LIST_OFF),
        abi::load_u64("%v9", abi::stack_pointer(), NEED_OFF),
        abi::move_immediate("%v13", "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers("%v13", "%v9", "%v13"),
        abi::add_immediate("%v13", "%v13", COLLECTION_HEADER_SIZE),
        abi::add_registers("%v11", "%v11", "%v13"),
        abi::store_u64("%v11", abi::stack_pointer(), SRC_OFF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), FRAMES_GOT_OFF),
    ]);
    if timeout {
        // deadline = GetTickCount64() + timeoutMs
        ins.push(abi::subtract_stack(0x20)); // shadow only, no args
        emit_external_int_call(platform, "GetTickCount64", symbol, 0, platform_imports, &mut ins, &mut rel)?;
        ins.extend([
            abi::add_stack(0x20),
            abi::load_u64("%v10", abi::stack_pointer(), TIMEOUT_OFF),
            abi::add_registers("%v9", abi::return_register(), "%v10"),
            abi::store_u64("%v9", abi::stack_pointer(), DEADLINE_OFF),
        ]);
    }
    ins.extend([
        abi::label(&loop_top),
        abi::load_u64("%v9", abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), FRAMES_OFF),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_ge(&loop_done),
    ]);
    if timeout {
        // remaining = deadline - now ; expired -> loop_done
        ins.push(abi::subtract_stack(0x20));
        emit_external_int_call(platform, "GetTickCount64", symbol, 0, platform_imports, &mut ins, &mut rel)?;
        ins.extend([
            abi::add_stack(0x20),
            abi::load_u64("%v10", abi::stack_pointer(), DEADLINE_OFF),
            abi::compare_registers(abi::return_register(), "%v10"),
            abi::branch_ge(&loop_done),
            abi::subtract_registers("%v9", "%v10", abi::return_register()),
            abi::store_u64("%v9", abi::stack_pointer(), TIMEOUT_OFF), // remaining ms
        ]);
        emit_wait_event(symbol, Some(TIMEOUT_OFF), None, platform_imports, platform, &mut ins, &mut rel)?;
        // WAIT_TIMEOUT (258) -> loop_done
        ins.extend([
            abi::compare_immediate(abi::return_register(), "258"),
            abi::branch_eq(&loop_done),
        ]);
    } else {
        emit_wait_event(symbol, None, None, platform_imports, platform, &mut ins, &mut rel)?;
    }
    // capture->GetBuffer(&pData, &numFrames, &flags, NULL, NULL)
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, "%v9", W_OUT0),
    ]);
    spill_obj(W_SERVICE, &mut ins);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::ARG[1], "%v9", W_OUT1), // ppData
        abi::add_immediate(abi::ARG[2], "%v9", W_OUT0), // pNumFrames
        abi::add_immediate(abi::ARG[3], "%v9", W_OUT2), // pdwFlags
        abi::move_register(abi::ARG[4], abi::ZERO),
        abi::move_register(abi::ARG[5], abi::ZERO),
    ]);
    com_call(SLOT_CAPTURE_GET_BUFFER, 6, &mut ins);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
        // numFrames == 0 -> wait again (S_BUFFER_EMPTY; do not ReleaseBuffer)
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::load_u32("%v10", "%v9", W_OUT0),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&loop_top),
        // copyFrames = min(numFrames, frames-got)
        abi::load_u64("%v11", abi::stack_pointer(), FRAMES_OFF),
        abi::load_u64("%v12", abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::subtract_registers("%v11", "%v11", "%v12"), // remaining
        abi::compare_registers("%v10", "%v11"),
        abi::branch_le(&cap_ok),
        abi::move_register("%v10", "%v11"),
        abi::label(&cap_ok),
        abi::store_u64("%v10", abi::stack_pointer(), OFFSET_OFF), // copyFrames
        // dst = payload + got*bpf ; src = pData ; n = copyFrames*bpf
        abi::load_u64("%v13", abi::stack_pointer(), BPF_OFF),
        abi::load_u64("%v12", abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::multiply_registers("%v14", "%v12", "%v13"),
        abi::load_u64("%v15", abi::stack_pointer(), SRC_OFF),
        abi::add_registers("%v15", "%v15", "%v14"), // dst
        abi::multiply_registers("%v14", "%v10", "%v13"), // n bytes
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v12", "%v9", W_OUT1), // src = pData
    ]);
    emit_copy_bytes("%v12", "%v15", "%v14", &format!("{symbol}_r"), &mut ins);
    // capture->ReleaseBuffer(numFrames) — the WHOLE packet
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::load_u32("%v10", "%v9", W_OUT0),
        abi::store_u64("%v10", abi::stack_pointer(), NAME_OFF), // stash numFrames
    ]);
    spill_obj(W_SERVICE, &mut ins);
    ins.push(abi::load_u64(abi::ARG[1], abi::stack_pointer(), NAME_OFF));
    com_call(SLOT_CAPTURE_RELEASE_BUFFER, 2, &mut ins);
    ins.extend([
        // got += copyFrames
        abi::load_u64("%v9", abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), OFFSET_OFF),
        abi::add_registers("%v9", "%v9", "%v10"),
        abi::store_u64("%v9", abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::branch(&loop_top),
        abi::label(&loop_done),
    ]);
    if timeout {
        // Right-size a partial read to `got` frames.
        let ret_full = format!("{symbol}_ret_full");
        let fin_loop = format!("{symbol}_fin");
        let fin_done = format!("{symbol}_fin_done");
        ins.extend([
            abi::load_u64("%v9", abi::stack_pointer(), FRAMES_GOT_OFF),
            abi::load_u64("%v10", abi::stack_pointer(), FRAMES_OFF),
            abi::compare_registers("%v9", "%v10"),
            abi::branch_ge(&ret_full),
            abi::load_u64("%v13", abi::stack_pointer(), BPF_OFF),
            abi::multiply_registers("%v9", "%v9", "%v13"),
            abi::store_u64("%v9", abi::stack_pointer(), GOTBYTES_OFF),
        ]);
        emit_alloc_byte_list(symbol, "final", GOTBYTES_OFF, FINAL_LIST_OFF, &alloc_fail, &mut ins, &mut rel);
        ins.extend([
            abi::load_u64("%v9", abi::stack_pointer(), GOTBYTES_OFF),
            abi::load_u64("%v11", abi::stack_pointer(), FINAL_LIST_OFF),
            abi::move_immediate("%v13", "Integer", &byte_list_entry_stride().to_string()),
            abi::multiply_registers("%v13", "%v9", "%v13"),
            abi::add_immediate("%v13", "%v13", COLLECTION_HEADER_SIZE),
            abi::add_registers("%v11", "%v11", "%v13"), // final payload
            abi::load_u64("%v12", abi::stack_pointer(), SRC_OFF), // source payload
            abi::move_immediate("%v10", "Integer", "0"),
            abi::label(&fin_loop),
            abi::compare_registers("%v10", "%v9"),
            abi::branch_ge(&fin_done),
            abi::add_registers("%v17", "%v12", "%v10"),
            abi::load_u8("%v18", "%v17", 0),
            abi::add_registers("%v17", "%v11", "%v10"),
            abi::store_u8("%v18", "%v17", 0),
            abi::add_immediate("%v10", "%v10", 1),
            abi::branch(&fin_loop),
            abi::label(&fin_done),
            // free the oversized pre-alloc (need*stride + HEADER + need)
            abi::load_u64("%v9", abi::stack_pointer(), NEED_OFF),
            abi::move_immediate("%v10", "Integer", &byte_list_entry_stride().to_string()),
            abi::multiply_registers("%v11", "%v9", "%v10"),
            abi::add_immediate("%v11", "%v11", COLLECTION_HEADER_SIZE),
            abi::add_registers("%v11", "%v11", "%v9"),
            abi::move_register(abi::ARG[1], "%v11"),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), LIST_OFF),
        ]);
        emit_arena_free(symbol, &mut ins, &mut rel);
        ins.extend([
            abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), FINAL_LIST_OFF),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
            abi::label(&ret_full),
        ]);
    }
    ins.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), LIST_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
    ]);
    emit_fail(symbol, ERR_INVALID_ARGUMENT_CODE, ERR_INVALID_ARGUMENT_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&dev_fail));
    emit_fail(symbol, ERR_AUDIO_DEVICE_CODE, ERR_AUDIO_DEVICE_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&done));
    ins.push(abi::return_());
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME);
    Ok((frame, ins, rel, slots))
}

fn lower_query(
    symbol: &str,
    kind: Query,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let closed = format!("{symbol}_closed");
    let done = format!("{symbol}_done");
    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), TIMEOUT_OFF),
        abi::load_u64("%v9", abi::return_register(), H_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("%v10", abi::return_register(), H_STATE),
        abi::store_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v10", abi::return_register(), H_BYTES_PER_FRAME),
        abi::store_u64("%v10", abi::stack_pointer(), BPF_OFF),
    ]);
    match kind {
        Query::Xruns => {
            ins.extend([
                abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
                abi::load_u64(RESULT_VALUE_REGISTER, "%v10", W_XRUNS),
            ]);
        }
        Query::Available | Query::Poll => {
            // padding = GetCurrentPadding()
            ins.extend([
                abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
                abi::store_u64(abi::ZERO, "%v9", W_OUT0),
            ]);
            spill_obj(W_CLIENT, &mut ins);
            ins.extend([
                abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
                abi::add_immediate(abi::ARG[1], "%v9", W_OUT0),
            ]);
            com_call(SLOT_AC_GET_CURRENT_PADDING, 2, &mut ins);
            // availFrames: input -> padding ; output -> W_BUFFER - padding
            let is_input = format!("{symbol}_isin");
            let have_avail = format!("{symbol}_avail");
            ins.extend([
                abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
                abi::load_u32("%v11", "%v9", W_OUT0), // padding
                abi::load_u64("%v12", abi::stack_pointer(), HANDLE_OFF),
                abi::load_u64("%v12", "%v12", H_KIND),
                abi::compare_immediate("%v12", KIND_INPUT),
                abi::branch_eq(&is_input),
                abi::load_u32("%v10", "%v9", W_BUFFER),
                abi::subtract_registers("%v10", "%v10", "%v11"),
                abi::branch(&have_avail),
                abi::label(&is_input),
                abi::move_register("%v10", "%v11"),
                abi::label(&have_avail),
            ]);
            match kind {
                Query::Available => {
                    ins.extend([
                        abi::load_u64("%v13", abi::stack_pointer(), BPF_OFF),
                        abi::multiply_registers(RESULT_VALUE_REGISTER, "%v10", "%v13"),
                    ]);
                }
                Query::Poll => {
                    let set = format!("{symbol}_pollset");
                    ins.extend([
                        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
                        abi::compare_immediate("%v10", "0"),
                        abi::branch_eq(&set),
                        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
                        abi::label(&set),
                    ]);
                }
                _ => unreachable!(),
            }
        }
        Query::PollTimeout => {
            emit_wait_event(symbol, Some(TIMEOUT_OFF), None, platform_imports, platform, &mut ins, &mut rel)?;
            let set = format!("{symbol}_ptset");
            ins.extend([
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
                abi::compare_immediate(abi::return_register(), "0"),
                abi::branch_ne(&set),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
                abi::label(&set),
            ]);
        }
    }
    ins.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&done),
        abi::return_(),
    ]);
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME);
    Ok((frame, ins, rel, slots))
}
