//! Linux ALSA `audio` shared codegen: dlopen/dlsym plumbing, constants, helpers, data objects, and the platform dispatcher.

use super::gen_alsa_devices::*;
use super::gen_alsa_io::*;
use super::gen_alsa_stream::*;
use super::gen_common::*;
use super::gen_os_seam::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::memory::arena::*;
use crate::codegen::string::util::*;
use crate::target::shared::abi;
use std::collections::HashMap;

pub(crate) const ALSA_SONAME: &str = "libasound.so.2";

pub(crate) const RTLD_NOW: &str = "2"; // RTLD_NOW | RTLD_LOCAL (RTLD_LOCAL == 0)

// snd_pcm constants (alsa/pcm.h).
pub(crate) const STREAM_PLAYBACK: &str = "0";

pub(crate) const STREAM_CAPTURE: &str = "1";

pub(crate) const ACCESS_RW_INTERLEAVED: &str = "3";

pub(crate) const FORMAT_S16_LE: &str = "2";

pub(crate) const EINTR: &str = "4"; // -EINTR

// Every ALSA symbol the backend may `dlsym`. A wrong-ABI `libasound` (a missing
// symbol) raises `ErrAudioUnavailable` naming it (§4).
pub(crate) const ALSA_SYMBOLS: &[&str] = &[
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

pub(crate) fn lib_data_symbol() -> String {
    "_mfb_audio_alsa_soname".to_string()
}

pub(crate) fn sym_data_symbol(name: &str) -> String {
    format!("_mfb_audio_alsa_sym_{name}")
}

/// The read-only C strings (soname + ALSA symbol names) the backend references.
pub(crate) fn data_objects() -> Vec<CodeDataObject> {
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
pub(crate) const FRAME: usize = 640;

pub(crate) const HANDLE_OFF: usize = 8;

pub(crate) const STATE_OFF: usize = 16;

pub(crate) const DL_HANDLE_OFF: usize = 24; // dlopen handle

pub(crate) const FNPTR_OFF: usize = 32; // scratch fn-ptr

pub(crate) const PARAMS_OFF: usize = 48; // snd_pcm_hw_params_t*

pub(crate) const SR_OFF: usize = 56;

pub(crate) const CH_OFF: usize = 64;

pub(crate) const BF_OFF: usize = 72;

pub(crate) const BPF_OFF: usize = 80;

pub(crate) const RATE_OFF: usize = 88; // unsigned rate (in/out)

pub(crate) const CHANS_OFF: usize = 96; // unsigned channels (out)

pub(crate) const DIR_OFF: usize = 104; // int dir

pub(crate) const PERIOD_OFF: usize = 112; // snd_pcm_uframes_t period

pub(crate) const BUFSZ_OFF: usize = 120; // snd_pcm_uframes_t buffer

pub(crate) const FRAMES_OFF: usize = 128;

pub(crate) const NEED_OFF: usize = 136;

pub(crate) const FRAMES_GOT_OFF: usize = 144;

pub(crate) const LIST_OFF: usize = 152;

pub(crate) const SRC_OFF: usize = 160; // byte payload src / dst

pub(crate) const TOTAL_OFF: usize = 168;

pub(crate) const OFFSET_OFF: usize = 176;

pub(crate) const N_OFF: usize = 184; // frames this iteration

pub(crate) const DEVID_OFF: usize = 192;

pub(crate) const RC_OFF: usize = 200; // ALSA return code

pub(crate) const NAME_OFF: usize = 208; // C-string device name for open

pub(crate) const FN2_OFF: usize = 216; // secondary fn-ptr (writei/readi kept across a loop)

pub(crate) const HINTS_OFF: usize = 224; // void** device hints

pub(crate) const HINT_PTR_OFF: usize = 232; // current hint cursor

pub(crate) const COUNT_OFF: usize = 240;

pub(crate) const NAME_BUF_OFF: usize = 256; // 128-byte device name buffer -> 256..384

// Timed-read (readTimeout) scratch; unused by the blocking read/write/open paths.
pub(crate) const FINAL_LIST_OFF: usize = 384; // right-sized result for a partial timed read

pub(crate) const GOTBYTES_OFF: usize = 392; // bytes gathered so far (frames * bpf)

pub(crate) const WANT_OFF: usize = 400; // frames to request from readi this iteration

pub(crate) const TIMEOUT_OFF: usize = 408; // timeoutMs (spilled at entry)

pub(crate) const DEADLINE_OFF: usize = 416; // absolute deadline (ns, CLOCK_MONOTONIC)

pub(crate) const CLK_OFF: usize = 424; // clock_gettime timespec -> 424..440

pub(crate) const WAIT_FN_OFF: usize = 440; // cached snd_pcm_wait fn-ptr

pub(crate) const AVAIL_FN_OFF: usize = 448; // cached snd_pcm_avail_update fn-ptr

/// Resolve `libasound.so.2` (dlopen), storing the handle at `DL_HANDLE_OFF`;
/// branch to `unavailable` if it does not load.
pub(crate) fn emit_dlopen(ctx: &mut EmitCtx, unavailable: &str) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    emit_data_address(
        symbol,
        abi::return_register(),
        &lib_data_symbol(),
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions
        .push(abi::move_immediate(abi::c_arg(1), "Integer", RTLD_NOW));
    platform.emit_external_call(
        "dlopen",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DL_HANDLE_OFF),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(unavailable),
    ]);
    Ok(())
}

