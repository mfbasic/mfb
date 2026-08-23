// write / read / readTimeout / poll / pollTimeout / available / xruns.
//
// The frame COUNT maps 1:1 between the caller's s16le stream and the device
// stream, so the period loop is geometry-independent; only the per-frame byte
// layout differs. `W_SHARED == 0` (EXCLUSIVE s16le) copies bytes verbatim;
// `W_SHARED == 1` (SHARED at the device mix format) converts each sample between
// s16le and 32-bit float, entirely in integer arithmetic (the codegen has no
// single-precision FP ops) by assembling / disassembling the IEEE-754 fields.

/// Copy `n_reg` bytes from `[src_reg]` to `[dst_reg]` (both consumed). Uses
/// `%v16`/`%v17`/`%v18` scratch, uniquely labelled by `tag`.
fn emit_copy_bytes(src_reg: &str, dst_reg: &str, n_reg: &str, tag: &str, ins: &mut Vec<CodeInstruction>, vregs: &mut Vregs) {
    let loop_l = format!("{tag}_cp");
    let done_l = format!("{tag}_cpd");
    let v16 = vregs.next();
    let v17 = vregs.next();
    let v18 = vregs.next();
    ins.extend([
        abi::move_immediate(&v16, "Integer", "0"),
        abi::label(&loop_l),
        abi::compare_registers(&v16, n_reg),
        abi::branch_ge(&done_l),
        abi::add_registers(&v17, src_reg, &v16),
        abi::load_u8(&v18, &v17, 0),
        abi::add_registers(&v17, dst_reg, &v16),
        abi::store_u8(&v18, &v17, 0),
        abi::add_immediate(&v16, &v16, 1),
        abi::branch(&loop_l),
        abi::label(&done_l),
    ]);
}

/// Convert the sign-extended s16 in `inp` to an IEEE-754 32-bit-float bit pattern
/// (value = sample/32768) in `outp`. Internal scratch `%v4`–`%v7`; callers must
/// pass `inp`/`outp` in `%v9`+.
fn emit_s16_to_f32(inp: &str, outp: &str, tag: &str, ins: &mut Vec<CodeInstruction>, vregs: &mut Vregs) {
    let pos = format!("{tag}_s2f_pos");
    let done = format!("{tag}_s2f_done");
    let v4 = vregs.next();
    let v5 = vregs.next();
    let v6 = vregs.next();
    let v7 = vregs.next();
    ins.extend([
        abi::move_register(&v4, inp), // mag (then |val|)
        abi::move_immediate(outp, "Integer", "0"),
        abi::compare_immediate(&v4, "0"),
        abi::branch_eq(&done), // 0.0
        abi::move_immediate(&v5, "Integer", "0"), // sign
        abi::compare_immediate(&v4, "0"),
        abi::branch_ge(&pos),
        abi::move_immediate(&v5, "Integer", "1"),
        abi::shift_left_immediate(&v5, &v5, 31), // 0x80000000
        abi::subtract_registers(&v4, abi::ZERO, &v4), // mag = -val
        abi::label(&pos),
        // msb = 63 - clz(mag)
        abi::count_leading_zeros(&v6, &v4),
        abi::move_immediate(&v7, "Integer", "63"),
        abi::subtract_registers(&v6, &v7, &v6), // msb
        // outp = sign | ((112 + msb) << 23)
        abi::move_immediate(&v7, "Integer", "112"),
        abi::add_registers(&v7, &v7, &v6),
        abi::shift_left_immediate(&v7, &v7, 23),
        abi::or_registers(outp, &v5, &v7),
        // mant = (mag << (23 - msb)) & 0x7FFFFF ; outp |= mant
        abi::move_immediate(&v5, "Integer", "23"),
        abi::subtract_registers(&v5, &v5, &v6),
        abi::shift_left_variable(&v4, &v4, &v5),
        abi::move_immediate(&v5, "Integer", "8388607"), // 0x7FFFFF
        abi::and_registers(&v4, &v4, &v5),
        abi::or_registers(outp, outp, &v4),
        abi::label(&done),
    ]);
}

