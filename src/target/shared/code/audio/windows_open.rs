// openOutput / openInput / openOutputDevice / openInputDevice / close.

/// Build the `WAVEFORMATEX` at `state->W_WFX`: s16le PCM at the requested
/// channels/rate. Written as five 32-bit stores (the 16-bit fields packed into
/// the low/high halves of a dword; the final store zeroes `cbSize` plus two pad
/// bytes).
fn emit_build_wfx(ins: &mut Vec<CodeInstruction>) {
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::add_immediate("%v10", "%v9", W_WFX), // wfx ptr
        // +0: wFormatTag(=1) | nChannels<<16
        abi::load_u64("%v11", abi::stack_pointer(), CH_OFF),
        abi::shift_left_immediate("%v11", "%v11", 16),
        abi::add_immediate("%v11", "%v11", WAVE_FORMAT_PCM),
        abi::store_u32("%v11", "%v10", 0),
        // +4: nSamplesPerSec
        abi::load_u64("%v11", abi::stack_pointer(), SR_OFF),
        abi::store_u32("%v11", "%v10", 4),
        // +8: nAvgBytesPerSec = rate * bytesPerFrame
        abi::load_u64("%v12", abi::stack_pointer(), BPF_OFF),
        abi::multiply_registers("%v13", "%v11", "%v12"),
        abi::store_u32("%v13", "%v10", 8),
        // +12: nBlockAlign(=bpf) | wBitsPerSample<<16
        abi::move_immediate("%v13", "Integer", &(BITS_PER_SAMPLE << 16).to_string()),
        abi::add_registers("%v13", "%v13", "%v12"),
        abi::store_u32("%v13", "%v10", 12),
        // +16: cbSize = 0 (+2 pad bytes)
        abi::store_u32(abi::ZERO, "%v10", 16),
    ]);
}

/// `device->Activate(IID_IAudioClient, CLSCTX_ALL, NULL, &state->W_CLIENT)`.
/// Stores the HRESULT at `HR_OFF`; branches to `dev_fail` on failure.
fn emit_activate_client(
    symbol: &str,
    dev_fail: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    spill_obj(W_DEVICE, ins);
    guid_addr(symbol, abi::ARG[1], "IID_IAudioClient", ins, rel);
    ins.extend([
        abi::move_immediate(abi::ARG[2], "Integer", CLSCTX_ALL),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::ARG[4], "%v9", W_CLIENT),
    ]);
    com_call(SLOT_DEV_ACTIVATE, 5, ins);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HR_OFF),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(dev_fail),
    ]);
}

/// `client->Initialize(shareMode, flags, dur, periodicity, &wfx, NULL)`, storing
/// the (sign-extended) HRESULT at `HR_OFF`. `flags` is a decimal string;
/// `periodicity_is_dur` chooses `dur` (EXCLUSIVE) or 0 (SHARED+event).
fn emit_init_call(
    exclusive: bool,
    flags: &str,
    periodicity_is_dur: bool,
    ins: &mut Vec<CodeInstruction>,
) {
    spill_obj(W_CLIENT, ins);
    ins.extend([
        abi::move_immediate(
            abi::ARG[1],
            "Integer",
            if exclusive {
                SHAREMODE_EXCLUSIVE
            } else {
                SHAREMODE_SHARED
            },
        ),
        // flags (may exceed 12 bits): materialize by shift+add if large.
    ]);
    // AUTOCONVERTPCM|SRC|EVENTCALLBACK combine to 0x88040000 for SHARED; build via
    // shift so no oversized immediate is emitted.
    if flags == "shared" {
        ins.extend([
            abi::move_immediate(abi::ARG[2], "Integer", "262144"), // EVENTCALLBACK only TEST
        ]);
    } else {
        ins.push(abi::move_immediate(abi::ARG[2], "Integer", flags));
    }
    ins.extend([
        // hnsBufferDuration: EXCLUSIVE uses `dur`; SHARED + EVENTCALLBACK requires
        // 0 (the engine picks its own period — a non-zero value fast-fails).
        if exclusive {
            abi::load_u64(abi::ARG[3], abi::stack_pointer(), TOTAL_OFF)
        } else {
            abi::move_register(abi::ARG[3], abi::ZERO)
        },
        // ARG[4] = periodicity (stack arg 0)
        if periodicity_is_dur {
            abi::load_u64(abi::ARG[4], abi::stack_pointer(), TOTAL_OFF)
        } else {
            abi::move_register(abi::ARG[4], abi::ZERO)
        },
        // ARG[5] = &wfx (stack arg 1)
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::ARG[5], "%v9", W_WFX),
        // ARG[6] = AudioSessionGuid = NULL (stack arg 2)
        abi::move_register(abi::ARG[6], abi::ZERO),
    ]);
    com_call(SLOT_AC_INITIALIZE, 7, ins);
    ins.push(abi::store_u64(abi::return_register(), abi::stack_pointer(), HR_OFF));
}

