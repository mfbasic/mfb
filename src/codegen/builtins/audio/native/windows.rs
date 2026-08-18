//! Windows WASAPI backend for the `audio` package (plan-66 G+H).
//!
//! WASAPI is driven over COM through the process IAT (ole32.dll for the COM
//! runtime, mmdevapi's objects created via `CoCreateInstance`, kernel32 for the
//! event handle). There is NO `dlopen`/`dlsym` bridge and — unlike CoreAudio —
//! NO OS callback thread: WASAPI's event-driven model signals an auto-reset event
//! each buffer period, and `audio::write`/`read` wait on it directly on the
//! calling thread (plan-33-A §6: this compiler emits no atomics, and the
//! event-driven model needs no cross-thread ring/mutex/cond at all).
//!
//! A COM interface pointer is a pointer to a vtable pointer: `obj -> vtable`,
//! `vtable[slot] -> method`. A method call puts `obj` (the `this` pointer) in
//! ARG[0] and the declared arguments in ARG[1..], loads the method function
//! pointer from `[[obj]+slot*8]`, and `blr`s it (the x86 encoder emits
//! `call r/m64`). Slot numbering starts at IUnknown's QueryInterface/AddRef/
//! Release (0/1/2), then the interface's own methods in declaration order.
//!
//! The 64-byte `AudioHandle` record (shared `H_*` layout) points at an arena
//! WASAPI STATE block (`W_*`) holding the four COM interface pointers, the event
//! handle, the negotiated buffer frame count, and the COM out-pointer scratch the
//! method calls write their results into (an absolute arena address is a stable,
//! DEPTH-0-safe out-param target). Open Decision 1 (EXCLUSIVE, no resampling) is
//! attempted first; if the device refuses that format (any FAILED `Initialize`
//! HRESULT) `lower_open` releases and re-activates the client and falls back to
//! SHARED at the device's own MIX FORMAT (`GetMixFormat`), converting each sample
//! between `s16le` and the mix's 32-bit float by hand — AUTOCONVERTPCM is NOT used
//! (it fast-fails on the test box). The fallback is a last resort, recorded in
//! `W_SHARED`; there is no buffer-alignment retry.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::error::emission::*;
use crate::codegen::memory::arena::*;
use crate::codegen::os::ffi::*;
use std::collections::HashMap;

use super::*;
use crate::target::shared::abi;
// --- COM / WASAPI constants -------------------------------------------------
const CLSCTX_ALL: &str = "23"; // INPROC_SERVER|INPROC_HANDLER|LOCAL_SERVER|REMOTE_SERVER
const COINIT_MULTITHREADED: &str = "0";
const E_RENDER: &str = "0"; // EDataFlow::eRender
const E_CAPTURE: &str = "1"; // EDataFlow::eCapture
const E_ALL: &str = "2"; // EDataFlow::eAll
const E_CONSOLE: &str = "0"; // ERole::eConsole
const DEVICE_STATE_ACTIVE: &str = "1";
const STGM_READ: &str = "0";
const SHAREMODE_SHARED: &str = "0";
const SHAREMODE_EXCLUSIVE: &str = "1";
const STREAMFLAGS_EVENTCALLBACK: &str = "262144"; // 0x00040000
const REFTIMES_PER_SEC: &str = "10000000";
const WAVE_FORMAT_PCM: usize = 1;
const BITS_PER_SAMPLE: usize = 16;

// COM vtable slots (IUnknown occupies 0/1/2).
const SLOT_RELEASE: usize = 2;
// IMMDeviceEnumerator
const SLOT_ENUM_ENDPOINTS: usize = 3; // EnumAudioEndpoints
const SLOT_GET_DEFAULT_ENDPOINT: usize = 4; // GetDefaultAudioEndpoint
                                            // IMMDeviceCollection
const SLOT_COLL_GET_COUNT: usize = 3;
const SLOT_COLL_ITEM: usize = 4;
// IMMDevice
const SLOT_DEV_ACTIVATE: usize = 3;
const SLOT_DEV_OPEN_PROPSTORE: usize = 4;
const SLOT_DEV_GET_ID: usize = 5;
// IAudioClient
const SLOT_AC_INITIALIZE: usize = 3;
const SLOT_AC_GET_BUFFER_SIZE: usize = 4;
const SLOT_AC_GET_CURRENT_PADDING: usize = 6;
const SLOT_AC_GET_MIX_FORMAT: usize = 8;
const SLOT_AC_START: usize = 10;
const SLOT_AC_STOP: usize = 11;
const SLOT_AC_SET_EVENT_HANDLE: usize = 13;
const SLOT_AC_GET_SERVICE: usize = 14;
// IAudioRenderClient
const SLOT_RENDER_GET_BUFFER: usize = 3;
const SLOT_RENDER_RELEASE_BUFFER: usize = 4;
// IAudioCaptureClient
const SLOT_CAPTURE_GET_BUFFER: usize = 3;
const SLOT_CAPTURE_RELEASE_BUFFER: usize = 4;
// IPropertyStore
const SLOT_PROPS_GET_VALUE: usize = 5;