/// Convert the 32-bit-float bit pattern in `inp` to a clamped s16 in `outp`
/// (out = round(value*32768)). Internal scratch `%v4`–`%v7`; callers pass in `%v9`+.
fn emit_f32_to_s16(inp: &str, outp: &str, tag: &str, ins: &mut Vec<CodeInstruction>, vregs: &mut Vregs) {
    let shl = format!("{tag}_f2s_shl");
    let shift_done = format!("{tag}_f2s_shd");
    let cap_ok = format!("{tag}_f2s_cap");
    let pos = format!("{tag}_f2s_pos");
    let done = format!("{tag}_f2s_done");
    let v4 = vregs.next();
    let v5 = vregs.next();
    let v6 = vregs.next();
    let v7 = vregs.next();
    ins.extend([
        abi::move_immediate(outp, "Integer", "0"),
        // expfield = (inp >> 23) & 0xFF
        abi::shift_right_immediate(&v4, inp, 23),
        abi::move_immediate(&v5, "Integer", "255"),
        abi::and_registers(&v4, &v4, &v5), // expfield
        abi::compare_immediate(&v4, "0"),
        abi::branch_eq(&done), // zero / denormal -> 0
        // significand = (inp & 0x7FFFFF) | 0x800000
        abi::move_immediate(&v5, "Integer", "8388607"),
        abi::and_registers(&v6, inp, &v5),
        abi::move_immediate(&v5, "Integer", "1"),
        abi::shift_left_immediate(&v5, &v5, 23), // 0x800000
        abi::or_registers(&v6, &v6, &v5), // significand (24-bit)
        // shift = (expfield - 127) - 8 = expfield - 135
        abi::move_immediate(&v5, "Integer", "135"),
        abi::subtract_registers(&v5, &v4, &v5), // shift (signed)
        abi::compare_immediate(&v5, "0"),
        abi::branch_ge(&shl),
        // mag = significand >> (-shift)
        abi::subtract_registers(&v7, abi::ZERO, &v5),
        abi::shift_right_variable(&v6, &v6, &v7),
        abi::branch(&shift_done),
        abi::label(&shl),
        abi::shift_left_variable(&v6, &v6, &v5),
        abi::label(&shift_done),
        // clamp mag to 32767
        abi::move_immediate(&v5, "Integer", "32767"),
        abi::compare_registers(&v6, &v5),
        abi::branch_le(&cap_ok),
        abi::move_register(&v6, &v5),
        abi::label(&cap_ok),
        // sign
        abi::shift_right_immediate(&v4, inp, 31),
        abi::compare_immediate(&v4, "0"),
        abi::branch_eq(&pos),
        abi::subtract_registers(&v6, abi::ZERO, &v6),
        abi::label(&pos),
        abi::move_register(outp, &v6),
        abi::label(&done),
    ]);
}

/// `WaitForSingleObject(state->W_EVENT, ms)`. `ms_off` selects a frame slot for
/// the timeout; `None` = INFINITE. Leaves the DWORD result in the return register.
fn emit_wait_event(
    symbol: &str,
    ms_off: Option<usize>,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let v9 = vregs.next();
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), &v9, W_EVENT),
    ]);
    match ms_off {
        Some(off) => ins.push(abi::load_u64(abi::c_arg(1), abi::stack_pointer(), off)),
        None => ins.push(abi::bitwise_not(abi::c_arg(1), abi::ZERO)), // INFINITE
    }
    ole_call(symbol, "WaitForSingleObject", 2, platform_imports, platform, ins, rel)
}

// Conversion loop counters (distinct from every other write/read frame slot,
// including the timeout read's DEADLINE_OFF/FINAL_LIST_OFF).
const F_OFF: usize = 208; // conversion outer frame counter
const C_OFF: usize = 216; // conversion inner channel counter