/// `dlsym(handle, name)` into `FNPTR_OFF`; branch to `unavailable` if null.
pub(crate) fn emit_dlsym(ctx: &mut EmitCtx, name: &str, unavailable: &str) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    ctx.instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        DL_HANDLE_OFF,
    ));
    emit_data_address(
        symbol,
        abi::c_arg(1),
        &sym_data_symbol(name),
        ctx.instructions,
        ctx.relocations,
    );
    platform.emit_external_call(
        "dlsym",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(unavailable),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FNPTR_OFF),
    ]);
    Ok(())
}

pub(crate) fn lower_audio_alsa(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
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
/// return in the return register. An `int`-returning libasound call is
/// sign-extended to 64 bits (for the signed error-code comparisons); a
/// pointer-returning call (`snd_device_name_get_hint` → `char*`) must NOT be
/// sign-extended, or its 64-bit pointer is truncated to its low 32 bits and the
/// subsequent cstr copy dereferences garbage (SIGSEGV on x86-64, where the image
/// base is above 4 GiB).
pub(crate) fn emit_call_fnptr(
    instructions: &mut Vec<CodeInstruction>,
    returns_pointer: bool,
    vregs: &mut Vregs,
) {
    let v8 = vregs.next();
    instructions.extend([
        abi::load_u64(&v8, abi::stack_pointer(), FNPTR_OFF),
        abi::branch_link_register(&v8),
    ]);
    if !returns_pointer {
        instructions.push(abi::sign_extend_word(
            abi::return_register(),
            abi::return_register(),
        ));
    }
}

// Upper bound on a timed `audio::read` (plan-33-A §3.5). The open-parameter
// ranges and `READ_FRAMES_MAX` are shared in `common` (bug-330); this bound is
// ALSA-only, so it stays here.
// plan-73-B: the convention clamps a too-large `timeoutMs` to INT_MAX (the host
// deadline math takes a C `int`) rather than raising the old 24h cap.
pub(crate) const TIMEOUT_CLAMP_MS: &str = "2147483647";

/// dlsym `name` into `FNPTR_OFF`, stage the args via `stage`, call it, and leave
/// the (sign-extended) result in the return register.
pub(crate) fn emit_alsa_call(
    vregs: &mut Vregs,
    ctx: &mut EmitCtx,
    name: &str,
    unavailable: &str,
    // `returns_pointer` is true for a libasound call whose result is a `char*`
    // (only `snd_device_name_get_hint`): its return must not be sign-extended.
    returns_pointer: bool,
    // The stage closure also receives the function's real `relocations` vec so a
    // staged `emit_data_address` (a string-pointer load) records its adrp/add
    // relocation instead of dropping it into a throwaway Vec (bug-206).
    stage: impl Fn(&mut Vec<CodeInstruction>, &mut Vec<CodeRelocation>),
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        name,
        unavailable,
    )?;
    stage(ctx.instructions, ctx.relocations);
    emit_call_fnptr(ctx.instructions, returns_pointer, vregs);
    Ok(())
}

