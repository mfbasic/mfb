// openOutput / openInput / openOutputDevice / openInputDevice / close.
//
// Open Decision 1 (no resampling) holds in both modes. SHARED at the device's own
// MIX FORMAT (`GetMixFormat`) comes first, whenever the mix rate is the requested
// rate, the mix is 32-bit float, and it has at least the requested channels; an
// EXCLUSIVE, event-driven s16le stream is initialized only when it cannot (plan-
// 158-C: the EXCLUSIVE attempt itself invalidates every SHARED stream on the
// endpoint). In SHARED, `write`/`read` convert between the caller's s16le frames
// and the mix format's 32-bit-float frames (`W_SHARED` selects the path;
// `W_MIX_CH`/`W_MIX_BPF` carry the mix geometry). AUTOCONVERTPCM is deliberately
// NOT used: on the Win11 test box it fast-fails (0xC0000409) inside WASAPI. An
// EXCLUSIVE output is not started at open: its first full buffer pre-rolls it
// (`emit_exclusive_submit`).

/// Build the caller's `WAVEFORMATEX` (s16le PCM) at `state->W_WFX`, as five 32-bit
/// stores (16-bit fields packed into dword halves; the last store zeroes `cbSize`
/// plus two pad bytes).
fn emit_build_wfx(ins: &mut Vec<CodeInstruction>, vregs: &mut Vregs) {
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(&v10, &v9, W_WFX),
        // +0: wFormatTag(=1) | nChannels<<16
        abi::load_u64(&v11, abi::stack_pointer(), CH_OFF),
        abi::shift_left_immediate(&v11, &v11, 16),
        abi::add_immediate(&v11, &v11, WAVE_FORMAT_PCM),
        abi::store_u32(&v11, &v10, 0),
        // +4: nSamplesPerSec
        abi::load_u64(&v11, abi::stack_pointer(), SR_OFF),
        abi::store_u32(&v11, &v10, 4),
        // +8: nAvgBytesPerSec = rate * bytesPerFrame
        abi::load_u64(&v12, abi::stack_pointer(), BPF_OFF),
        abi::multiply_registers(&v13, &v11, &v12),
        abi::store_u32(&v13, &v10, 8),
        // +12: nBlockAlign(=bpf) | wBitsPerSample<<16
        abi::move_immediate(&v13, "Integer", &(BITS_PER_SAMPLE << 16).to_string()),
        abi::add_registers(&v13, &v13, &v12),
        abi::store_u32(&v13, &v10, 12),
        // +16: cbSize = 0 (+2 pad bytes)
        abi::store_u32(abi::ZERO, &v10, 16),
    ]);
}

/// `device->Activate(IID_IAudioClient, CLSCTX_ALL, NULL, &state->W_CLIENT)`.
/// Stores the HRESULT at `HR_OFF`; raises through `dev_fail` on failure.
fn emit_activate_client(
    symbol: &str,
    dev_fail: &mut StatusFailSites,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) {
    spill_obj(W_DEVICE, ins, vregs);
    guid_addr(symbol, abi::c_arg(1), "IID_IAudioClient", ins, rel);
    let v9 = vregs.next();
    ins.extend([
        abi::move_immediate(abi::c_arg(2), "Integer", CLSCTX_ALL),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::c_arg(4), &v9, W_CLIENT),
    ]);
    com_call(SLOT_DEV_ACTIVATE, 5, ins, vregs);
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HR_OFF),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail.site("IMMDevice::Activate", abi::return_register())),
    ]);
}