// --- arena WASAPI STATE block ----------------------------------------------
// Arena, not mmap: no OS callback thread touches it (event-driven WASAPI runs on
// the calling thread), so no ring/mutex/cond is needed.
const W_ENUM: usize = 0; // IMMDeviceEnumerator*
const W_DEVICE: usize = 8; // IMMDevice*
const W_CLIENT: usize = 16; // IAudioClient*
const W_SERVICE: usize = 24; // IAudioRenderClient* / IAudioCaptureClient*
const W_EVENT: usize = 32; // auto-reset HANDLE
const W_XRUNS: usize = 40;
const W_STARTED: usize = 48;
const W_BUFFER: usize = 56; // negotiated buffer frame count (u32)
const W_SHARED: usize = 64; // 1 = SHARED fallback, 0 = EXCLUSIVE
const W_OUT0: usize = 72; // COM out-ptr scratch (pointer results / counts)
const W_OUT1: usize = 80; // COM out-ptr scratch (data pointer)
const W_OUT2: usize = 88; // COM out-ptr scratch (flags)
const W_WFX: usize = 120; // WAVEFORMATEX (18 bytes, +pad) -> 120..138
const W_MIX_CH: usize = 144; // SHARED-mix: device mix channel count
const W_MIX_BPF: usize = 152; // SHARED-mix: device mix bytes-per-frame (frame stride)
                              // bug-416 (2): capture carry-over. WASAPI requires a whole IAudioCaptureClient
                              // packet be released (`ReleaseBuffer(numFrames)` == the `GetBuffer` count or 0) —
                              // a partial consume is illegal. When a `read` whose length isn't packet-aligned
                              // consumes only part of the final packet, the unconsumed tail is stashed here (in
                              // the DEVICE mix format, `W_MIX_BPF` stride) and drained by the next `read` so no
                              // captured frame is dropped. `W_CARRY_PTR` sizes to `W_BUFFER * W_MIX_BPF` bytes
                              // (input streams only); `W_CARRY_HEAD` is the frame cursor into the stash.
const W_CARRY_PTR: usize = 160; // arena carry buffer (input only), or null
const W_CARRY_FRAMES: usize = 168; // total frames stashed
const W_CARRY_HEAD: usize = 176; // frames already drained (cursor)
const W_SIZE: usize = 184;

const SLOT_ENUM_GET_DEVICE: usize = 5; // IMMDeviceEnumerator::GetDevice

// --- shared stack frame -----------------------------------------------------
const FRAME: usize = 832;
const WIDEID_OFF: usize = 288; // openOutputDevice: UTF-16 endpoint id (512 bytes)
const HANDLE_OFF: usize = 8;
const STATE_OFF: usize = 16;
const OBJ_OFF: usize = 24; // spilled COM object for a vtable call
const SR_OFF: usize = 32;
const CH_OFF: usize = 40;
const BF_OFF: usize = 48;
const BPF_OFF: usize = 56;
const SRC_OFF: usize = 64; // byte payload base (write src / read dst)
const TOTAL_OFF: usize = 72; // total frames (write)
const OFFSET_OFF: usize = 80; // frames done (write) / index (devices)
const HR_OFF: usize = 88; // last HRESULT
const NEED_OFF: usize = 96; // read: result byte count
const LIST_OFF: usize = 104; // read/devices: result list
const FRAMES_OFF: usize = 112; // read: frames requested
const FRAMES_GOT_OFF: usize = 120; // read: frames gathered
const TIMEOUT_OFF: usize = 128; // read/query: timeoutMs
const DEVID_OFF: usize = 136; // devices: id string
const NAME_OFF: usize = 144; // devices: name string
const COUNT_OFF: usize = 152; // devices: endpoint count
const COLL_SRC_OFF: usize = 160; // devices: record data-region base
const COLL_ENTRY_OFF: usize = 168; // devices: entry-array base
const DEADLINE_OFF: usize = 176; // read timeout: absolute deadline (ms tick)
const FINAL_LIST_OFF: usize = 184; // read timeout: right-sized result
const GOTBYTES_OFF: usize = 192;
const CSTR_OFF: usize = 200; // wstr->string scratch (source pointer save)

// plan-73-B: the convention clamps a too-large `timeoutMs` to INT_MAX (the
// deadline math takes a C `int`) rather than raising the old 24h cap.
const TIMEOUT_CLAMP_MS: &str = "2147483647";

// --- GUID / CLSID / IID data objects (Windows GUID byte order) --------------
fn guid_symbol(name: &str) -> String {
    format!("_mfb_audio_w_{name}")
}