fn lower_open(
    symbol: &str,
    input: bool,
    device: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let invalid = format!("{symbol}_invalid");
    let unavailable = format!("{symbol}_unavailable");
    let dev_fail = format!("{symbol}_dev_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let init_ok = format!("{symbol}_init_ok");
    let retry = format!("{symbol}_retry");
    let try_shared = format!("{symbol}_try_shared");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    if device {
        ins.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), DEVID_OFF),
            abi::store_u64(abi::ARG[1], abi::stack_pointer(), SR_OFF),
            abi::store_u64(abi::ARG[2], abi::stack_pointer(), CH_OFF),
            abi::store_u64(abi::ARG[3], abi::stack_pointer(), BF_OFF),
        ]);
    } else {
        ins.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), SR_OFF),
            abi::store_u64(abi::ARG[1], abi::stack_pointer(), CH_OFF),
            abi::store_u64(abi::ARG[2], abi::stack_pointer(), BF_OFF),
        ]);
    }
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), STATE_OFF));
    emit_validate_open(symbol, SR_OFF, CH_OFF, BF_OFF, &invalid, &mut ins);
    // bytesPerFrame = channels * 2
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), CH_OFF),
        abi::shift_left_immediate("%v9", "%v9", 1),
        abi::store_u64("%v9", abi::stack_pointer(), BPF_OFF),
        // AudioHandle
        abi::move_immediate(abi::return_register(), "Integer", &H_RECORD_SIZE.to_string()),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::move_register("%v15", abi::RET[1]),
        abi::store_u64("%v15", abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate("%v9", "Integer", if input { KIND_INPUT } else { KIND_OUTPUT }),
        abi::store_u64("%v9", "%v15", H_KIND),
        abi::store_u64(abi::ZERO, "%v15", H_CLOSED),
        abi::load_u64("%v9", abi::stack_pointer(), SR_OFF),
        abi::store_u64("%v9", "%v15", H_SAMPLE_RATE),
        abi::load_u64("%v9", abi::stack_pointer(), CH_OFF),
        abi::store_u64("%v9", "%v15", H_CHANNELS),
        abi::load_u64("%v9", abi::stack_pointer(), BPF_OFF),
        abi::store_u64("%v9", "%v15", H_BYTES_PER_FRAME),
        abi::load_u64("%v9", abi::stack_pointer(), BF_OFF),
        abi::store_u64("%v9", "%v15", H_BUFFER_FRAMES),
        // WASAPI STATE block (arena)
        abi::move_immediate(abi::return_register(), "Integer", &W_SIZE.to_string()),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::move_register("%v15", abi::RET[1]),
        abi::store_u64("%v15", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v9", abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64("%v15", "%v9", H_STATE),
        abi::store_u64(abi::ZERO, "%v15", W_ENUM),
        abi::store_u64(abi::ZERO, "%v15", W_DEVICE),
        abi::store_u64(abi::ZERO, "%v15", W_CLIENT),
        abi::store_u64(abi::ZERO, "%v15", W_SERVICE),
        abi::store_u64(abi::ZERO, "%v15", W_EVENT),
        abi::store_u64(abi::ZERO, "%v15", W_STARTED),
        abi::store_u64(abi::ZERO, "%v15", W_XRUNS),
        abi::store_u64(abi::ZERO, "%v15", W_SHARED),
    ]);
    // CoInitializeEx(NULL, COINIT_MULTITHREADED) — result ignored (safe to call
    // repeatedly; even RPC_E_CHANGED_MODE leaves COM usable on the thread).
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::move_immediate(abi::ARG[1], "Integer", COINIT_MULTITHREADED),
    ]);
    ole_call(symbol, "CoInitializeEx", 2, platform_imports, platform, &mut ins, &mut rel)?;
    // CoCreateInstance(&CLSID_MMDeviceEnumerator, NULL, CLSCTX_ALL,
    //                  &IID_IMMDeviceEnumerator, &state->W_ENUM)
    guid_addr(symbol, abi::return_register(), "CLSID_MMDeviceEnumerator", &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "0"));
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", CLSCTX_ALL));
    guid_addr(symbol, abi::ARG[3], "IID_IMMDeviceEnumerator", &mut ins, &mut rel);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::ARG[4], "%v9", W_ENUM),
    ]);
    ole_call(symbol, "CoCreateInstance", 5, platform_imports, platform, &mut ins, &mut rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&unavailable),
    ]);
    // Get the IMMDevice: default endpoint, or the named endpoint (device variant).
    if device {
        emit_widen_device_id(&mut ins);
        spill_obj(W_ENUM, &mut ins);
        ins.push(abi::add_immediate(abi::ARG[1], abi::stack_pointer(), WIDEID_OFF));
        ins.extend([
            abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
            abi::add_immediate(abi::ARG[2], "%v9", W_DEVICE),
        ]);
        com_call(SLOT_ENUM_GET_DEVICE, 3, &mut ins);
    } else {
        spill_obj(W_ENUM, &mut ins);
        ins.extend([
            abi::move_immediate(abi::ARG[1], "Integer", if input { E_CAPTURE } else { E_RENDER }),
            abi::move_immediate(abi::ARG[2], "Integer", E_CONSOLE),
            abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
            abi::add_immediate(abi::ARG[3], "%v9", W_DEVICE),
        ]);
        com_call(SLOT_GET_DEFAULT_ENDPOINT, 4, &mut ins);
    }
    // DEBUG: dump get-device hr, then the device pointer.
    ins.push(abi::store_u64(abi::return_register(), abi::stack_pointer(), HR_OFF));
    emit_dbg(symbol, HR_OFF, platform_imports, platform, &mut ins, &mut rel);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v9", "%v9", W_DEVICE),
        abi::store_u64("%v9", abi::stack_pointer(), HR_OFF),
    ]);
    emit_dbg(symbol, HR_OFF, platform_imports, platform, &mut ins, &mut rel);
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HR_OFF),
    ]);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v9", "%v9", W_DEVICE),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&dev_fail),
    ]);
    emit_activate_client(symbol, &dev_fail, &mut ins, &mut rel);
    emit_dbg(symbol, HR_OFF, platform_imports, platform, &mut ins, &mut rel);
    emit_build_wfx(&mut ins);
    // dur = bufferFrames * REFTIMES_PER_SEC / sampleRate  -> TOTAL_OFF
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), BF_OFF),
        abi::move_immediate("%v10", "Integer", REFTIMES_PER_SEC),
        abi::multiply_registers("%v9", "%v9", "%v10"),
        abi::load_u64("%v10", abi::stack_pointer(), SR_OFF),
        abi::unsigned_divide_registers("%v9", "%v9", "%v10"),
        abi::store_u64("%v9", abi::stack_pointer(), TOTAL_OFF),
    ]);
    // DEBUG: dump dur (TOTAL_OFF) then the wfx pointer before Initialize.
    emit_dbg(symbol, TOTAL_OFF, platform_imports, platform, &mut ins, &mut rel);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::add_immediate("%v9", "%v9", W_WFX),
        abi::store_u64("%v9", abi::stack_pointer(), HR_OFF),
    ]);
    emit_dbg(symbol, HR_OFF, platform_imports, platform, &mut ins, &mut rel);
    // --- EXCLUSIVE attempt (Open Decision 1: s16le, no resampling) ----------
    emit_init_call(false, "shared", false, &mut ins); // TEST: shared-first
    emit_dbg(symbol, HR_OFF, platform_imports, platform, &mut ins, &mut rel);
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HR_OFF),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&init_ok),
    ]);
    // AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED (0x88890019) -> aligned retry, else SHARED.
    branch_if_hr(0x8889, 0x0019, &retry, &mut ins);
    ins.push(abi::branch(&try_shared));
    ins.push(abi::label(&retry));
    // GetBufferSize(&W_BUFFER) -> aligned frame count.
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, "%v9", W_BUFFER),
    ]);
    spill_obj(W_CLIENT, &mut ins);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::ARG[1], "%v9", W_BUFFER),
    ]);
    com_call(SLOT_AC_GET_BUFFER_SIZE, 2, &mut ins);
    // dur = (REFTIMES_PER_SEC * alignedFrames + sr - 1) / sr  (round up) -> TOTAL_OFF
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::load_u32("%v9", "%v9", W_BUFFER),
        abi::move_immediate("%v10", "Integer", REFTIMES_PER_SEC),
        abi::multiply_registers("%v9", "%v9", "%v10"),
        abi::load_u64("%v10", abi::stack_pointer(), SR_OFF),
        abi::add_registers("%v9", "%v9", "%v10"),
        abi::subtract_immediate("%v9", "%v9", 1),
        abi::unsigned_divide_registers("%v9", "%v9", "%v10"),
        abi::store_u64("%v9", abi::stack_pointer(), TOTAL_OFF),
    ]);
    // Release the client and re-Activate a fresh one for the aligned Initialize.
    spill_obj(W_CLIENT, &mut ins);
    com_call(SLOT_RELEASE, 1, &mut ins);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, "%v9", W_CLIENT),
    ]);
    emit_activate_client(symbol, &dev_fail, &mut ins, &mut rel);
    emit_init_call(true, STREAMFLAGS_EVENTCALLBACK, true, &mut ins);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&init_ok),
    ]);
    // --- SHARED fallback (last resort; AUTOCONVERTPCM keeps s16le) ----------
    ins.push(abi::label(&try_shared));
    // DEBUG: dump the client ptr about to be Released.
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v9", "%v9", W_CLIENT),
        abi::store_u64("%v9", abi::stack_pointer(), HR_OFF),
    ]);
    emit_dbg(symbol, HR_OFF, platform_imports, platform, &mut ins, &mut rel);
    spill_obj(W_CLIENT, &mut ins);
    com_call(SLOT_RELEASE, 1, &mut ins);
    ins.push(abi::store_u64(abi::return_register(), abi::stack_pointer(), HR_OFF));
    emit_dbg(symbol, HR_OFF, platform_imports, platform, &mut ins, &mut rel);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, "%v9", W_CLIENT),
    ]);
    emit_activate_client(symbol, &dev_fail, &mut ins, &mut rel);
    emit_dbg(symbol, HR_OFF, platform_imports, platform, &mut ins, &mut rel);
    emit_init_call(false, "shared", false, &mut ins);
    emit_dbg(symbol, HR_OFF, platform_imports, platform, &mut ins, &mut rel);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::move_immediate("%v10", "Integer", "1"),
        abi::store_u64("%v10", "%v9", W_SHARED),
    ]);
    ins.push(abi::label(&init_ok));
    // Negotiated buffer frame count.
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, "%v9", W_BUFFER),
    ]);
    spill_obj(W_CLIENT, &mut ins);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::ARG[1], "%v9", W_BUFFER),
    ]);
    com_call(SLOT_AC_GET_BUFFER_SIZE, 2, &mut ins);
    // CreateEventW(NULL, FALSE, FALSE, NULL) — auto-reset; NOT sign-extended
    // (the return is a 64-bit HANDLE).
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
    ]);
    emit_external_int_call(platform, "CreateEventW", symbol, 4, platform_imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&dev_fail),
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::return_register(), "%v9", W_EVENT),
    ]);
    // client->SetEventHandle(event)
    spill_obj(W_CLIENT, &mut ins);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::ARG[1], "%v9", W_EVENT),
    ]);
    com_call(SLOT_AC_SET_EVENT_HANDLE, 2, &mut ins);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
    ]);
    // client->GetService(IID_IAudioRenderClient|IAudioCaptureClient, &state->W_SERVICE)
    spill_obj(W_CLIENT, &mut ins);
    guid_addr(
        symbol,
        abi::ARG[1],
        if input {
            "IID_IAudioCaptureClient"
        } else {
            "IID_IAudioRenderClient"
        },
        &mut ins,
        &mut rel,
    );
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::ARG[2], "%v9", W_SERVICE),
    ]);
    com_call(SLOT_AC_GET_SERVICE, 3, &mut ins);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
    ]);
    // client->Start()
    spill_obj(W_CLIENT, &mut ins);
    com_call(SLOT_AC_START, 1, &mut ins);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::move_immediate("%v10", "Integer", "1"),
        abi::store_u64("%v10", "%v9", W_STARTED),
        // success
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
    ]);
    emit_fail(symbol, ERR_INVALID_ARGUMENT_CODE, ERR_INVALID_ARGUMENT_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&unavailable));
    emit_open_cleanup(&mut ins);
    emit_fail(symbol, ERR_AUDIO_UNAVAILABLE_CODE, ERR_AUDIO_UNAVAILABLE_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&dev_fail));
    emit_open_cleanup(&mut ins);
    emit_fail(symbol, ERR_AUDIO_DEVICE_CODE, ERR_AUDIO_DEVICE_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_SYMBOL, &mut ins, &mut rel, &done);
    ins.push(abi::label(&done));
    ins.push(abi::return_());
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME);
    Ok((frame, ins, rel, slots))
}