/// `client->Initialize(shareMode, EVENTCALLBACK, bufDur, periodicity, pFormat, NULL)`.
/// `exclusive` selects the share mode and (per WASAPI's rules) the durations:
/// EXCLUSIVE passes `dur` for both buffer and periodicity; SHARED passes 0/0 (the
/// engine picks its own period — a non-zero value fast-fails). `mix_ptr` selects
/// the format: `false` → the inline `state->W_WFX`; `true` → the pointer at
/// `state->W_OUT0` (the `GetMixFormat` result). The HRESULT lands at `HR_OFF`.
fn emit_initialize(exclusive: bool, mix_ptr: bool, ins: &mut Vec<CodeInstruction>, vregs: &mut Vregs) {
    spill_obj(W_CLIENT, ins, vregs);
    let v9 = vregs.next();
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
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        if mix_ptr {
            abi::load_u64(abi::c_arg(5), &v9, W_OUT0)
        } else {
            abi::add_immediate(abi::c_arg(5), &v9, W_WFX)
        },
        // AudioSessionGuid = NULL (stack arg 2)
        abi::move_register(abi::c_arg(6), abi::ZERO),
    ]);
    com_call(SLOT_AC_INITIALIZE, 7, ins, vregs);
    ins.push(abi::store_u64(abi::return_register(), abi::stack_pointer(), HR_OFF));
}

/// `CoTaskMemFree` the `GetMixFormat` result at `state->W_OUT0` and clear the slot,
/// so the failed-open release (`emit_open_failure_release`) sees nothing to free.
fn emit_free_mix_format(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let v9 = vregs.next();
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), &v9, W_OUT0),
    ]);
    ole_call(symbol, "CoTaskMemFree", 1, platform_imports, platform, ins, rel)?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, &v9, W_OUT0),
    ]);
    Ok(())
}

