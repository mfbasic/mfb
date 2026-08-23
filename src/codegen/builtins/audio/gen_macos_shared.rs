//! macOS Core Audio `audio` shared codegen: constants, the AudioQueue/property helpers, and the platform dispatcher.

use super::gen_common::*;
use super::gen_macos_devices::*;
use super::gen_macos_io::*;
use super::gen_macos_stream::*;
use super::gen_os_seam::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::target::shared::abi;
use std::collections::HashMap;

// --- Core Audio constants (verified against CoreAudio/AudioHardware.h) --------
pub(crate) const SYS_OBJECT: &str = "1"; // kAudioObjectSystemObject

pub(crate) const SEL_DEVICES: &str = "1684370979"; // 0x64657623 'dev#' kAudioHardwarePropertyDevices

pub(crate) const SEL_NAME: &str = "1819173229"; // 0x6C6E616D 'lnam' kAudioObjectPropertyName

pub(crate) const SEL_UID: &str = "1969841184"; // 0x75696420 'uid ' kAudioDevicePropertyDeviceUID

pub(crate) const SEL_STREAMCFG: &str = "1936482681"; // 0x736C6179 'slay' kAudioDevicePropertyStreamConfiguration

pub(crate) const SEL_DEFIN: &str = "1682533920"; // 0x64496E20 'dIn ' kAudioHardwarePropertyDefaultInputDevice

pub(crate) const SEL_DEFOUT: &str = "1682929012"; // 0x644F7574 'dOut' kAudioHardwarePropertyDefaultOutputDevice

pub(crate) const SCOPE_GLOBAL: &str = "1735159650"; // 0x676C6F62 'glob'

pub(crate) const SCOPE_INPUT: &str = "1768845428"; // 0x696E7074 'inpt'

pub(crate) const SCOPE_OUTPUT: &str = "1869968496"; // 0x6F757470 'outp'

pub(crate) const ENC_UTF8: &str = "134217984"; // kCFStringEncodingUTF8 = 0x08000100

// --- devices() stack frame ---------------------------------------------------
// Offsets are kept small (< ~1 KiB) so every `sp`-relative access stays within
// the AArch64 12-bit immediate range once the frame is finalized past the
// callee-saved area (a large offset would silently mis-address the buffer).
pub(crate) const FRAME_SIZE: usize = 1024;

pub(crate) const PROPADDR_OFF: usize = 16; // AudioObjectPropertyAddress (12 bytes)

pub(crate) const SIZE_OFF: usize = 32; // UInt32 ioDataSize

pub(crate) const COUNT_OFF: usize = 40;

pub(crate) const LIST_OFF: usize = 48;

pub(crate) const ENTRY_OFF: usize = 56; // entry-array cursor base

pub(crate) const DATA_OFF: usize = 64; // inline record data region base

pub(crate) const INDEX_OFF: usize = 72;

pub(crate) const CURID_OFF: usize = 80;

pub(crate) const DEFIN_OFF: usize = 88;

pub(crate) const DEFOUT_OFF: usize = 96;

pub(crate) const CFREF_OFF: usize = 104;

pub(crate) const IDPTR_OFF: usize = 112;

pub(crate) const NAMEPTR_OFF: usize = 120;

pub(crate) const CANIN_OFF: usize = 128;

pub(crate) const CANOUT_OFF: usize = 136;

pub(crate) const BOOLTMP_OFF: usize = 144;

pub(crate) const CSTRBUF_OFF: usize = 160; // 256-byte CFStringGetCString buffer

pub(crate) const CSTRBUF_CAP: &str = "256";

pub(crate) const IDSBUF_OFF: usize = 416; // up to 64 AudioDeviceID (u32)

pub(crate) const IDSBUF_CAP: &str = "256";

pub(crate) const BUFLIST_OFF: usize = 672; // AudioBufferList scratch

pub(crate) const BUFLIST_CAP: &str = "256";

