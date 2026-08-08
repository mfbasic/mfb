// openOutput / openInput / openOutputDevice / openInputDevice / close.
//
// Open Decision 1 is honoured device-permitting: an EXCLUSIVE, event-driven
// s16le stream (no resampling) is attempted first. If the device refuses that
// format (AUDCLNT_E_UNSUPPORTED_FORMAT — the common case for a shared consumer
// endpoint), the sanctioned fallback is SHARED mode at the device's own MIX
// FORMAT (`GetMixFormat`): the engine's native geometry always initializes.
// `write`/`read` then convert between the caller's s16le frames and the mix
// format's 32-bit-float frames (`W_SHARED` selects the path; `W_MIX_CH`/
// `W_MIX_BPF` carry the mix geometry). AUTOCONVERTPCM is deliberately NOT used:
// on the Win11 test box it fast-fails (0xC0000409) inside WASAPI.

/// Build the caller's `WAVEFORMATEX` (s16le PCM) at `state->W_WFX`, as five 32-bit
/// stores (16-bit fields packed into dword halves; the last store zeroes `cbSize`
/// plus two pad bytes).
fn emit_build_wfx(ins: &mut Vec<CodeInstruction>) {
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::add_immediate("%v10", "%v9", W_WFX),
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
    guid_addr(symbol, abi::c_arg(1), "IID_IAudioClient", ins, rel);
    ins.extend([
        abi::move_immediate(abi::c_arg(2), "Integer", CLSCTX_ALL),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::c_arg(4), "%v9", W_CLIENT),
    ]);
    com_call(SLOT_DEV_ACTIVATE, 5, ins);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HR_OFF),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(dev_fail),
    ]);
}

/// `client->Initialize(shareMode, EVENTCALLBACK, bufDur, periodicity, pFormat, NULL)`.
/// `exclusive` selects the share mode and (per WASAPI's rules) the durations:
/// EXCLUSIVE passes `dur` for both buffer and periodicity; SHARED passes 0/0 (the
/// engine picks its own period — a non-zero value fast-fails). `mix_ptr` selects
/// the format: `false` → the inline `state->W_WFX`; `true` → the pointer at
/// `state->W_OUT0` (the `GetMixFormat` result). The HRESULT lands at `HR_OFF`.
fn emit_initialize(exclusive: bool, mix_ptr: bool, ins: &mut Vec<CodeInstruction>) {
    spill_obj(W_CLIENT, ins);
    ins.extend([
        abi::move_immediate(
            abi::c_arg(1),
            "Integer",
            if exclusive {
                SHAREMODE_EXCLUSIVE
            } else {
                SHAREMODE_SHARED
            },
        ),
        abi::move_immediate(abi::c_arg(2), "Integer", STREAMFLAGS_EVENTCALLBACK),
        // hnsBufferDuration / hnsPeriodicity
        if exclusive {
            abi::load_u64(abi::c_arg(3), abi::stack_pointer(), TOTAL_OFF)
        } else {
            abi::move_register(abi::c_arg(3), abi::ZERO)
        },
        if exclusive {
            abi::load_u64(abi::c_arg(4), abi::stack_pointer(), TOTAL_OFF)
        } else {
            abi::move_register(abi::c_arg(4), abi::ZERO)
        },
        // pFormat (stack arg 1)
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        if mix_ptr {
            abi::load_u64(abi::c_arg(5), "%v9", W_OUT0)
        } else {
            abi::add_immediate(abi::c_arg(5), "%v9", W_WFX)
        },
        // AudioSessionGuid = NULL (stack arg 2)
        abi::move_register(abi::c_arg(6), abi::ZERO),
    ]);
    com_call(SLOT_AC_INITIALIZE, 7, ins);
    ins.push(abi::store_u64(abi::return_register(), abi::stack_pointer(), HR_OFF));
}