pub(crate) fn lower_open(
    symbol: &str,
    input: bool,
    device: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let invalid = format!("{symbol}_invalid");
    let unavailable = format!("{symbol}_unavailable");
    let dev_fail = format!("{symbol}_dev_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let init_ok = format!("{symbol}_init_ok");
    let use_exclusive = format!("{symbol}_use_exclusive");
    let event_fail = format!("{symbol}_event_fail");
    let carry_fail = format!("{symbol}_carry_fail");
    let done = format!("{symbol}_done");
    // plan-158-A: every failing OS call reports its name and status. The two tails
    // run the same COM cleanup and differ only in the error they raise.
    let mut dev_sites =
        StatusFailSites::new(symbol, "ErrAudioDevice", &dev_fail, FAIL_STATUS_OFF, FAIL_PREFIX_OFF);
    let mut unavailable_sites = StatusFailSites::new(
        symbol,
        "ErrAudioUnavailable",
        &unavailable,
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
    let v15 = vregs.next();
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
    emit_validate_open(symbol, SR_OFF, CH_OFF, BF_OFF, &invalid, &mut ins, &mut vregs);
    // bytesPerFrame = channels * 2
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CH_OFF),
        abi::shift_left_immediate(&v9, &v9, 1),
        abi::store_u64(&v9, abi::stack_pointer(), BPF_OFF),
        // AudioHandle
        abi::move_immediate(abi::return_register(), "Integer", &H_RECORD_SIZE.to_string()),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::move_register(&v15, abi::mfb_return(1)),
        abi::store_u64(&v15, abi::stack_pointer(), HANDLE_OFF),
        // Canonical plan-80 header: tag@0, kind (handle)@8, closed@16, STATE@24.
        abi::move_immediate(&v9, "Integer", RESOURCE_TAG_AUDIO),
        abi::store_u64(&v9, &v15, RESOURCE_OFFSET_TAG),
        abi::store_u64(abi::ZERO, &v15, RESOURCE_OFFSET_STATE),
        abi::move_immediate(&v9, "Integer", if input { KIND_INPUT } else { KIND_OUTPUT }),
        abi::store_u64(&v9, &v15, H_KIND),
        abi::store_u64(abi::ZERO, &v15, H_CLOSED),
        abi::load_u64(&v9, abi::stack_pointer(), SR_OFF),
        abi::store_u64(&v9, &v15, H_SAMPLE_RATE),
        abi::load_u64(&v9, abi::stack_pointer(), CH_OFF),
        abi::store_u64(&v9, &v15, H_CHANNELS),
        abi::load_u64(&v9, abi::stack_pointer(), BPF_OFF),
        abi::store_u64(&v9, &v15, H_BYTES_PER_FRAME),
        abi::load_u64(&v9, abi::stack_pointer(), BF_OFF),
        abi::store_u64(&v9, &v15, H_BUFFER_FRAMES),
        // WASAPI STATE block (arena)
        abi::move_immediate(abi::return_register(), "Integer", &W_SIZE.to_string()),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::move_register(&v15, abi::mfb_return(1)),
        abi::store_u64(&v15, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v9, abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(&v15, &v9, H_STATE),
        abi::store_u64(abi::ZERO, &v15, W_ENUM),
        abi::store_u64(abi::ZERO, &v15, W_DEVICE),
        abi::store_u64(abi::ZERO, &v15, W_CLIENT),
        abi::store_u64(abi::ZERO, &v15, W_SERVICE),
        abi::store_u64(abi::ZERO, &v15, W_EVENT),
        abi::store_u64(abi::ZERO, &v15, W_STARTED),
        abi::store_u64(abi::ZERO, &v15, W_XRUNS),
        abi::store_u64(abi::ZERO, &v15, W_SHARED),
        // plan-158-A: `W_OUT0` holds the `GetMixFormat` allocation while the SHARED
        // fallback negotiates; null means nothing to free on a failed open.
        abi::store_u64(abi::ZERO, &v15, W_OUT0),
        // bug-416 (2): capture carry-over, empty until the first partial packet.
        abi::store_u64(abi::ZERO, &v15, W_CARRY_PTR),
        abi::store_u64(abi::ZERO, &v15, W_CARRY_FRAMES),
        abi::store_u64(abi::ZERO, &v15, W_CARRY_HEAD),
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
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::c_arg(4), &v9, W_ENUM),
    ]);
    ole_call(symbol, "CoCreateInstance", 5, platform_imports, platform, &mut ins, &mut rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&unavailable_sites.site("CoCreateInstance", abi::return_register())),
    ]);
    // Get the IMMDevice: default endpoint, or the named endpoint (device variant).
    if device {
        emit_widen_device_id(&mut ins, &mut vregs);
        spill_obj(W_ENUM, &mut ins, &mut vregs);
        ins.push(abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), WIDEID_OFF));
        ins.extend([
            abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
            abi::add_immediate(abi::c_arg(2), &v9, W_DEVICE),
        ]);
        com_call(SLOT_ENUM_GET_DEVICE, 3, &mut ins, &mut vregs);
        ins.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&dev_sites.site("IMMDeviceEnumerator::GetDevice", abi::return_register())),
        ]);
    } else {
        spill_obj(W_ENUM, &mut ins, &mut vregs);
        ins.extend([
            abi::move_immediate(abi::c_arg(1), "Integer", if input { E_CAPTURE } else { E_RENDER }),
            abi::move_immediate(abi::c_arg(2), "Integer", E_CONSOLE),
            abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
            abi::add_immediate(abi::c_arg(3), &v9, W_DEVICE),
        ]);
        com_call(SLOT_GET_DEFAULT_ENDPOINT, 4, &mut ins, &mut vregs);
        ins.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&dev_sites.site(
                "IMMDeviceEnumerator::GetDefaultAudioEndpoint",
                abi::return_register(),
            )),
        ]);
    }
    emit_activate_client(symbol, &mut dev_sites, &mut ins, &mut rel, &mut vregs);
    emit_build_wfx(&mut ins, &mut vregs);
    // dur = bufferFrames * REFTIMES_PER_SEC / sampleRate  -> TOTAL_OFF
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), BF_OFF),
        abi::move_immediate(&v10, "Integer", REFTIMES_PER_SEC),
        abi::multiply_registers(&v9, &v9, &v10),
        abi::load_u64(&v10, abi::stack_pointer(), SR_OFF),
        abi::unsigned_divide_registers(&v9, &v9, &v10),
        abi::store_u64(&v9, abi::stack_pointer(), TOTAL_OFF),
    ]);
    // --- SHARED first, at the device MIX FORMAT (plan-158-C) ------------------
    // An EXCLUSIVE `Initialize` ATTEMPT invalidates every SHARED stream on the
    // endpoint, even when it fails and the open falls back (plan-158-C C2). So the
    // mix format is read first, and SHARED is used whenever it can serve the
    // request without resampling; EXCLUSIVE is attempted only when it cannot.
    // GetMixFormat(&state->W_OUT0)
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, &v9, W_OUT0),
    ]);
    spill_obj(W_CLIENT, &mut ins, &mut vregs);
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::c_arg(1), &v9, W_OUT0),
    ]);
    com_call(SLOT_AC_GET_MIX_FORMAT, 2, &mut ins, &mut vregs);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_sites.site("IAudioClient::GetMixFormat", abi::return_register())),
        // Read the mix WAVEFORMATEX: nChannels @+2 (u16), nSamplesPerSec @+4 (u32),
        // nBlockAlign @+12 (u16), wBitsPerSample @+14 (u16).
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v10, &v9, W_OUT0), // mix wfx ptr
        abi::load_u16(&v11, &v10, 2),     // mixChannels
        abi::store_u64(&v11, &v9, W_MIX_CH),
        abi::load_u16(&v11, &v10, 12), // mixBlockAlign (frame stride)
        abi::store_u64(&v11, &v9, W_MIX_BPF),
        // No resampling: SHARED serves only the mix rate.
        abi::load_u32(&v11, &v10, 4), // mix rate
        abi::load_u64(&v12, abi::stack_pointer(), SR_OFF),
        abi::compare_registers(&v11, &v12),
        abi::branch_ne(&use_exclusive),
        // Only a 32-bit-float mix format is convertible by the integer s16<->f32 path.
        abi::load_u16(&v11, &v10, 14), // mix bits per sample
        abi::compare_immediate(&v11, "32"),
        abi::branch_ne(&use_exclusive),
        // bug-416 (3): the SHARED read converter (`emit_read_fill`) reads userCh
        // channels per frame from the device mix buffer (c in [0, userCh)); a mix
        // with fewer channels than the caller opened would read past the capture
        // buffer on the final frame. SHARED cannot serve mixCh < userCh — the read
        // path cannot synthesize channels the device does not provide. (The write
        // path is safe: it bounds c on mixCh and folds the user channel with
        // `c mod userCh`.)
        abi::load_u64(&v11, &v9, W_MIX_CH),
        abi::load_u64(&v12, abi::stack_pointer(), CH_OFF),
        abi::compare_registers(&v11, &v12),
        abi::branch_lt(&use_exclusive),
        abi::move_immediate(&v11, "Integer", "1"),
        abi::store_u64(&v11, &v9, W_SHARED),
    ]);
    // Initialize SHARED with the mix format (pointer at W_OUT0).
    emit_initialize(false, true, &mut ins, &mut vregs);
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HR_OFF),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_sites.site(
            "IAudioClient::Initialize(SHARED)",
            abi::return_register(),
        )),
    ]);
    emit_free_mix_format(symbol, platform_imports, platform, &mut ins, &mut rel, &mut vregs)?;
    ins.extend([
        abi::branch(&init_ok),
        // --- EXCLUSIVE s16le (Open Decision 1: no resampling) -----------------
        abi::label(&use_exclusive),
    ]);
    emit_free_mix_format(symbol, platform_imports, platform, &mut ins, &mut rel, &mut vregs)?;
    ins.extend([
        // Direct s16le: the mix geometry is the user geometry.
        abi::load_u64(&v15, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v9, abi::stack_pointer(), CH_OFF),
        abi::store_u64(&v9, &v15, W_MIX_CH),
        abi::load_u64(&v9, abi::stack_pointer(), BPF_OFF),
        abi::store_u64(&v9, &v15, W_MIX_BPF),
        abi::store_u64(abi::ZERO, &v15, W_SHARED),
    ]);
    emit_initialize(true, false, &mut ins, &mut vregs);
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HR_OFF),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_sites.site(
            "IAudioClient::Initialize(EXCLUSIVE)",
            abi::return_register(),
        )),
        abi::label(&init_ok),
    ]);
    // Negotiated buffer frame count.
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, &v9, W_BUFFER),
    ]);
    spill_obj(W_CLIENT, &mut ins, &mut vregs);
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::c_arg(1), &v9, W_BUFFER),
    ]);
    com_call(SLOT_AC_GET_BUFFER_SIZE, 2, &mut ins, &mut vregs);
    // plan-158-A: unchecked, a failure here left `W_BUFFER` at 0, so every later
    // `write` saw zero free frames and waited forever.
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_sites.site("IAudioClient::GetBufferSize", abi::return_register())),
    ]);
    // The carry: one full device buffer in the device format. bug-416 (2): an
    // input stream stashes the unconsumed tail of a capture packet there.
    // plan-158-C: an EXCLUSIVE output collects whole buffers there, since
    // EXCLUSIVE event-driven render takes exactly one full buffer per period.
    let no_carry = format!("{symbol}_no_carry");
    if !input {
        ins.extend([
            abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
            abi::load_u64(&v10, &v9, W_SHARED),
            abi::compare_immediate(&v10, "0"),
            abi::branch_ne(&no_carry),
        ]);
    }
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u32(&v10, &v9, W_BUFFER), // negotiated buffer frames
        abi::load_u64(&v11, &v9, W_MIX_BPF),
        abi::multiply_registers(abi::return_register(), &v10, &v11),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &carry_fail);
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::mfb_return(1), &v9, W_CARRY_PTR),
        abi::label(&no_carry),
    ]);
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
        abi::branch_eq(&event_fail),
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::return_register(), &v9, W_EVENT),
    ]);
    // client->SetEventHandle(event)
    spill_obj(W_CLIENT, &mut ins, &mut vregs);
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::c_arg(1), &v9, W_EVENT),
    ]);
    com_call(SLOT_AC_SET_EVENT_HANDLE, 2, &mut ins, &mut vregs);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_sites.site("IAudioClient::SetEventHandle", abi::return_register())),
    ]);
    // client->GetService(IID_IAudioRenderClient|IAudioCaptureClient, &state->W_SERVICE)
    spill_obj(W_CLIENT, &mut ins, &mut vregs);
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
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::c_arg(2), &v9, W_SERVICE),
    ]);
    com_call(SLOT_AC_GET_SERVICE, 3, &mut ins, &mut vregs);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_sites.site("IAudioClient::GetService", abi::return_register())),
    ]);
    // client->Start(). plan-158-C: not for an EXCLUSIVE output, which must be
    // pre-rolled with a full buffer before it starts; its first `write` submit
    // starts it (`emit_exclusive_submit`).
    let opened = format!("{symbol}_opened");
    if !input {
        ins.extend([
            abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
            abi::load_u64(&v10, &v9, W_SHARED),
            abi::compare_immediate(&v10, "0"),
            abi::branch_eq(&opened),
        ]);
    }
    spill_obj(W_CLIENT, &mut ins, &mut vregs);
    com_call(SLOT_AC_START, 1, &mut ins, &mut vregs);
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_sites.site("IAudioClient::Start", abi::return_register())),
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u64(&v10, &v9, W_STARTED),
        abi::label(&opened),
        // success
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
    ]);
    emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    // `CreateEventW` returns NULL, not a status: the status is `GetLastError()`,
    // read before any other Win32 call can overwrite it.
    let event_site = dev_sites.site("CreateEventW", abi::return_register());
    ins.push(abi::label(&event_fail));
    ole_call(symbol, "GetLastError", 0, platform_imports, platform, &mut ins, &mut rel)?;
    ins.push(abi::branch(&event_site));
    dev_sites.emit_stubs(&v9, &mut ins, &mut rel);
    unavailable_sites.emit_stubs(&v9, &mut ins, &mut rel);
    ins.push(abi::label(unavailable_sites.tail()));
    emit_open_cleanup(&mut ins, &mut vregs);
    emit_open_failure_release(symbol, platform_imports, platform, &mut ins, &mut rel, &mut vregs)?;
    unavailable_sites.emit_raise(&mut ins, &mut rel, &done);
    ins.push(abi::label(dev_sites.tail()));
    emit_open_cleanup(&mut ins, &mut vregs);
    emit_open_failure_release(symbol, platform_imports, platform, &mut ins, &mut rel, &mut vregs)?;
    dev_sites.emit_raise(&mut ins, &mut rel, &done);
    ins.push(abi::label(&carry_fail));
    emit_open_cleanup(&mut ins, &mut vregs);
    emit_open_failure_release(symbol, platform_imports, platform, &mut ins, &mut rel, &mut vregs)?;
    emit_fail(symbol, "ErrOutOfMemory", &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, "ErrOutOfMemory", &mut ins, &mut rel, &done);
    ins.push(abi::label(&done));
    ins.push(abi::return_());
    Ok((ins, rel, FRAME))
}