pub(crate) fn lower_audio_macos(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
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
pub(crate) const FORMAT_LPCM: &str = "1819304813"; // 0x6C70636D 'lpcm' kAudioFormatLinearPCM

pub(crate) const FORMAT_FLAGS: &str = "12"; // kAudioFormatFlagIsSignedInteger | ...IsPacked

pub(crate) const MMAP_PROT: &str = "3"; // PROT_READ | PROT_WRITE

pub(crate) const MMAP_FLAGS: &str = "4098"; // MAP_ANON(0x1000) | MAP_PRIVATE(0x0002)

pub(crate) const MAP_FAILED_CMP: &str = "0";

/// Emit `pthread_<op>(state + field)` — object pointer in x0, called through the
/// platform ABI. `state_off` is the stack slot holding the AudioState pointer.
pub(crate) fn emit_pthread1(
    ctx: &mut EmitCtx,
    op: &str,
    state_off: usize,
    field: usize,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    ctx.instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), state_off),
        abi::add_immediate(abi::return_register(), abi::return_register(), field),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    platform.emit_external_call(
        op,
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )
}

// Stream-helper stack frame.
pub(crate) const F: usize = 512;

pub(crate) const SR_OFF: usize = 8;

pub(crate) const CH_OFF: usize = 16;

pub(crate) const BF_OFF: usize = 24;

pub(crate) const BPF_OFF: usize = 32; // bytesPerFrame

pub(crate) const HANDLE_OFF: usize = 40;

pub(crate) const STATE_OFF: usize = 48;

pub(crate) const QUEUE_OFF: usize = 56;

pub(crate) const BUFPTR_OFF: usize = 64;

pub(crate) const I_OFF: usize = 72;

pub(crate) const CAP_OFF: usize = 80; // buffer capacity bytes

pub(crate) const OFFSET_OFF: usize = 88; // write byte cursor

pub(crate) const TOTAL_OFF: usize = 96; // write total bytes

pub(crate) const DEVID_OFF: usize = 104; // AudioDevice arg (device overloads)

pub(crate) const FILL_OFF: usize = 112; // bytes already in the buffer being filled

pub(crate) const ASBD_OFF: usize = 128; // 40-byte AudioStreamBasicDescription -> 128..168

pub(crate) const UID_CFREF_OFF: usize = 168; // CFStringRef for device selection

pub(crate) const UID_CSTR_OFF: usize = 176; // 256-byte C string for the device UID -> 176..432

/// AudioQueueSetProperty(queue, kAudioQueueProperty_CurrentDevice, &uidCF, 8)
/// from the `AudioDevice.id` string, selecting the named device.
pub(crate) fn emit_select_device(
    ctx: &mut EmitCtx,
    dev_fail: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    // kAudioQueueProperty_CurrentDevice = 'aqcd' = 0x61716364 = 1634820964.
    // Build a CFString from the device id, set it, release it.
    let copy_loop = format!("{symbol}_uid_copy");
    let copy_done = format!("{symbol}_uid_copy_done");
    let clamp_ok = format!("{symbol}_uid_clamp_ok");
    // The device record's `id` String field pointer is at DEVID_OFF's record + H? No:
    // DEVID_OFF holds the AudioDevice record pointer; its `id` field is at offset 0.
    ctx.instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), DEVID_OFF),
        abi::load_u64(&v9, &v9, DEVICE_FIELD_ID), // id String ptr
        abi::store_u64(&v9, abi::stack_pointer(), BUFPTR_OFF),
        // Copy the String (len-prefixed) into the UID C-string buffer.
        abi::load_u64(&v10, &v9, 0),      // len
        abi::add_immediate(&v11, &v9, 8), // src bytes
        // Clamp the copy count to the 256-byte UID buffer minus the NUL
        // terminator; an oversized device id would otherwise overrun it.
        abi::move_immediate(&v9, "Integer", "255"),
        abi::compare_registers(&v10, &v9),
        abi::branch_le(&clamp_ok),
        abi::move_register(&v10, &v9),
        abi::label(&clamp_ok),
        abi::add_immediate(&v12, abi::stack_pointer(), UID_CSTR_OFF),
        abi::move_immediate(&v13, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&v13, &v10),
        abi::branch_eq(&copy_done),
        abi::load_u8(&v14, &v11, 0),
        abi::store_u8(&v14, &v12, 0),
        abi::add_immediate(&v11, &v11, 1),
        abi::add_immediate(&v12, &v12, 1),
        abi::add_immediate(&v13, &v13, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, &v12, 0),
        // CFStringCreateWithCString(NULL, uidCStr, kCFStringEncodingUTF8)
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), UID_CSTR_OFF),
        abi::move_immediate(abi::c_arg(2), "Integer", ENC_UTF8),
    ]);
    platform.emit_external_call(
        "CFStringCreateWithCString",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), UID_CFREF_OFF),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(dev_fail),
        // AudioQueueSetProperty(queue, 'aqcd', &cfref, 8)
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), &v10, S_OSOBJECT),
        abi::move_immediate(abi::c_arg(1), "Integer", "1634820964"),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), UID_CFREF_OFF),
        abi::move_immediate(abi::c_arg(3), "Integer", "8"),
    ]);
    platform.emit_external_call(
        "AudioQueueSetProperty",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CAP_OFF), // save status
        // CFRelease(cfref)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), UID_CFREF_OFF),
    ]);
    platform.emit_external_call(
        "CFRelease",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CAP_OFF),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(dev_fail),
    ]);
    Ok(())
}

