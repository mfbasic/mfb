//! Linux ALSA backend for the `audio` package (plan-33-C).
//!
//! `libasound.so.2` is resolved lazily on the first `audio::` call via
//! `dlopen`/`dlsym` — never a `DT_NEEDED`, so a binary that mentions `audio`
//! still `exec`s on a host without alsa-lib (the riscv64/musl test box), and a
//! missing library becomes an assertable `ErrAudioUnavailable` at the call site
//! rather than a dynamic-linker fatal (§3.1).
//!
//! ALSA's blocking `snd_pcm_readi`/`snd_pcm_writei` run directly on the calling
//! thread — there is no OS callback thread, so the `AudioState` ring/mutex/cond
//! are unused on Linux (§3.2). The state page still exists (one page) for a
//! single `AudioHandle` layout across platforms; it holds only the `snd_pcm_t*`
//! (S_OSOBJECT) and the `xruns` counter (S_XRUNS).

use std::collections::HashMap;

use super::*;
use crate::target::shared::abi;

const ALSA_SONAME: &str = "libasound.so.2";
const RTLD_NOW: &str = "2"; // RTLD_NOW | RTLD_LOCAL (RTLD_LOCAL == 0)

// snd_pcm constants (alsa/pcm.h).
const STREAM_PLAYBACK: &str = "0";
const STREAM_CAPTURE: &str = "1";
const ACCESS_RW_INTERLEAVED: &str = "3";
const FORMAT_S16_LE: &str = "2";
const EINTR: &str = "4"; // -EINTR

// Every ALSA symbol the backend may `dlsym`. A wrong-ABI `libasound` (a missing
// symbol) raises `ErrAudioUnavailable` naming it (§4).
const ALSA_SYMBOLS: &[&str] = &[
    "snd_pcm_open",
    "snd_pcm_close",
    "snd_pcm_hw_params_malloc",
    "snd_pcm_hw_params_free",
    "snd_pcm_hw_params_any",
    "snd_pcm_hw_params_set_access",
    "snd_pcm_hw_params_set_format",
    "snd_pcm_hw_params_set_channels",
    "snd_pcm_hw_params_set_rate_near",
    "snd_pcm_hw_params_set_period_size_near",
    "snd_pcm_hw_params_set_buffer_size_near",
    "snd_pcm_hw_params_get_rate",
    "snd_pcm_hw_params_get_channels",
    "snd_pcm_hw_params",
    "snd_pcm_prepare",
    "snd_pcm_readi",
    "snd_pcm_writei",
    "snd_pcm_avail_update",
    "snd_pcm_wait",
    "snd_pcm_drain",
    "snd_pcm_drop",
    "snd_pcm_recover",
    "snd_device_name_hint",
    "snd_device_name_get_hint",
    "snd_device_name_free_hint",
];

fn lib_data_symbol() -> String {
    "_mfb_audio_alsa_soname".to_string()
}

fn sym_data_symbol(name: &str) -> String {
    format!("_mfb_audio_alsa_sym_{name}")
}

fn hex_encode_cstring(text: &str) -> String {
    let mut hex = String::new();
    for byte in text.bytes() {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex.push_str("00");
    hex
}

/// The read-only C strings (soname + ALSA symbol names) the backend references.
pub(super) fn data_objects() -> Vec<CodeDataObject> {
    let mut objects = vec![
        CodeDataObject {
            symbol: lib_data_symbol(),
            kind: "raw".to_string(),
            layout: "C string (NUL-terminated)".to_string(),
            align: 1,
            size: ALSA_SONAME.len() + 1,
            value: hex_encode_cstring(ALSA_SONAME),
        },
        // The default PCM device name + hint interface / id strings.
        CodeDataObject {
            symbol: "_mfb_audio_alsa_default".to_string(),
            kind: "raw".to_string(),
            layout: "C string (NUL-terminated)".to_string(),
            align: 1,
            size: "default".len() + 1,
            value: hex_encode_cstring("default"),
        },
        CodeDataObject {
            symbol: "_mfb_audio_alsa_pcm".to_string(),
            kind: "raw".to_string(),
            layout: "C string (NUL-terminated)".to_string(),
            align: 1,
            size: "pcm".len() + 1,
            value: hex_encode_cstring("pcm"),
        },
        CodeDataObject {
            symbol: "_mfb_audio_alsa_hint_name".to_string(),
            kind: "raw".to_string(),
            layout: "C string (NUL-terminated)".to_string(),
            align: 1,
            size: "NAME".len() + 1,
            value: hex_encode_cstring("NAME"),
        },
        CodeDataObject {
            symbol: "_mfb_audio_alsa_hint_desc".to_string(),
            kind: "raw".to_string(),
            layout: "C string (NUL-terminated)".to_string(),
            align: 1,
            size: "DESC".len() + 1,
            value: hex_encode_cstring("DESC"),
        },
        CodeDataObject {
            symbol: "_mfb_audio_alsa_hint_ioid".to_string(),
            kind: "raw".to_string(),
            layout: "C string (NUL-terminated)".to_string(),
            align: 1,
            size: "IOID".len() + 1,
            value: hex_encode_cstring("IOID"),
        },
    ];
    for name in ALSA_SYMBOLS {
        objects.push(CodeDataObject {
            symbol: sym_data_symbol(name),
            kind: "raw".to_string(),
            layout: "C string (NUL-terminated)".to_string(),
            align: 1,
            size: name.len() + 1,
            value: hex_encode_cstring(name),
        });
    }
    objects
}

// --- shared stack frame ------------------------------------------------------
// All ALSA fn-ptrs and scratch stay on the stack; ALSA calls clobber the
// caller-saved registers. Offsets are kept small (< ~1 KiB) for the AArch64
// 12-bit addressing range.
const FRAME: usize = 640;
const HANDLE_OFF: usize = 8;
const STATE_OFF: usize = 16;
const DL_HANDLE_OFF: usize = 24; // dlopen handle
const FNPTR_OFF: usize = 32; // scratch fn-ptr
const PARAMS_OFF: usize = 48; // snd_pcm_hw_params_t*
const SR_OFF: usize = 56;
const CH_OFF: usize = 64;
const BF_OFF: usize = 72;
const BPF_OFF: usize = 80;
const RATE_OFF: usize = 88; // unsigned rate (in/out)
const CHANS_OFF: usize = 96; // unsigned channels (out)
const DIR_OFF: usize = 104; // int dir
const PERIOD_OFF: usize = 112; // snd_pcm_uframes_t period
const BUFSZ_OFF: usize = 120; // snd_pcm_uframes_t buffer
const FRAMES_OFF: usize = 128;
const NEED_OFF: usize = 136;
const GOT_OFF: usize = 144;
const LIST_OFF: usize = 152;
const SRC_OFF: usize = 160; // byte payload src / dst
const TOTAL_OFF: usize = 168;
const OFFSET_OFF: usize = 176;
const N_OFF: usize = 184; // frames this iteration
const DEVID_OFF: usize = 192;
const RC_OFF: usize = 200; // ALSA return code
const NAME_OFF: usize = 208; // C-string device name for open
const FN2_OFF: usize = 216; // secondary fn-ptr (writei/readi kept across a loop)
const HINTS_OFF: usize = 224; // void** device hints
const HINT_PTR_OFF: usize = 232; // current hint cursor
const COUNT_OFF: usize = 240;
const NAME_BUF_OFF: usize = 256; // 128-byte device name buffer -> 256..384
// Timed-read (readTimeout) scratch; unused by the blocking read/write/open paths.
const FINAL_LIST_OFF: usize = 384; // right-sized result for a partial timed read
const GOTBYTES_OFF: usize = 392; // bytes gathered so far (frames * bpf)
const WANT_OFF: usize = 400; // frames to request from readi this iteration
const TIMEOUT_OFF: usize = 408; // timeoutMs (spilled at entry)
const DEADLINE_OFF: usize = 416; // absolute deadline (ns, CLOCK_MONOTONIC)
const CLK_OFF: usize = 424; // clock_gettime timespec -> 424..440
const WAIT_FN_OFF: usize = 440; // cached snd_pcm_wait fn-ptr
const AVAIL_FN_OFF: usize = 448; // cached snd_pcm_avail_update fn-ptr

/// Resolve `libasound.so.2` (dlopen), storing the handle at `DL_HANDLE_OFF`;
/// branch to `unavailable` if it does not load.
fn emit_dlopen(
    symbol: &str,
    unavailable: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    emit_data_address(symbol, abi::return_register(), &lib_data_symbol(), instructions, relocations);
    instructions.push(abi::move_immediate(abi::ARG[1], "Integer", RTLD_NOW));
    platform.emit_libc_call("dlopen", symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DL_HANDLE_OFF),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(unavailable),
    ]);
    Ok(())
}