/// Widen the UTF-8 `id` of the `audio::AudioDevice` record at `DEVID_OFF` (the open's
/// first argument) into a NUL-terminated UTF-16 buffer at `WIDEID_OFF` (endpoint ids
/// are ASCII, so a byte->wchar zero-extend is exact). Clamps to 255 wchars.
///
/// plan-132 C4: this used to take `DEVID_OFF` itself as the id `String`, reading the
/// record's first word as the length and the next as bytes, so a device-specific
/// open handed WASAPI a garbage endpoint id. The `id` is the `String` inlined at
/// `record + [record + DEVICE_FIELD_ID]`.
fn emit_widen_device_id(ins: &mut Vec<CodeInstruction>, vregs: &mut Vregs) {
    let n = ins.len();
    let copy = format!("widen_dev_copy_{n}");
    let done = format!("widen_dev_done_{n}");
    let clamp_ok = format!("widen_dev_clamp_{n}");
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), DEVID_OFF),
        abi::load_u64(&v10, &v9, DEVICE_FIELD_ID),
        abi::add_registers(&v9, &v9, &v10), // the inlined `id` String
        abi::load_u64(&v10, &v9, 0),      // len
        abi::add_immediate(&v11, &v9, 8), // src bytes
        abi::move_immediate(&v9, "Integer", "255"),
        abi::compare_registers(&v10, &v9),
        abi::branch_le(&clamp_ok),
        abi::move_register(&v10, &v9),
        abi::label(&clamp_ok),
        abi::add_immediate(&v12, abi::stack_pointer(), WIDEID_OFF), // dst
        abi::move_immediate(&v13, "Integer", "0"),
        abi::label(&copy),
        abi::compare_registers(&v13, &v10),
        abi::branch_ge(&done),
        abi::load_u8(&v14, &v11, 0),
        abi::store_u8(&v14, &v12, 0),
        abi::store_u8(abi::ZERO, &v12, 1),
        abi::add_immediate(&v11, &v11, 1),
        abi::add_immediate(&v12, &v12, 2),
        abi::add_immediate(&v13, &v13, 1),
        abi::branch(&copy),
        abi::label(&done),
        abi::store_u8(abi::ZERO, &v12, 0),
        abi::store_u8(abi::ZERO, &v12, 1),
    ]);
}

