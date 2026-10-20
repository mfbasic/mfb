// poll / pollTimeout / available / xruns — the query side of `gen_windows` (plan-176-R split it
// out of `gen_windows_io.rs`). Spliced into `gen_windows.rs` by `include!`, like its siblings.

/// plan-158-C: an EXCLUSIVE output is not started until its first full buffer is
/// submitted, so its period event never fires before then, and a `poll` that waited
/// on it would block forever. Such a stream can take a write at once: branch to
/// `ready`.
fn emit_unstarted_exclusive_output_ready(
    symbol: &str,
    ready: &str,
    ins: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let wait = format!("{symbol}_poll_wait_{}", ins.len());
    let v9 = vregs.next();
    let v10 = vregs.next();
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), HANDLE_OFF),
        abi::load_u64(&v10, &v9, H_KIND),
        abi::compare_immediate(&v10, KIND_INPUT),
        abi::branch_eq(&wait),
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v10, &v9, W_SHARED),
        abi::compare_immediate(&v10, "0"),
        abi::branch_ne(&wait),
        abi::load_u64(&v10, &v9, W_STARTED),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(ready),
        abi::label(&wait),
    ]);
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
            let shared_output = format!("{symbol}_avail_shared");
            ins.extend([
                abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
                abi::load_u32(&v11, &v9, W_OUT0), // padding
                abi::load_u64(&v12, abi::stack_pointer(), HANDLE_OFF),
                abi::load_u64(&v12, &v12, H_KIND),
                abi::compare_immediate(&v12, KIND_INPUT),
                abi::branch_eq(&is_input),
                // plan-158-C: an EXCLUSIVE output takes whatever fits in its render
                // carry without blocking; its padding is always the whole buffer.
                abi::load_u64(&v12, &v9, W_SHARED),
                abi::compare_immediate(&v12, "0"),
                abi::branch_ne(&shared_output),
                abi::load_u32(&v10, &v9, W_BUFFER),
                abi::load_u64(&v11, &v9, W_CARRY_FRAMES),
                abi::subtract_registers(&v10, &v10, &v11),
                abi::branch(&have_avail),
                abi::label(&shared_output),
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
            let set = format!("{symbol}_pollset");
            let ready = format!("{symbol}_pollready");
            emit_unstarted_exclusive_output_ready(symbol, &ready, &mut ins, &mut vregs);
            emit_wait_event(symbol, None, platform_imports, platform, &mut ins, &mut rel, &mut vregs)?;
            ins.extend([
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
                abi::compare_immediate(abi::return_register(), "0"),
                abi::branch_ne(&set),
                abi::label(&ready),
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
            let set = format!("{symbol}_ptset");
            let ready = format!("{symbol}_ptready");
            emit_unstarted_exclusive_output_ready(symbol, &ready, &mut ins, &mut vregs);
            emit_wait_event(symbol, Some(TIMEOUT_OFF), platform_imports, platform, &mut ins, &mut rel, &mut vregs)?;
            ins.extend([
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
                abi::compare_immediate(abi::return_register(), "0"),
                abi::branch_ne(&set),
                abi::label(&ready),
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