/// Fill `toWrite` frames of the render buffer at `pData` (`W_OUT1`) from the s16le
/// payload, converting to the device format when `W_SHARED`. `base_frame_off` is
/// the frame index already written; `n_off` is the toWrite count.
fn emit_write_fill(base_frame_off: usize, n_off: usize, ins: &mut Vec<CodeInstruction>, vregs: &mut Vregs) {
    let n = ins.len();
    let direct = format!("wfill_direct_{n}");
    let done = format!("wfill_done_{n}");
    let floop = format!("wfill_f_{n}");
    let fdone = format!("wfill_fd_{n}");
    let cloop = format!("wfill_c_{n}");
    let cdone = format!("wfill_cd_{n}");
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
    // --- SHARED mix conversion (s16le -> 32-bit float) ---------------------
    ins.extend([
        abi::move_immediate(&v9, "Integer", "0"),
        abi::store_u64(&v9, abi::stack_pointer(), F_OFF),
        abi::label(&floop),
        abi::load_u64(&v9, abi::stack_pointer(), F_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), n_off),
        abi::compare_registers(&v9, &v10),
        abi::branch_ge(&fdone),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), C_OFF),
        abi::label(&cloop),
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v11, &v9, W_MIX_CH),
        abi::load_u64(&v12, abi::stack_pointer(), C_OFF),
        abi::compare_registers(&v12, &v11),
        abi::branch_ge(&cdone),
        // userSampleIdx = (baseFrame + f) * userCh + (c mod userCh)
        abi::load_u64(&v13, abi::stack_pointer(), base_frame_off),
        abi::load_u64(&v14, abi::stack_pointer(), F_OFF),
        abi::add_registers(&v13, &v13, &v14), // absolute frame
        abi::load_u64(&v15, abi::stack_pointer(), HANDLE_OFF),
        abi::load_u64(&v15, &v15, H_CHANNELS), // userCh (1 or 2)
        abi::multiply_registers(&v13, &v13, &v15),
        abi::subtract_immediate(&v14, &v15, 1), // userCh-1 (mask, userCh power of two)
        abi::and_registers(&v14, &v12, &v14), // c mod userCh
        abi::add_registers(&v13, &v13, &v14), // userSampleIdx
        // s16 = *(s16*)(userSrc + idx*2), sign-extended
        abi::load_u64(&v9, abi::stack_pointer(), SRC_OFF),
        abi::shift_left_immediate(&v13, &v13, 1),
        abi::add_registers(&v9, &v9, &v13),
        abi::load_u16(&v9, &v9, 0),
        abi::shift_left_immediate(&v9, &v9, 48), // sign-extend a 16-bit value
        abi::arithmetic_shift_right_immediate(&v9, &v9, 48),
    ]);
    emit_s16_to_f32(&v9, &v10, "wfill", ins, vregs);
    ins.extend([
        // *(u32*)(pData + f*mixBpf + c*4) = f32bits
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v11, &v9, W_MIX_BPF),
        abi::load_u64(&v13, &v9, W_OUT1), // pData
        abi::load_u64(&v14, abi::stack_pointer(), F_OFF),
        abi::multiply_registers(&v14, &v14, &v11),
        abi::add_registers(&v13, &v13, &v14),
        abi::load_u64(&v12, abi::stack_pointer(), C_OFF),
        abi::shift_left_immediate(&v12, &v12, 2),
        abi::add_registers(&v13, &v13, &v12),
        abi::store_u32(&v10, &v13, 0),
        // c++
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
    // --- EXCLUSIVE direct byte copy (s16le == device format) ---------------
    ins.extend([
        abi::label(&direct),
        abi::load_u64(&v9, abi::stack_pointer(), base_frame_off),
        abi::load_u64(&v10, abi::stack_pointer(), BPF_OFF),
        abi::multiply_registers(&v11, &v9, &v10),
        abi::load_u64(&v12, abi::stack_pointer(), SRC_OFF),
        abi::add_registers(&v12, &v12, &v11), // src
        abi::load_u64(&v13, abi::stack_pointer(), n_off),
        abi::multiply_registers(&v14, &v13, &v10), // n bytes
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v15, &v9, W_OUT1), // dst = pData
    ]);
    emit_copy_bytes(&v12, &v15, &v14, &format!("wfill_dc_{n}"), ins, vregs);
    ins.push(abi::label(&done));
}

