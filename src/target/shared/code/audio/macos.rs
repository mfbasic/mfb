//! macOS Core Audio / AudioQueue backend for the `audio` package (plan-33-B).
//!
//! Device enumeration (`audio.devices`) queries Core Audio's
//! `AudioObjectGetPropertyData` on `kAudioObjectSystemObject`, converting each
//! device's `CFStringRef` name/UID with `CFStringGetCString`. Stream helpers land
//! in later phases. Every value that must survive a framework `bl` lives on the
//! stack and is reloaded afterward (runtime helpers clobber all caller-saved
//! registers).

use std::collections::HashMap;

use super::*;
use crate::target::shared::abi;

// --- Core Audio constants (verified against CoreAudio/AudioHardware.h) --------
const SYS_OBJECT: &str = "1"; // kAudioObjectSystemObject
const SEL_DEVICES: &str = "1684370979"; // 0x64657623 'dev#' kAudioHardwarePropertyDevices
const SEL_NAME: &str = "1819173229"; // 0x6C6E616D 'lnam' kAudioObjectPropertyName
const SEL_UID: &str = "1969841184"; // 0x75696420 'uid ' kAudioDevicePropertyDeviceUID
const SEL_STREAMCFG: &str = "1936482681"; // 0x736C6179 'slay' kAudioDevicePropertyStreamConfiguration
const SEL_DEFIN: &str = "1682533920"; // 0x64496E20 'dIn ' kAudioHardwarePropertyDefaultInputDevice
const SEL_DEFOUT: &str = "1682929012"; // 0x644F7574 'dOut' kAudioHardwarePropertyDefaultOutputDevice
const SCOPE_GLOBAL: &str = "1735159650"; // 0x676C6F62 'glob'
const SCOPE_INPUT: &str = "1768845428"; // 0x696E7074 'inpt'
const SCOPE_OUTPUT: &str = "1869968496"; // 0x6F757470 'outp'
const ENC_UTF8: &str = "134217984"; // kCFStringEncodingUTF8 = 0x08000100

// --- devices() stack frame ---------------------------------------------------
// Offsets are kept small (< ~1 KiB) so every `sp`-relative access stays within
// the AArch64 12-bit immediate range once the frame is finalized past the
// callee-saved area (a large offset would silently mis-address the buffer).
const FRAME_SIZE: usize = 1024;
const PROPADDR_OFF: usize = 16; // AudioObjectPropertyAddress (12 bytes)
const SIZE_OFF: usize = 32; // UInt32 ioDataSize
const COUNT_OFF: usize = 40;
const LIST_OFF: usize = 48;
const ENTRY_OFF: usize = 56; // entry-array cursor base
const DATA_OFF: usize = 64; // inline record data region base
const INDEX_OFF: usize = 72;
const CURID_OFF: usize = 80;
const DEFIN_OFF: usize = 88;
const DEFOUT_OFF: usize = 96;
const CFREF_OFF: usize = 104;
const IDPTR_OFF: usize = 112;
const NAMEPTR_OFF: usize = 120;
const CANIN_OFF: usize = 128;
const CANOUT_OFF: usize = 136;
const BOOLTMP_OFF: usize = 144;
const CSTRBUF_OFF: usize = 160; // 256-byte CFStringGetCString buffer
const CSTRBUF_CAP: &str = "256";
const IDSBUF_OFF: usize = 416; // up to 64 AudioDeviceID (u32)
const IDSBUF_CAP: &str = "256";
const BUFLIST_OFF: usize = 672; // AudioBufferList scratch
const BUFLIST_CAP: &str = "256";

pub(super) fn lower_audio_macos(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    match call {
        "audio.devices" => lower_devices(symbol, platform_imports, platform),
        "audio.openOutput" => lower_open_output(symbol, false, platform_imports, platform),
        "audio.openOutputDevice" => lower_open_output(symbol, true, platform_imports, platform),
        "audio.openInput" => lower_open_input(symbol, false, platform_imports, platform),
        "audio.openInputDevice" => lower_open_input(symbol, true, platform_imports, platform),
        "audio.read" => lower_read(symbol, false, platform_imports, platform),
        "audio.readTimeout" => lower_read(symbol, true, platform_imports, platform),
        "audio.closeInput" => lower_close_input(symbol, platform_imports, platform),
        "audio.write" => lower_write(symbol, platform_imports, platform),
        "audio.available" => lower_query(symbol, Query::Available, platform_imports, platform),
        "audio.xruns" => lower_query(symbol, Query::Xruns, platform_imports, platform),
        "audio.poll" => lower_query(symbol, Query::Poll, platform_imports, platform),
        "audio.pollTimeout" => lower_query(symbol, Query::PollTimeout, platform_imports, platform),
        "audio.closeOutput" => lower_close_output(symbol, platform_imports, platform),
        other => Err(format!(
            "native code plan does not emit runtime call '{other}' for macos-aarch64"
        )),
    }
}

// --- AudioQueue / mmap / format constants ------------------------------------
const FORMAT_LPCM: &str = "1819304813"; // 0x6C70636D 'lpcm' kAudioFormatLinearPCM
const FORMAT_FLAGS: &str = "12"; // kAudioFormatFlagIsSignedInteger | ...IsPacked
const MMAP_PROT: &str = "3"; // PROT_READ | PROT_WRITE
const MMAP_FLAGS: &str = "4098"; // MAP_ANON(0x1000) | MAP_PRIVATE(0x0002)
const MAP_FAILED_CMP: &str = "0";

// Parameter validation ranges (plan-33-A §3.5).
const SR_MIN: &str = "8000";
const SR_MAX: &str = "192000";
const BUF_MIN: &str = "64";
const BUF_MAX: &str = "8192";

/// Emit `pthread_<op>(state + field)` — object pointer in x0, called through the
/// platform ABI. `state_off` is the stack slot holding the AudioState pointer.
fn emit_pthread1(
    symbol: &str,
    op: &str,
    state_off: usize,
    field: usize,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), state_off),
        abi::add_immediate(abi::return_register(), abi::return_register(), field),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
    ]);
    platform.emit_libc_call(op, symbol, platform_imports, instructions, relocations)
}

/// Validate `openOutput`/`openInput` scalar parameters (sampleRate x-reg from
/// `sr_off`, channels from `ch_off`, bufferFrames from `bf_off`), branching to
/// `invalid` (→ ErrInvalidArgument) on any §3.5 violation.
fn emit_validate_open(
    symbol: &str,
    sr_off: usize,
    ch_off: usize,
    bf_off: usize,
    invalid: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let ch_ok = format!("{symbol}_ch_ok");
    instructions.extend([
        // sampleRate in 8000..=192000
        abi::load_u64("%v9", abi::stack_pointer(), sr_off),
        abi::move_immediate("%v10", "Integer", SR_MIN),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_lt(invalid),
        abi::move_immediate("%v10", "Integer", SR_MAX),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_gt(invalid),
        // channels 1 or 2
        abi::load_u64("%v9", abi::stack_pointer(), ch_off),
        abi::compare_immediate("%v9", "1"),
        abi::branch_eq(&ch_ok),
        abi::compare_immediate("%v9", "2"),
        abi::branch_ne(invalid),
        abi::label(&ch_ok),
        // bufferFrames in 64..=8192
        abi::load_u64("%v9", abi::stack_pointer(), bf_off),
        abi::move_immediate("%v10", "Integer", BUF_MIN),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_lt(invalid),
        abi::move_immediate("%v10", "Integer", BUF_MAX),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_gt(invalid),
    ]);
}

// Stream-helper stack frame.
const F: usize = 512;
const SR_OFF: usize = 8;
const CH_OFF: usize = 16;
const BF_OFF: usize = 24;
const BPF_OFF: usize = 32; // bytesPerFrame
const HANDLE_OFF: usize = 40;
const STATE_OFF: usize = 48;
const QUEUE_OFF: usize = 56;
const BUFPTR_OFF: usize = 64;
const I_OFF: usize = 72;
const CAP_OFF: usize = 80; // buffer capacity bytes
const OFFSET_OFF: usize = 88; // write byte cursor
const TOTAL_OFF: usize = 96; // write total bytes
const DEVID_OFF: usize = 104; // AudioDevice arg (device overloads)
const ASBD_OFF: usize = 128; // 40-byte AudioStreamBasicDescription -> 128..168
const UID_CFREF_OFF: usize = 168; // CFStringRef for device selection
const UID_CSTR_OFF: usize = 176; // 256-byte C string for the device UID -> 176..432