fn guid_object(name: &str, size: usize, bytes: &str) -> CodeDataObject {
    CodeDataObject {
        symbol: guid_symbol(name),
        kind: "raw".to_string(),
        layout: "GUID (Windows byte order)".to_string(),
        align: 4,
        size,
        value: bytes.to_string(),
    }
}

/// The read-only GUIDs the COM calls reference. First three fields little-endian,
/// last eight bytes big-endian — Windows GUID byte order.
pub(crate) fn data_objects() -> Vec<CodeDataObject> {
    vec![
        // CLSID_MMDeviceEnumerator {BCDE0395-E52F-467C-8E3D-C4579291692E}
        guid_object(
            "CLSID_MMDeviceEnumerator",
            16,
            "9503debc2fe57c468e3dc4579291692e",
        ),
        // IID_IMMDeviceEnumerator {A95664D2-9614-4F35-A746-DE8DB63617E6}
        guid_object(
            "IID_IMMDeviceEnumerator",
            16,
            "d26456a91496354fa746de8db63617e6",
        ),
        // IID_IAudioClient {1CB9AD4C-DBFA-4C32-B178-C2F568A703B2}
        guid_object("IID_IAudioClient", 16, "4cadb91cfadb324cb178c2f568a703b2"),
        // IID_IAudioRenderClient {F294ACFC-3146-4483-A7BF-ADDCA7C260E2}
        guid_object(
            "IID_IAudioRenderClient",
            16,
            "fcac94f246318344a7bfaddca7c260e2",
        ),
        // IID_IAudioCaptureClient {C8ADBD64-E71E-48A0-A4DE-185C395CD317}
        guid_object(
            "IID_IAudioCaptureClient",
            16,
            "64bdadc81ee7a048a4de185c395cd317",
        ),
        // PKEY_Device_FriendlyName: fmtid {A45C254E-DF1C-4EFD-8020-67D146A850E0}, pid 14
        guid_object(
            "PKEY_Device_FriendlyName",
            20,
            "4e255ca41cdffd4e802067d146a850e00e000000",
        ),
    ]
}

fn guid_addr(
    from: &str,
    dst: impl Into<Operand>,
    name: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    emit_data_address(from, dst, &guid_symbol(name), ins, rel);
}

// --- call helpers -----------------------------------------------------------

/// Emit a plain external (IAT) call whose arguments beyond the four Win64
/// register slots spill to the outgoing stack tail. Arg 0 staged in
/// `return_register()`, args 1.. in `ARG[1..]`. Sign-extends the HRESULT/DWORD
/// return so a `< 0` FAILED(hr) check is correct.
fn ole_call(
    from: &str,
    symbol: &str,
    n_args: usize,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    emit_external_int_call(platform, symbol, from, n_args, imports, ins, rel)?;
    ins.push(abi::sign_extend_word(
        abi::return_register(),
        abi::return_register(),
    ));
    Ok(())
}

/// Emit a COM vtable method call. Precondition: the `this` pointer is spilled at
/// `OBJ_OFF`, register method args are staged in `ARG[1..3]`, and any args beyond
/// the fourth are staged in `ARG[4..]` (this helper spills those to the outgoing
/// tail). Loads `this` into arg0, resolves `method = [[this]+slot*8]`, and
/// `blr`s it. Sign-extends the HRESULT return.
fn com_call(slot: usize, n_args: usize, ins: &mut Vec<CodeInstruction>, vregs: &mut Vregs) {
    // Args 5.. (index 4..) go on the stack above the 32-byte shadow. Four
    // register args on Win64: `this` + three method args.
    for n in 4..n_args {
        ins.push(abi::outgoing_stack_arg_store(abi::c_arg(n), n - 4));
    }
    let v8 = vregs.next();
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), OBJ_OFF), // this -> arg0
        abi::load_u64(&v8, abi::stack_pointer(), OBJ_OFF),
        abi::load_u64(&v8, &v8, 0),        // vtable
        abi::load_u64(&v8, &v8, slot * 8), // method
        abi::branch_link_register(&v8),
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
    ]);
}

/// Load `state->field` into the `OBJ_OFF` spill slot for the next `com_call`.
fn spill_obj(field: usize, ins: &mut Vec<CodeInstruction>, vregs: &mut Vregs) {
    let v9 = vregs.next();
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v9, &v9, field),
        abi::store_u64(&v9, abi::stack_pointer(), OBJ_OFF),
    ]);
}

pub(crate) fn lower_audio_windows(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
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
            "native code plan does not emit runtime call '{other}' for windows (wasapi)"
        )),
    }
}

include!("windows_open.rs");
include!("windows_io.rs");
include!("windows_devices.rs");

#[cfg(test)]
#[path = "windows_tests.rs"]
mod windows_tests;