/// `dlsym(handle, name)` into `FNPTR_OFF`; branch to `unavailable` if null.
fn emit_dlsym(
    symbol: &str,
    name: &str,
    unavailable: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    instructions.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), DL_HANDLE_OFF));
    emit_data_address(symbol, abi::ARG[1], &sym_data_symbol(name), instructions, relocations);
    platform.emit_libc_call("dlsym", symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(unavailable),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FNPTR_OFF),
    ]);
    Ok(())
}

pub(super) fn lower_audio_alsa(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    match call {
        "audio.devices" => lower_devices(symbol, platform_imports, platform),
        "audio.openOutput" => lower_open(symbol, false, false, platform_imports, platform),
        "audio.openOutputDevice" => lower_open(symbol, false, true, platform_imports, platform),
        "audio.openInput" => lower_open(symbol, true, false, platform_imports, platform),
        "audio.openInputDevice" => lower_open(symbol, true, true, platform_imports, platform),
        "audio.write" => lower_write(symbol, platform_imports, platform),
        "audio.read" => lower_read(symbol, false, platform_imports, platform),
        "audio.readTimeout" => lower_read(symbol, true, platform_imports, platform),
        "audio.poll" => lower_query(symbol, Query::Poll, platform_imports, platform),
        "audio.pollTimeout" => lower_query(symbol, Query::PollTimeout, platform_imports, platform),
        "audio.available" => lower_query(symbol, Query::Available, platform_imports, platform),
        "audio.xruns" => lower_query(symbol, Query::Xruns, platform_imports, platform),
        "audio.closeInput" => lower_close(symbol, true, platform_imports, platform),
        "audio.closeOutput" => lower_close(symbol, false, platform_imports, platform),
        other => Err(format!(
            "native code plan does not emit runtime call '{other}' for linux (alsa)"
        )),
    }
}

/// Call the fn-ptr currently in `FNPTR_OFF` (args already staged), leaving its
/// return in the return register (sign-extended to 64 bits).
fn emit_call_fnptr(instructions: &mut Vec<CodeInstruction>) {
    instructions.extend([
        abi::load_u64("%v8", abi::stack_pointer(), FNPTR_OFF),
        abi::branch_link_register("%v8"),
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
    ]);
}

#[derive(Clone, Copy)]
enum Query {
    Available,
    Poll,
    PollTimeout,
    Xruns,
}

// Parameter validation ranges (plan-33-A §3.5).
const SR_MIN: &str = "8000";
const SR_MAX: &str = "192000";
const BUF_MIN: &str = "64";
const BUF_MAX: &str = "8192";
const READ_FRAMES_MAX: &str = "1048576";
const TIMEOUT_MAX: &str = "86400000"; // 24h, matches the macOS timed-read bound

/// dlsym `name` into `FNPTR_OFF`, stage the args via `stage`, call it, and leave
/// the (sign-extended) result in the return register.
#[allow(clippy::too_many_arguments)]
fn emit_alsa_call(
    symbol: &str,
    name: &str,
    unavailable: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    stage: impl Fn(&mut Vec<CodeInstruction>),
) -> Result<(), String> {
    emit_dlsym(symbol, name, unavailable, platform, platform_imports, instructions, relocations)?;
    stage(instructions);
    emit_call_fnptr(instructions);
    Ok(())
}

fn emit_validate_open(
    symbol: &str,
    invalid: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let ch_ok = format!("{symbol}_ch_ok");
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), SR_OFF),
        abi::move_immediate("%v10", "Integer", SR_MIN),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_lt(invalid),
        abi::move_immediate("%v10", "Integer", SR_MAX),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_gt(invalid),
        abi::load_u64("%v9", abi::stack_pointer(), CH_OFF),
        abi::compare_immediate("%v9", "1"),
        abi::branch_eq(&ch_ok),
        abi::compare_immediate("%v9", "2"),
        abi::branch_ne(invalid),
        abi::label(&ch_ok),
        abi::load_u64("%v9", abi::stack_pointer(), BF_OFF),
        abi::move_immediate("%v10", "Integer", BUF_MIN),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_lt(invalid),
        abi::move_immediate("%v10", "Integer", BUF_MAX),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_gt(invalid),
    ]);
}

/// Copy an MFBASIC `String` (pointer at `str_off`'s record field) into the
/// NUL-terminated name buffer at `NAME_BUF_OFF`, storing its address at
/// `NAME_OFF`.
fn emit_device_cstring(device_off: usize, instructions: &mut Vec<CodeInstruction>, symbol: &str) {
    let copy = format!("{symbol}_dev_copy");
    let done = format!("{symbol}_dev_copy_done");
    let clamp_ok = format!("{symbol}_dev_clamp_ok");
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), device_off),
        abi::load_u64("%v9", "%v9", DEVICE_FIELD_ID), // id String ptr
        abi::load_u64("%v10", "%v9", 0),              // len
        abi::add_immediate("%v11", "%v9", 8),         // src bytes
        // Clamp the copy count to NAME_BUF's 128 bytes minus the NUL terminator;
        // an oversized device id would otherwise overrun the fixed buffer.
        abi::move_immediate("%v9", "Integer", "127"),
        abi::compare_registers("%v10", "%v9"),
        abi::branch_le(&clamp_ok),
        abi::move_register("%v10", "%v9"),
        abi::label(&clamp_ok),
        abi::add_immediate("%v12", abi::stack_pointer(), NAME_BUF_OFF),
        abi::store_u64("%v12", abi::stack_pointer(), NAME_OFF),
        abi::move_immediate("%v13", "Integer", "0"),
        abi::label(&copy),
        abi::compare_registers("%v13", "%v10"),
        abi::branch_ge(&done),
        abi::load_u8("%v14", "%v11", 0),
        abi::store_u8("%v14", "%v12", 0),
        abi::add_immediate("%v11", "%v11", 1),
        abi::add_immediate("%v12", "%v12", 1),
        abi::add_immediate("%v13", "%v13", 1),
        abi::branch(&copy),
        abi::label(&done),
        abi::store_u8(abi::ZERO, "%v12", 0),
    ]);
}