/// Open-error cleanup (bug-180): before an open fails, dispose any AudioQueue
/// that was created and munmap the state page, so a device error does not leak
/// them. Safe to reach before either exists — `STATE_OFF` is zeroed at entry (so
/// the page-mapped test fails when mmap never ran) and mmap zero-fills
/// `S_OSOBJECT` (so the queue-created test fails before `AudioQueueNew*`).
pub(crate) fn emit_open_cleanup(ctx: &mut EmitCtx, vregs: &mut Vregs) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let v10 = vregs.next();
    let v9 = vregs.next();
    let munmap = format!("{symbol}_cleanup_munmap");
    let skip = format!("{symbol}_cleanup_skip");
    ctx.instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&skip),
        abi::load_u64(&v9, &v10, S_OSOBJECT),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&munmap),
        // AudioQueueDispose(queue, 1)
        abi::move_register(abi::return_register(), &v9),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    platform.emit_external_call(
        "AudioQueueDispose",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::label(&munmap),
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
    ctx.instructions.push(abi::label(&skip));
    Ok(())
}

/// Store a 12-byte `AudioObjectPropertyAddress { selector, scope, element=0 }`
/// into `sp + PROPADDR_OFF`.
pub(crate) fn build_propaddr(
    selector: &str,
    scope: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let v9 = vregs.next();
    instructions.extend([
        abi::move_immediate(&v9, "Integer", selector),
        abi::store_u32(&v9, abi::stack_pointer(), PROPADDR_OFF),
        abi::move_immediate(&v9, "Integer", scope),
        abi::store_u32(&v9, abi::stack_pointer(), PROPADDR_OFF + 4),
        abi::store_u32(abi::ZERO, abi::stack_pointer(), PROPADDR_OFF + 8),
    ]);
}

/// `AudioObjectGetPropertyData(object, &PROPADDR, 0, NULL, &SIZE, out_ptr)`.
/// `object` is loaded from `object_off` (a stack slot). `SIZE` is preloaded with
/// `size_val`. Leaves the `OSStatus` in the return register.
pub(crate) fn call_get_property(
    ctx: &mut EmitCtx,
    object_off: usize,
    size_val: &str,
    out_off: usize,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let v9 = vregs.next();
    ctx.instructions.extend([
        abi::move_immediate(&v9, "Integer", size_val),
        abi::store_u32(&v9, abi::stack_pointer(), SIZE_OFF),
        abi::load_u32(abi::return_register(), abi::stack_pointer(), object_off),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), PROPADDR_OFF),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
        abi::add_immediate(abi::c_arg(4), abi::stack_pointer(), SIZE_OFF),
        abi::add_immediate(abi::c_arg(5), abi::stack_pointer(), out_off),
    ]);
    platform.emit_external_call(
        "AudioObjectGetPropertyData",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    // OSStatus is a 32-bit SInt32 returned in w0; the upper half of x0 is
    // undefined, so extend before any full-width compare (bug-04).
    ctx.instructions.push(abi::sign_extend_word(
        abi::return_register(),
        abi::return_register(),
    ));
    Ok(())
}