/// openOutput(sampleRate, channels, bufferFrames) or the device overload.
fn lower_open_output(
    symbol: &str,
    device: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let invalid = format!("{symbol}_invalid");
    let dev_fail = format!("{symbol}_dev_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let buf_loop = format!("{symbol}_buf_loop");
    let buf_done = format!("{symbol}_buf_done");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();

    // Argument staging. The device overload shifts the scalar args by one.
    if device {
        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), DEVID_OFF),
            abi::store_u64(abi::ARG[1], abi::stack_pointer(), SR_OFF),
            abi::store_u64(abi::ARG[2], abi::stack_pointer(), CH_OFF),
            abi::store_u64(abi::ARG[3], abi::stack_pointer(), BF_OFF),
        ]);
    } else {
        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), SR_OFF),
            abi::store_u64(abi::ARG[1], abi::stack_pointer(), CH_OFF),
            abi::store_u64(abi::ARG[2], abi::stack_pointer(), BF_OFF),
        ]);
    }
    // Zero the state slot so the open-error cleanup can tell the page was not yet
    // mapped (nothing to munmap/dispose before mmap and AudioQueueNew* run).
    instructions.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), STATE_OFF));
    emit_validate_open(symbol, SR_OFF, CH_OFF, BF_OFF, &invalid, &mut instructions);
    // bytesPerFrame = channels * 2
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), CH_OFF),
        abi::move_immediate("%v10", "Integer", "2"),
        abi::multiply_registers("%v9", "%v9", "%v10"),
        abi::store_u64("%v9", abi::stack_pointer(), BPF_OFF),
        // AudioHandle (arena, 64 B).
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &H_RECORD_SIZE.to_string(),
        ),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register("%v15", abi::RET[1]),
        abi::store_u64("%v15", abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate("%v9", "Integer", KIND_OUTPUT),
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
    ]);
    // mmap the AudioState page.
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"), // addr
        abi::move_immediate(abi::ARG[1], "Integer", &STATE_PAGE.to_string()),
        abi::move_immediate(abi::ARG[2], "Integer", MMAP_PROT),
        abi::move_immediate(abi::ARG[3], "Integer", MMAP_FLAGS),
        abi::bitwise_not(abi::ARG[4], abi::ZERO), // fd = -1
        abi::move_immediate(abi::ARG[5], "Integer", "0"), // offset
    ]);
    platform.emit_libc_call(
        "mmap",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // MAP_FAILED == (void*)-1
        abi::add_immediate("%v9", abi::return_register(), 1),
        abi::compare_immediate("%v9", MAP_FAILED_CMP),
        abi::branch_eq(&dev_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v15", abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::return_register(), "%v15", H_STATE),
        // Zero the bookkeeping words (mmap zero-fills, but be explicit).
        abi::load_u64("%v15", abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, "%v15", S_XRUNS),
        abi::store_u64(abi::ZERO, "%v15", S_CLOSED),
        abi::store_u64(abi::ZERO, "%v15", S_STARTED),
        abi::store_u64(abi::ZERO, "%v15", S_FREE_TOP),
        abi::store_u64(abi::ZERO, "%v15", S_RING_CAP),
        abi::move_immediate("%v9", "Integer", &STATE_PAGE.to_string()),
        abi::store_u64("%v9", "%v15", S_MAP_SIZE),
    ]);
    // pthread_mutex_init(state+S_MUTEX, NULL); pthread_cond_init(state+S_COND, NULL)
    emit_pthread1(
        symbol,
        "pthread_mutex_init",
        STATE_OFF,
        S_MUTEX,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_pthread1(
        symbol,
        "pthread_cond_init",
        STATE_OFF,
        S_COND,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // Build the ASBD.
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), SR_OFF),
        abi::signed_convert_to_float_d(abi::FP_SCRATCH[0], "%v9"),
        abi::store_double(abi::FP_SCRATCH[0], abi::stack_pointer(), ASBD_OFF),
        abi::move_immediate("%v9", "Integer", FORMAT_LPCM),
        abi::store_u32("%v9", abi::stack_pointer(), ASBD_OFF + 8),
        abi::move_immediate("%v9", "Integer", FORMAT_FLAGS),
        abi::store_u32("%v9", abi::stack_pointer(), ASBD_OFF + 12),
        abi::load_u64("%v9", abi::stack_pointer(), BPF_OFF),
        abi::store_u32("%v9", abi::stack_pointer(), ASBD_OFF + 16), // mBytesPerPacket
        abi::move_immediate("%v10", "Integer", "1"),
        abi::store_u32("%v10", abi::stack_pointer(), ASBD_OFF + 20), // mFramesPerPacket
        abi::store_u32("%v9", abi::stack_pointer(), ASBD_OFF + 24),  // mBytesPerFrame
        abi::load_u64("%v9", abi::stack_pointer(), CH_OFF),
        abi::store_u32("%v9", abi::stack_pointer(), ASBD_OFF + 28), // mChannelsPerFrame
        abi::move_immediate("%v9", "Integer", "16"),
        abi::store_u32("%v9", abi::stack_pointer(), ASBD_OFF + 32), // mBitsPerChannel
        abi::store_u32(abi::ZERO, abi::stack_pointer(), ASBD_OFF + 36),
    ]);
    // AudioQueueNewOutput(&asbd, callback, handle, NULL, NULL, 0, &state->osobject)
    instructions.extend([abi::add_immediate(
        abi::return_register(),
        abi::stack_pointer(),
        ASBD_OFF,
    )]);
    emit_data_address(
        symbol,
        abi::ARG[1],
        AUDIO_OUTPUT_CALLBACK_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
        abi::move_immediate(abi::ARG[4], "Integer", "0"),
        abi::move_immediate(abi::ARG[5], "Integer", "0"),
        abi::load_u64(abi::ARG[6], abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::ARG[6], abi::ARG[6], S_OSOBJECT),
    ]);
    platform.emit_libc_call(
        "AudioQueueNewOutput",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&dev_fail),
    ]);
    // Optionally select the named device.
    if device {
        emit_select_device(
            symbol,
            &dev_fail,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    }
    // Allocate NUM_BUFFERS buffers; all start free.
    instructions.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), I_OFF),
        abi::label(&buf_loop),
        abi::load_u64("%v9", abi::stack_pointer(), I_OFF),
        abi::compare_immediate("%v9", &NUM_BUFFERS.to_string()),
        abi::branch_eq(&buf_done),
        // AudioQueueAllocateBuffer(queue, bufferFrames*bytesPerFrame, &buf)
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), "%v10", S_OSOBJECT),
        abi::load_u64("%v11", abi::stack_pointer(), BF_OFF),
        abi::load_u64("%v12", abi::stack_pointer(), BPF_OFF),
        abi::multiply_registers("%v11", "%v11", "%v12"),
        abi::move_register(abi::ARG[1], "%v11"),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), BUFPTR_OFF),
    ]);
    platform.emit_libc_call(
        "AudioQueueAllocateBuffer",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&dev_fail),
        // freebufs[i] = buf; (i already == free_top since all free)
        abi::load_u64("%v9", abi::stack_pointer(), I_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::add_immediate("%v10", "%v10", S_FREE_BUFS),
        abi::move_immediate("%v11", "Integer", "8"),
        abi::multiply_registers("%v12", "%v9", "%v11"),
        abi::add_registers("%v10", "%v10", "%v12"),
        abi::load_u64("%v11", abi::stack_pointer(), BUFPTR_OFF),
        abi::store_u64("%v11", "%v10", 0),
        abi::add_immediate("%v9", "%v9", 1),
        abi::store_u64("%v9", abi::stack_pointer(), I_OFF),
        abi::branch(&buf_loop),
        abi::label(&buf_done),
        // free_top = NUM_BUFFERS
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::move_immediate("%v9", "Integer", &NUM_BUFFERS.to_string()),
        abi::store_u64("%v9", "%v10", S_FREE_TOP),
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u64("%v9", "%v10", S_STARTED),
        // AudioQueueStart(queue, NULL)
        abi::load_u64(abi::return_register(), "%v10", S_OSOBJECT),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
    ]);
    platform.emit_libc_call(
        "AudioQueueStart",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&dev_fail),
        // Success: return the handle.
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
    ]);
    emit_fail(
        symbol,
        ERR_INVALID_ARGUMENT_CODE,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&dev_fail));
    emit_open_cleanup(
        symbol,
        &mut instructions,
        &mut relocations,
        platform,
        platform_imports,
    )?;
    emit_fail(
        symbol,
        ERR_AUDIO_DEVICE_CODE,
        ERR_AUDIO_DEVICE_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());

    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], F);
    Ok((frame, instructions, relocations, stack_slots))
}

/// AudioQueueSetProperty(queue, kAudioQueueProperty_CurrentDevice, &uidCF, 8)
/// from the `AudioDevice.id` string, selecting the named device.
fn emit_select_device(
    symbol: &str,
    dev_fail: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    // kAudioQueueProperty_CurrentDevice = 'aqcd' = 0x61716364 = 1634820964.
    // Build a CFString from the device id, set it, release it.
    let copy_loop = format!("{symbol}_uid_copy");
    let copy_done = format!("{symbol}_uid_copy_done");
    let clamp_ok = format!("{symbol}_uid_clamp_ok");
    // The device record's `id` String field pointer is at DEVID_OFF's record + H? No:
    // DEVID_OFF holds the AudioDevice record pointer; its `id` field is at offset 0.
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), DEVID_OFF),
        abi::load_u64("%v9", "%v9", DEVICE_FIELD_ID), // id String ptr
        abi::store_u64("%v9", abi::stack_pointer(), BUFPTR_OFF),
        // Copy the String (len-prefixed) into the UID C-string buffer.
        abi::load_u64("%v10", "%v9", 0),      // len
        abi::add_immediate("%v11", "%v9", 8), // src bytes
        // Clamp the copy count to the 256-byte UID buffer minus the NUL
        // terminator; an oversized device id would otherwise overrun it.
        abi::move_immediate("%v9", "Integer", "255"),
        abi::compare_registers("%v10", "%v9"),
        abi::branch_le(&clamp_ok),
        abi::move_register("%v10", "%v9"),
        abi::label(&clamp_ok),
        abi::add_immediate("%v12", abi::stack_pointer(), UID_CSTR_OFF),
        abi::move_immediate("%v13", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("%v13", "%v10"),
        abi::branch_eq(&copy_done),
        abi::load_u8("%v14", "%v11", 0),
        abi::store_u8("%v14", "%v12", 0),
        abi::add_immediate("%v11", "%v11", 1),
        abi::add_immediate("%v12", "%v12", 1),
        abi::add_immediate("%v13", "%v13", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, "%v12", 0),
        // CFStringCreateWithCString(NULL, uidCStr, kCFStringEncodingUTF8)
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), UID_CSTR_OFF),
        abi::move_immediate(abi::ARG[2], "Integer", ENC_UTF8),
    ]);
    platform.emit_libc_call(
        "CFStringCreateWithCString",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), UID_CFREF_OFF),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(dev_fail),
        // AudioQueueSetProperty(queue, 'aqcd', &cfref, 8)
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), "%v10", S_OSOBJECT),
        abi::move_immediate(abi::ARG[1], "Integer", "1634820964"),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), UID_CFREF_OFF),
        abi::move_immediate(abi::ARG[3], "Integer", "8"),
    ]);
    platform.emit_libc_call(
        "AudioQueueSetProperty",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CAP_OFF), // save status
        // CFRelease(cfref)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), UID_CFREF_OFF),
    ]);
    platform.emit_libc_call(
        "CFRelease",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), CAP_OFF),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(dev_fail),
    ]);
    Ok(())
}

/// Open-error cleanup (bug-180): before an open fails, dispose any AudioQueue
/// that was created and munmap the state page, so a device error does not leak
/// them. Safe to reach before either exists — `STATE_OFF` is zeroed at entry (so
/// the page-mapped test fails when mmap never ran) and mmap zero-fills
/// `S_OSOBJECT` (so the queue-created test fails before `AudioQueueNew*`).
fn emit_open_cleanup(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
) -> Result<(), String> {
    let munmap = format!("{symbol}_cleanup_munmap");
    let skip = format!("{symbol}_cleanup_skip");
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&skip),
        abi::load_u64("%v9", "%v10", S_OSOBJECT),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&munmap),
        // AudioQueueDispose(queue, 1)
        abi::move_register(abi::return_register(), "%v9"),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    platform.emit_libc_call(
        "AudioQueueDispose",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::label(&munmap),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::ARG[1], abi::return_register(), S_MAP_SIZE),
    ]);
    platform.emit_libc_call(
        "munmap",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.push(abi::label(&skip));
    Ok(())
}

/// Store a 12-byte `AudioObjectPropertyAddress { selector, scope, element=0 }`
/// into `sp + PROPADDR_OFF`.
fn build_propaddr(selector: &str, scope: &str, instructions: &mut Vec<CodeInstruction>) {
    instructions.extend([
        abi::move_immediate("%v9", "Integer", selector),
        abi::store_u32("%v9", abi::stack_pointer(), PROPADDR_OFF),
        abi::move_immediate("%v9", "Integer", scope),
        abi::store_u32("%v9", abi::stack_pointer(), PROPADDR_OFF + 4),
        abi::store_u32(abi::ZERO, abi::stack_pointer(), PROPADDR_OFF + 8),
    ]);
}