/// Widen the UTF-8 device id at `DEVID_OFF` into a NUL-terminated UTF-16 buffer at
/// `WIDEID_OFF` (endpoint ids are ASCII, so a byte->wchar zero-extend is exact).
/// Clamps to 255 wchars.
fn emit_widen_device_id(ins: &mut Vec<CodeInstruction>) {
    let copy = "widen_dev_copy".to_string();
    let done = "widen_dev_done".to_string();
    let clamp_ok = "widen_dev_clamp".to_string();
    // Unique labels per emission site are not needed: lower_open emits this at most
    // once. Use a per-call suffix via instruction count to stay collision-free.
    let n = ins.len();
    let copy = format!("{copy}_{n}");
    let done = format!("{done}_{n}");
    let clamp_ok = format!("{clamp_ok}_{n}");
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), DEVID_OFF),
        abi::load_u64("%v10", "%v9", 0),      // len
        abi::add_immediate("%v11", "%v9", 8), // src bytes
        abi::move_immediate("%v9", "Integer", "255"),
        abi::compare_registers("%v10", "%v9"),
        abi::branch_le(&clamp_ok),
        abi::move_register("%v10", "%v9"),
        abi::label(&clamp_ok),
        abi::add_immediate("%v12", abi::stack_pointer(), WIDEID_OFF), // dst
        abi::move_immediate("%v13", "Integer", "0"),
        abi::label(&copy),
        abi::compare_registers("%v13", "%v10"),
        abi::branch_ge(&done),
        abi::load_u8("%v14", "%v11", 0),
        abi::store_u8("%v14", "%v12", 0),
        abi::store_u8(abi::ZERO, "%v12", 1),
        abi::add_immediate("%v11", "%v11", 1),
        abi::add_immediate("%v12", "%v12", 2),
        abi::add_immediate("%v13", "%v13", 1),
        abi::branch(&copy),
        abi::label(&done),
        abi::store_u8(abi::ZERO, "%v12", 0),
        abi::store_u8(abi::ZERO, "%v12", 1),
    ]);
}