/// Read the `CFStringRef` property `selector` of the device in `CURID_OFF`,
/// convert it to an MFBASIC `String` at `out_off`, and `CFRelease` it. Branches
/// to `dev_fail` on any Core Audio / CoreFoundation failure, `alloc_fail` on OOM.
pub(crate) fn emit_cfstring_field(
    ctx: &mut EmitCtx,
    selector: &str,
    out_off: usize,
    dev_fail: &str,
    alloc_fail: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v15 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    let field = out_off; // unique label suffix
    let copy_loop = format!("{symbol}_cf{field}_copy");
    let copy_done = format!("{symbol}_cf{field}_copy_done");
    let len_loop = format!("{symbol}_cf{field}_len");
    let len_done = format!("{symbol}_cf{field}_len_done");

    build_propaddr(selector, SCOPE_GLOBAL, ctx.instructions, vregs);
    call_get_property(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        CURID_OFF,
        "8",
        CFREF_OFF,
        vregs,
    )?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(dev_fail),
        // CFStringGetCString(cfref, CSTRBUF, 256, kCFStringEncodingUTF8)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CFREF_OFF),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), CSTRBUF_OFF),
        abi::move_immediate(abi::c_arg(2), "Integer", CSTRBUF_CAP),
        abi::move_immediate(abi::c_arg(3), "Integer", ENC_UTF8),
    ]);
    platform.emit_external_call(
        "CFStringGetCString",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        // Boolean is a 32-bit result in w0 (bug-04).
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), BOOLTMP_OFF),
        // CFRelease(cfref)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CFREF_OFF),
    ]);
    platform.emit_external_call(
        "CFRelease",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), BOOLTMP_OFF),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(dev_fail),
        // strlen(CSTRBUF)
        abi::add_immediate(&v9, abi::stack_pointer(), CSTRBUF_OFF),
        abi::move_immediate(&v10, "Integer", "0"),
        abi::label(&len_loop),
        abi::load_u8(&v11, &v9, 0),
        abi::compare_immediate(&v11, "0"),
        abi::branch_eq(&len_done),
        abi::add_immediate(&v9, &v9, 1),
        abi::add_immediate(&v10, &v10, 1),
        abi::branch(&len_loop),
        abi::label(&len_done),
        abi::store_u64(&v10, abi::stack_pointer(), SIZE_OFF),
        // Allocate String: [u64 len][bytes][nul].
        abi::add_immediate(abi::return_register(), &v10, 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, ctx.instructions, ctx.relocations, alloc_fail);
    ctx.instructions.extend([
        abi::move_register(&v15, abi::mfb_return(1)),
        abi::load_u64(&v10, abi::stack_pointer(), SIZE_OFF),
        abi::store_u64(&v10, &v15, 0),
        abi::store_u64(&v15, abi::stack_pointer(), out_off),
        abi::add_immediate(&v11, abi::stack_pointer(), CSTRBUF_OFF),
        abi::add_immediate(&v12, &v15, 8),
        abi::move_immediate(&v13, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&v13, &v10),
        abi::branch_eq(&copy_done),
        abi::load_u8(&v14, &v11, 0),
        abi::store_u8(&v14, &v12, 0),
        abi::add_immediate(&v11, &v11, 1),
        abi::add_immediate(&v12, &v12, 1),
        abi::add_immediate(&v13, &v13, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, &v12, 0),
    ]);
    Ok(())
}