/// `AudioObjectGetPropertyData(object, &PROPADDR, 0, NULL, &SIZE, out_ptr)`.
/// `object` is loaded from `object_off` (a stack slot). `SIZE` is preloaded with
/// `size_val`. Leaves the `OSStatus` in the return register.
#[allow(clippy::too_many_arguments)]
fn call_get_property(
    symbol: &str,
    object_off: usize,
    size_val: &str,
    out_off: usize,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    instructions.extend([
        abi::move_immediate("%v9", "Integer", size_val),
        abi::store_u32("%v9", abi::stack_pointer(), SIZE_OFF),
        abi::load_u32(abi::return_register(), abi::stack_pointer(), object_off),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), PROPADDR_OFF),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
        abi::add_immediate(abi::ARG[4], abi::stack_pointer(), SIZE_OFF),
        abi::add_immediate(abi::ARG[5], abi::stack_pointer(), out_off),
    ]);
    platform.emit_libc_call(
        "AudioObjectGetPropertyData",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    // OSStatus is a 32-bit SInt32 returned in w0; the upper half of x0 is
    // undefined, so extend before any full-width compare (bug-04).
    instructions.push(abi::sign_extend_word(
        abi::return_register(),
        abi::return_register(),
    ));
    Ok(())
}

/// Read the `CFStringRef` property `selector` of the device in `CURID_OFF`,
/// convert it to an MFBASIC `String` at `out_off`, and `CFRelease` it. Branches
/// to `dev_fail` on any Core Audio / CoreFoundation failure, `alloc_fail` on OOM.
#[allow(clippy::too_many_arguments)]
fn emit_cfstring_field(
    symbol: &str,
    selector: &str,
    out_off: usize,
    dev_fail: &str,
    alloc_fail: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let field = out_off; // unique label suffix
    let copy_loop = format!("{symbol}_cf{field}_copy");
    let copy_done = format!("{symbol}_cf{field}_copy_done");
    let len_loop = format!("{symbol}_cf{field}_len");
    let len_done = format!("{symbol}_cf{field}_len_done");

    build_propaddr(selector, SCOPE_GLOBAL, instructions);
    call_get_property(
        symbol,
        CURID_OFF,
        "8",
        CFREF_OFF,
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(dev_fail),
        // CFStringGetCString(cfref, CSTRBUF, 256, kCFStringEncodingUTF8)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CFREF_OFF),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), CSTRBUF_OFF),
        abi::move_immediate(abi::ARG[2], "Integer", CSTRBUF_CAP),
        abi::move_immediate(abi::ARG[3], "Integer", ENC_UTF8),
    ]);
    platform.emit_libc_call(
        "CFStringGetCString",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        // Boolean is a 32-bit result in w0 (bug-04).
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), BOOLTMP_OFF),
        // CFRelease(cfref)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CFREF_OFF),
    ]);
    platform.emit_libc_call(
        "CFRelease",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), BOOLTMP_OFF),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(dev_fail),
        // strlen(CSTRBUF)
        abi::add_immediate("%v9", abi::stack_pointer(), CSTRBUF_OFF),
        abi::move_immediate("%v10", "Integer", "0"),
        abi::label(&len_loop),
        abi::load_u8("%v11", "%v9", 0),
        abi::compare_immediate("%v11", "0"),
        abi::branch_eq(&len_done),
        abi::add_immediate("%v9", "%v9", 1),
        abi::add_immediate("%v10", "%v10", 1),
        abi::branch(&len_loop),
        abi::label(&len_done),
        abi::store_u64("%v10", abi::stack_pointer(), SIZE_OFF),
        // Allocate String: [u64 len][bytes][nul].
        abi::add_immediate(abi::return_register(), "%v10", 9),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::move_register("%v15", abi::RET[1]),
        abi::load_u64("%v10", abi::stack_pointer(), SIZE_OFF),
        abi::store_u64("%v10", "%v15", 0),
        abi::store_u64("%v15", abi::stack_pointer(), out_off),
        abi::add_immediate("%v11", abi::stack_pointer(), CSTRBUF_OFF),
        abi::add_immediate("%v12", "%v15", 8),
        abi::move_immediate("%v13", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("%v13", "%v10"),
        abi::branch_eq(&copy_done),
        abi::load_u8("%v14", "%v11", 0),
        abi::store_u8("%v14", "%v12", 0),
        abi::add_immediate("%v11", "%v11", 1),
        abi::add_immediate("%v12", "%v12", 1),
        abi::add_immediate("%v13", "%v13", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, "%v12", 0),
    ]);
    Ok(())
}

/// Sum `mNumberChannels` across the device's stream configuration in `scope`,
/// storing `1` (any channel) or `0` into `out_off`. A failed query means the
/// direction is unsupported → `0`.
#[allow(clippy::too_many_arguments)]
fn emit_channel_flag(
    symbol: &str,
    scope: &str,
    out_off: usize,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let unsupported = format!("{symbol}_ch{out_off}_none");
    let sum_loop = format!("{symbol}_ch{out_off}_loop");
    let sum_done = format!("{symbol}_ch{out_off}_done");
    let set_flag = format!("{symbol}_ch{out_off}_flag");

    instructions.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), out_off));
    build_propaddr(SEL_STREAMCFG, scope, instructions);
    call_get_property(
        symbol,
        CURID_OFF,
        BUFLIST_CAP,
        BUFLIST_OFF,
        platform,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&unsupported),
        // mNumberBuffers @ BUFLIST[0]; buffers start at BUFLIST+8, stride 16,
        // mNumberChannels at +0.
        abi::add_immediate("%v9", abi::stack_pointer(), BUFLIST_OFF),
        abi::load_u32("%v10", "%v9", 0),             // nbuf
        abi::add_immediate("%v11", "%v9", 8),        // buffer cursor
        abi::move_immediate("%v12", "Integer", "0"), // i
        abi::move_immediate("%v13", "Integer", "0"), // sum
        abi::label(&sum_loop),
        abi::compare_registers("%v12", "%v10"),
        abi::branch_eq(&sum_done),
        abi::load_u32("%v14", "%v11", 0),
        abi::add_registers("%v13", "%v13", "%v14"),
        abi::add_immediate("%v11", "%v11", 16),
        abi::add_immediate("%v12", "%v12", 1),
        abi::branch(&sum_loop),
        abi::label(&sum_done),
        abi::compare_immediate("%v13", "0"),
        abi::branch_ne(&set_flag),
        abi::branch(&unsupported),
        abi::label(&set_flag),
        abi::move_immediate("%v13", "Integer", "1"),
        abi::store_u64("%v13", abi::stack_pointer(), out_off),
        abi::label(&unsupported),
    ]);
    Ok(())
}

/// write(output, bytes): block until every byte is queued for playback.
fn lower_write(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let invalid = format!("{symbol}_invalid");
    let dev_fail = format!("{symbol}_dev_fail");
    let write_loop = format!("{symbol}_write_loop");
    let write_done = format!("{symbol}_write_done");
    let wait_loop = format!("{symbol}_wait_loop");
    let wait_ready = format!("{symbol}_wait_ready");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let cap_ok = format!("{symbol}_cap_ok");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), DEVID_OFF), // byteList ptr
        // Guard write-after-close via the arena-resident mirror (state may be
        // unmapped): if handle->H_CLOSED, raise.
        abi::load_u64("%v9", abi::return_register(), H_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&dev_fail),
        abi::load_u64("%v10", abi::return_register(), H_STATE),
        abi::store_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v10", abi::return_register(), H_BYTES_PER_FRAME),
        abi::store_u64("%v10", abi::stack_pointer(), BPF_OFF),
        abi::load_u64("%v11", abi::return_register(), H_BUFFER_FRAMES),
        abi::multiply_registers("%v12", "%v11", "%v10"),
        abi::store_u64("%v12", abi::stack_pointer(), CAP_OFF),
        abi::load_u64("%v13", abi::ARG[1], COLLECTION_OFFSET_COUNT),
        abi::store_u64("%v13", abi::stack_pointer(), TOTAL_OFF),
        // The byte payload starts past the CAPACITY-sized entry array, not the
        // COUNT-sized one: an append-built list carries spare capacity, so
        // HEADER + CAPACITY*ENTRY is the data-region base (byteList + count*ENTRY
        // would land in the middle of the entry array — bug: static playback).
        abi::load_u64("%v12", abi::ARG[1], COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("%v14", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v14", "%v12", "%v14"),
        abi::add_immediate("%v14", "%v14", COLLECTION_HEADER_SIZE),
        abi::add_registers("%v14", abi::ARG[1], "%v14"),
        abi::store_u64("%v14", abi::stack_pointer(), QUEUE_OFF), // src base
        abi::compare_immediate("%v13", "0"),
        abi::branch_eq(&invalid),
        abi::load_u64("%v10", abi::stack_pointer(), BPF_OFF),
        abi::subtract_immediate("%v10", "%v10", 1), // mask = bpf-1
        abi::and_registers("%v11", "%v13", "%v10"),
        abi::compare_immediate("%v11", "0"),
        abi::branch_ne(&invalid),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), OFFSET_OFF),
        abi::label(&write_loop),
        abi::load_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), TOTAL_OFF),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_ge(&write_done),
    ]);
    emit_pthread1(
        symbol,
        "pthread_mutex_lock",
        STATE_OFF,
        S_MUTEX,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&wait_loop),
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v9", "%v10", S_FREE_TOP),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&wait_ready),
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::return_register(), "%v10", S_COND),
        abi::add_immediate(abi::ARG[1], "%v10", S_MUTEX),
    ]);
    platform.emit_libc_call(
        "pthread_cond_wait",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::branch(&wait_loop),
        abi::label(&wait_ready),
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v9", "%v10", S_FREE_TOP),
        abi::subtract_immediate("%v9", "%v9", 1),
        abi::store_u64("%v9", "%v10", S_FREE_TOP),
        abi::add_immediate("%v11", "%v10", S_FREE_BUFS),
        abi::move_immediate("%v12", "Integer", "8"),
        abi::multiply_registers("%v13", "%v9", "%v12"),
        abi::add_registers("%v11", "%v11", "%v13"),
        abi::load_u64("%v14", "%v11", 0),
        abi::store_u64("%v14", abi::stack_pointer(), BUFPTR_OFF),
    ]);
    emit_pthread1(
        symbol,
        "pthread_mutex_unlock",
        STATE_OFF,
        S_MUTEX,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), TOTAL_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), OFFSET_OFF),
        abi::subtract_registers("%v9", "%v9", "%v10"),
        abi::load_u64("%v11", abi::stack_pointer(), CAP_OFF),
        abi::compare_registers("%v9", "%v11"),
        abi::branch_le(&cap_ok),
        abi::move_register("%v9", "%v11"),
        abi::label(&cap_ok),
        abi::store_u64("%v9", abi::stack_pointer(), I_OFF), // n
        abi::load_u64("%v12", abi::stack_pointer(), QUEUE_OFF),
        abi::load_u64("%v13", abi::stack_pointer(), OFFSET_OFF),
        abi::add_registers("%v12", "%v12", "%v13"), // src
        abi::load_u64("%v14", abi::stack_pointer(), BUFPTR_OFF),
        abi::load_u64("%v15", "%v14", 8), // mAudioData
        abi::move_immediate("%v16", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("%v16", "%v9"),
        abi::branch_ge(&copy_done),
        abi::load_u8("%v17", "%v12", 0),
        abi::store_u8("%v17", "%v15", 0),
        abi::add_immediate("%v12", "%v12", 1),
        abi::add_immediate("%v15", "%v15", 1),
        abi::add_immediate("%v16", "%v16", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::load_u64("%v14", abi::stack_pointer(), BUFPTR_OFF),
        abi::load_u64("%v9", abi::stack_pointer(), I_OFF),
        abi::store_u32("%v9", "%v14", 16), // mAudioDataByteSize
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), "%v10", S_OSOBJECT),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), BUFPTR_OFF),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
    ]);
    platform.emit_libc_call(
        "AudioQueueEnqueueBuffer",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&dev_fail),
        abi::load_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), I_OFF),
        abi::add_registers("%v9", "%v9", "%v10"),
        abi::store_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::branch(&write_loop),
        abi::label(&write_done),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
    ]);
    emit_fail(
        symbol,
        ERR_INVALID_ARGUMENT_CODE,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&dev_fail));
    emit_fail(
        symbol,
        ERR_AUDIO_DEVICE_CODE,
        ERR_AUDIO_DEVICE_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], F);
    Ok((frame, instructions, relocations, stack_slots))
}