pub(crate) fn lower_write(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let invalid = format!("{symbol}_invalid");
    let dev_fail = format!("{symbol}_dev_fail");
    let loop_top = format!("{symbol}_loop");
    let loop_done = format!("{symbol}_loop_done");
    let cap_ok = format!("{symbol}_cap_ok");
    let done = format!("{symbol}_done");
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::load_u64(&v9, abi::return_register(), H_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&dev_fail),
        abi::load_u64(&v10, abi::return_register(), H_STATE),
        abi::store_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v10, abi::return_register(), H_BYTES_PER_FRAME),
        abi::store_u64(&v10, abi::stack_pointer(), BPF_OFF),
        // total bytes, frame-alignment
        abi::load_u64(&v13, abi::c_arg(1), COLLECTION_OFFSET_COUNT),
        abi::compare_immediate(&v13, "0"),
        abi::branch_eq(&invalid),
        abi::subtract_immediate(&v11, &v10, 1),
        abi::and_registers(&v12, &v13, &v11),
        abi::compare_immediate(&v12, "0"),
        abi::branch_ne(&invalid),
    ]);
    push_collection_data_base_from_capacity(&mut ins, &v14, abi::c_arg(1), &v12, &v14, &v14);
    ins.extend([
        abi::store_u64(&v14, abi::stack_pointer(), SRC_OFF),
        abi::unsigned_divide_registers(&v13, &v13, &v10),
        abi::store_u64(&v13, abi::stack_pointer(), TOTAL_OFF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), OFFSET_OFF),
        abi::label(&loop_top),
        abi::load_u64(&v9, abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), TOTAL_OFF),
        abi::compare_registers(&v9, &v10),
        abi::branch_ge(&loop_done),
    ]);
    emit_wait_event(symbol, None, platform_imports, platform, &mut ins, &mut rel, &mut vregs)?;
    // padding = GetCurrentPadding()
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, &v9, W_OUT0),
    ]);
    spill_obj(W_CLIENT, &mut ins, &mut vregs);
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::c_arg(1), &v9, W_OUT0),
    ]);
    com_call(SLOT_AC_GET_CURRENT_PADDING, 2, &mut ins, &mut vregs);
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u32(&v10, &v9, W_BUFFER),
        abi::load_u32(&v11, &v9, W_OUT0),
        abi::subtract_registers(&v10, &v10, &v11), // avail frames
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&loop_top),
        // toWrite = min(total-offset, avail)
        abi::load_u64(&v9, abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64(&v12, abi::stack_pointer(), TOTAL_OFF),
        abi::subtract_registers(&v12, &v12, &v9),
        abi::compare_registers(&v12, &v10),
        abi::branch_le(&cap_ok),
        abi::move_register(&v12, &v10),
        abi::label(&cap_ok),
        abi::store_u64(&v12, abi::stack_pointer(), FRAMES_GOT_OFF),
    ]);
    // render->GetBuffer(toWrite, &pData)
    spill_obj(W_SERVICE, &mut ins, &mut vregs);
    ins.extend([
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::c_arg(2), &v9, W_OUT1),
    ]);
    com_call(SLOT_RENDER_GET_BUFFER, 3, &mut ins, &mut vregs);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
    ]);
    emit_write_fill(OFFSET_OFF, FRAMES_GOT_OFF, &mut ins, &mut vregs);
    // render->ReleaseBuffer(toWrite, 0)
    spill_obj(W_SERVICE, &mut ins, &mut vregs);
    ins.extend([
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    com_call(SLOT_RENDER_RELEASE_BUFFER, 3, &mut ins, &mut vregs);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
        abi::load_u64(&v9, abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::add_registers(&v9, &v9, &v10),
        abi::store_u64(&v9, abi::stack_pointer(), OFFSET_OFF),
        abi::branch(&loop_top),
        abi::label(&loop_done),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
    ]);
    emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    ins.push(abi::label(&dev_fail));
    emit_fail(symbol, "ErrAudioDevice", &mut ins, &mut rel, &done);
    ins.push(abi::label(&done));
    ins.push(abi::return_());
    Ok((ins, rel, FRAME))
}

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
        abi::branch_lt(&dev_fail),
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
    ins.extend([
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
    ins.push(abi::label(&done));
    ins.push(abi::return_());
    Ok((ins, rel, FRAME))
}

pub(crate) fn lower_query(
    symbol: &str,
    kind: Query,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let done = format!("{symbol}_done");
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), TIMEOUT_OFF),
        abi::load_u64(&v9, abi::return_register(), H_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&v10, abi::return_register(), H_STATE),
        abi::store_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v10, abi::return_register(), H_BYTES_PER_FRAME),
        abi::store_u64(&v10, abi::stack_pointer(), BPF_OFF),
    ]);
    match kind {
        Query::Xruns => {
            ins.extend([
                abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
                abi::load_u64(RESULT_VALUE_REGISTER, &v10, W_XRUNS),
            ]);
        }
        Query::Available => {
            ins.extend([
                abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
                abi::store_u64(abi::ZERO, &v9, W_OUT0),
            ]);
            spill_obj(W_CLIENT, &mut ins, &mut vregs);
            ins.extend([
                abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
                abi::add_immediate(abi::c_arg(1), &v9, W_OUT0),
            ]);
            com_call(SLOT_AC_GET_CURRENT_PADDING, 2, &mut ins, &mut vregs);
            let is_input = format!("{symbol}_isin");
            let have_avail = format!("{symbol}_avail");
            ins.extend([
                abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
                abi::load_u32(&v11, &v9, W_OUT0), // padding
                abi::load_u64(&v12, abi::stack_pointer(), HANDLE_OFF),
                abi::load_u64(&v12, &v12, H_KIND),
                abi::compare_immediate(&v12, KIND_INPUT),
                abi::branch_eq(&is_input),
                abi::load_u32(&v10, &v9, W_BUFFER),
                abi::subtract_registers(&v10, &v10, &v11),
                abi::branch(&have_avail),
                abi::label(&is_input),
                abi::move_register(&v10, &v11),
                abi::label(&have_avail),
                // bug-416 (1): `audio::available` returns whole FRAMES (man
                // `audio/available.md`), matching the macOS/ALSA backends and the
                // sibling `Query::Poll` arm — NOT bytes. Do not scale by BPF.
                abi::move_register(RESULT_VALUE_REGISTER, &v10),
            ]);
        }
        Query::Poll => {
            // plan-73-B: omit=block — wait indefinitely (INFINITE
            // WaitForSingleObject on the stream's buffer event) until the stream is
            // ready, then return TRUE (the convention's readiness-query omit rule).
            // Reuses the same infinite-wait primitive the blocking read uses.
            // Callers wanting the old immediate check pass `, 0` (PollTimeout).
            emit_wait_event(symbol, None, platform_imports, platform, &mut ins, &mut rel, &mut vregs)?;
            let set = format!("{symbol}_pollset");
            ins.extend([
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
                abi::compare_immediate(abi::return_register(), "0"),
                abi::branch_ne(&set),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
                abi::label(&set),
            ]);
        }
        Query::PollTimeout => {
            // plan-73-B: reject a negative `timeoutMs` (ErrInvalidArgument); clamp a
            // too-large one to INT_MAX, storing the clamped value back for the wait.
            let timeout_clamped = format!("{symbol}_pt_clamped");
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
            emit_wait_event(symbol, Some(TIMEOUT_OFF), platform_imports, platform, &mut ins, &mut rel, &mut vregs)?;
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
    ]);
    // plan-73-B: negative `timeoutMs` on `pollTimeout` → ErrInvalidArgument. Guarded
    // to PollTimeout so the other queries' codegen stays byte-identical.
    if matches!(kind, Query::PollTimeout) {
        ins.push(abi::label(&invalid));
        emit_fail(
            symbol,
            "ErrInvalidArgument",
            &mut ins,
            &mut rel,
            &done,
        );
    }
    ins.extend([abi::label(&done), abi::return_()]);
    Ok((ins, rel, FRAME))
}