/// Sum `mNumberChannels` across the device's stream configuration in `scope`,
/// storing `1` (any channel) or `0` into `out_off`. A failed query means the
/// direction is unsupported → `0`.
pub(crate) fn emit_channel_flag(
    ctx: &mut EmitCtx,
    scope: &str,
    out_off: usize,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    let unsupported = format!("{symbol}_ch{out_off}_none");
    let sum_loop = format!("{symbol}_ch{out_off}_loop");
    let sum_done = format!("{symbol}_ch{out_off}_done");
    let set_flag = format!("{symbol}_ch{out_off}_flag");

    ctx.instructions
        .push(abi::store_u64(abi::ZERO, abi::stack_pointer(), out_off));
    build_propaddr(SEL_STREAMCFG, scope, ctx.instructions, vregs);
    call_get_property(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        CURID_OFF,
        BUFLIST_CAP,
        BUFLIST_OFF,
        vregs,
    )?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&unsupported),
        // mNumberBuffers @ BUFLIST[0]; buffers start at BUFLIST+8, stride 16,
        // mNumberChannels at +0.
        abi::add_immediate(&v9, abi::stack_pointer(), BUFLIST_OFF),
        abi::load_u32(&v10, &v9, 0),               // nbuf
        abi::add_immediate(&v11, &v9, 8),          // buffer cursor
        abi::move_immediate(&v12, "Integer", "0"), // i
        abi::move_immediate(&v13, "Integer", "0"), // sum
        abi::label(&sum_loop),
        abi::compare_registers(&v12, &v10),
        abi::branch_eq(&sum_done),
        abi::load_u32(&v14, &v11, 0),
        abi::add_registers(&v13, &v13, &v14),
        abi::add_immediate(&v11, &v11, 16),
        abi::add_immediate(&v12, &v12, 1),
        abi::branch(&sum_loop),
        abi::label(&sum_done),
        abi::compare_immediate(&v13, "0"),
        abi::branch_ne(&set_flag),
        abi::branch(&unsupported),
        abi::label(&set_flag),
        abi::move_immediate(&v13, "Integer", "1"),
        abi::store_u64(&v13, abi::stack_pointer(), out_off),
        abi::label(&unsupported),
    ]);
    Ok(())
}

// Input/read frame offsets (read never builds the ASBD/UID buffers, so it
// reuses that region for the timespec/clock scratch).
pub(crate) const TS_OFF: usize = 128; // relative timespec (16 B)

pub(crate) const CLK_OFF: usize = 144; // clock_gettime result (16 B)

pub(crate) const FINAL_LIST_OFF: usize = 160; // right-sized result for a partial timed read

pub(crate) const DEADLINE_OFF: usize = 56; // absolute deadline (ns)

pub(crate) const PAYLOAD_OFF: usize = 64; // byte-list payload cursor

pub(crate) const NEED_OFF: usize = 72; // requested bytes

pub(crate) const LIST_PTR_OFF: usize = 80; // result list ptr

pub(crate) const BYTES_GOT_OFF: usize = 88; // bytes accumulated so far

pub(crate) const FRAMES_OFF: usize = 96; // requested frames

pub(crate) const TIMEOUT_OFF: usize = 104; // timeoutMs

// openInput-only scratch (device path uses UID buffers; default path uses these).
pub(crate) const PRECHK_ADDR: usize = 440;

pub(crate) const PRECHK_SIZE: usize = 456;

pub(crate) const PRECHK_ID: usize = 464;

pub(crate) const RINGCAP_OFF: usize = 472;

pub(crate) const MAPSIZE_OFF: usize = 480;

// plan-73-B: the convention clamps a too-large `timeoutMs` to INT_MAX (the
// deadline math takes a C `int`) rather than raising the old 24h cap.
pub(crate) const TIMEOUT_CLAMP_MS: &str = "2147483647";

/// Store `1` into `out_off` of the record `%v12` (record ptr) when the u64 at
/// `a_off` equals the u64 at `b_off`, else `0`. Uses the record ptr already in
/// `%v12`.
pub(crate) fn emit_id_matches(
    a_off: usize,
    b_off: usize,
    field: usize,
    symbol: &str,
    tag: &str,
    record: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let v13 = vregs.next();
    let v14 = vregs.next();
    let v16 = vregs.next();
    let store = format!("{symbol}_defstore_{tag}");
    instructions.extend([
        abi::load_u64(&v13, abi::stack_pointer(), a_off),
        abi::load_u64(&v14, abi::stack_pointer(), b_off),
        abi::move_immediate(&v16, "Integer", "0"),
        abi::compare_registers(&v13, &v14),
        abi::branch_ne(&store),
        abi::move_immediate(&v16, "Integer", "1"),
        abi::label(&store),
        abi::store_u64(&v16, record, field),
    ]);
}