/// closeOutput(output): drain, stop, dispose, destroy, munmap. Idempotent.
fn lower_close_output(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let already = format!("{symbol}_already");
    let drain_loop = format!("{symbol}_drain_loop");
    let drain_done = format!("{symbol}_drain_done");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::load_u64("%v9", abi::return_register(), H_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&already),
        abi::load_u64("%v10", abi::return_register(), H_STATE),
        abi::store_u64("%v10", abi::stack_pointer(), STATE_OFF),
    ]);
    emit_pthread1(
        symbol,
        "pthread_mutex_lock",
        STATE_OFF,
        S_MUTEX,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&drain_loop),
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v9", "%v10", S_FREE_TOP),
        abi::compare_immediate("%v9", &NUM_BUFFERS.to_string()),
        abi::branch_ge(&drain_done),
        abi::add_immediate(abi::return_register(), "%v10", S_COND),
        abi::add_immediate(abi::ARG[1], "%v10", S_MUTEX),
    ]);
    platform.emit_libc_call(
        "pthread_cond_wait",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::branch(&drain_loop),
        abi::label(&drain_done),
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u64("%v9", "%v10", S_CLOSED),
    ]);
    emit_pthread1(
        symbol,
        "pthread_mutex_unlock",
        STATE_OFF,
        S_MUTEX,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), "%v10", S_OSOBJECT),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    platform.emit_libc_call(
        "AudioQueueStop",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), "%v10", S_OSOBJECT),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    platform.emit_libc_call(
        "AudioQueueDispose",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_pthread1(
        symbol,
        "pthread_cond_destroy",
        STATE_OFF,
        S_COND,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_pthread1(
        symbol,
        "pthread_mutex_destroy",
        STATE_OFF,
        S_MUTEX,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u64("%v9", "%v10", H_CLOSED),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::ARG[1], abi::return_register(), S_MAP_SIZE),
    ]);
    platform.emit_libc_call(
        "munmap",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&already),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], F);
    Ok((frame, instructions, relocations, stack_slots))
}

#[derive(Clone, Copy)]
enum Query {
    Available,
    Poll,
    PollTimeout,
    Xruns,
}

/// available/poll/xruns(stream): read the mutex-guarded counters, branching on
/// handle->kind. Output uses free_top*bufferFrames; input the ring (lands with
/// the input phase).
fn lower_query(
    symbol: &str,
    kind: Query,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let is_input = format!("{symbol}_input");
    let have = format!("{symbol}_have");
    let closed = format!("{symbol}_closed");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        HANDLE_OFF,
    ));
    if let Query::PollTimeout = kind {
        // Spill `timeoutMs` (ARG[1]) before any libc call clobbers it.
        instructions.push(abi::store_u64(
            abi::ARG[1],
            abi::stack_pointer(),
            TIMEOUT_OFF,
        ));
    }
    instructions.extend([
        // Closed-resource guard: a defaulted/closed handle has an invalid (null)
        // state page, so return the empty answer (0 / FALSE) without locking it.
        abi::load_u64("%v9", abi::return_register(), H_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("%v10", abi::return_register(), H_STATE),
        abi::store_u64("%v10", abi::stack_pointer(), STATE_OFF),
    ]);
    if let Query::PollTimeout = kind {
        // pollTimeout(input, timeoutMs): wait up to `timeoutMs` for data (input:
        // ring fill; output: a free buffer), returning TRUE the moment it is
        // available and FALSE at the deadline. Mirrors the timed-read wait but
        // yields a Boolean. The result is stashed in I_OFF and loaded into the
        // result register only after the unlock (which clobbers caller-saved).
        let pt_loop = format!("{symbol}_pt_loop");
        let pt_ready = format!("{symbol}_pt_ready");
        let pt_expired = format!("{symbol}_pt_expired");
        let pt_input = format!("{symbol}_pt_input");
        let pt_have = format!("{symbol}_pt_have");
        let pt_result = format!("{symbol}_pt_result");
        emit_pthread1(
            symbol,
            "pthread_mutex_lock",
            STATE_OFF,
            S_MUTEX,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        // deadline = now + timeoutMs*1e6 (CLOCK_MONOTONIC = 6 on macOS).
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", "6"),
            abi::add_immediate(abi::ARG[1], abi::stack_pointer(), CLK_OFF),
        ]);
        platform.emit_libc_call(
            "clock_gettime",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::load_u64("%v9", abi::stack_pointer(), CLK_OFF),
            abi::move_immediate("%v10", "Integer", "1000000000"),
            abi::multiply_registers("%v9", "%v9", "%v10"),
            abi::load_u64("%v11", abi::stack_pointer(), CLK_OFF + 8),
            abi::add_registers("%v9", "%v9", "%v11"),
            abi::load_u64("%v12", abi::stack_pointer(), TIMEOUT_OFF),
            abi::move_immediate("%v13", "Integer", "1000000"),
            abi::multiply_registers("%v12", "%v12", "%v13"),
            abi::add_registers("%v9", "%v9", "%v12"),
            abi::store_u64("%v9", abi::stack_pointer(), DEADLINE_OFF),
            abi::label(&pt_loop),
            // available = input ? S_RING_FILL : S_FREE_TOP (nonzero => ready).
            abi::load_u64("%v9", abi::stack_pointer(), HANDLE_OFF),
            abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
            abi::load_u64("%v11", "%v9", H_KIND),
            abi::compare_immediate("%v11", KIND_INPUT),
            abi::branch_eq(&pt_input),
            abi::load_u64("%v12", "%v10", S_FREE_TOP),
            abi::branch(&pt_have),
            abi::label(&pt_input),
            abi::load_u64("%v12", "%v10", S_RING_FILL),
            abi::label(&pt_have),
            abi::compare_immediate("%v12", "0"),
            abi::branch_ne(&pt_ready),
            // No data yet: has the deadline passed?
            abi::move_immediate(abi::return_register(), "Integer", "6"),
            abi::add_immediate(abi::ARG[1], abi::stack_pointer(), CLK_OFF),
        ]);
        platform.emit_libc_call(
            "clock_gettime",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::load_u64("%v9", abi::stack_pointer(), CLK_OFF),
            abi::move_immediate("%v10", "Integer", "1000000000"),
            abi::multiply_registers("%v9", "%v9", "%v10"),
            abi::load_u64("%v11", abi::stack_pointer(), CLK_OFF + 8),
            abi::add_registers("%v9", "%v9", "%v11"), // now
            abi::load_u64("%v12", abi::stack_pointer(), DEADLINE_OFF),
            abi::compare_registers("%v9", "%v12"),
            abi::branch_ge(&pt_expired),
            // remaining = deadline - now, split into a relative timespec.
            abi::subtract_registers("%v12", "%v12", "%v9"),
            abi::move_immediate("%v13", "Integer", "1000000000"),
            abi::unsigned_divide_registers("%v14", "%v12", "%v13"),
            abi::store_u64("%v14", abi::stack_pointer(), TS_OFF),
            abi::multiply_registers("%v14", "%v14", "%v13"),
            abi::subtract_registers("%v14", "%v12", "%v14"),
            abi::store_u64("%v14", abi::stack_pointer(), TS_OFF + 8),
            // pthread_cond_timedwait_relative_np(cond, mutex, ts)
            abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
            abi::add_immediate(abi::return_register(), "%v10", S_COND),
            abi::add_immediate(abi::ARG[1], "%v10", S_MUTEX),
            abi::add_immediate(abi::ARG[2], abi::stack_pointer(), TS_OFF),
        ]);
        platform.emit_libc_call(
            "pthread_cond_timedwait_relative_np",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::branch(&pt_loop),
            abi::label(&pt_ready),
            abi::move_immediate("%v9", "Integer", "1"),
            abi::store_u64("%v9", abi::stack_pointer(), I_OFF),
            abi::branch(&pt_result),
            abi::label(&pt_expired),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), I_OFF),
            abi::label(&pt_result),
        ]);
        emit_pthread1(
            symbol,
            "pthread_mutex_unlock",
            STATE_OFF,
            S_MUTEX,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.push(abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            I_OFF,
        ));
    } else {
        emit_pthread1(
            symbol,
            "pthread_mutex_lock",
            STATE_OFF,
            S_MUTEX,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::load_u64("%v9", abi::stack_pointer(), HANDLE_OFF),
            abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
            abi::load_u64("%v11", "%v9", H_KIND),
            abi::compare_immediate("%v11", KIND_INPUT),
            abi::branch_eq(&is_input),
            abi::load_u64("%v12", "%v10", S_FREE_TOP),
            abi::load_u64("%v13", "%v9", H_BUFFER_FRAMES),
            abi::multiply_registers("%v12", "%v12", "%v13"),
            abi::branch(&have),
            abi::label(&is_input),
            abi::load_u64("%v12", "%v10", S_RING_FILL),
            abi::load_u64("%v13", "%v9", H_BYTES_PER_FRAME),
            // frames = fill / bytesPerFrame; bytesPerFrame is 2 (mono) or 4 (stereo),
            // so >>1 then a further >>1 when stereo.
            abi::shift_right_immediate("%v12", "%v12", 1),
            abi::compare_immediate("%v13", "2"),
            abi::branch_eq(&have),
            abi::shift_right_immediate("%v12", "%v12", 1),
            abi::label(&have),
            abi::store_u64("%v12", abi::stack_pointer(), I_OFF),
            abi::load_u64("%v14", "%v10", S_XRUNS),
            abi::store_u64("%v14", abi::stack_pointer(), CAP_OFF),
        ]);
        emit_pthread1(
            symbol,
            "pthread_mutex_unlock",
            STATE_OFF,
            S_MUTEX,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        match kind {
            Query::Available => instructions.push(abi::load_u64(
                RESULT_VALUE_REGISTER,
                abi::stack_pointer(),
                I_OFF,
            )),
            Query::Xruns => instructions.push(abi::load_u64(
                RESULT_VALUE_REGISTER,
                abi::stack_pointer(),
                CAP_OFF,
            )),
            Query::Poll => {
                let poll_set = format!("{symbol}_poll_set");
                instructions.extend([
                    abi::load_u64("%v9", abi::stack_pointer(), I_OFF),
                    abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
                    abi::compare_immediate("%v9", "0"),
                    abi::branch_eq(&poll_set),
                    abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
                    abi::label(&poll_set),
                ]);
            }
            Query::PollTimeout => unreachable!("pollTimeout handled above"),
        }
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        // Closed handle: the empty answer (available/xruns 0, poll FALSE).
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], F);
    Ok((frame, instructions, relocations, stack_slots))
}