/// Copy an MFBASIC `String` (pointer at `str_off`'s record field) into the
/// NUL-terminated name buffer at `NAME_BUF_OFF`, storing its address at
/// `NAME_OFF`.
pub(crate) fn emit_device_cstring(
    device_off: usize,
    instructions: &mut Vec<CodeInstruction>,
    symbol: &str,
    vregs: &mut Vregs,
) {
    let copy = format!("{symbol}_dev_copy");
    let done = format!("{symbol}_dev_copy_done");
    let clamp_ok = format!("{symbol}_dev_clamp_ok");
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), device_off),
        abi::load_u64(&v9, &v9, DEVICE_FIELD_ID), // id String ptr
        abi::load_u64(&v10, &v9, 0),              // len
        abi::add_immediate(&v11, &v9, 8),         // src bytes
        // Clamp the copy count to NAME_BUF's 128 bytes minus the NUL terminator;
        // an oversized device id would otherwise overrun the fixed buffer.
        abi::move_immediate(&v9, "Integer", "127"),
        abi::compare_registers(&v10, &v9),
        abi::branch_le(&clamp_ok),
        abi::move_register(&v10, &v9),
        abi::label(&clamp_ok),
        abi::add_immediate(&v12, abi::stack_pointer(), NAME_BUF_OFF),
        abi::store_u64(&v12, abi::stack_pointer(), NAME_OFF),
        abi::move_immediate(&v13, "Integer", "0"),
        abi::label(&copy),
        abi::compare_registers(&v13, &v10),
        abi::branch_ge(&done),
        abi::load_u8(&v14, &v11, 0),
        abi::store_u8(&v14, &v12, 0),
        abi::add_immediate(&v11, &v11, 1),
        abi::add_immediate(&v12, &v12, 1),
        abi::add_immediate(&v13, &v13, 1),
        abi::branch(&copy),
        abi::label(&done),
        abi::store_u8(abi::ZERO, &v12, 0),
    ]);
}

/// Release everything `lower_open` may have acquired, in acquisition-reverse
/// order: the `snd_pcm_hw_params_t`, the open PCM handle, and the mmap'd state
/// page. Every disposal is guarded on its own slot, so this is correct at any
/// point after entry — `STATE_OFF`/`PARAMS_OFF` are zeroed there and `mmap`
/// zero-fills `S_OSOBJECT` (bug-180, bug-319).
///
/// `tag` disambiguates the labels so both error exits can inline it. A `dlsym`
/// miss inside the cleanup skips only that disposal and continues to the next —
/// it must never branch back to an error exit, which would loop.
pub(crate) fn emit_open_cleanup(
    vregs: &mut Vregs,
    ctx: &mut EmitCtx,
    tag: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let params_done = format!("{symbol}_{tag}_params_done");
    let cleanup_munmap = format!("{symbol}_{tag}_munmap");
    let cleanup_done = format!("{symbol}_{tag}_cleanup_done");
    let v10 = vregs.next();
    let v9 = vregs.next();
    ctx.instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), PARAMS_OFF),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&params_done),
    ]);
    emit_alsa_call(
        &mut *vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "snd_pcm_hw_params_free",
        &params_done,
        false,
        |ins, _relocs| {
            ins.push(abi::load_u64(
                abi::return_register(),
                abi::stack_pointer(),
                PARAMS_OFF,
            ));
        },
    )?;
    ctx.instructions.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), PARAMS_OFF),
        abi::label(&params_done),
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&cleanup_done),
        abi::load_u64(&v9, &v10, S_OSOBJECT),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&cleanup_munmap),
    ]);
    emit_alsa_call(
        &mut *vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "snd_pcm_close",
        &cleanup_munmap,
        false,
        |ins, _relocs| {
            ins.extend([
                abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
                abi::load_u64(abi::return_register(), &v10, S_OSOBJECT),
            ]);
        },
    )?;
    ctx.instructions.extend([
        abi::label(&cleanup_munmap),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::c_arg(1), abi::return_register(), S_MAP_SIZE),
    ]);
    platform.emit_external_call(
        "munmap",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), STATE_OFF),
        abi::label(&cleanup_done),
    ]);
    Ok(())
}

