// read / readTimeout — the capture side of `gen_windows` (plan-176-R split it out of
// `gen_windows_io.rs`). Spliced into `gen_windows.rs` by `include!`, like its siblings.

/// Copy `copyFrames` captured frames at `pData` (`W_OUT1`) into the s16le result
/// payload, converting from the device format when `W_SHARED`. `got_off` is the
/// frames already gathered; `cf_off` the copyFrames count.
fn emit_read_fill(got_off: usize, cf_off: usize, ins: &mut Vec<CodeInstruction>, vregs: &mut Vregs) {
    let n = ins.len();
    let direct = format!("rfill_direct_{n}");
    let done = format!("rfill_done_{n}");
    let floop = format!("rfill_f_{n}");
    let fdone = format!("rfill_fd_{n}");
    let cloop = format!("rfill_c_{n}");
    let cdone = format!("rfill_cd_{n}");
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    let v15 = vregs.next();
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v10, &v9, W_SHARED),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&direct),
    ]);
    // --- SHARED mix conversion (32-bit float -> s16le) ---------------------
    ins.extend([
        abi::move_immediate(&v9, "Integer", "0"),
        abi::store_u64(&v9, abi::stack_pointer(), F_OFF),
        abi::label(&floop),
        abi::load_u64(&v9, abi::stack_pointer(), F_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), cf_off),
        abi::compare_registers(&v9, &v10),
        abi::branch_ge(&fdone),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), C_OFF),
        abi::label(&cloop),
        abi::load_u64(&v11, abi::stack_pointer(), HANDLE_OFF),
        abi::load_u64(&v11, &v11, H_CHANNELS), // userCh
        abi::load_u64(&v12, abi::stack_pointer(), C_OFF),
        abi::compare_registers(&v12, &v11),
        abi::branch_ge(&cdone),
        // f32 = *(u32*)(pData + f*mixBpf + c*4)   (mix channel c; userCh<=mixCh)
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v11, &v9, W_MIX_BPF),
        abi::load_u64(&v13, &v9, W_OUT1),
        abi::load_u64(&v14, abi::stack_pointer(), F_OFF),
        abi::multiply_registers(&v14, &v14, &v11),
        abi::add_registers(&v13, &v13, &v14),
        abi::shift_left_immediate(&v14, &v12, 2),
        abi::add_registers(&v13, &v13, &v14),
        abi::load_u32(&v9, &v13, 0),
    ]);
    emit_f32_to_s16(&v9, &v10, "rfill", ins, vregs);
    ins.extend([
        // *(s16*)(userDst + (got+f)*userBpf + c*2) = s16
        abi::load_u64(&v13, abi::stack_pointer(), got_off),
        abi::load_u64(&v14, abi::stack_pointer(), F_OFF),
        abi::add_registers(&v13, &v13, &v14),
        abi::load_u64(&v11, abi::stack_pointer(), BPF_OFF),
        abi::multiply_registers(&v13, &v13, &v11),
        abi::load_u64(&v9, abi::stack_pointer(), SRC_OFF),
        abi::add_registers(&v9, &v9, &v13),
        abi::load_u64(&v12, abi::stack_pointer(), C_OFF),
        abi::shift_left_immediate(&v12, &v12, 1),
        abi::add_registers(&v9, &v9, &v12),
        abi::store_u16(&v10, &v9, 0),
        abi::load_u64(&v12, abi::stack_pointer(), C_OFF),
        abi::add_immediate(&v12, &v12, 1),
        abi::store_u64(&v12, abi::stack_pointer(), C_OFF),
        abi::branch(&cloop),
        abi::label(&cdone),
        abi::load_u64(&v9, abi::stack_pointer(), F_OFF),
        abi::add_immediate(&v9, &v9, 1),
        abi::store_u64(&v9, abi::stack_pointer(), F_OFF),
        abi::branch(&floop),
        abi::label(&fdone),
        abi::branch(&done),
    ]);
    // --- EXCLUSIVE direct byte copy ---------------------------------------
    ins.extend([
        abi::label(&direct),
        abi::load_u64(&v13, abi::stack_pointer(), BPF_OFF),
        abi::load_u64(&v9, abi::stack_pointer(), got_off),
        abi::multiply_registers(&v14, &v9, &v13),
        abi::load_u64(&v15, abi::stack_pointer(), SRC_OFF),
        abi::add_registers(&v15, &v15, &v14), // dst
        abi::load_u64(&v9, abi::stack_pointer(), cf_off),
        abi::multiply_registers(&v14, &v9, &v13), // n bytes
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v12, &v9, W_OUT1), // src = pData
    ]);
    emit_copy_bytes(&v12, &v15, &v14, &format!("rfill_dc_{n}"), ins, vregs);
    ins.push(abi::label(&done));
}