/// The AudioQueue output callback (C-ABI): void cb(void* handle, AudioQueueRef,
/// AudioQueueBufferRef). Runs on an ordinary AudioQueue thread, so taking the
/// mutex is legal (plan-33-B §3.1). Marks the played buffer free and signals.
pub(in crate::target::shared::code) fn lower_audio_output_callback(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    const CB_FRAME: usize = 64;
    const CB_HANDLE: usize = 8;
    const CB_BUF: usize = 16;
    const CB_STATE: usize = 24;
    let symbol = AUDIO_OUTPUT_CALLBACK_SYMBOL;
    let ret = format!("{symbol}_ret");
    let no_underrun = format!("{symbol}_no_underrun");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CB_HANDLE),
        abi::store_u64(abi::ARG[2], abi::stack_pointer(), CB_BUF),
        abi::load_u64("%v9", abi::return_register(), H_STATE),
        abi::store_u64("%v9", abi::stack_pointer(), CB_STATE),
    ]);
    emit_pthread1(
        symbol,
        "pthread_mutex_lock",
        CB_STATE,
        S_MUTEX,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), CB_STATE),
        abi::load_u64("%v9", "%v10", S_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&ret),
        abi::load_u64("%v9", "%v10", S_FREE_TOP),
        abi::add_immediate("%v11", "%v10", S_FREE_BUFS),
        abi::move_immediate("%v12", "Integer", "8"),
        abi::multiply_registers("%v13", "%v9", "%v12"),
        abi::add_registers("%v11", "%v11", "%v13"),
        abi::load_u64("%v14", abi::stack_pointer(), CB_BUF),
        abi::store_u64("%v14", "%v11", 0),
        abi::add_immediate("%v9", "%v9", 1),
        abi::store_u64("%v9", "%v10", S_FREE_TOP),
        abi::compare_immediate("%v9", &NUM_BUFFERS.to_string()),
        abi::branch_lt(&no_underrun),
        abi::load_u64("%v12", "%v10", S_STARTED),
        abi::compare_immediate("%v12", "0"),
        abi::branch_eq(&no_underrun),
        abi::load_u64("%v13", "%v10", S_XRUNS),
        abi::add_immediate("%v13", "%v13", 1),
        abi::store_u64("%v13", "%v10", S_XRUNS),
        abi::label(&no_underrun),
        abi::add_immediate(abi::return_register(), "%v10", S_COND),
    ]);
    platform.emit_libc_call(
        "pthread_cond_signal",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&ret));
    emit_pthread1(
        symbol,
        "pthread_mutex_unlock",
        CB_STATE,
        S_MUTEX,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], CB_FRAME);
    Ok(CodeFunction {
        name: "runtime.audio.outputCallback".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        stack_slots,
        instructions,
        relocations,
    })
}

// Input/read frame offsets (read never builds the ASBD/UID buffers, so it
// reuses that region for the timespec/clock scratch).
const TS_OFF: usize = 128; // relative timespec (16 B)
const CLK_OFF: usize = 144; // clock_gettime result (16 B)
const FINAL_LIST_OFF: usize = 160; // right-sized result for a partial timed read
const DEADLINE_OFF: usize = 56; // absolute deadline (ns)
const PAYLOAD_OFF: usize = 64; // byte-list payload cursor
const NEED_OFF: usize = 72; // requested bytes
const LIST_PTR_OFF: usize = 80; // result list ptr
const GOT_OFF: usize = 88; // bytes accumulated so far
const FRAMES_OFF: usize = 96; // requested frames
const TIMEOUT_OFF: usize = 104; // timeoutMs
                                // openInput-only scratch (device path uses UID buffers; default path uses these).
const PRECHK_ADDR: usize = 440;
const PRECHK_SIZE: usize = 456;
const PRECHK_ID: usize = 464;
const RINGCAP_OFF: usize = 472;
const MAPSIZE_OFF: usize = 480;
const READ_FRAMES_MAX: &str = "1048576";
const TIMEOUT_MAX: &str = "86400000";

/// Allocate a `List OF Byte` of `count` bytes (count in `count_off`), filling the
/// header + entry array; the byte payload (at list + HEADER + count*ENTRY) is
/// left uninitialized. Stores the list pointer at `list_off`.
fn emit_alloc_byte_list(
    symbol: &str,
    tag: &str,
    count_off: usize,
    list_off: usize,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let entry_loop = format!("{symbol}_{tag}_bl_entry");
    let entry_done = format!("{symbol}_{tag}_bl_entry_done");
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), count_off),
        abi::move_immediate("%v11", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v12", "%v10", "%v11"),
        abi::add_immediate("%v12", "%v12", COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), "%v12", "%v10"), // + count payload bytes
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::move_register("%v15", abi::RET[1]),
        abi::store_u64("%v15", abi::stack_pointer(), list_off),
        abi::move_immediate("%v9", "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_KIND),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("%v9", "Byte", "1"),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64("%v10", abi::stack_pointer(), count_off),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_COUNT),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_CAPACITY),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_DATA_CAPACITY),
        // entry array: entry[i] = { USED, value_offset=i, value_length=1 }
        abi::add_immediate("%v11", "%v15", COLLECTION_HEADER_SIZE), // entry cursor
        abi::move_immediate("%v13", "Integer", "0"),                // i
        abi::label(&entry_loop),
        abi::compare_registers("%v13", "%v10"),
        abi::branch_ge(&entry_done),
        abi::move_immediate("%v14", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8("%v14", "%v11", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::store_u64("%v13", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::move_immediate("%v14", "Integer", "1"),
        abi::store_u64("%v14", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_immediate("%v11", "%v11", COLLECTION_ENTRY_SIZE),
        abi::add_immediate("%v13", "%v13", 1),
        abi::branch(&entry_loop),
        abi::label(&entry_done),
    ]);
}

/// openInput(sampleRate, channels, bufferFrames) or the device overload.
fn lower_open_input(
    symbol: &str,
    device: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let invalid = format!("{symbol}_invalid");
    let dev_fail = format!("{symbol}_dev_fail");
    let unavailable = format!("{symbol}_unavailable");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let buf_loop = format!("{symbol}_buf_loop");
    let buf_done = format!("{symbol}_buf_done");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    if device {
        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), DEVID_OFF),
            abi::store_u64(abi::ARG[1], abi::stack_pointer(), SR_OFF),
            abi::store_u64(abi::ARG[2], abi::stack_pointer(), CH_OFF),
            abi::store_u64(abi::ARG[3], abi::stack_pointer(), BF_OFF),
        ]);
    } else {
        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), SR_OFF),
            abi::store_u64(abi::ARG[1], abi::stack_pointer(), CH_OFF),
            abi::store_u64(abi::ARG[2], abi::stack_pointer(), BF_OFF),
        ]);
    }
    // Zero the state slot so the open-error cleanup can tell the page was not yet
    // mapped (nothing to munmap/dispose before mmap and AudioQueueNew* run).
    instructions.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), STATE_OFF));
    emit_validate_open(symbol, SR_OFF, CH_OFF, BF_OFF, &invalid, &mut instructions);
    // §4.5: for the default overload, require a default input device.
    if !device {
        instructions.extend([
            abi::move_immediate("%v9", "Integer", SEL_DEFIN),
            abi::store_u32("%v9", abi::stack_pointer(), PRECHK_ADDR),
            abi::move_immediate("%v9", "Integer", SCOPE_GLOBAL),
            abi::store_u32("%v9", abi::stack_pointer(), PRECHK_ADDR + 4),
            abi::store_u32(abi::ZERO, abi::stack_pointer(), PRECHK_ADDR + 8),
            abi::move_immediate("%v9", "Integer", "4"),
            abi::store_u32("%v9", abi::stack_pointer(), PRECHK_SIZE),
            abi::store_u32(abi::ZERO, abi::stack_pointer(), PRECHK_ID),
            abi::move_immediate(abi::return_register(), "Integer", SYS_OBJECT),
            abi::add_immediate(abi::ARG[1], abi::stack_pointer(), PRECHK_ADDR),
            abi::move_immediate(abi::ARG[2], "Integer", "0"),
            abi::move_immediate(abi::ARG[3], "Integer", "0"),
            abi::add_immediate(abi::ARG[4], abi::stack_pointer(), PRECHK_SIZE),
            abi::add_immediate(abi::ARG[5], abi::stack_pointer(), PRECHK_ID),
        ]);
        platform.emit_libc_call(
            "AudioObjectGetPropertyData",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(&unavailable),
            abi::load_u32("%v9", abi::stack_pointer(), PRECHK_ID),
            abi::compare_immediate("%v9", "0"),
            abi::branch_eq(&unavailable),
        ]);
    }
    // bytesPerFrame, ringCap, mapSize.
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), CH_OFF),
        abi::move_immediate("%v10", "Integer", "2"),
        abi::multiply_registers("%v9", "%v9", "%v10"),
        abi::store_u64("%v9", abi::stack_pointer(), BPF_OFF),
        // ringCap = bufferFrames * bytesPerFrame * NUM_BUFFERS
        abi::load_u64("%v11", abi::stack_pointer(), BF_OFF),
        abi::multiply_registers("%v11", "%v11", "%v9"),
        abi::move_immediate("%v12", "Integer", &NUM_BUFFERS.to_string()),
        abi::multiply_registers("%v11", "%v11", "%v12"),
        abi::store_u64("%v11", abi::stack_pointer(), RINGCAP_OFF),
        // mapSize = round_up(S_RING + ringCap, STATE_PAGE)
        abi::add_immediate("%v11", "%v11", S_RING),
        abi::add_immediate("%v11", "%v11", STATE_PAGE - 1),
        abi::shift_right_immediate("%v11", "%v11", 14),
        abi::shift_left_immediate("%v11", "%v11", 14),
        abi::store_u64("%v11", abi::stack_pointer(), MAPSIZE_OFF),
        // AudioHandle.
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &H_RECORD_SIZE.to_string(),
        ),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register("%v15", abi::RET[1]),
        abi::store_u64("%v15", abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate("%v9", "Integer", KIND_INPUT),
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
        // mmap(0, mapSize, PROT, FLAGS, -1, 0)
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), MAPSIZE_OFF),
        abi::move_immediate(abi::ARG[2], "Integer", MMAP_PROT),
        abi::move_immediate(abi::ARG[3], "Integer", MMAP_FLAGS),
        abi::bitwise_not(abi::ARG[4], abi::ZERO),
        abi::move_immediate(abi::ARG[5], "Integer", "0"),
    ]);
    platform.emit_libc_call(
        "mmap",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::add_immediate("%v9", abi::return_register(), 1),
        abi::compare_immediate("%v9", MAP_FAILED_CMP),
        abi::branch_eq(&dev_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v15", abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::return_register(), "%v15", H_STATE),
        abi::load_u64("%v15", abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, "%v15", S_XRUNS),
        abi::store_u64(abi::ZERO, "%v15", S_CLOSED),
        abi::store_u64(abi::ZERO, "%v15", S_STARTED),
        abi::store_u64(abi::ZERO, "%v15", S_RING_HEAD),
        abi::store_u64(abi::ZERO, "%v15", S_RING_TAIL),
        abi::store_u64(abi::ZERO, "%v15", S_RING_FILL),
        abi::load_u64("%v9", abi::stack_pointer(), RINGCAP_OFF),
        abi::store_u64("%v9", "%v15", S_RING_CAP),
        abi::load_u64("%v9", abi::stack_pointer(), MAPSIZE_OFF),
        abi::store_u64("%v9", "%v15", S_MAP_SIZE),
    ]);
    emit_pthread1(
        symbol,
        "pthread_mutex_init",
        STATE_OFF,
        S_MUTEX,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_pthread1(
        symbol,
        "pthread_cond_init",
        STATE_OFF,
        S_COND,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // ASBD.
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), SR_OFF),
        abi::signed_convert_to_float_d(abi::FP_SCRATCH[0], "%v9"),
        abi::store_double(abi::FP_SCRATCH[0], abi::stack_pointer(), ASBD_OFF),
        abi::move_immediate("%v9", "Integer", FORMAT_LPCM),
        abi::store_u32("%v9", abi::stack_pointer(), ASBD_OFF + 8),
        abi::move_immediate("%v9", "Integer", FORMAT_FLAGS),
        abi::store_u32("%v9", abi::stack_pointer(), ASBD_OFF + 12),
        abi::load_u64("%v9", abi::stack_pointer(), BPF_OFF),
        abi::store_u32("%v9", abi::stack_pointer(), ASBD_OFF + 16),
        abi::move_immediate("%v10", "Integer", "1"),
        abi::store_u32("%v10", abi::stack_pointer(), ASBD_OFF + 20),
        abi::store_u32("%v9", abi::stack_pointer(), ASBD_OFF + 24),
        abi::load_u64("%v9", abi::stack_pointer(), CH_OFF),
        abi::store_u32("%v9", abi::stack_pointer(), ASBD_OFF + 28),
        abi::move_immediate("%v9", "Integer", "16"),
        abi::store_u32("%v9", abi::stack_pointer(), ASBD_OFF + 32),
        abi::store_u32(abi::ZERO, abi::stack_pointer(), ASBD_OFF + 36),
        // AudioQueueNewInput(&asbd, callback, handle, NULL, NULL, 0, &osobject)
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), ASBD_OFF),
    ]);
    emit_data_address(
        symbol,
        abi::ARG[1],
        AUDIO_INPUT_CALLBACK_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
        abi::move_immediate(abi::ARG[4], "Integer", "0"),
        abi::move_immediate(abi::ARG[5], "Integer", "0"),
        abi::load_u64(abi::ARG[6], abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::ARG[6], abi::ARG[6], S_OSOBJECT),
    ]);
    platform.emit_libc_call(
        "AudioQueueNewInput",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&dev_fail),
    ]);
    if device {
        emit_select_device(
            symbol,
            &dev_fail,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    }
    // Allocate NUM_BUFFERS buffers and enqueue each (input buffers must be
    // enqueued to receive captured data).
    instructions.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), I_OFF),
        abi::label(&buf_loop),
        abi::load_u64("%v9", abi::stack_pointer(), I_OFF),
        abi::compare_immediate("%v9", &NUM_BUFFERS.to_string()),
        abi::branch_eq(&buf_done),
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), "%v10", S_OSOBJECT),
        abi::load_u64("%v11", abi::stack_pointer(), BF_OFF),
        abi::load_u64("%v12", abi::stack_pointer(), BPF_OFF),
        abi::multiply_registers("%v11", "%v11", "%v12"),
        abi::move_register(abi::ARG[1], "%v11"),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), BUFPTR_OFF),
    ]);
    platform.emit_libc_call(
        "AudioQueueAllocateBuffer",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&dev_fail),
        // AudioQueueEnqueueBuffer(queue, buf, 0, NULL)
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), "%v10", S_OSOBJECT),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), BUFPTR_OFF),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
    ]);
    platform.emit_libc_call(
        "AudioQueueEnqueueBuffer",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&dev_fail),
        abi::load_u64("%v9", abi::stack_pointer(), I_OFF),
        abi::add_immediate("%v9", "%v9", 1),
        abi::store_u64("%v9", abi::stack_pointer(), I_OFF),
        abi::branch(&buf_loop),
        abi::label(&buf_done),
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u64("%v9", "%v10", S_STARTED),
        abi::load_u64(abi::return_register(), "%v10", S_OSOBJECT),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
    ]);
    platform.emit_libc_call(
        "AudioQueueStart",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&dev_fail),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
    ]);
    emit_fail(
        symbol,
        ERR_INVALID_ARGUMENT_CODE,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&unavailable));
    emit_fail(
        symbol,
        ERR_AUDIO_UNAVAILABLE_CODE,
        ERR_AUDIO_UNAVAILABLE_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&dev_fail));
    emit_open_cleanup(
        symbol,
        &mut instructions,
        &mut relocations,
        platform,
        platform_imports,
    )?;
    emit_fail(
        symbol,
        ERR_AUDIO_DEVICE_CODE,
        ERR_AUDIO_DEVICE_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], F);
    Ok((frame, instructions, relocations, stack_slots))
}