/// Configure and commit the hw params (§3.3): interleaved S16_LE at the
/// requested channels/rate, buffer = bufferFrames*4. Verify the committed rate
/// and channels match the request, else `ErrAudioDevice` (no silent resampling).
pub(crate) fn emit_configure_hw_params(
    vregs: &mut Vregs,
    ctx: &mut EmitCtx,
    unavailable: &str,
    dev_fail: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let v9 = vregs.next();
    let v10 = vregs.next();
    let pcm = |ins: &mut Vec<CodeInstruction>| {
        ins.extend([
            abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
            abi::load_u64(abi::return_register(), &v9, S_OSOBJECT),
        ]);
    };
    let params = |ins: &mut Vec<CodeInstruction>| {
        ins.push(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            PARAMS_OFF,
        ));
    };
    let check = |ins: &mut Vec<CodeInstruction>, fail: &str| {
        ins.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(fail),
        ]);
    };
    // snd_pcm_hw_params_malloc(&params)
    emit_alsa_call(
        &mut *vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "snd_pcm_hw_params_malloc",
        unavailable,
        false,
        |ins, _relocs| {
            ins.push(abi::add_immediate(
                abi::return_register(),
                abi::stack_pointer(),
                PARAMS_OFF,
            ));
        },
    )?;
    check(ctx.instructions, dev_fail);
    // any
    emit_alsa_call(
        &mut *vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "snd_pcm_hw_params_any",
        unavailable,
        false,
        |ins, _relocs| {
            pcm(ins);
            params(ins);
        },
    )?;
    check(ctx.instructions, dev_fail);
    // set_access(INTERLEAVED)
    emit_alsa_call(
        &mut *vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "snd_pcm_hw_params_set_access",
        unavailable,
        false,
        |ins, _relocs| {
            pcm(ins);
            params(ins);
            ins.push(abi::move_immediate(
                abi::c_arg(2),
                "Integer",
                ACCESS_RW_INTERLEAVED,
            ));
        },
    )?;
    check(ctx.instructions, dev_fail);
    // set_format(S16_LE)
    emit_alsa_call(
        &mut *vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "snd_pcm_hw_params_set_format",
        unavailable,
        false,
        |ins, _relocs| {
            pcm(ins);
            params(ins);
            ins.push(abi::move_immediate(abi::c_arg(2), "Integer", FORMAT_S16_LE));
        },
    )?;
    check(ctx.instructions, dev_fail);
    // set_channels(channels)
    emit_alsa_call(
        &mut *vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "snd_pcm_hw_params_set_channels",
        unavailable,
        false,
        |ins, _relocs| {
            pcm(ins);
            params(ins);
            ins.push(abi::load_u64(abi::c_arg(2), abi::stack_pointer(), CH_OFF));
        },
    )?;
    check(ctx.instructions, dev_fail);
    // set_rate_near(&rate, &dir)
    ctx.instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), SR_OFF),
        abi::store_u32(&v9, abi::stack_pointer(), RATE_OFF),
        abi::store_u32(abi::ZERO, abi::stack_pointer(), DIR_OFF),
    ]);
    emit_alsa_call(
        &mut *vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "snd_pcm_hw_params_set_rate_near",
        unavailable,
        false,
        |ins, _relocs| {
            pcm(ins);
            params(ins);
            ins.push(abi::add_immediate(
                abi::c_arg(2),
                abi::stack_pointer(),
                RATE_OFF,
            ));
            ins.push(abi::add_immediate(
                abi::c_arg(3),
                abi::stack_pointer(),
                DIR_OFF,
            ));
        },
    )?;
    check(ctx.instructions, dev_fail);
    // set_period_size_near(&period, &dir) — period = bufferFrames
    ctx.instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), BF_OFF),
        abi::store_u64(&v9, abi::stack_pointer(), PERIOD_OFF),
        abi::store_u32(abi::ZERO, abi::stack_pointer(), DIR_OFF),
    ]);
    emit_alsa_call(
        &mut *vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "snd_pcm_hw_params_set_period_size_near",
        unavailable,
        false,
        |ins, _relocs| {
            pcm(ins);
            params(ins);
            ins.push(abi::add_immediate(
                abi::c_arg(2),
                abi::stack_pointer(),
                PERIOD_OFF,
            ));
            ins.push(abi::add_immediate(
                abi::c_arg(3),
                abi::stack_pointer(),
                DIR_OFF,
            ));
        },
    )?;
    check(ctx.instructions, dev_fail);
    // set_buffer_size_near(&buffer) — buffer = bufferFrames * 4 (mirror macOS depth)
    ctx.instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), BF_OFF),
        abi::move_immediate(&v10, "Integer", "4"),
        abi::multiply_registers(&v9, &v9, &v10),
        abi::store_u64(&v9, abi::stack_pointer(), BUFSZ_OFF),
    ]);
    emit_alsa_call(
        &mut *vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "snd_pcm_hw_params_set_buffer_size_near",
        unavailable,
        false,
        |ins, _relocs| {
            pcm(ins);
            params(ins);
            ins.push(abi::add_immediate(
                abi::c_arg(2),
                abi::stack_pointer(),
                BUFSZ_OFF,
            ));
        },
    )?;
    check(ctx.instructions, dev_fail);
    // commit
    emit_alsa_call(
        &mut *vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "snd_pcm_hw_params",
        unavailable,
        false,
        |ins, _relocs| {
            pcm(ins);
            params(ins);
        },
    )?;
    check(ctx.instructions, dev_fail);
    // get_rate(&rate) and get_channels(&chans); verify == request (§3.3).
    // The getters take `params` as their FIRST argument (unlike the setters, which
    // take `pcm` in ARG[0] and `params` in ARG[1]). Load `params` into ARG[0]
    // directly — calling the ARG[1]-targeting `params` closure and then overwriting
    // ARG[1] with `&rate`/`&chans` left ARG[0] holding the leftover dlsym fn-ptr, so
    // the getter read garbage and open failed the rate/channel verification (bug-207).
    emit_alsa_call(
        &mut *vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "snd_pcm_hw_params_get_rate",
        unavailable,
        false,
        |ins, _relocs| {
            ins.push(abi::load_u64(
                abi::return_register(),
                abi::stack_pointer(),
                PARAMS_OFF,
            ));
            ins.push(abi::add_immediate(
                abi::c_arg(1),
                abi::stack_pointer(),
                RATE_OFF,
            ));
            ins.push(abi::add_immediate(
                abi::c_arg(2),
                abi::stack_pointer(),
                DIR_OFF,
            ));
        },
    )?;
    emit_alsa_call(
        &mut *vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "snd_pcm_hw_params_get_channels",
        unavailable,
        false,
        |ins, _relocs| {
            ins.push(abi::load_u64(
                abi::return_register(),
                abi::stack_pointer(),
                PARAMS_OFF,
            ));
            ins.push(abi::add_immediate(
                abi::c_arg(1),
                abi::stack_pointer(),
                CHANS_OFF,
            ));
        },
    )?;
    ctx.instructions.extend([
        abi::load_u32(&v9, abi::stack_pointer(), RATE_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), SR_OFF),
        abi::compare_registers(&v9, &v10),
        abi::branch_ne(dev_fail),
        abi::load_u32(&v9, abi::stack_pointer(), CHANS_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), CH_OFF),
        abi::compare_registers(&v9, &v10),
        abi::branch_ne(dev_fail),
    ]);
    // free the hw_params object.
    emit_alsa_call(
        &mut *vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "snd_pcm_hw_params_free",
        unavailable,
        false,
        |ins, _relocs| {
            ins.push(abi::load_u64(
                abi::return_register(),
                abi::stack_pointer(),
                PARAMS_OFF,
            ));
        },
    )?;
    // Clear the slot: `lower_open` continues to prepare/start after this, and
    // those can still branch to `dev_fail`, whose cleanup frees a non-NULL
    // PARAMS_OFF — a stale pointer here would be a double free (bug-319).
    ctx.instructions
        .push(abi::store_u64(abi::ZERO, abi::stack_pointer(), PARAMS_OFF));
    Ok(())
}