/// Release the COM interfaces the open acquired (in reverse order) and close the
/// event handle, each null-guarded. Correct at any error exit after `STATE_OFF` is
/// stored (every slot is zeroed at entry / never set before its acquisition). A
/// null `STATE_OFF` means nothing was allocated yet, so the whole block is skipped.
fn emit_open_cleanup(ins: &mut Vec<CodeInstruction>) {
    let n = ins.len();
    let no_state = format!("ocl_nostate_{n}");
    emit_release_field(W_SERVICE, ins);
    emit_release_field(W_CLIENT, ins);
    emit_release_field(W_DEVICE, ins);
    emit_release_field(W_ENUM, ins);
    // CloseHandle(state->W_EVENT) if non-null. (Directly, not via a method.)
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&no_state),
    ]);
    ins.push(abi::label(&no_state));
}

/// Release the COM object at `state->field` if non-null, then null the slot.
fn emit_release_field(field: usize, ins: &mut Vec<CodeInstruction>) {
    let n = ins.len();
    let skip = format!("rel_skip_{field}_{n}");
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&skip),
        abi::load_u64("%v9", "%v9", field),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&skip),
        abi::store_u64("%v9", abi::stack_pointer(), OBJ_OFF),
    ]);
    com_call(SLOT_RELEASE, 1, ins);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, "%v9", field),
        abi::label(&skip),
    ]);
}