fn lower_open(
    symbol: &str,
    input: bool,
    device: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    let invalid = format!("{symbol}_invalid");
    let unavailable = format!("{symbol}_unavailable");
    let dev_fail = format!("{symbol}_dev_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
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
    // mapped (nothing to close/munmap before mmap and snd_pcm_open run).
    instructions.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), STATE_OFF));
    emit_validate_open(symbol, &invalid, &mut instructions);
    // bytesPerFrame, AudioHandle, mmap state.
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), CH_OFF),
        abi::move_immediate("%v10", "Integer", "2"),
        abi::multiply_registers("%v9", "%v9", "%v10"),
        abi::store_u64("%v9", abi::stack_pointer(), BPF_OFF),
        abi::move_immediate(abi::return_register(), "Integer", &H_RECORD_SIZE.to_string()),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
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
        // mmap one state page (ring unused on Linux; §3.2).
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::move_immediate(abi::ARG[1], "Integer", &STATE_PAGE.to_string()),
        abi::move_immediate(abi::ARG[2], "Integer", "3"), // PROT_READ|WRITE
        abi::move_immediate(abi::ARG[3], "Integer", "34"), // MAP_PRIVATE|MAP_ANONYMOUS (Linux)
        abi::bitwise_not(abi::ARG[4], abi::ZERO),
        abi::move_immediate(abi::ARG[5], "Integer", "0"),
    ]);
    platform.emit_libc_call("mmap", symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::add_immediate("%v9", abi::return_register(), 1),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&dev_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v15", abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::return_register(), "%v15", H_STATE),
        abi::load_u64("%v15", abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, "%v15", S_XRUNS),
        abi::store_u64(abi::ZERO, "%v15", S_CLOSED),
        abi::store_u64(abi::ZERO, "%v15", S_OSOBJECT),
        abi::move_immediate("%v9", "Integer", &STATE_PAGE.to_string()),
        abi::store_u64("%v9", "%v15", S_MAP_SIZE),
    ]);
    emit_dlopen(symbol, &unavailable, platform, platform_imports, &mut instructions, &mut relocations)?;
    // Device name: the id string, or "default".
    if device {
        emit_device_cstring(DEVID_OFF, &mut instructions, symbol);
    } else {
        emit_data_address(symbol, "%v9", "_mfb_audio_alsa_default", &mut instructions, &mut relocations);
        instructions.push(abi::store_u64("%v9", abi::stack_pointer(), NAME_OFF));
    }
    // snd_pcm_open(&state->osobject, name, stream, 0)
    emit_alsa_call(symbol, "snd_pcm_open", &unavailable, platform, platform_imports, &mut instructions, &mut relocations, |ins| {
        ins.extend([
            abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
            abi::add_immediate(abi::return_register(), "%v9", S_OSOBJECT),
            abi::load_u64(abi::ARG[1], abi::stack_pointer(), NAME_OFF),
            abi::move_immediate(abi::ARG[2], "Integer", if input { STREAM_CAPTURE } else { STREAM_PLAYBACK }),
            abi::move_immediate(abi::ARG[3], "Integer", "0"),
        ]);
    })?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
    ]);
    emit_configure_hw_params(symbol, &unavailable, &dev_fail, input, platform, platform_imports, &mut instructions, &mut relocations)?;
    // snd_pcm_prepare(pcm)
    emit_alsa_call(symbol, "snd_pcm_prepare", &unavailable, platform, platform_imports, &mut instructions, &mut relocations, |ins| {
        ins.extend([
            abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
            abi::load_u64(abi::return_register(), "%v9", S_OSOBJECT),
        ]);
    })?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
        abi::load_u64("%v15", abi::stack_pointer(), STATE_OFF),
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u64("%v9", "%v15", S_STARTED),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
    ]);
    emit_fail(symbol, ERR_INVALID_ARGUMENT_CODE, ERR_INVALID_ARGUMENT_SYMBOL, &mut instructions, &mut relocations, &done);
    instructions.push(abi::label(&unavailable));
    emit_fail(symbol, ERR_AUDIO_UNAVAILABLE_CODE, ERR_AUDIO_UNAVAILABLE_SYMBOL, &mut instructions, &mut relocations, &done);
    instructions.push(abi::label(&dev_fail));
    // Open-error cleanup (bug-180): close the PCM (if opened) and munmap the state
    // page (if mapped) before failing. `STATE_OFF` is zeroed at entry and mmap
    // zero-fills `S_OSOBJECT`, so each disposal is guarded when reached early.
    {
        let cleanup_munmap = format!("{symbol}_dev_munmap");
        let cleanup_done = format!("{symbol}_dev_cleanup_done");
        instructions.extend([
            abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
            abi::compare_immediate("%v10", "0"),
            abi::branch_eq(&cleanup_done),
            abi::load_u64("%v9", "%v10", S_OSOBJECT),
            abi::compare_immediate("%v9", "0"),
            abi::branch_eq(&cleanup_munmap),
        ]);
        emit_alsa_call(symbol, "snd_pcm_close", &unavailable, platform, platform_imports, &mut instructions, &mut relocations, |ins| {
            ins.extend([
                abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
                abi::load_u64(abi::return_register(), "%v10", S_OSOBJECT),
            ]);
        })?;
        instructions.extend([
            abi::label(&cleanup_munmap),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE_OFF),
            abi::load_u64(abi::ARG[1], abi::return_register(), S_MAP_SIZE),
        ]);
        platform.emit_libc_call("munmap", symbol, platform_imports, &mut instructions, &mut relocations)?;
        instructions.push(abi::label(&cleanup_done));
    }
    emit_fail(symbol, ERR_AUDIO_DEVICE_CODE, ERR_AUDIO_DEVICE_SYMBOL, &mut instructions, &mut relocations, &done);
    instructions.push(abi::label(&alloc_fail));
    emit_fail(symbol, ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_SYMBOL, &mut instructions, &mut relocations, &done);
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME);
    Ok((frame, instructions, relocations, stack_slots))
}