/// read(input, frames[, timeoutMs]). Drains the ring incrementally into the
/// pre-allocated result across multiple callback fills, so the ring can be small
/// relative to a large read. On timeout expiry, returns the whole frames
/// gathered so far (possibly none) in a right-sized list.
fn lower_read(
    symbol: &str,
    timeout: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let invalid = format!("{symbol}_invalid");
    let dev_fail = format!("{symbol}_dev_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let drain_loop = format!("{symbol}_drain");
    let drain_done = format!("{symbol}_drain_done");
    let wait_data = format!("{symbol}_wait");
    let have_data = format!("{symbol}_have");
    let copy_loop = format!("{symbol}_copy");
    let copy_wrap_ok = format!("{symbol}_copy_wrap");
    let copy_done = format!("{symbol}_copy_done");
    let chunk_ok = format!("{symbol}_chunk_ok");
    let ret_full = format!("{symbol}_ret_full");
    let fin_loop = format!("{symbol}_fin");
    let fin_done = format!("{symbol}_fin_done");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), FRAMES_OFF),
    ]);
    if timeout {
        instructions.push(abi::store_u64(
            abi::ARG[2],
            abi::stack_pointer(),
            TIMEOUT_OFF,
        ));
    }
    instructions.extend([
        abi::load_u64("%v9", abi::return_register(), H_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&dev_fail),
        abi::load_u64("%v10", abi::return_register(), H_STATE),
        abi::store_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v10", abi::return_register(), H_BYTES_PER_FRAME),
        abi::store_u64("%v10", abi::stack_pointer(), BPF_OFF),
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
        instructions.extend([
            abi::load_u64("%v9", abi::stack_pointer(), TIMEOUT_OFF),
            abi::move_immediate("%v11", "Integer", TIMEOUT_MAX),
            abi::compare_registers("%v9", "%v11"),
            abi::branch_gt(&invalid),
        ]);
    }
    // Allocate the result (sized for the full request) before the lock, and cache
    // its payload base. `got` accumulates across ring fills.
    emit_alloc_byte_list(
        symbol,
        "main",
        NEED_OFF,
        LIST_PTR_OFF,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64("%v11", abi::stack_pointer(), LIST_PTR_OFF),
        abi::load_u64("%v9", abi::stack_pointer(), NEED_OFF),
        abi::move_immediate("%v13", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v13", "%v9", "%v13"),
        abi::add_immediate("%v13", "%v13", COLLECTION_HEADER_SIZE),
        abi::add_registers("%v11", "%v11", "%v13"),
        abi::store_u64("%v11", abi::stack_pointer(), PAYLOAD_OFF), // payload base
        abi::store_u64(abi::ZERO, abi::stack_pointer(), GOT_OFF),
    ]);
    emit_pthread1(
        symbol,
        "pthread_mutex_lock",
        STATE_OFF,
        S_MUTEX,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    if timeout {
        // deadline = now + timeoutMs*1e6 (CLOCK_MONOTONIC = 6).
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", "6"),
            abi::add_immediate(abi::ARG[1], abi::stack_pointer(), CLK_OFF),
        ]);
        platform.emit_libc_call(
            "clock_gettime",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::load_u64("%v9", abi::stack_pointer(), CLK_OFF),
            abi::move_immediate("%v10", "Integer", "1000000000"),
            abi::multiply_registers("%v9", "%v9", "%v10"),
            abi::load_u64("%v11", abi::stack_pointer(), CLK_OFF + 8),
            abi::add_registers("%v9", "%v9", "%v11"),
            abi::load_u64("%v12", abi::stack_pointer(), TIMEOUT_OFF),
            abi::move_immediate("%v13", "Integer", "1000000"),
            abi::multiply_registers("%v12", "%v12", "%v13"),
            abi::add_registers("%v9", "%v9", "%v12"),
            abi::store_u64("%v9", abi::stack_pointer(), DEADLINE_OFF),
        ]);
    }
    instructions.extend([
        abi::label(&drain_loop),
        abi::load_u64("%v9", abi::stack_pointer(), GOT_OFF),
        abi::load_u64("%v11", abi::stack_pointer(), NEED_OFF),
        abi::compare_registers("%v9", "%v11"),
        abi::branch_ge(&drain_done),
        // wait for data
        abi::label(&wait_data),
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v9", "%v10", S_RING_FILL),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&have_data),
    ]);
    if timeout {
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", "6"),
            abi::add_immediate(abi::ARG[1], abi::stack_pointer(), CLK_OFF),
        ]);
        platform.emit_libc_call(
            "clock_gettime",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::load_u64("%v9", abi::stack_pointer(), CLK_OFF),
            abi::move_immediate("%v10", "Integer", "1000000000"),
            abi::multiply_registers("%v9", "%v9", "%v10"),
            abi::load_u64("%v11", abi::stack_pointer(), CLK_OFF + 8),
            abi::add_registers("%v9", "%v9", "%v11"), // now
            abi::load_u64("%v12", abi::stack_pointer(), DEADLINE_OFF),
            abi::compare_registers("%v9", "%v12"),
            abi::branch_ge(&drain_done),                    // expired
            abi::subtract_registers("%v12", "%v12", "%v9"), // remaining
            abi::move_immediate("%v13", "Integer", "1000000000"),
            abi::unsigned_divide_registers("%v14", "%v12", "%v13"),
            abi::store_u64("%v14", abi::stack_pointer(), TS_OFF),
            abi::multiply_registers("%v14", "%v14", "%v13"),
            abi::subtract_registers("%v14", "%v12", "%v14"),
            abi::store_u64("%v14", abi::stack_pointer(), TS_OFF + 8),
            abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
            abi::add_immediate(abi::return_register(), "%v10", S_COND),
            abi::add_immediate(abi::ARG[1], "%v10", S_MUTEX),
            abi::add_immediate(abi::ARG[2], abi::stack_pointer(), TS_OFF),
        ]);
        platform.emit_libc_call(
            "pthread_cond_timedwait_relative_np",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.push(abi::branch(&wait_data));
    } else {
        instructions.extend([
            abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
            abi::add_immediate(abi::return_register(), "%v10", S_COND),
            abi::add_immediate(abi::ARG[1], "%v10", S_MUTEX),
        ]);
        platform.emit_libc_call(
            "pthread_cond_wait",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.push(abi::branch(&wait_data));
    }
    // have_data: chunk = min(need-got, fill); copy chunk bytes from ring[tail] to
    // payload[got], wrapping; advance tail/fill/got.
    instructions.extend([
        abi::label(&have_data),
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v13", "%v10", S_RING_FILL),
        abi::load_u64("%v9", abi::stack_pointer(), NEED_OFF),
        abi::load_u64("%v11", abi::stack_pointer(), GOT_OFF),
        abi::subtract_registers("%v9", "%v9", "%v11"), // remaining = need-got
        abi::compare_registers("%v9", "%v13"),
        abi::branch_le(&chunk_ok),
        abi::move_register("%v9", "%v13"), // chunk = fill
        abi::label(&chunk_ok),
        // dst = payload + got
        abi::load_u64("%v12", abi::stack_pointer(), PAYLOAD_OFF),
        abi::add_registers("%v12", "%v12", "%v11"),
        abi::add_immediate("%v14", "%v10", S_RING), // ring base
        abi::load_u64("%v16", "%v10", S_RING_TAIL),
        abi::load_u64("%v20", "%v10", S_RING_CAP),
        abi::move_immediate("%v17", "Integer", "0"), // i
        abi::label(&copy_loop),
        abi::compare_registers("%v17", "%v9"),
        abi::branch_ge(&copy_done),
        abi::add_registers("%v18", "%v14", "%v16"),
        abi::load_u8("%v19", "%v18", 0),
        abi::add_registers("%v18", "%v12", "%v17"),
        abi::store_u8("%v19", "%v18", 0),
        abi::add_immediate("%v16", "%v16", 1),
        abi::compare_registers("%v16", "%v20"),
        abi::branch_lt(&copy_wrap_ok),
        abi::move_immediate("%v16", "Integer", "0"),
        abi::label(&copy_wrap_ok),
        abi::add_immediate("%v17", "%v17", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        // tail = %v16; fill -= chunk; got += chunk
        abi::store_u64("%v16", "%v10", S_RING_TAIL),
        abi::load_u64("%v13", "%v10", S_RING_FILL),
        abi::subtract_registers("%v13", "%v13", "%v9"),
        abi::store_u64("%v13", "%v10", S_RING_FILL),
        abi::load_u64("%v11", abi::stack_pointer(), GOT_OFF),
        abi::add_registers("%v11", "%v11", "%v9"),
        abi::store_u64("%v11", abi::stack_pointer(), GOT_OFF),
        abi::branch(&drain_loop),
        abi::label(&drain_done),
    ]);
    emit_pthread1(
        symbol,
        "pthread_mutex_unlock",
        STATE_OFF,
        S_MUTEX,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // If we filled the request, return the pre-allocated result; otherwise (timed
    // partial) build a right-sized list of `got` bytes.
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), GOT_OFF),
        abi::load_u64("%v11", abi::stack_pointer(), NEED_OFF),
        abi::compare_registers("%v9", "%v11"),
        abi::branch_ge(&ret_full),
    ]);
    emit_alloc_byte_list(
        symbol,
        "final",
        GOT_OFF,
        FINAL_LIST_OFF,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        // copy `got` bytes from the oversized payload into the final payload.
        abi::load_u64("%v9", abi::stack_pointer(), GOT_OFF),
        abi::load_u64("%v11", abi::stack_pointer(), FINAL_LIST_OFF),
        abi::move_immediate("%v13", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v13", "%v9", "%v13"),
        abi::add_immediate("%v13", "%v13", COLLECTION_HEADER_SIZE),
        abi::add_registers("%v11", "%v11", "%v13"), // final payload
        abi::load_u64("%v12", abi::stack_pointer(), PAYLOAD_OFF), // source payload
        abi::move_immediate("%v17", "Integer", "0"),
        abi::label(&fin_loop),
        abi::compare_registers("%v17", "%v9"),
        abi::branch_ge(&fin_done),
        abi::add_registers("%v18", "%v12", "%v17"),
        abi::load_u8("%v19", "%v18", 0),
        abi::add_registers("%v18", "%v11", "%v17"),
        abi::store_u8("%v19", "%v18", 0),
        abi::add_immediate("%v17", "%v17", 1),
        abi::branch(&fin_loop),
        abi::label(&fin_done),
        // Return the oversized pre-allocated list to the arena — the right-sized
        // `final` list is what we return, so the full `need`-byte block leaks
        // otherwise. size = need*ENTRY + HEADER + need (emit_alloc_byte_list).
        abi::load_u64("%v9", abi::stack_pointer(), NEED_OFF),
        abi::move_immediate("%v10", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v11", "%v9", "%v10"),
        abi::add_immediate("%v11", "%v11", COLLECTION_HEADER_SIZE),
        abi::add_registers("%v11", "%v11", "%v9"),
        abi::move_register(abi::ARG[1], "%v11"),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LIST_PTR_OFF),
    ]);
    emit_arena_free(symbol, &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), FINAL_LIST_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&ret_full),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), LIST_PTR_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
    ]);
    emit_fail(
        symbol,
        ERR_INVALID_ARGUMENT_CODE,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&dev_fail));
    emit_fail(
        symbol,
        ERR_AUDIO_DEVICE_CODE,
        ERR_AUDIO_DEVICE_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], F);
    Ok((frame, instructions, relocations, stack_slots))
}