/// Release the four COM interfaces the open acquired (reverse order), each
/// null-guarded. A null `STATE_OFF` (nothing allocated yet) skips the whole block.
fn emit_open_cleanup(ins: &mut Vec<CodeInstruction>, vregs: &mut Vregs) {
    emit_release_field(W_SERVICE, ins, vregs);
    emit_release_field(W_CLIENT, ins, vregs);
    emit_release_field(W_DEVICE, ins, vregs);
    emit_release_field(W_ENUM, ins, vregs);
}

/// plan-158-A: give back everything else a failed open acquired, after
/// [`emit_open_cleanup`] has released the COM interfaces. Every failure tail that
/// runs this is reached only after the handle record and the STATE block were
/// allocated, and nothing hands them to the caller, so they are freed here with
/// the mix format (`W_OUT0`, still held when a SHARED check failed), the event
/// handle, and an input stream's carry buffer. Each of those is null until
/// acquired. Measured on the box before this: 336 arena bytes per failed open.
fn emit_open_failure_release(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let n = ins.len();
    let no_mix = format!("{symbol}_frel_mix_{n}");
    let no_event = format!("{symbol}_frel_event_{n}");
    let v9 = vregs.next();
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), &v9, W_OUT0),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&no_mix),
    ]);
    ole_call(symbol, "CoTaskMemFree", 1, platform_imports, platform, ins, rel)?;
    ins.extend([
        abi::label(&no_mix),
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), &v9, W_EVENT),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&no_event),
    ]);
    ole_call(symbol, "CloseHandle", 1, platform_imports, platform, ins, rel)?;
    ins.push(abi::label(&no_event));
    emit_free_wasapi_state(symbol, ins, rel, vregs);
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate(abi::c_arg(1), "Integer", &H_RECORD_SIZE.to_string()),
    ]);
    crate::codegen::memory::arena::emit_arena_free(symbol, ins, rel);
    Ok(())
}