/// Configure and commit the hw params (§3.3): interleaved S16_LE at the
/// requested channels/rate, buffer = bufferFrames*4. Verify the committed rate
/// and channels match the request, else `ErrAudioDevice` (no silent resampling).
#[allow(clippy::too_many_arguments)]
fn emit_configure_hw_params(
    symbol: &str,
    unavailable: &str,
    dev_fail: &str,
    _input: bool,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let pcm = |ins: &mut Vec<CodeInstruction>| {
        ins.extend([
            abi::load_u64("%v9", abi::stack_pointer(), STATE_OFF),
            abi::load_u64(abi::return_register(), "%v9", S_OSOBJECT),
        ]);
    };
    let params = |ins: &mut Vec<CodeInstruction>| {
        ins.push(abi::load_u64(abi::ARG[1], abi::stack_pointer(), PARAMS_OFF));
    };
    let check = |ins: &mut Vec<CodeInstruction>, fail: &str| {
        ins.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(fail),
        ]);
    };
    // snd_pcm_hw_params_malloc(&params)
    emit_alsa_call(symbol, "snd_pcm_hw_params_malloc", unavailable, platform, platform_imports, instructions, relocations, |ins| {
        ins.push(abi::add_immediate(abi::return_register(), abi::stack_pointer(), PARAMS_OFF));
    })?;
    check(instructions, dev_fail);
    // any
    emit_alsa_call(symbol, "snd_pcm_hw_params_any", unavailable, platform, platform_imports, instructions, relocations, |ins| {
        pcm(ins);
        params(ins);
    })?;
    check(instructions, dev_fail);
    // set_access(INTERLEAVED)
    emit_alsa_call(symbol, "snd_pcm_hw_params_set_access", unavailable, platform, platform_imports, instructions, relocations, |ins| {
        pcm(ins);
        params(ins);
        ins.push(abi::move_immediate(abi::ARG[2], "Integer", ACCESS_RW_INTERLEAVED));
    })?;
    check(instructions, dev_fail);
    // set_format(S16_LE)
    emit_alsa_call(symbol, "snd_pcm_hw_params_set_format", unavailable, platform, platform_imports, instructions, relocations, |ins| {
        pcm(ins);
        params(ins);
        ins.push(abi::move_immediate(abi::ARG[2], "Integer", FORMAT_S16_LE));
    })?;
    check(instructions, dev_fail);
    // set_channels(channels)
    emit_alsa_call(symbol, "snd_pcm_hw_params_set_channels", unavailable, platform, platform_imports, instructions, relocations, |ins| {
        pcm(ins);
        params(ins);
        ins.push(abi::load_u64(abi::ARG[2], abi::stack_pointer(), CH_OFF));
    })?;
    check(instructions, dev_fail);
    // set_rate_near(&rate, &dir)
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), SR_OFF),
        abi::store_u32("%v9", abi::stack_pointer(), RATE_OFF),
        abi::store_u32(abi::ZERO, abi::stack_pointer(), DIR_OFF),
    ]);
    emit_alsa_call(symbol, "snd_pcm_hw_params_set_rate_near", unavailable, platform, platform_imports, instructions, relocations, |ins| {
        pcm(ins);
        params(ins);
        ins.push(abi::add_immediate(abi::ARG[2], abi::stack_pointer(), RATE_OFF));
        ins.push(abi::add_immediate(abi::ARG[3], abi::stack_pointer(), DIR_OFF));
    })?;
    check(instructions, dev_fail);
    // set_period_size_near(&period, &dir) — period = bufferFrames
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), BF_OFF),
        abi::store_u64("%v9", abi::stack_pointer(), PERIOD_OFF),
        abi::store_u32(abi::ZERO, abi::stack_pointer(), DIR_OFF),
    ]);
    emit_alsa_call(symbol, "snd_pcm_hw_params_set_period_size_near", unavailable, platform, platform_imports, instructions, relocations, |ins| {
        pcm(ins);
        params(ins);
        ins.push(abi::add_immediate(abi::ARG[2], abi::stack_pointer(), PERIOD_OFF));
        ins.push(abi::add_immediate(abi::ARG[3], abi::stack_pointer(), DIR_OFF));
    })?;
    check(instructions, dev_fail);
    // set_buffer_size_near(&buffer) — buffer = bufferFrames * 4 (mirror macOS depth)
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), BF_OFF),
        abi::move_immediate("%v10", "Integer", "4"),
        abi::multiply_registers("%v9", "%v9", "%v10"),
        abi::store_u64("%v9", abi::stack_pointer(), BUFSZ_OFF),
    ]);
    emit_alsa_call(symbol, "snd_pcm_hw_params_set_buffer_size_near", unavailable, platform, platform_imports, instructions, relocations, |ins| {
        pcm(ins);
        params(ins);
        ins.push(abi::add_immediate(abi::ARG[2], abi::stack_pointer(), BUFSZ_OFF));
    })?;
    check(instructions, dev_fail);
    // commit
    emit_alsa_call(symbol, "snd_pcm_hw_params", unavailable, platform, platform_imports, instructions, relocations, |ins| {
        pcm(ins);
        params(ins);
    })?;
    check(instructions, dev_fail);
    // get_rate(&rate) and get_channels(&chans); verify == request (§3.3).
    emit_alsa_call(symbol, "snd_pcm_hw_params_get_rate", unavailable, platform, platform_imports, instructions, relocations, |ins| {
        params(ins);
        ins.push(abi::add_immediate(abi::ARG[1], abi::stack_pointer(), RATE_OFF));
        ins.push(abi::add_immediate(abi::ARG[2], abi::stack_pointer(), DIR_OFF));
    })?;
    emit_alsa_call(symbol, "snd_pcm_hw_params_get_channels", unavailable, platform, platform_imports, instructions, relocations, |ins| {
        params(ins);
        ins.push(abi::add_immediate(abi::ARG[1], abi::stack_pointer(), CHANS_OFF));
    })?;
    instructions.extend([
        abi::load_u32("%v9", abi::stack_pointer(), RATE_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), SR_OFF),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_ne(dev_fail),
        abi::load_u32("%v9", abi::stack_pointer(), CHANS_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), CH_OFF),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_ne(dev_fail),
    ]);
    // free the hw_params object.
    emit_alsa_call(symbol, "snd_pcm_hw_params_free", unavailable, platform, platform_imports, instructions, relocations, |ins| {
        ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), PARAMS_OFF));
    })?;
    Ok(())
}

/// write(output, bytes): loop snd_pcm_writei until every frame is accepted,
/// recovering from xruns (§3.5).
fn lower_write(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    let invalid = format!("{symbol}_invalid");
    let unavailable = format!("{symbol}_unavailable");
    let dev_fail = format!("{symbol}_dev_fail");
    let loop_top = format!("{symbol}_loop");
    let loop_done = format!("{symbol}_loop_done");
    let ok_frames = format!("{symbol}_ok");
    let recover = format!("{symbol}_recover");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), SRC_OFF), // byteList
        abi::load_u64("%v9", abi::return_register(), H_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&dev_fail),
        abi::load_u64("%v10", abi::return_register(), H_STATE),
        abi::store_u64("%v10", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v10", abi::return_register(), H_BYTES_PER_FRAME),
        abi::store_u64("%v10", abi::stack_pointer(), BPF_OFF),
        // total bytes, frame-alignment check
        abi::load_u64("%v13", abi::ARG[1], COLLECTION_OFFSET_COUNT),
        abi::compare_immediate("%v13", "0"),
        abi::branch_eq(&invalid),
        abi::load_u64("%v10", abi::stack_pointer(), BPF_OFF),
        abi::subtract_immediate("%v11", "%v10", 1),
        abi::and_registers("%v12", "%v13", "%v11"),
        abi::compare_immediate("%v12", "0"),
        abi::branch_ne(&invalid),
        // src = byteList + HEADER + CAPACITY*ENTRY (the data region starts past
        // the CAPACITY-sized entry array; an append-built list has spare
        // capacity, so COUNT*ENTRY would mis-address it). totalFrames = total/bpf.
        abi::load_u64("%v12", abi::ARG[1], COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("%v14", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v14", "%v12", "%v14"),
        abi::add_immediate("%v14", "%v14", COLLECTION_HEADER_SIZE),
        abi::add_registers("%v14", abi::ARG[1], "%v14"),
        abi::store_u64("%v14", abi::stack_pointer(), SRC_OFF),
        abi::unsigned_divide_registers("%v13", "%v13", "%v10"),
        abi::store_u64("%v13", abi::stack_pointer(), TOTAL_OFF), // total frames
        abi::store_u64(abi::ZERO, abi::stack_pointer(), OFFSET_OFF),
    ]);
    emit_dlopen(symbol, &unavailable, platform, platform_imports, &mut instructions, &mut relocations)?;
    // cache writei and recover fn-ptrs
    emit_dlsym(symbol, "snd_pcm_writei", &unavailable, platform, platform_imports, &mut instructions, &mut relocations)?;
    instructions.push(abi::load_u64("%v9", abi::stack_pointer(), FNPTR_OFF));
    instructions.push(abi::store_u64("%v9", abi::stack_pointer(), FN2_OFF));
    emit_dlsym(symbol, "snd_pcm_recover", &unavailable, platform, platform_imports, &mut instructions, &mut relocations)?;
    // (recover fn-ptr stays in FNPTR_OFF)
    instructions.extend([
        abi::label(&loop_top),
        abi::load_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), TOTAL_OFF),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_ge(&loop_done),
        // snd_pcm_writei(pcm, src + offset*bpf, total-offset)
        abi::load_u64("%v11", abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), "%v11", S_OSOBJECT),
        abi::load_u64("%v12", abi::stack_pointer(), SRC_OFF),
        abi::load_u64("%v13", abi::stack_pointer(), BPF_OFF),
        abi::multiply_registers("%v14", "%v9", "%v13"),
        abi::add_registers(abi::ARG[1], "%v12", "%v14"),
        abi::subtract_registers(abi::ARG[2], "%v10", "%v9"),
        abi::load_u64("%v8", abi::stack_pointer(), FN2_OFF),
        abi::branch_link_register("%v8"),
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), N_OFF),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&ok_frames),
        // negative: -EINTR retries, else recover.
        abi::move_immediate("%v10", "Integer", EINTR),
        abi::subtract_registers("%v10", abi::ZERO, "%v10"),
        abi::compare_registers(abi::return_register(), "%v10"),
        abi::branch_eq(&loop_top),
        abi::branch(&recover),
        abi::label(&ok_frames),
        abi::load_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), N_OFF),
        abi::add_registers("%v9", "%v9", "%v10"),
        abi::store_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::branch(&loop_top),
        abi::label(&recover),
        // xruns++ ; snd_pcm_recover(pcm, err, 1)
        abi::load_u64("%v11", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v12", "%v11", S_XRUNS),
        abi::add_immediate("%v12", "%v12", 1),
        abi::store_u64("%v12", "%v11", S_XRUNS),
        abi::load_u64(abi::return_register(), "%v11", S_OSOBJECT),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), N_OFF), // err
        abi::move_immediate(abi::ARG[2], "Integer", "1"),
        abi::load_u64("%v8", abi::stack_pointer(), FNPTR_OFF),
        abi::branch_link_register("%v8"),
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
        abi::branch(&loop_top),
        abi::label(&loop_done),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
    ]);
    emit_fail(symbol, ERR_INVALID_ARGUMENT_CODE, ERR_INVALID_ARGUMENT_SYMBOL, &mut instructions, &mut relocations, &done);
    instructions.push(abi::label(&unavailable));
    emit_fail(symbol, ERR_AUDIO_UNAVAILABLE_CODE, ERR_AUDIO_UNAVAILABLE_SYMBOL, &mut instructions, &mut relocations, &done);
    instructions.push(abi::label(&dev_fail));
    emit_fail(symbol, ERR_AUDIO_DEVICE_CODE, ERR_AUDIO_DEVICE_SYMBOL, &mut instructions, &mut relocations, &done);
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME);
    Ok((frame, instructions, relocations, stack_slots))
}