/// closeInput(input): drop captured data, stop, dispose, destroy, munmap.
fn lower_close_input(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let already = format!("{symbol}_already");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::load_u64("%v9", abi::return_register(), H_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&already),
        abi::load_u64("%v10", abi::return_register(), H_STATE),
        abi::store_u64("%v10", abi::stack_pointer(), STATE_OFF),
    ]);
    // Set closed under the mutex first (a racing callback must not touch the ring).
    emit_pthread1(
        symbol,
        "pthread_mutex_lock",
        STATE_OFF,
        S_MUTEX,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u64("%v9", "%v10", S_CLOSED),
    ]);
    emit_pthread1(
        symbol,
        "pthread_mutex_unlock",
        STATE_OFF,
        S_MUTEX,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), "%v10", S_OSOBJECT),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    platform.emit_libc_call(
        "AudioQueueStop",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), "%v10", S_OSOBJECT),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    platform.emit_libc_call(
        "AudioQueueDispose",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_pthread1(
        symbol,
        "pthread_cond_destroy",
        STATE_OFF,
        S_COND,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_pthread1(
        symbol,
        "pthread_mutex_destroy",
        STATE_OFF,
        S_MUTEX,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u64("%v9", "%v10", H_CLOSED),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::ARG[1], abi::return_register(), S_MAP_SIZE),
    ]);
    platform.emit_libc_call(
        "munmap",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&already),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], F);
    Ok((frame, instructions, relocations, stack_slots))
}