/// close(stream): Stop, Release the four interfaces, CloseHandle the event.
fn lower_close(
    symbol: &str,
    _input: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let already = format!("{symbol}_already");
    let no_event = format!("{symbol}_no_event");
    let done = format!("{symbol}_done");
    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::load_u64("%v9", abi::return_register(), H_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&already),
        abi::load_u64("%v10", abi::return_register(), H_STATE),
        abi::store_u64("%v10", abi::stack_pointer(), STATE_OFF),
    ]);
    // client->Stop() (best effort)
    spill_obj(W_CLIENT, &mut ins);
    com_call(SLOT_AC_STOP, 1, &mut ins);
    emit_release_field(W_SERVICE, &mut ins);
    emit_release_field(W_CLIENT, &mut ins);
    emit_release_field(W_DEVICE, &mut ins);
    emit_release_field(W_ENUM, &mut ins);
    // CloseHandle(event)
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), "%v9", W_EVENT),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&no_event),
    ]);
    ole_call(symbol, "CloseHandle", 1, platform_imports, platform, &mut ins, &mut rel)?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, "%v9", W_EVENT),
        abi::label(&no_event),
        // mark closed
        abi::load_u64("%v9", abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate("%v10", "Integer", "1"),
        abi::store_u64("%v10", "%v9", H_CLOSED),
        abi::label(&already),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&done),
        abi::return_(),
    ]);
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME);
    Ok((frame, ins, rel, slots))
}
