// write, and the helpers the read (`gen_windows_read.rs`) and query (`gen_windows_query.rs`) sides share.
//
// The frame COUNT maps 1:1 between the caller's s16le stream and the device
// stream, so the period loop is geometry-independent; only the per-frame byte
// layout differs. `W_SHARED == 0` (EXCLUSIVE s16le) copies bytes verbatim;
// `W_SHARED == 1` (SHARED at the device mix format) converts each sample between
// s16le and 32-bit float, entirely in integer arithmetic (the codegen has no
// single-precision FP ops) by assembling / disassembling the IEEE-754 fields.

/// Copy `n_reg` bytes from `[src_reg]` to `[dst_reg]` (both consumed). Uses
/// `%v16`/`%v17`/`%v18` scratch, uniquely labelled by `tag`.
fn emit_copy_bytes(src_reg: impl Into<Reg>, dst_reg: impl Into<Reg>, n_reg: impl Into<Reg>, tag: &str, ins: &mut Vec<CodeInstruction>, vregs: &mut Vregs) {
    let (src_reg, dst_reg, n_reg): (Reg, Reg, Reg) = (src_reg.into(), dst_reg.into(), n_reg.into());
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
fn emit_s16_to_f32(inp: impl Into<Reg>, outp: impl Into<Reg>, tag: &str, ins: &mut Vec<CodeInstruction>, vregs: &mut Vregs) {
    let (inp, outp): (Reg, Reg) = (inp.into(), outp.into());
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
fn emit_f32_to_s16(inp: impl Into<Reg>, outp: impl Into<Reg>, tag: &str, ins: &mut Vec<CodeInstruction>, vregs: &mut Vregs) {
    let (inp, outp): (Reg, Reg) = (inp.into(), outp.into());
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

/// plan-158-C: store the render wait's timeout at `TIMEOUT_OFF`: twice the
/// buffer's duration, at least 200 ms. A healthy stream signals its event once a
/// period, so a wait that outlasts two of them is a stream that will never signal
/// again (an invalidated device, a stalled engine) and must fail rather than block
/// forever. Reads `W_BUFFER` from the STATE at `STATE_OFF` and the rate from the
/// handle at `HANDLE_OFF`.
fn emit_render_wait_timeout(ins: &mut Vec<CodeInstruction>, vregs: &mut Vregs) {
    let n = ins.len();
    let floor_ok = format!("wait_timeout_floor_{n}");
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u32(&v10, &v9, W_BUFFER),
        abi::move_immediate(&v11, "Integer", "2000"),
        abi::multiply_registers(&v10, &v10, &v11),
        abi::load_u64(&v9, abi::stack_pointer(), HANDLE_OFF),
        abi::load_u64(&v11, &v9, H_SAMPLE_RATE),
        abi::unsigned_divide_registers(&v10, &v10, &v11),
        abi::compare_immediate(&v10, RENDER_WAIT_FLOOR_MS),
        abi::branch_ge(&floor_ok),
        abi::move_immediate(&v10, "Integer", RENDER_WAIT_FLOOR_MS),
        abi::label(&floor_ok),
        abi::store_u64(&v10, abi::stack_pointer(), TIMEOUT_OFF),
    ]);
}

/// Where an EXCLUSIVE submit's failures go: raised with their status (`write`), or
/// abandoned for the best-effort flush of `close`.
enum SubmitFail<'a> {
    Raise(&'a mut StatusFailSites),
    Skip(&'a str),
}

impl SubmitFail<'_> {
    fn target(&mut self, call: &str) -> String {
        match self {
            SubmitFail::Raise(sites) => sites.site(call, abi::return_register()),
            SubmitFail::Skip(label) => label.to_string(),
        }
    }
}

/// plan-158-C: hand the full render carry (`W_BUFFER` frames at `W_CARRY_PTR`) to an
/// EXCLUSIVE stream. WASAPI's EXCLUSIVE event-driven render wants a whole buffer per
/// period, and `GetCurrentPadding` is no guide there: it reports the whole buffer on
/// every pass (plan-158-C C1), which is why the padding-driven loop spun forever.
/// The first submit pre-rolls the stream (`GetBuffer` before `Start`), then starts
/// it; every later one waits for the period event first.
fn emit_exclusive_submit(
    symbol: &str,
    fail: &mut SubmitFail,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let n = ins.len();
    let no_wait = format!("{symbol}_xsub_nowait_{n}");
    let started = format!("{symbol}_xsub_started_{n}");
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v10, &v9, W_STARTED),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&no_wait),
    ]);
    emit_wait_event(symbol, Some(TIMEOUT_OFF), platform_imports, platform, ins, rel, vregs)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&fail.target("WaitForSingleObject")),
        abi::label(&no_wait),
    ]);
    // render->GetBuffer(W_BUFFER, &pData)
    spill_obj(W_SERVICE, ins, vregs);
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u32(abi::c_arg(1), &v9, W_BUFFER),
        abi::add_immediate(abi::c_arg(2), &v9, W_OUT1),
    ]);
    com_call(SLOT_RENDER_GET_BUFFER, 3, ins, vregs);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&fail.target("IAudioRenderClient::GetBuffer")),
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v10, &v9, W_CARRY_PTR), // src
        abi::load_u64(&v11, &v9, W_OUT1),      // dst
        abi::load_u32(&v12, &v9, W_BUFFER),
        abi::load_u64(&v9, &v9, W_MIX_BPF),
        abi::multiply_registers(&v12, &v12, &v9), // bytes
    ]);
    emit_copy_bytes(&v10, &v11, &v12, &format!("{symbol}_xsub_{n}"), ins, vregs);
    // render->ReleaseBuffer(W_BUFFER, 0)
    spill_obj(W_SERVICE, ins, vregs);
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u32(abi::c_arg(1), &v9, W_BUFFER),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    com_call(SLOT_RENDER_RELEASE_BUFFER, 3, ins, vregs);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&fail.target("IAudioRenderClient::ReleaseBuffer")),
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, &v9, W_CARRY_FRAMES),
        abi::load_u64(&v10, &v9, W_STARTED),
        abi::compare_immediate(&v10, "0"),
        abi::branch_ne(&started),
    ]);
    // The pre-rolled first buffer is in: start the stream.
    spill_obj(W_CLIENT, ins, vregs);
    com_call(SLOT_AC_START, 1, ins, vregs);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&fail.target("IAudioClient::Start")),
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u64(&v10, &v9, W_STARTED),
        abi::label(&started),
    ]);
    Ok(())
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
    ]);
    emit_render_wait_timeout(&mut ins, &mut vregs);
    let exclusive_top = format!("{symbol}_excl");
    let exclusive_room = format!("{symbol}_excl_room");
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v10, &v9, W_SHARED),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&exclusive_top),
        abi::label(&loop_top),
        abi::load_u64(&v9, abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), TOTAL_OFF),
        abi::compare_registers(&v9, &v10),
        abi::branch_ge(&loop_done),
    ]);
    // plan-158-C: bounded, so a stream that stops signalling fails instead of hanging.
    emit_wait_event(symbol, Some(TIMEOUT_OFF), platform_imports, platform, &mut ins, &mut rel, &mut vregs)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&status_sites.site("WaitForSingleObject", abi::return_register())),
    ]);
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
    // plan-158-A: unchecked, a failed call read the zeroed padding and went on to a
    // `GetBuffer` that failed for the same reason under the wrong name.
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&status_sites.site("IAudioClient::GetCurrentPadding", abi::return_register())),
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
        abi::branch_lt(&status_sites.site("IAudioRenderClient::GetBuffer", abi::return_register())),
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
        abi::branch_lt(&status_sites.site("IAudioRenderClient::ReleaseBuffer", abi::return_register())),
        abi::load_u64(&v9, abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::add_registers(&v9, &v9, &v10),
        abi::store_u64(&v9, abi::stack_pointer(), OFFSET_OFF),
        abi::branch(&loop_top),
    ]);
    // --- EXCLUSIVE: fill the render carry, submit it whole (plan-158-C) ------
    ins.extend([
        abi::label(&exclusive_top),
        abi::load_u64(&v9, abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), TOTAL_OFF),
        abi::compare_registers(&v9, &v10),
        abi::branch_ge(&loop_done),
        // n = min(total - offset, W_BUFFER - carry)
        abi::subtract_registers(&v12, &v10, &v9),
        abi::load_u64(&v11, abi::stack_pointer(), STATE_OFF),
        abi::load_u32(&v13, &v11, W_BUFFER),
        abi::load_u64(&v14, &v11, W_CARRY_FRAMES),
        abi::subtract_registers(&v13, &v13, &v14),
        abi::compare_registers(&v12, &v13),
        abi::branch_le(&exclusive_room),
        abi::move_register(&v12, &v13),
        abi::label(&exclusive_room),
        abi::store_u64(&v12, abi::stack_pointer(), FRAMES_GOT_OFF),
        // copy n frames: src + offset*bpf -> carry + carry*bpf
        abi::load_u64(&v13, abi::stack_pointer(), BPF_OFF),
        abi::multiply_registers(&v10, &v9, &v13),
        abi::load_u64(&v9, abi::stack_pointer(), SRC_OFF),
        abi::add_registers(&v9, &v9, &v10),
        abi::multiply_registers(&v14, &v14, &v13),
        abi::load_u64(&v10, &v11, W_CARRY_PTR),
        abi::add_registers(&v10, &v10, &v14),
        abi::multiply_registers(&v12, &v12, &v13),
    ]);
    emit_copy_bytes(&v9, &v10, &v12, &format!("{symbol}_excl_fill"), &mut ins, &mut vregs);
    let exclusive_partial = format!("{symbol}_excl_partial");
    ins.extend([
        // carry += n; offset += n
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::load_u64(&v11, &v9, W_CARRY_FRAMES),
        abi::add_registers(&v11, &v11, &v10),
        abi::store_u64(&v11, &v9, W_CARRY_FRAMES),
        abi::load_u64(&v12, abi::stack_pointer(), OFFSET_OFF),
        abi::add_registers(&v12, &v12, &v10),
        abi::store_u64(&v12, abi::stack_pointer(), OFFSET_OFF),
        // Only a full carry is submitted; a partial one waits for the next write,
        // or for close to pad it with silence.
        abi::load_u32(&v13, &v9, W_BUFFER),
        abi::compare_registers(&v11, &v13),
        abi::branch_lt(&exclusive_partial),
    ]);
    emit_exclusive_submit(
        symbol,
        &mut SubmitFail::Raise(&mut status_sites),
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
        &mut vregs,
    )?;
    ins.extend([
        abi::label(&exclusive_partial),
        abi::branch(&exclusive_top),
        abi::label(&loop_done),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
    ]);
    emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    ins.push(abi::label(&dev_fail));
    emit_fail(symbol, "ErrAudioDevice", &mut ins, &mut rel, &done);
    status_sites.emit_stubs(&v9, &mut ins, &mut rel);
    ins.push(abi::label(status_sites.tail()));
    status_sites.emit_raise(&mut ins, &mut rel, &done);
    ins.push(abi::label(&done));
    ins.push(abi::return_());
    Ok((ins, rel, FRAME))
}