pub(crate) fn lower_read(
    symbol: &str,
    timeout: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let invalid = format!("{symbol}_invalid");
    let dev_fail = format!("{symbol}_dev_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let loop_top = format!("{symbol}_loop");
    let loop_done = format!("{symbol}_loop_done");
    let cap_ok = format!("{symbol}_cap_ok");
    let done = format!("{symbol}_done");
    // plan-158-A: a failing WASAPI call reports its name and HRESULT.
    let mut status_sites = StatusFailSites::new(
        symbol,
        "ErrAudioDevice",
        &format!("{symbol}_status_fail"),
        FAIL_STATUS_OFF,
        FAIL_PREFIX_OFF,
    );
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    let v15 = vregs.next();
    let v17 = vregs.next();
    let v18 = vregs.next();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), FRAMES_OFF),
    ]);
    if timeout {
        ins.push(abi::store_u64(abi::c_arg(2), abi::stack_pointer(), TIMEOUT_OFF));
    }
    ins.extend([
        abi::load_u64(&v9, abi::return_register(), H_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&dev_fail),
        abi::load_u64(&v10, abi::return_register(), H_STATE),
        abi::store_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v10, abi::return_register(), H_BYTES_PER_FRAME),
        abi::store_u64(&v10, abi::stack_pointer(), BPF_OFF),
        abi::load_u64(&v9, abi::stack_pointer(), FRAMES_OFF),
        abi::compare_immediate(&v9, "1"),
        abi::branch_lt(&invalid),
        abi::move_immediate(&v11, "Integer", READ_FRAMES_MAX),
        abi::compare_registers(&v9, &v11),
        abi::branch_gt(&invalid),
        abi::multiply_registers(&v12, &v9, &v10),
        abi::store_u64(&v12, abi::stack_pointer(), NEED_OFF),
    ]);
    if timeout {
        // plan-73-B: reject a negative `timeoutMs` (ErrInvalidArgument); clamp a
        // too-large one to INT_MAX rather than raising, then store the clamped value
        // back for the wait below. Mirrors net::poll's clamp.
        let timeout_clamped = format!("{symbol}_timeout_clamped");
        ins.extend([
            abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT_OFF),
            abi::compare_immediate(&v9, "0"),
            abi::branch_lt(&invalid),
            abi::move_immediate(&v11, "Integer", TIMEOUT_CLAMP_MS),
            abi::compare_registers(&v9, &v11),
            abi::branch_le(&timeout_clamped),
            abi::move_register(&v9, &v11),
            abi::label(&timeout_clamped),
            abi::store_u64(&v9, abi::stack_pointer(), TIMEOUT_OFF),
        ]);
    }
    emit_alloc_byte_list(symbol, "main", NEED_OFF, LIST_OFF, &alloc_fail, &mut ins, &mut rel);
    ins.extend([
        abi::load_u64(&v11, abi::stack_pointer(), LIST_OFF),
        abi::load_u64(&v9, abi::stack_pointer(), NEED_OFF),
        abi::move_immediate(&v13, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&v13, &v9, &v13),
        abi::add_immediate(&v13, &v13, COLLECTION_HEADER_SIZE),
        abi::add_registers(&v11, &v11, &v13),
        abi::store_u64(&v11, abi::stack_pointer(), SRC_OFF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), FRAMES_GOT_OFF),
    ]);
    // bug-416 (2): drain the previous read's unconsumed capture tail first, so a
    // non-packet-aligned read loses no frames. The stash holds DEVICE-format bytes
    // (W_MIX_BPF stride); point W_OUT1 at it and reuse the packet fill/convert.
    {
        let drain_cf = format!("{symbol}_drain_cf");
        let drain_skip = format!("{symbol}_drain_skip");
        ins.extend([
            abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
            abi::load_u64(&v10, &v9, W_CARRY_FRAMES),
            abi::load_u64(&v11, &v9, W_CARRY_HEAD),
            abi::subtract_registers(&v10, &v10, &v11), // remaining
            abi::compare_immediate(&v10, "0"),
            abi::branch_le(&drain_skip),
            // copyFrames = min(remaining, framesRequested)   (frames_got == 0 here)
            abi::load_u64(&v12, abi::stack_pointer(), FRAMES_OFF),
            abi::compare_registers(&v10, &v12),
            abi::branch_le(&drain_cf),
            abi::move_register(&v10, &v12),
            abi::label(&drain_cf),
            abi::store_u64(&v10, abi::stack_pointer(), OFFSET_OFF), // copyFrames
            // W_OUT1 = carryPtr + head * mixBpf  (fill source)
            abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
            abi::load_u64(&v13, &v9, W_CARRY_PTR),
            abi::load_u64(&v11, &v9, W_CARRY_HEAD),
            abi::load_u64(&v14, &v9, W_MIX_BPF),
            abi::multiply_registers(&v11, &v11, &v14),
            abi::add_registers(&v13, &v13, &v11),
            abi::store_u64(&v13, &v9, W_OUT1),
        ]);
        emit_read_fill(FRAMES_GOT_OFF, OFFSET_OFF, &mut ins, &mut vregs);
        ins.extend([
            // frames_got += copyFrames
            abi::load_u64(&v9, abi::stack_pointer(), FRAMES_GOT_OFF),
            abi::load_u64(&v10, abi::stack_pointer(), OFFSET_OFF),
            abi::add_registers(&v9, &v9, &v10),
            abi::store_u64(&v9, abi::stack_pointer(), FRAMES_GOT_OFF),
            // carry_head += copyFrames; reset the stash once fully drained
            abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
            abi::load_u64(&v11, &v9, W_CARRY_HEAD),
            abi::add_registers(&v11, &v11, &v10),
            abi::store_u64(&v11, &v9, W_CARRY_HEAD),
            abi::load_u64(&v12, &v9, W_CARRY_FRAMES),
            abi::compare_registers(&v11, &v12),
            abi::branch_lt(&drain_skip), // still stashed frames -> keep cursor
            abi::store_u64(abi::ZERO, &v9, W_CARRY_HEAD),
            abi::store_u64(abi::ZERO, &v9, W_CARRY_FRAMES),
            abi::label(&drain_skip),
        ]);
    }
    if timeout {
        ins.push(abi::subtract_stack(0x20));
        emit_external_int_call(platform, "GetTickCount64", symbol, 0, platform_imports, &mut ins, &mut rel)?;
        ins.extend([
            abi::add_stack(0x20),
            abi::load_u64(&v10, abi::stack_pointer(), TIMEOUT_OFF),
            abi::add_registers(&v9, abi::return_register(), &v10),
            abi::store_u64(&v9, abi::stack_pointer(), DEADLINE_OFF),
        ]);
    }
    ins.extend([
        abi::label(&loop_top),
        abi::load_u64(&v9, abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), FRAMES_OFF),
        abi::compare_registers(&v9, &v10),
        abi::branch_ge(&loop_done),
    ]);
    if timeout {
        ins.push(abi::subtract_stack(0x20));
        emit_external_int_call(platform, "GetTickCount64", symbol, 0, platform_imports, &mut ins, &mut rel)?;
        ins.extend([
            abi::add_stack(0x20),
            abi::load_u64(&v10, abi::stack_pointer(), DEADLINE_OFF),
            abi::compare_registers(abi::return_register(), &v10),
            abi::branch_ge(&loop_done),
            abi::subtract_registers(&v9, &v10, abi::return_register()),
            abi::store_u64(&v9, abi::stack_pointer(), TIMEOUT_OFF),
        ]);
        emit_wait_event(symbol, Some(TIMEOUT_OFF), platform_imports, platform, &mut ins, &mut rel, &mut vregs)?;
        ins.extend([
            abi::compare_immediate(abi::return_register(), "258"),
            abi::branch_eq(&loop_done),
        ]);
    } else {
        emit_wait_event(symbol, None, platform_imports, platform, &mut ins, &mut rel, &mut vregs)?;
    }
    // capture->GetBuffer(&pData, &numFrames, &flags, NULL, NULL)
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, &v9, W_OUT0),
    ]);
    spill_obj(W_SERVICE, &mut ins, &mut vregs);
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::c_arg(1), &v9, W_OUT1),
        abi::add_immediate(abi::c_arg(2), &v9, W_OUT0),
        abi::add_immediate(abi::c_arg(3), &v9, W_OUT2),
        abi::move_register(abi::c_arg(4), abi::ZERO),
        abi::move_register(abi::c_arg(5), abi::ZERO),
    ]);
    com_call(SLOT_CAPTURE_GET_BUFFER, 6, &mut ins, &mut vregs);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&status_sites.site("IAudioCaptureClient::GetBuffer", abi::return_register())),
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u32(&v10, &v9, W_OUT0),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&loop_top), // S_BUFFER_EMPTY: wait again
        abi::load_u64(&v11, abi::stack_pointer(), FRAMES_OFF),
        abi::load_u64(&v12, abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::subtract_registers(&v11, &v11, &v12),
        abi::compare_registers(&v10, &v11),
        abi::branch_le(&cap_ok),
        abi::move_register(&v10, &v11),
        abi::label(&cap_ok),
        abi::store_u64(&v10, abi::stack_pointer(), OFFSET_OFF), // copyFrames
    ]);
    emit_read_fill(FRAMES_GOT_OFF, OFFSET_OFF, &mut ins, &mut vregs);
    // bug-416 (2): before releasing the WHOLE packet, stash the frames that did
    // not fit (numFrames - copyFrames) so the next read continues them. WASAPI
    // forbids a partial ReleaseBuffer, so the tail must be copied out here or it is
    // lost — this is the data-loss defect on any non-packet-aligned read.
    {
        let carry_tail_skip = format!("{symbol}_carry_tail_skip");
        ins.extend([
            abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
            abi::load_u32(&v10, &v9, W_OUT0),                    // numFrames
            abi::load_u64(&v11, abi::stack_pointer(), OFFSET_OFF), // copyFrames
            abi::subtract_registers(&v12, &v10, &v11),        // tail
            abi::compare_immediate(&v12, "0"),
            abi::branch_le(&carry_tail_skip),
            abi::load_u64(&v13, &v9, W_MIX_BPF),
            abi::load_u64(&v14, &v9, W_OUT1),                    // pData
            abi::multiply_registers(&v15, &v11, &v13),        // copyFrames*mixBpf
            abi::add_registers(&v14, &v14, &v15),             // src
            abi::load_u64(&v15, &v9, W_CARRY_PTR),              // dst = carry base
            abi::multiply_registers(&v13, &v12, &v13),       // n = tail*mixBpf
        ]);
        emit_copy_bytes(&v14, &v15, &v13, &format!("{symbol}_carry_tail"), &mut ins, &mut vregs);
        ins.extend([
            abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
            abi::store_u64(abi::ZERO, &v9, W_CARRY_HEAD),
            abi::store_u64(&v12, &v9, W_CARRY_FRAMES),          // tail frames stashed
            abi::label(&carry_tail_skip),
        ]);
    }
    // capture->ReleaseBuffer(numFrames) — the WHOLE packet
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u32(&v10, &v9, W_OUT0),
        abi::store_u64(&v10, abi::stack_pointer(), NAME_OFF),
    ]);
    spill_obj(W_SERVICE, &mut ins, &mut vregs);
    ins.push(abi::load_u64(abi::c_arg(1), abi::stack_pointer(), NAME_OFF));
    com_call(SLOT_CAPTURE_RELEASE_BUFFER, 2, &mut ins, &mut vregs);
    // plan-158-A: unchecked, a failed release was followed by a `GetBuffer` that
    // WASAPI refuses (the previous packet is still held), under the wrong name.
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&status_sites.site("IAudioCaptureClient::ReleaseBuffer", abi::return_register())),
        abi::load_u64(&v9, abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), OFFSET_OFF),
        abi::add_registers(&v9, &v9, &v10),
        abi::store_u64(&v9, abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::branch(&loop_top),
        abi::label(&loop_done),
    ]);
    if timeout {
        let ret_full = format!("{symbol}_ret_full");
        let fin_loop = format!("{symbol}_fin");
        let fin_done = format!("{symbol}_fin_done");
        ins.extend([
            abi::load_u64(&v9, abi::stack_pointer(), FRAMES_GOT_OFF),
            abi::load_u64(&v10, abi::stack_pointer(), FRAMES_OFF),
            abi::compare_registers(&v9, &v10),
            abi::branch_ge(&ret_full),
            abi::load_u64(&v13, abi::stack_pointer(), BPF_OFF),
            abi::multiply_registers(&v9, &v9, &v13),
            abi::store_u64(&v9, abi::stack_pointer(), GOTBYTES_OFF),
        ]);
        emit_alloc_byte_list(symbol, "final", GOTBYTES_OFF, FINAL_LIST_OFF, &alloc_fail, &mut ins, &mut rel);
        ins.extend([
            abi::load_u64(&v9, abi::stack_pointer(), GOTBYTES_OFF),
            abi::load_u64(&v11, abi::stack_pointer(), FINAL_LIST_OFF),
            abi::move_immediate(&v13, "Integer", &byte_list_entry_stride().to_string()),
            abi::multiply_registers(&v13, &v9, &v13),
            abi::add_immediate(&v13, &v13, COLLECTION_HEADER_SIZE),
            abi::add_registers(&v11, &v11, &v13),
            abi::load_u64(&v12, abi::stack_pointer(), SRC_OFF),
            abi::move_immediate(&v10, "Integer", "0"),
            abi::label(&fin_loop),
            abi::compare_registers(&v10, &v9),
            abi::branch_ge(&fin_done),
            abi::add_registers(&v17, &v12, &v10),
            abi::load_u8(&v18, &v17, 0),
            abi::add_registers(&v17, &v11, &v10),
            abi::store_u8(&v18, &v17, 0),
            abi::add_immediate(&v10, &v10, 1),
            abi::branch(&fin_loop),
            abi::label(&fin_done),
            abi::load_u64(&v9, abi::stack_pointer(), NEED_OFF),
            abi::move_immediate(&v10, "Integer", &byte_list_entry_stride().to_string()),
            abi::multiply_registers(&v11, &v9, &v10),
            abi::add_immediate(&v11, &v11, COLLECTION_HEADER_SIZE),
            abi::add_registers(&v11, &v11, &v9),
            abi::move_register(abi::c_arg(1), &v11),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), LIST_OFF),
        ]);
        crate::codegen::memory::arena::emit_arena_free(symbol, &mut ins, &mut rel);
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
    emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    ins.push(abi::label(&dev_fail));
    emit_fail(symbol, "ErrAudioDevice", &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, "ErrOutOfMemory", &mut ins, &mut rel, &done);
    status_sites.emit_stubs(&v9, &mut ins, &mut rel);
    ins.push(abi::label(status_sites.tail()));
    status_sites.emit_raise(&mut ins, &mut rel, &done);
    ins.push(abi::label(&done));
    ins.push(abi::return_());
    Ok((ins, rel, FRAME))
}