/// Allocate a `List OF Byte` of `count` bytes (count at `count_off`); header +
/// entries filled, payload left uninitialized. Stores the list ptr at `list_off`.
fn emit_alloc_byte_list(
    symbol: &str,
    tag: &str,
    count_off: usize,
    list_off: usize,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let entry = format!("{symbol}_{tag}_bl");
    let entry_done = format!("{symbol}_{tag}_bl_done");
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), count_off),
        abi::move_immediate("%v11", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v12", "%v10", "%v11"),
        abi::add_immediate("%v12", "%v12", COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), "%v12", "%v10"),
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
        abi::add_immediate("%v11", "%v15", COLLECTION_HEADER_SIZE),
        abi::move_immediate("%v13", "Integer", "0"),
        abi::label(&entry),
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
        abi::branch(&entry),
        abi::label(&entry_done),
    ]);
}

/// read(input, frames[, timeoutMs]): loop snd_pcm_readi into the pre-allocated
/// result. The blocking form fills exactly `frames`; the timed form stops at the
/// deadline and returns the whole frames gathered (§3.4).
fn lower_read(
    symbol: &str,
    timeout: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    let invalid = format!("{symbol}_invalid");
    let unavailable = format!("{symbol}_unavailable");
    let dev_fail = format!("{symbol}_dev_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let loop_top = format!("{symbol}_loop");
    let loop_done = format!("{symbol}_loop_done");
    let ok_frames = format!("{symbol}_ok");
    let recover = format!("{symbol}_recover");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), FRAMES_OFF),
    ]);
    if timeout {
        // Spill `timeoutMs` (ARG[2]) before any dlopen/libc call clobbers it.
        instructions.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), TIMEOUT_OFF));
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
    emit_alloc_byte_list(symbol, "main", NEED_OFF, LIST_OFF, &alloc_fail, &mut instructions, &mut relocations);
    // payload base = list + HEADER + need*ENTRY
    instructions.extend([
        abi::load_u64("%v11", abi::stack_pointer(), LIST_OFF),
        abi::load_u64("%v9", abi::stack_pointer(), NEED_OFF),
        abi::move_immediate("%v13", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v13", "%v9", "%v13"),
        abi::add_immediate("%v13", "%v13", COLLECTION_HEADER_SIZE),
        abi::add_registers("%v11", "%v11", "%v13"),
        abi::store_u64("%v11", abi::stack_pointer(), SRC_OFF), // payload base
        abi::store_u64(abi::ZERO, abi::stack_pointer(), GOT_OFF), // frames read
    ]);
    emit_dlopen(symbol, &unavailable, platform, platform_imports, &mut instructions, &mut relocations)?;
    if timeout {
        // Cache the poll fn-ptrs (dlsym clobbers FNPTR_OFF, which later holds the
        // recover fn-ptr, so resolve these first) and pin the absolute deadline.
        emit_dlsym(symbol, "snd_pcm_wait", &unavailable, platform, platform_imports, &mut instructions, &mut relocations)?;
        instructions.push(abi::load_u64("%v9", abi::stack_pointer(), FNPTR_OFF));
        instructions.push(abi::store_u64("%v9", abi::stack_pointer(), WAIT_FN_OFF));
        emit_dlsym(symbol, "snd_pcm_avail_update", &unavailable, platform, platform_imports, &mut instructions, &mut relocations)?;
        instructions.push(abi::load_u64("%v9", abi::stack_pointer(), FNPTR_OFF));
        instructions.push(abi::store_u64("%v9", abi::stack_pointer(), AVAIL_FN_OFF));
        // deadline = now + timeoutMs*1e6 (Linux CLOCK_MONOTONIC = 1).
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", "1"),
            abi::add_immediate(abi::ARG[1], abi::stack_pointer(), CLK_OFF),
        ]);
        platform.emit_libc_call("clock_gettime", symbol, platform_imports, &mut instructions, &mut relocations)?;
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
    emit_dlsym(symbol, "snd_pcm_readi", &unavailable, platform, platform_imports, &mut instructions, &mut relocations)?;
    instructions.push(abi::load_u64("%v9", abi::stack_pointer(), FNPTR_OFF));
    instructions.push(abi::store_u64("%v9", abi::stack_pointer(), FN2_OFF));
    emit_dlsym(symbol, "snd_pcm_recover", &unavailable, platform, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::label(&loop_top),
        abi::load_u64("%v9", abi::stack_pointer(), GOT_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), FRAMES_OFF),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_ge(&loop_done),
    ]);
    if timeout {
        // Bound the blocking read by the deadline: on expiry return the partial
        // frames gathered so far; otherwise wait (bounded) for a period and then
        // read only what is available, so `snd_pcm_readi` returns promptly.
        let want_cap = format!("{symbol}_want_cap");
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", "1"),
            abi::add_immediate(abi::ARG[1], abi::stack_pointer(), CLK_OFF),
        ]);
        platform.emit_libc_call("clock_gettime", symbol, platform_imports, &mut instructions, &mut relocations)?;
        instructions.extend([
            abi::load_u64("%v9", abi::stack_pointer(), CLK_OFF),
            abi::move_immediate("%v10", "Integer", "1000000000"),
            abi::multiply_registers("%v9", "%v9", "%v10"),
            abi::load_u64("%v11", abi::stack_pointer(), CLK_OFF + 8),
            abi::add_registers("%v9", "%v9", "%v11"), // now
            abi::load_u64("%v12", abi::stack_pointer(), DEADLINE_OFF),
            abi::compare_registers("%v9", "%v12"),
            abi::branch_ge(&loop_done), // expired -> partial
            // remaining_ms = (deadline - now) / 1e6; sub-ms remaining -> partial.
            abi::subtract_registers("%v12", "%v12", "%v9"),
            abi::move_immediate("%v13", "Integer", "1000000"),
            abi::unsigned_divide_registers("%v13", "%v12", "%v13"),
            abi::compare_immediate("%v13", "0"),
            abi::branch_eq(&loop_done),
            // snd_pcm_wait(pcm, remaining_ms): 1 ready, 0 timeout, <0 error.
            abi::load_u64("%v11", abi::stack_pointer(), STATE_OFF),
            abi::load_u64(abi::return_register(), "%v11", S_OSOBJECT),
            abi::move_register(abi::ARG[1], "%v13"),
            abi::load_u64("%v8", abi::stack_pointer(), WAIT_FN_OFF),
            abi::branch_link_register("%v8"),
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
            abi::store_u64(abi::return_register(), abi::stack_pointer(), N_OFF),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&loop_done), // timeout -> partial
            abi::branch_lt(&recover),   // error (e.g. xrun) -> recover (N_OFF = err)
            // avail = snd_pcm_avail_update(pcm)
            abi::load_u64("%v11", abi::stack_pointer(), STATE_OFF),
            abi::load_u64(abi::return_register(), "%v11", S_OSOBJECT),
            abi::load_u64("%v8", abi::stack_pointer(), AVAIL_FN_OFF),
            abi::branch_link_register("%v8"),
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
            abi::store_u64(abi::return_register(), abi::stack_pointer(), N_OFF),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&recover), // avail error -> recover
            // want = min(frames - got, avail); a zero avail re-arms the wait.
            abi::move_register("%v14", abi::return_register()), // avail frames
            abi::load_u64("%v9", abi::stack_pointer(), GOT_OFF),
            abi::load_u64("%v10", abi::stack_pointer(), FRAMES_OFF),
            abi::subtract_registers("%v10", "%v10", "%v9"), // remaining frames
            abi::compare_registers("%v14", "%v10"),
            abi::branch_ge(&want_cap),
            abi::move_register("%v10", "%v14"), // want = avail
            abi::label(&want_cap),
            abi::compare_immediate("%v10", "0"),
            abi::branch_eq(&loop_top),
            abi::store_u64("%v10", abi::stack_pointer(), WANT_OFF),
            abi::load_u64("%v9", abi::stack_pointer(), GOT_OFF), // reload for readi math
        ]);
    }
    instructions.extend([
        // snd_pcm_readi(pcm, payload + got*bpf, <count>)
        abi::load_u64("%v11", abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), "%v11", S_OSOBJECT),
        abi::load_u64("%v12", abi::stack_pointer(), SRC_OFF),
        abi::load_u64("%v13", abi::stack_pointer(), BPF_OFF),
        abi::multiply_registers("%v14", "%v9", "%v13"),
        abi::add_registers(abi::ARG[1], "%v12", "%v14"),
    ]);
    if timeout {
        instructions.push(abi::load_u64(abi::ARG[2], abi::stack_pointer(), WANT_OFF));
    } else {
        instructions.push(abi::subtract_registers(abi::ARG[2], "%v10", "%v9"));
    }
    instructions.extend([
        abi::load_u64("%v8", abi::stack_pointer(), FN2_OFF),
        abi::branch_link_register("%v8"),
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), N_OFF),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&ok_frames),
        abi::move_immediate("%v10", "Integer", EINTR),
        abi::subtract_registers("%v10", abi::ZERO, "%v10"),
        abi::compare_registers(abi::return_register(), "%v10"),
        abi::branch_eq(&loop_top),
        abi::branch(&recover),
        abi::label(&ok_frames),
        abi::load_u64("%v9", abi::stack_pointer(), GOT_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), N_OFF),
        abi::add_registers("%v9", "%v9", "%v10"),
        abi::store_u64("%v9", abi::stack_pointer(), GOT_OFF),
        abi::branch(&loop_top),
        abi::label(&recover),
        abi::load_u64("%v11", abi::stack_pointer(), STATE_OFF),
        abi::load_u64("%v12", "%v11", S_XRUNS),
        abi::add_immediate("%v12", "%v12", 1),
        abi::store_u64("%v12", "%v11", S_XRUNS),
        abi::load_u64(abi::return_register(), "%v11", S_OSOBJECT),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), N_OFF),
        abi::move_immediate(abi::ARG[2], "Integer", "1"),
        abi::load_u64("%v8", abi::stack_pointer(), FNPTR_OFF),
        abi::branch_link_register("%v8"),
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
        abi::branch(&loop_top),
        abi::label(&loop_done),
    ]);
    if timeout {
        // Partial timed read: if fewer than `frames` gathered, return a
        // right-sized list of `got` frames and free the oversized pre-alloc.
        let ret_full = format!("{symbol}_ret_full");
        let fin_loop = format!("{symbol}_fin");
        let fin_done = format!("{symbol}_fin_done");
        instructions.extend([
            abi::load_u64("%v9", abi::stack_pointer(), GOT_OFF),
            abi::load_u64("%v10", abi::stack_pointer(), FRAMES_OFF),
            abi::compare_registers("%v9", "%v10"),
            abi::branch_ge(&ret_full),
            abi::load_u64("%v13", abi::stack_pointer(), BPF_OFF),
            abi::multiply_registers("%v9", "%v9", "%v13"), // gotBytes = got * bpf
            abi::store_u64("%v9", abi::stack_pointer(), GOTBYTES_OFF),
        ]);
        emit_alloc_byte_list(symbol, "final", GOTBYTES_OFF, FINAL_LIST_OFF, &alloc_fail, &mut instructions, &mut relocations);
        instructions.extend([
            // copy gotBytes from the oversized payload into the final payload.
            abi::load_u64("%v9", abi::stack_pointer(), GOTBYTES_OFF),
            abi::load_u64("%v11", abi::stack_pointer(), FINAL_LIST_OFF),
            abi::move_immediate("%v13", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
            abi::multiply_registers("%v13", "%v9", "%v13"),
            abi::add_immediate("%v13", "%v13", COLLECTION_HEADER_SIZE),
            abi::add_registers("%v11", "%v11", "%v13"), // final payload
            abi::load_u64("%v12", abi::stack_pointer(), SRC_OFF), // source payload
            abi::move_immediate("%v16", "Integer", "0"),
            abi::label(&fin_loop),
            abi::compare_registers("%v16", "%v9"),
            abi::branch_ge(&fin_done),
            abi::add_registers("%v17", "%v12", "%v16"),
            abi::load_u8("%v18", "%v17", 0),
            abi::add_registers("%v17", "%v11", "%v16"),
            abi::store_u8("%v18", "%v17", 0),
            abi::add_immediate("%v16", "%v16", 1),
            abi::branch(&fin_loop),
            abi::label(&fin_done),
            // Return the oversized pre-alloc to the arena (size matches
            // emit_alloc_byte_list: need*ENTRY + HEADER + need).
            abi::load_u64("%v9", abi::stack_pointer(), NEED_OFF),
            abi::move_immediate("%v10", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
            abi::multiply_registers("%v11", "%v9", "%v10"),
            abi::add_immediate("%v11", "%v11", COLLECTION_HEADER_SIZE),
            abi::add_registers("%v11", "%v11", "%v9"),
            abi::move_register(abi::ARG[1], "%v11"),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), LIST_OFF),
        ]);
        emit_arena_free(symbol, &mut instructions, &mut relocations);
        instructions.extend([
            abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), FINAL_LIST_OFF),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
            abi::label(&ret_full),
        ]);
    }
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), LIST_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
    ]);
    emit_fail(symbol, ERR_INVALID_ARGUMENT_CODE, ERR_INVALID_ARGUMENT_SYMBOL, &mut instructions, &mut relocations, &done);
    instructions.push(abi::label(&unavailable));
    emit_fail(symbol, ERR_AUDIO_UNAVAILABLE_CODE, ERR_AUDIO_UNAVAILABLE_SYMBOL, &mut instructions, &mut relocations, &done);
    instructions.push(abi::label(&dev_fail));
    emit_fail(symbol, ERR_AUDIO_DEVICE_CODE, ERR_AUDIO_DEVICE_SYMBOL, &mut instructions, &mut relocations, &done);
    instructions.push(abi::label(&alloc_fail));
    emit_fail(symbol, ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_SYMBOL, &mut instructions, &mut relocations, &done);
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME);
    Ok((frame, instructions, relocations, stack_slots))
}