/// plan-158-A: free the arena WASAPI STATE block at `STATE_OFF` and the capture
/// carry buffer it points at (null until an input open allocates it). The carry
/// was sized `W_BUFFER * W_MIX_BPF`, both final before it was allocated.
fn emit_free_wasapi_state(
    symbol: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) {
    let no_carry = format!("{symbol}_free_state_carry_{}", ins.len());
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), &v9, W_CARRY_PTR),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&no_carry),
        abi::load_u32(&v10, &v9, W_BUFFER),
        abi::load_u64(&v11, &v9, W_MIX_BPF),
        abi::multiply_registers(abi::c_arg(1), &v10, &v11),
    ]);
    crate::codegen::memory::arena::emit_arena_free(symbol, ins, rel);
    ins.extend([
        abi::label(&no_carry),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE_OFF),
        abi::move_immediate(abi::c_arg(1), "Integer", &W_SIZE.to_string()),
    ]);
    crate::codegen::memory::arena::emit_arena_free(symbol, ins, rel);
}

/// Release the COM object at `state->field` if non-null, then null the slot.
fn emit_release_field(field: usize, ins: &mut Vec<CodeInstruction>, vregs: &mut Vregs) {
    let n = ins.len();
    let skip = format!("rel_skip_{field}_{n}");
    let v9 = vregs.next();
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&skip),
        abi::load_u64(&v9, &v9, field),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&skip),
        abi::store_u64(&v9, abi::stack_pointer(), OBJ_OFF),
    ]);
    com_call(SLOT_RELEASE, 1, ins, vregs);
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, &v9, field),
        abi::label(&skip),
    ]);
}