/// Build an MFBASIC `String` at `out_off` from the malloc'd C string whose
/// pointer is in `%v9` (stops at NUL or the first newline for DESC). A null
/// pointer yields an empty String.
pub(crate) fn emit_string_from_cstr(
    symbol: &str,
    tag: &str,
    out_off: usize,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    cstr: &str,
    vregs: &mut Vregs,
) {
    let len_loop = format!("{symbol}_{tag}_len");
    let len_done = format!("{symbol}_{tag}_len_done");
    let copy_loop = format!("{symbol}_{tag}_copy");
    let copy_done = format!("{symbol}_{tag}_copy_done");
    let v9 = cstr;
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v15 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    // %v9 = cstr ptr; save it, strlen (stop at NUL or '\n').
    instructions.extend([
        abi::store_u64(&v9, abi::stack_pointer(), RC_OFF), // reuse RC_OFF as cstr save
        abi::move_immediate(&v10, "Integer", "0"),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&len_done),
        abi::label(&len_loop),
        abi::load_u8(&v11, &v9, 0),
        abi::compare_immediate(&v11, "0"),
        abi::branch_eq(&len_done),
        abi::compare_immediate(&v11, "10"), // '\n'
        abi::branch_eq(&len_done),
        abi::add_immediate(&v9, &v9, 1),
        abi::add_immediate(&v10, &v10, 1),
        abi::branch(&len_loop),
        abi::label(&len_done),
        abi::store_u64(&v10, abi::stack_pointer(), N_OFF), // len
        abi::add_immediate(abi::return_register(), &v10, 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::move_register(&v15, abi::mfb_return(1)),
        abi::load_u64(&v10, abi::stack_pointer(), N_OFF),
        abi::store_u64(&v10, &v15, 0),
        abi::store_u64(&v15, abi::stack_pointer(), out_off),
        abi::load_u64(&v11, abi::stack_pointer(), RC_OFF), // cstr
        abi::add_immediate(&v12, &v15, 8),
        abi::move_immediate(&v13, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&v13, &v10),
        abi::branch_ge(&copy_done),
        abi::load_u8(&v14, &v11, 0),
        abi::store_u8(&v14, &v12, 0),
        abi::add_immediate(&v11, &v11, 1),
        abi::add_immediate(&v12, &v12, 1),
        abi::add_immediate(&v13, &v13, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, &v12, 0),
    ]);
}