/// available/poll/xruns via snd_pcm_avail_update / snd_pcm_wait / the xruns
/// counter (§3.4).
fn lower_query(
    symbol: &str,
    kind: Query,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    let unavailable = format!("{symbol}_unavailable");
    let closed = format!("{symbol}_closed");
    let clamp = format!("{symbol}_clamp");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        // Spill the incoming `timeoutMs` (ARG[1]) before any dlopen/libc call
        // clobbers it; the `PollTimeout` arm reloads it from `FRAMES_OFF` as the
        // `snd_pcm_wait` timeout. Without this store that slot is uninitialized
        // stack (bug-167 finding A). `FRAMES_OFF` is otherwise unused in this
        // function, so the store is harmless for the other queries.
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), FRAMES_OFF),
        abi::load_u64("%v9", abi::return_register(), H_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("%v10", abi::return_register(), H_STATE),
        abi::store_u64("%v10", abi::stack_pointer(), STATE_OFF),
    ]);
    match kind {
        Query::Xruns => {
            instructions.extend([
                abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
                abi::load_u64(RESULT_VALUE_REGISTER, "%v10", S_XRUNS),
                abi::branch(&clamp),
                abi::label(&clamp),
            ]);
        }
        Query::Available | Query::Poll => {
            emit_dlopen(symbol, &unavailable, platform, platform_imports, &mut instructions, &mut relocations)?;
            emit_alsa_call(symbol, "snd_pcm_avail_update", &unavailable, platform, platform_imports, &mut instructions, &mut relocations, |ins| {
                ins.extend([
                    abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
                    abi::load_u64(abi::return_register(), "%v10", S_OSOBJECT),
                ]);
            })?;
            // clamp negative to 0
            instructions.extend([
                abi::move_register("%v12", abi::return_register()),
                abi::compare_immediate("%v12", "0"),
                abi::branch_ge(&clamp),
                abi::move_immediate("%v12", "Integer", "0"),
                abi::label(&clamp),
            ]);
            if let Query::Poll = kind {
                let set = format!("{symbol}_poll_set");
                instructions.extend([
                    abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
                    abi::compare_immediate("%v12", "0"),
                    abi::branch_eq(&set),
                    abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
                    abi::label(&set),
                ]);
            } else {
                instructions.push(abi::move_register(RESULT_VALUE_REGISTER, "%v12"));
            }
        }
        Query::PollTimeout => {
            emit_dlopen(symbol, &unavailable, platform, platform_imports, &mut instructions, &mut relocations)?;
            emit_alsa_call(symbol, "snd_pcm_wait", &unavailable, platform, platform_imports, &mut instructions, &mut relocations, |ins| {
                ins.extend([
                    abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
                    abi::load_u64(abi::return_register(), "%v10", S_OSOBJECT),
                    abi::load_u64(abi::ARG[1], abi::stack_pointer(), FRAMES_OFF),
                ]);
            })?;
            // snd_pcm_wait returns 1 ready, 0 timeout, <0 error → Boolean(>0)
            let set = format!("{symbol}_pt_set");
            instructions.extend([
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
                abi::compare_immediate(abi::return_register(), "1"),
                abi::branch_ne(&set),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
                abi::label(&set),
                abi::label(&clamp),
            ]);
        }
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&unavailable),
    ]);
    emit_fail(symbol, ERR_AUDIO_UNAVAILABLE_CODE, ERR_AUDIO_UNAVAILABLE_SYMBOL, &mut instructions, &mut relocations, &done);
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME);
    Ok((frame, instructions, relocations, stack_slots))
}