/// `client->Release()` then a fresh `device->Activate` — used before the SHARED
/// retry, since a client that failed `Initialize` cannot be reinitialized.
fn emit_reactivate_client(
    symbol: &str,
    dev_fail: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    spill_obj(W_CLIENT, ins);
    com_call(SLOT_RELEASE, 1, ins);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, "%v9", W_CLIENT),
    ]);
    emit_activate_client(symbol, dev_fail, ins, rel);
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
    let use_shared = format!("{symbol}_use_shared");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    if device {
        ins.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), DEVID_OFF),
            abi::store_u64(abi::c_arg(1), abi::stack_pointer(), SR_OFF),
            abi::store_u64(abi::c_arg(2), abi::stack_pointer(), CH_OFF),
            abi::store_u64(abi::c_arg(3), abi::stack_pointer(), BF_OFF),
        ]);
    } else {
        ins.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), SR_OFF),
            abi::store_u64(abi::c_arg(1), abi::stack_pointer(), CH_OFF),
            abi::store_u64(abi::c_arg(2), abi::stack_pointer(), BF_OFF),
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
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::move_register("%v15", abi::RET[1]),
        abi::store_u64("%v15", abi::stack_pointer(), HANDLE_OFF),
        // Canonical plan-80 header: tag@0, kind (handle)@8, closed@16, STATE@24.
        abi::move_immediate("%v9", "Integer", RESOURCE_TAG_AUDIO),
        abi::store_u64("%v9", "%v15", RESOURCE_OFFSET_TAG),
        abi::store_u64(abi::ZERO, "%v15", RESOURCE_OFFSET_STATE),
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
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
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
        // bug-416 (2): capture carry-over, empty until the first partial packet.
        abi::store_u64(abi::ZERO, "%v15", W_CARRY_PTR),
        abi::store_u64(abi::ZERO, "%v15", W_CARRY_FRAMES),
        abi::store_u64(abi::ZERO, "%v15", W_CARRY_HEAD),
    ]);
    // CoInitializeEx(NULL, COINIT_MULTITHREADED) — result ignored.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::move_immediate(abi::c_arg(1), "Integer", COINIT_MULTITHREADED),
    ]);
    ole_call(symbol, "CoInitializeEx", 2, platform_imports, platform, &mut ins, &mut rel)?;
    // CoCreateInstance(&CLSID_MMDeviceEnumerator, NULL, CLSCTX_ALL,
    //                  &IID_IMMDeviceEnumerator, &state->W_ENUM)
    guid_addr(symbol, abi::return_register(), "CLSID_MMDeviceEnumerator", &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    ins.push(abi::move_immediate(abi::c_arg(2), "Integer", CLSCTX_ALL));
    guid_addr(symbol, abi::c_arg(3), "IID_IMMDeviceEnumerator", &mut ins, &mut rel);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::c_arg(4), "%v9", W_ENUM),
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
        ins.push(abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), WIDEID_OFF));
        ins.extend([
            abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
            abi::add_immediate(abi::c_arg(2), "%v9", W_DEVICE),
        ]);
        com_call(SLOT_ENUM_GET_DEVICE, 3, &mut ins);
    } else {
        spill_obj(W_ENUM, &mut ins);
        ins.extend([
            abi::move_immediate(abi::c_arg(1), "Integer", if input { E_CAPTURE } else { E_RENDER }),
            abi::move_immediate(abi::c_arg(2), "Integer", E_CONSOLE),
            abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
            abi::add_immediate(abi::c_arg(3), "%v9", W_DEVICE),
        ]);
        com_call(SLOT_GET_DEFAULT_ENDPOINT, 4, &mut ins);
    }
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
    ]);
    emit_activate_client(symbol, &dev_fail, &mut ins, &mut rel);
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
    // --- EXCLUSIVE attempt (Open Decision 1: s16le, no resampling) ----------
    emit_initialize(true, false, &mut ins);
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HR_OFF),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&use_shared),
        // EXCLUSIVE succeeded: direct s16le, mix geometry == user geometry.
        abi::load_u64("%v15", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v9", abi::stack_pointer(), CH_OFF),
        abi::store_u64("%v9", "%v15", W_MIX_CH),
        abi::load_u64("%v9", abi::stack_pointer(), BPF_OFF),
        abi::store_u64("%v9", "%v15", W_MIX_BPF),
        abi::store_u64(abi::ZERO, "%v15", W_SHARED),
        abi::branch(&init_ok),
        abi::label(&use_shared),
    ]);
    // --- SHARED fallback at the device MIX FORMAT (clearly a last resort) -----
    emit_reactivate_client(symbol, &dev_fail, &mut ins, &mut rel);
    // GetMixFormat(&state->W_OUT0)
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, "%v9", W_OUT0),
    ]);
    spill_obj(W_CLIENT, &mut ins);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::c_arg(1), "%v9", W_OUT0),
    ]);
    com_call(SLOT_AC_GET_MIX_FORMAT, 2, &mut ins);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
        // Read the mix WAVEFORMATEX: nChannels @+2 (u16), nSamplesPerSec @+4 (u32),
        // nBlockAlign @+12 (u16), wBitsPerSample @+14 (u16).
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v10", "%v9", W_OUT0), // mix wfx ptr
        abi::load_u16("%v11", "%v10", 2),     // mixChannels
        abi::store_u64("%v11", "%v9", W_MIX_CH),
        abi::load_u16("%v11", "%v10", 12), // mixBlockAlign (frame stride)
        abi::store_u64("%v11", "%v9", W_MIX_BPF),
        abi::move_immediate("%v11", "Integer", "1"),
        abi::store_u64("%v11", "%v9", W_SHARED),
        // No resampling: the mix rate must equal the requested rate.
        abi::load_u32("%v11", "%v10", 4), // mix rate
        abi::load_u64("%v12", abi::stack_pointer(), SR_OFF),
        abi::compare_registers("%v11", "%v12"),
        abi::branch_ne(&dev_fail),
        // Only a 32-bit-float mix format is convertible by the integer s16<->f32 path.
        abi::load_u16("%v11", "%v10", 14), // mix bits per sample
        abi::compare_immediate("%v11", "32"),
        abi::branch_ne(&dev_fail),
        // bug-416 (3): the SHARED read converter (`emit_read_fill`) reads userCh
        // channels per frame from the device mix buffer (c in [0, userCh)); a mix
        // with fewer channels than the caller opened would read past the capture
        // buffer on the final frame. Reject mixCh < userCh — the read path cannot
        // synthesize channels the device does not provide. (The write path is safe:
        // it bounds c on mixCh and folds the user channel with `c mod userCh`.)
        abi::load_u64("%v11", "%v9", W_MIX_CH),
        abi::load_u64("%v12", abi::stack_pointer(), CH_OFF),
        abi::compare_registers("%v11", "%v12"),
        abi::branch_lt(&dev_fail),
    ]);
    // Initialize SHARED with the mix format (pointer at W_OUT0).
    emit_initialize(false, true, &mut ins);
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HR_OFF),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
        // CoTaskMemFree(mixWfx)
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), "%v9", W_OUT0),
    ]);
    ole_call(symbol, "CoTaskMemFree", 1, platform_imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::label(&init_ok));
    // Negotiated buffer frame count.
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, "%v9", W_BUFFER),
    ]);
    spill_obj(W_CLIENT, &mut ins);
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::c_arg(1), "%v9", W_BUFFER),
    ]);
    com_call(SLOT_AC_GET_BUFFER_SIZE, 2, &mut ins);
    if input {
        // bug-416 (2): allocate the capture carry-over buffer — one full device
        // buffer's worth (the maximum a single `GetBuffer` packet can hold) in the
        // device mix format. Input streams only; render never carries.
        ins.extend([
            abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
            abi::load_u32("%v10", "%v9", W_BUFFER), // negotiated buffer frames
            abi::load_u64("%v11", "%v9", W_MIX_BPF),
            abi::multiply_registers(abi::return_register(), "%v10", "%v11"),
            abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
        ins.extend([
            abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
            abi::store_u64(abi::RET[1], "%v9", W_CARRY_PTR),
        ]);
    }
    // CreateEventW(NULL, FALSE, FALSE, NULL) — auto-reset; NOT sign-extended.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
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
        abi::load_u64(abi::c_arg(1), "%v9", W_EVENT),
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
        abi::c_arg(1),
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
        abi::add_immediate(abi::c_arg(2), "%v9", W_SERVICE),
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
    let n = ins.len();
    let copy = format!("widen_dev_copy_{n}");
    let done = format!("widen_dev_done_{n}");
    let clamp_ok = format!("widen_dev_clamp_{n}");
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

/// Release the four COM interfaces the open acquired (reverse order), each
/// null-guarded. A null `STATE_OFF` (nothing allocated yet) skips the whole block.
fn emit_open_cleanup(ins: &mut Vec<CodeInstruction>) {
    emit_release_field(W_SERVICE, ins);
    emit_release_field(W_CLIENT, ins);
    emit_release_field(W_DEVICE, ins);
    emit_release_field(W_ENUM, ins);
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