/// close(stream): Stop, Release the four interfaces, CloseHandle the event.
pub(crate) fn lower_close(
    symbol: &str,
    _input: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let already = format!("{symbol}_already");
    let no_event = format!("{symbol}_no_event");
    let done = format!("{symbol}_done");
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::load_u64(&v9, abi::return_register(), H_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&already),
        abi::load_u64(&v10, abi::return_register(), H_STATE),
        abi::store_u64(&v10, abi::stack_pointer(), STATE_OFF),
    ]);
    // plan-158-C: an EXCLUSIVE output holds a partial buffer in its carry (render
    // takes whole buffers only). Pad it with silence, submit it, and wait one period
    // so it plays before `Stop`. Best effort, like the rest of close: a failure
    // skips to the teardown.
    let flush_done = format!("{symbol}_flush_done");
    let flush_wait = format!("{symbol}_flush_wait");
    let pad_loop = format!("{symbol}_flush_pad");
    let v11 = vregs.next();
    let v12 = vregs.next();
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), HANDLE_OFF),
        abi::load_u64(&v10, &v9, H_KIND),
        abi::compare_immediate(&v10, KIND_INPUT),
        abi::branch_eq(&flush_done),
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v10, &v9, W_SHARED),
        abi::compare_immediate(&v10, "0"),
        abi::branch_ne(&flush_done),
    ]);
    emit_render_wait_timeout(&mut ins, &mut vregs);
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v10, &v9, W_CARRY_FRAMES),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&flush_wait),
        // zero bytes [carry*bpf, W_BUFFER*bpf) of the carry
        abi::load_u64(&v11, &v9, W_MIX_BPF),
        abi::multiply_registers(&v10, &v10, &v11), // first byte to clear
        abi::load_u32(&v12, &v9, W_BUFFER),
        abi::multiply_registers(&v12, &v12, &v11), // end
        abi::load_u64(&v11, &v9, W_CARRY_PTR),
        abi::label(&pad_loop),
        abi::compare_registers(&v10, &v12),
        abi::branch_ge(&format!("{pad_loop}_done")),
        abi::add_registers(&v9, &v11, &v10),
        abi::store_u8(abi::ZERO, &v9, 0),
        abi::add_immediate(&v10, &v10, 1),
        abi::branch(&pad_loop),
        abi::label(&format!("{pad_loop}_done")),
    ]);
    emit_exclusive_submit(
        symbol,
        &mut SubmitFail::Skip(&flush_done),
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
        &mut vregs,
    )?;
    ins.extend([
        abi::label(&flush_wait),
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v10, &v9, W_STARTED),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&flush_done),
    ]);
    emit_wait_event(symbol, Some(TIMEOUT_OFF), platform_imports, platform, &mut ins, &mut rel, &mut vregs)?;
    ins.push(abi::label(&flush_done));
    // client->Stop() (best effort)
    spill_obj(W_CLIENT, &mut ins, &mut vregs);
    com_call(SLOT_AC_STOP, 1, &mut ins, &mut vregs);
    emit_release_field(W_SERVICE, &mut ins, &mut vregs);
    emit_release_field(W_CLIENT, &mut ins, &mut vregs);
    emit_release_field(W_DEVICE, &mut ins, &mut vregs);
    emit_release_field(W_ENUM, &mut ins, &mut vregs);
    // CloseHandle(event)
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), &v9, W_EVENT),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&no_event),
    ]);
    ole_call(symbol, "CloseHandle", 1, platform_imports, platform, &mut ins, &mut rel)?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, &v9, W_EVENT),
        abi::label(&no_event),
    ]);
    // plan-158-A: the STATE block is the open's arena allocation and nothing else
    // frees it; every later operation on the handle reads `H_CLOSED` first and
    // never reaches `H_STATE`. Measured before this: 192 arena bytes per
    // open/close cycle on box 206.
    emit_free_wasapi_state(symbol, &mut ins, &mut rel, &mut vregs);
    ins.extend([
        // mark closed
        abi::load_u64(&v9, abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::ZERO, &v9, H_STATE),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u64(&v10, &v9, H_CLOSED),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        // The success path used to FALL THROUGH into `already`, which shared its
        // OK tag. It has to branch past the refusal now.
        abi::branch(&done),
        abi::label(&already),
    ]);
    // bug-525: an already-closed (or defaulted) stream is refused, not reported
    // as success. `mfb spec language resource-management` §15: "an already-closed
    // record is flagged, and a second close is a defined no-op reported as
    // `ErrResourceClosed` rather than an operation on a dead handle" — the same
    // answer `fs`, `tcp` and `udp` have always given. Letting a stream close at
    // the end of its scope after an explicit close still costs nothing: lexical
    // cleanup discards a close failure (§15).
    emit_fail(symbol, "ErrResourceClosed", &mut ins, &mut rel, &done);
    ins.extend([abi::label(&done), abi::return_()]);
    Ok((ins, rel, FRAME))
}