/// close(stream): drain (playback) or drop (capture), snd_pcm_close, munmap.
fn lower_close(
    symbol: &str,
    input: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    let already = format!("{symbol}_already");
    let unavailable = format!("{symbol}_unavailable");
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
    emit_dlopen(symbol, &unavailable, platform, platform_imports, &mut instructions, &mut relocations)?;
    // snd_pcm_drain (playback) / snd_pcm_drop (capture); failure is reported but
    // must not skip close.
    emit_alsa_call(symbol, if input { "snd_pcm_drop" } else { "snd_pcm_drain" }, &unavailable, platform, platform_imports, &mut instructions, &mut relocations, |ins| {
        ins.extend([
            abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
            abi::load_u64(abi::return_register(), "%v10", S_OSOBJECT),
        ]);
    })?;
    emit_alsa_call(symbol, "snd_pcm_close", &unavailable, platform, platform_imports, &mut instructions, &mut relocations, |ins| {
        ins.extend([
            abi::load_u64("%v10", abi::stack_pointer(), STATE_OFF),
            abi::load_u64(abi::return_register(), "%v10", S_OSOBJECT),
        ]);
    })?;
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u64("%v9", "%v10", H_CLOSED),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::ARG[1], abi::return_register(), S_MAP_SIZE),
    ]);
    platform.emit_libc_call("munmap", symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::label(&already),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&unavailable),
    ]);
    emit_fail(symbol, ERR_AUDIO_UNAVAILABLE_CODE, ERR_AUDIO_UNAVAILABLE_SYMBOL, &mut instructions, &mut relocations, &done);
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME);
    Ok((frame, instructions, relocations, stack_slots))
}

/// Build an MFBASIC `String` at `out_off` from the malloc'd C string whose
/// pointer is in `%v9` (stops at NUL or the first newline for DESC). A null
/// pointer yields an empty String.
fn emit_string_from_cstr(
    symbol: &str,
    tag: &str,
    out_off: usize,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let len_loop = format!("{symbol}_{tag}_len");
    let len_done = format!("{symbol}_{tag}_len_done");
    let copy_loop = format!("{symbol}_{tag}_copy");
    let copy_done = format!("{symbol}_{tag}_copy_done");
    // %v9 = cstr ptr; save it, strlen (stop at NUL or '\n').
    instructions.extend([
        abi::store_u64("%v9", abi::stack_pointer(), RC_OFF), // reuse RC_OFF as cstr save
        abi::move_immediate("%v10", "Integer", "0"),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&len_done),
        abi::label(&len_loop),
        abi::load_u8("%v11", "%v9", 0),
        abi::compare_immediate("%v11", "0"),
        abi::branch_eq(&len_done),
        abi::compare_immediate("%v11", "10"), // '\n'
        abi::branch_eq(&len_done),
        abi::add_immediate("%v9", "%v9", 1),
        abi::add_immediate("%v10", "%v10", 1),
        abi::branch(&len_loop),
        abi::label(&len_done),
        abi::store_u64("%v10", abi::stack_pointer(), N_OFF), // len
        abi::add_immediate(abi::return_register(), "%v10", 9),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::move_register("%v15", abi::RET[1]),
        abi::load_u64("%v10", abi::stack_pointer(), N_OFF),
        abi::store_u64("%v10", "%v15", 0),
        abi::store_u64("%v15", abi::stack_pointer(), out_off),
        abi::load_u64("%v11", abi::stack_pointer(), RC_OFF), // cstr
        abi::add_immediate("%v12", "%v15", 8),
        abi::move_immediate("%v13", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("%v13", "%v10"),
        abi::branch_ge(&copy_done),
        abi::load_u8("%v14", "%v11", 0),
        abi::store_u8("%v14", "%v12", 0),
        abi::add_immediate("%v11", "%v11", 1),
        abi::add_immediate("%v12", "%v12", 1),
        abi::add_immediate("%v13", "%v13", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, "%v12", 0),
    ]);
}