/// The AudioQueue input callback (C-ABI, 6 args): copies the captured buffer into
/// the ring (discarding oldest whole frames on overrun, xruns++), signals, and
/// re-enqueues the buffer.
pub(in crate::target::shared::code) fn lower_audio_input_callback(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    const CB_FRAME: usize = 128;
    const CB_HANDLE: usize = 8;
    const CB_AQ: usize = 16;
    const CB_BUF: usize = 24;
    const CB_STATE: usize = 32;
    const CB_N: usize = 40;
    const CB_SRC: usize = 48;
    let symbol = AUDIO_INPUT_CALLBACK_SYMBOL;
    let closed_exit = format!("{symbol}_closed");
    let no_overrun = format!("{symbol}_no_overrun");
    let copy_loop = format!("{symbol}_copy");
    let copy_done = format!("{symbol}_copy_done");
    let head_ok = format!("{symbol}_head_ok");
    let tail_ok = format!("{symbol}_tail_ok");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CB_HANDLE),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), CB_AQ),
        abi::store_u64(abi::ARG[2], abi::stack_pointer(), CB_BUF),
        abi::load_u64("%v9", abi::return_register(), H_STATE),
        abi::store_u64("%v9", abi::stack_pointer(), CB_STATE),
    ]);
    emit_pthread1(
        symbol,
        "pthread_mutex_lock",
        CB_STATE,
        S_MUTEX,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), CB_STATE),
        abi::load_u64("%v9", "%v10", S_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed_exit),
        // n = buffer->mAudioDataByteSize; src = buffer->mAudioData
        abi::load_u64("%v11", abi::stack_pointer(), CB_BUF),
        abi::load_u32("%v9", "%v11", 16),
        abi::store_u64("%v9", abi::stack_pointer(), CB_N),
        abi::load_u64("%v12", "%v11", 8),
        abi::store_u64("%v12", abi::stack_pointer(), CB_SRC),
        // overrun: if fill + n > ringCap, drop oldest whole frames.
        abi::load_u64("%v13", "%v10", S_RING_FILL),
        abi::load_u64("%v14", "%v10", S_RING_CAP),
        abi::add_registers("%v15", "%v13", "%v9"), // fill + n
        abi::compare_registers("%v15", "%v14"),
        abi::branch_le(&no_overrun),
        // drop = (fill+n) - ringCap, rounded up to a whole frame.
        abi::subtract_registers("%v15", "%v15", "%v14"),
        abi::load_u64("%v16", abi::stack_pointer(), CB_HANDLE),
        abi::load_u64("%v16", "%v16", H_BYTES_PER_FRAME),
        abi::add_registers("%v15", "%v15", "%v16"),
        abi::subtract_immediate("%v15", "%v15", 1),
        abi::unsigned_divide_registers("%v15", "%v15", "%v16"),
        abi::multiply_registers("%v15", "%v15", "%v16"), // drop bytes
        // tail = (tail + drop) mod ringCap
        abi::load_u64("%v17", "%v10", S_RING_TAIL),
        abi::add_registers("%v17", "%v17", "%v15"),
        abi::compare_registers("%v17", "%v14"),
        abi::branch_lt(&tail_ok),
        abi::subtract_registers("%v17", "%v17", "%v14"),
        abi::label(&tail_ok),
        abi::store_u64("%v17", "%v10", S_RING_TAIL),
        // fill -= drop
        abi::load_u64("%v13", "%v10", S_RING_FILL),
        abi::subtract_registers("%v13", "%v13", "%v15"),
        abi::store_u64("%v13", "%v10", S_RING_FILL),
        // xruns++
        abi::load_u64("%v18", "%v10", S_XRUNS),
        abi::add_immediate("%v18", "%v18", 1),
        abi::store_u64("%v18", "%v10", S_XRUNS),
        abi::label(&no_overrun),
        // copy n bytes from src into ring at head, wrapping.
        abi::load_u64("%v9", abi::stack_pointer(), CB_N),
        abi::load_u64("%v12", abi::stack_pointer(), CB_SRC),
        abi::add_immediate("%v19", "%v10", S_RING), // ring base
        abi::load_u64("%v16", "%v10", S_RING_HEAD),
        abi::load_u64("%v14", "%v10", S_RING_CAP),
        abi::move_immediate("%v17", "Integer", "0"), // copied
        abi::label(&copy_loop),
        abi::compare_registers("%v17", "%v9"),
        abi::branch_ge(&copy_done),
        abi::add_registers("%v18", "%v12", "%v17"), // src+copied
        abi::load_u8("%v20", "%v18", 0),
        abi::add_registers("%v18", "%v19", "%v16"), // ring+head
        abi::store_u8("%v20", "%v18", 0),
        abi::add_immediate("%v16", "%v16", 1),
        abi::compare_registers("%v16", "%v14"),
        abi::branch_lt(&head_ok),
        abi::move_immediate("%v16", "Integer", "0"),
        abi::label(&head_ok),
        abi::add_immediate("%v17", "%v17", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u64("%v16", "%v10", S_RING_HEAD),
        // fill += n
        abi::load_u64("%v13", "%v10", S_RING_FILL),
        abi::load_u64("%v9", abi::stack_pointer(), CB_N),
        abi::add_registers("%v13", "%v13", "%v9"),
        abi::store_u64("%v13", "%v10", S_RING_FILL),
        // signal
        abi::add_immediate(abi::return_register(), "%v10", S_COND),
    ]);
    platform.emit_libc_call(
        "pthread_cond_signal",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_pthread1(
        symbol,
        "pthread_mutex_unlock",
        CB_STATE,
        S_MUTEX,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // Re-enqueue the buffer.
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CB_AQ),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), CB_BUF),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
    ]);
    platform.emit_libc_call(
        "AudioQueueEnqueueBuffer",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::return_());
    // Closed path: unlock and return without touching the ring or re-enqueuing.
    instructions.push(abi::label(&closed_exit));
    emit_pthread1(
        symbol,
        "pthread_mutex_unlock",
        CB_STATE,
        S_MUTEX,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], CB_FRAME);
    Ok(CodeFunction {
        name: "runtime.audio.inputCallback".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        stack_slots,
        instructions,
        relocations,
    })
}

fn lower_devices(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let dev_fail = format!("{symbol}_dev_fail");
    let unavailable = format!("{symbol}_unavailable");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let fill_loop = format!("{symbol}_fill_loop");
    let fill_done = format!("{symbol}_fill_done");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();

    // Seed CURID_OFF with the system object id — `call_get_property` loads its
    // object from that slot, and the default-device / device-list queries all
    // run against `kAudioObjectSystemObject`. Default ids start at 0 (absent).
    instructions.extend([
        abi::move_immediate("%v9", "Integer", SYS_OBJECT),
        abi::store_u64("%v9", abi::stack_pointer(), CURID_OFF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), DEFIN_OFF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), DEFOUT_OFF),
    ]);
    build_propaddr(SEL_DEFIN, SCOPE_GLOBAL, &mut instructions);
    call_get_property(
        symbol,
        CURID_OFF,
        "4",
        DEFIN_OFF,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    build_propaddr(SEL_DEFOUT, SCOPE_GLOBAL, &mut instructions);
    call_get_property(
        symbol,
        CURID_OFF,
        "4",
        DEFOUT_OFF,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;

    // Device list.
    build_propaddr(SEL_DEVICES, SCOPE_GLOBAL, &mut instructions);
    // object is still the system object (CURID_OFF = 1).
    call_get_property(
        symbol,
        CURID_OFF,
        IDSBUF_CAP,
        IDSBUF_OFF,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&dev_fail),
        // count = SIZE / 4
        abi::load_u32("%v9", abi::stack_pointer(), SIZE_OFF),
        abi::shift_right_immediate("%v9", "%v9", 2),
        abi::store_u64("%v9", abi::stack_pointer(), COUNT_OFF),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&unavailable),
        // Allocate List OF AudioDevice: count*ENTRY + HEADER + count*RECORD.
        abi::move_immediate("%v10", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v11", "%v9", "%v10"),
        abi::add_immediate("%v11", "%v11", COLLECTION_HEADER_SIZE),
        abi::move_immediate("%v12", "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers("%v13", "%v9", "%v12"),
        abi::add_registers(abi::return_register(), "%v11", "%v13"),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register("%v15", abi::RET[1]),
        abi::store_u64("%v15", abi::stack_pointer(), LIST_OFF),
        abi::move_immediate("%v9", "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_KIND),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_OBJECT.to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("%v9", "Byte", "1"),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64("%v10", abi::stack_pointer(), COUNT_OFF),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_COUNT),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("%v12", "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers("%v13", "%v10", "%v12"),
        abi::store_u64("%v13", "%v15", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("%v13", "%v15", COLLECTION_OFFSET_DATA_CAPACITY),
        // entry cursor base and record data region base.
        abi::add_immediate("%v11", "%v15", COLLECTION_HEADER_SIZE),
        abi::store_u64("%v11", abi::stack_pointer(), ENTRY_OFF),
        abi::move_immediate("%v12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v13", "%v10", "%v12"),
        abi::add_registers("%v14", "%v11", "%v13"),
        abi::store_u64("%v14", abi::stack_pointer(), DATA_OFF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), INDEX_OFF),
        abi::label(&fill_loop),
        abi::load_u64("%v9", abi::stack_pointer(), INDEX_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), COUNT_OFF),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_eq(&fill_done),
        // CURID = IDSBUF[index]
        abi::add_immediate("%v11", abi::stack_pointer(), IDSBUF_OFF),
        abi::move_immediate("%v12", "Integer", "4"),
        abi::multiply_registers("%v13", "%v9", "%v12"),
        abi::add_registers("%v11", "%v11", "%v13"),
        abi::load_u32("%v14", "%v11", 0),
        abi::store_u64("%v14", abi::stack_pointer(), CURID_OFF),
    ]);
    // name, id (UID), channel-capability flags.
    emit_cfstring_field(
        symbol,
        SEL_NAME,
        NAMEPTR_OFF,
        &dev_fail,
        &alloc_fail,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_cfstring_field(
        symbol,
        SEL_UID,
        IDPTR_OFF,
        &dev_fail,
        &alloc_fail,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_channel_flag(
        symbol,
        SCOPE_INPUT,
        CANIN_OFF,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_channel_flag(
        symbol,
        SCOPE_OUTPUT,
        CANOUT_OFF,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // Build the record at DATA_OFF + index*RECORD.
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), INDEX_OFF),
        abi::move_immediate("%v10", "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers("%v11", "%v9", "%v10"),
        abi::load_u64("%v12", abi::stack_pointer(), DATA_OFF),
        abi::add_registers("%v12", "%v12", "%v11"), // record ptr
        abi::load_u64("%v13", abi::stack_pointer(), IDPTR_OFF),
        abi::store_u64("%v13", "%v12", DEVICE_FIELD_ID),
        abi::load_u64("%v13", abi::stack_pointer(), NAMEPTR_OFF),
        abi::store_u64("%v13", "%v12", DEVICE_FIELD_NAME),
        abi::load_u64("%v13", abi::stack_pointer(), CANIN_OFF),
        abi::store_u64("%v13", "%v12", DEVICE_FIELD_CAN_INPUT),
        abi::load_u64("%v13", abi::stack_pointer(), CANOUT_OFF),
        abi::store_u64("%v13", "%v12", DEVICE_FIELD_CAN_OUTPUT),
    ]);
    // isDefaultInput = (CURID == DEFIN) ? 1 : 0
    emit_id_matches(
        CURID_OFF,
        DEFIN_OFF,
        DEVICE_FIELD_IS_DEFAULT_INPUT,
        symbol,
        "in",
        &mut instructions,
    );
    emit_id_matches(
        CURID_OFF,
        DEFOUT_OFF,
        DEVICE_FIELD_IS_DEFAULT_OUTPUT,
        symbol,
        "out",
        &mut instructions,
    );
    // Entry descriptor at ENTRY_OFF + index*ENTRY.
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), INDEX_OFF),
        abi::move_immediate("%v10", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v11", "%v9", "%v10"),
        abi::load_u64("%v12", abi::stack_pointer(), ENTRY_OFF),
        abi::add_registers("%v12", "%v12", "%v11"), // entry ptr
        abi::move_immediate("%v13", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8("%v13", "%v12", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(abi::ZERO, "%v12", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64(abi::ZERO, "%v12", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        // value_offset = index * RECORD
        abi::move_immediate("%v10", "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers("%v11", "%v9", "%v10"),
        abi::store_u64("%v11", "%v12", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::move_immediate("%v13", "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::store_u64("%v13", "%v12", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        // index++
        abi::add_immediate("%v9", "%v9", 1),
        abi::store_u64("%v9", abi::stack_pointer(), INDEX_OFF),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), LIST_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&dev_fail),
    ]);
    emit_fail(
        symbol,
        ERR_AUDIO_DEVICE_CODE,
        ERR_AUDIO_DEVICE_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&unavailable));
    emit_fail(
        symbol,
        ERR_AUDIO_UNAVAILABLE_CODE,
        ERR_AUDIO_UNAVAILABLE_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());

    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

/// Store `1` into `out_off` of the record `%v12` (record ptr) when the u64 at
/// `a_off` equals the u64 at `b_off`, else `0`. Uses the record ptr already in
/// `%v12`.
fn emit_id_matches(
    a_off: usize,
    b_off: usize,
    field: usize,
    symbol: &str,
    tag: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let store = format!("{symbol}_defstore_{tag}");
    instructions.extend([
        abi::load_u64("%v13", abi::stack_pointer(), a_off),
        abi::load_u64("%v14", abi::stack_pointer(), b_off),
        abi::move_immediate("%v16", "Integer", "0"),
        abi::compare_registers("%v13", "%v14"),
        abi::branch_ne(&store),
        abi::move_immediate("%v16", "Integer", "1"),
        abi::label(&store),
        abi::store_u64("%v16", "%v12", field),
    ]);
}