fn lower_devices(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    let unavailable = format!("{symbol}_unavailable");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let count_loop = format!("{symbol}_count");
    let count_done = format!("{symbol}_count_done");
    let fill_loop = format!("{symbol}_fill");
    let fill_done = format!("{symbol}_fill_done");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    emit_dlopen(symbol, &unavailable, platform, platform_imports, &mut instructions, &mut relocations)?;
    // snd_device_name_hint(-1, "pcm", &hints)
    emit_alsa_call(symbol, "snd_device_name_hint", &unavailable, platform, platform_imports, &mut instructions, &mut relocations, |ins| {
        ins.push(abi::bitwise_not(abi::return_register(), abi::ZERO)); // -1
        emit_data_address(symbol, abi::ARG[1], "_mfb_audio_alsa_pcm", ins, &mut Vec::new());
        ins.push(abi::add_immediate(abi::ARG[2], abi::stack_pointer(), HINTS_OFF));
    })?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&unavailable),
        // count NULL-terminated hints
        abi::load_u64("%v9", abi::stack_pointer(), HINTS_OFF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), COUNT_OFF),
        abi::label(&count_loop),
        abi::load_u64("%v10", "%v9", 0),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&count_done),
        abi::load_u64("%v11", abi::stack_pointer(), COUNT_OFF),
        abi::add_immediate("%v11", "%v11", 1),
        abi::store_u64("%v11", abi::stack_pointer(), COUNT_OFF),
        abi::add_immediate("%v9", "%v9", 8),
        abi::branch(&count_loop),
        abi::label(&count_done),
    ]);
    // Allocate List OF AudioDevice (48-byte records inline).
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), COUNT_OFF),
        abi::move_immediate("%v11", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v12", "%v10", "%v11"),
        abi::add_immediate("%v12", "%v12", COLLECTION_HEADER_SIZE),
        abi::move_immediate("%v13", "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers("%v14", "%v10", "%v13"),
        abi::add_registers(abi::return_register(), "%v12", "%v14"),
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
        abi::move_immediate("%v13", "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers("%v14", "%v10", "%v13"),
        abi::store_u64("%v14", "%v15", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("%v14", "%v15", COLLECTION_OFFSET_DATA_CAPACITY),
        // data region base = list + HEADER + count*ENTRY
        abi::add_immediate("%v11", "%v15", COLLECTION_HEADER_SIZE),
        abi::move_immediate("%v12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v13", "%v10", "%v12"),
        abi::add_registers("%v14", "%v11", "%v13"),
        abi::store_u64("%v14", abi::stack_pointer(), SRC_OFF), // data region base
        abi::store_u64("%v11", abi::stack_pointer(), TOTAL_OFF), // entry cursor base
        abi::load_u64("%v9", abi::stack_pointer(), HINTS_OFF),
        abi::store_u64("%v9", abi::stack_pointer(), HINT_PTR_OFF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), OFFSET_OFF), // index
        abi::label(&fill_loop),
        abi::load_u64("%v9", abi::stack_pointer(), HINT_PTR_OFF),
        abi::load_u64("%v10", "%v9", 0), // hint
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&fill_done),
        abi::store_u64("%v10", abi::stack_pointer(), N_OFF), // current hint
    ]);
    // id = get_hint(hint, "NAME")
    emit_alsa_call(symbol, "snd_device_name_get_hint", &unavailable, platform, platform_imports, &mut instructions, &mut relocations, |ins| {
        ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), N_OFF));
        emit_data_address(symbol, abi::ARG[1], "_mfb_audio_alsa_hint_name", ins, &mut Vec::new());
    })?;
    instructions.push(abi::move_register("%v9", abi::return_register()));
    emit_string_from_cstr(symbol, "id", DEVID_OFF, &alloc_fail, &mut instructions, &mut relocations);
    // free the id cstring
    instructions.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), RC_OFF));
    platform.emit_libc_call("free", symbol, platform_imports, &mut instructions, &mut relocations)?;
    // name = get_hint(hint, "DESC")
    emit_alsa_call(symbol, "snd_device_name_get_hint", &unavailable, platform, platform_imports, &mut instructions, &mut relocations, |ins| {
        // Reload the hint by dereferencing HINT_PTR_OFF rather than reading N_OFF:
        // `emit_string_from_cstr` reused N_OFF as strlen scratch while building the
        // id String, so N_OFF now holds the id length, not the hint pointer. Using
        // it here passed libasound an integer as `const void* hint` (bug-167
        // finding B: SIGSEGV / empty device name).
        ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), HINT_PTR_OFF));
        ins.push(abi::load_u64(abi::return_register(), abi::return_register(), 0));
        emit_data_address(symbol, abi::ARG[1], "_mfb_audio_alsa_hint_desc", ins, &mut Vec::new());
    })?;
    instructions.push(abi::move_register("%v9", abi::return_register()));
    emit_string_from_cstr(symbol, "name", NAME_OFF, &alloc_fail, &mut instructions, &mut relocations);
    instructions.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), RC_OFF));
    platform.emit_libc_call("free", symbol, platform_imports, &mut instructions, &mut relocations)?;
    // Build the record: id, name, canInput=1, canOutput=1, defaults=0 (a precise
    // IOID split is a refinement; ALSA hints usually permit both directions).
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::move_immediate("%v10", "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers("%v11", "%v9", "%v10"),
        abi::load_u64("%v12", abi::stack_pointer(), SRC_OFF),
        abi::add_registers("%v12", "%v12", "%v11"), // record ptr
        abi::load_u64("%v13", abi::stack_pointer(), DEVID_OFF),
        abi::store_u64("%v13", "%v12", DEVICE_FIELD_ID),
        abi::load_u64("%v13", abi::stack_pointer(), NAME_OFF),
        abi::store_u64("%v13", "%v12", DEVICE_FIELD_NAME),
        abi::move_immediate("%v13", "Integer", "1"),
        abi::store_u64("%v13", "%v12", DEVICE_FIELD_CAN_INPUT),
        abi::store_u64("%v13", "%v12", DEVICE_FIELD_CAN_OUTPUT),
        abi::store_u64(abi::ZERO, "%v12", DEVICE_FIELD_IS_DEFAULT_INPUT),
        abi::store_u64(abi::ZERO, "%v12", DEVICE_FIELD_IS_DEFAULT_OUTPUT),
        // entry descriptor
        abi::load_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::move_immediate("%v10", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v11", "%v9", "%v10"),
        abi::load_u64("%v12", abi::stack_pointer(), TOTAL_OFF),
        abi::add_registers("%v12", "%v12", "%v11"),
        abi::move_immediate("%v13", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8("%v13", "%v12", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(abi::ZERO, "%v12", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64(abi::ZERO, "%v12", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::move_immediate("%v10", "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers("%v11", "%v9", "%v10"),
        abi::store_u64("%v11", "%v12", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::move_immediate("%v13", "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::store_u64("%v13", "%v12", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        // advance
        abi::add_immediate("%v9", "%v9", 1),
        abi::store_u64("%v9", abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64("%v9", abi::stack_pointer(), HINT_PTR_OFF),
        abi::add_immediate("%v9", "%v9", 8),
        abi::store_u64("%v9", abi::stack_pointer(), HINT_PTR_OFF),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
    ]);
    // snd_device_name_free_hint(hints)
    emit_alsa_call(symbol, "snd_device_name_free_hint", &unavailable, platform, platform_imports, &mut instructions, &mut relocations, |ins| {
        ins.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), HINTS_OFF));
    })?;
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), LIST_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&unavailable),
    ]);
    emit_fail(symbol, ERR_AUDIO_UNAVAILABLE_CODE, ERR_AUDIO_UNAVAILABLE_SYMBOL, &mut instructions, &mut relocations, &done);
    instructions.push(abi::label(&alloc_fail));
    emit_fail(symbol, ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_SYMBOL, &mut instructions, &mut relocations, &done);
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME);
    Ok((frame, instructions, relocations, stack_slots))
}
