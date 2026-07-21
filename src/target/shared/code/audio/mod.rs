//! Native code generation for the built-in `audio` package (raw interleaved
//! `s16le` PCM). The macOS backend (`macos` submodule) drives Core Audio's
//! `AudioQueue`; the Linux backend (`alsa` submodule) drives ALSA's blocking PCM
//! API through a `dlopen`'d `libasound.so.2`. Neither uses a lock-free ring —
//! this compiler emits no atomics, so all cross-thread sync is pthread
//! mutex/cond (plan-33-A §6).
//!
//! `AudioDevice` is a plain read-only record (pointer-`String` layout, like
//! `net::Address`): six 8-byte field slots.

use std::collections::HashMap;

use super::*;

// --- AudioHandle: arena record, pointer-sized reference (plan-33-A §5.1) ------
// Identical layout for both resource types.
pub(super) const H_KIND: usize = 0; // 1 = input, 2 = output
pub(super) const H_CLOSED: usize = 8; // mirror; authoritative `closed` is in state
pub(super) const H_SAMPLE_RATE: usize = 16;
pub(super) const H_CHANNELS: usize = 24;
pub(super) const H_BYTES_PER_FRAME: usize = 32; // channels * 2
pub(super) const H_BUFFER_FRAMES: usize = 40;
pub(super) const H_STATE: usize = 48; // -> mmap'd AudioState
pub(super) const H_RECORD_SIZE: usize = 64;

// The offset-8 `closed` mirror is the canonical resource closed-flag offset
// (plan-38): the closed-default (`lower_default_value`) sets exactly this byte,
// and the whole handle record fits inside the shared closed-default record so
// the zeroed default covers it. `S_CLOSED` (in the mmap'd state) is the
// authoritative flag; the guards read this arena-resident mirror, so offset 8 is
// what the default needs.
const _: () = assert!(H_CLOSED == RESOURCE_OFFSET_CLOSED);
const _: () = assert!(H_RECORD_SIZE <= RESOURCE_RECORD_SIZE_BYTES);

pub(super) const KIND_INPUT: &str = "1";
pub(super) const KIND_OUTPUT: &str = "2";
pub(super) const NUM_BUFFERS: usize = 4;

// --- AudioState: one mmap'd page, NOT arena (an OS callback thread touches it) -
// pthread_mutex_t (64 B) / pthread_cond_t (48 B) get 128 B each (§5.1). Compile-
// time asserts below guard the reservations against the platform sizes.
pub(super) const S_MUTEX: usize = 0;
pub(super) const S_COND: usize = 128;
pub(super) const S_XRUNS: usize = 256;
pub(super) const S_CLOSED: usize = 264;
pub(super) const S_STARTED: usize = 272;
pub(super) const S_OSOBJECT: usize = 280; // AudioQueueRef (macOS) / snd_pcm_t* (Linux)
pub(super) const S_FREE_TOP: usize = 288; // count of free output buffers
pub(super) const S_FREE_BUFS: usize = 296; // [NUM_BUFFERS] AudioQueueBufferRef -> 296..328
pub(super) const S_RING_CAP: usize = 328;
pub(super) const S_RING_HEAD: usize = 336; // wrapped write index [0, ringCap)
pub(super) const S_RING_TAIL: usize = 344; // wrapped read index [0, ringCap)
pub(super) const S_MAP_SIZE: usize = 352; // total mmap length, for munmap
pub(super) const S_RING_FILL: usize = 360; // bytes currently buffered
pub(super) const S_RING: usize = 384; // input ring payload (page-area)

// `AudioState` bookkeeping fits in the first page; output uses no ring so one
// page suffices. Input sizes the mapping to `S_RING + ringCapacity`.
pub(super) const STATE_PAGE: usize = 16384;

// Build-time guards (plan-33-B §6): the pthread reservations must exceed the
// platform sizes (macOS pthread_mutex_t = 64 B, pthread_cond_t = 48 B; glibc 40 /
// 48). Both backends `pthread_*_init` these regions, so an undersized reservation
// would corrupt the following fields.
const _: () = assert!(S_COND - S_MUTEX >= 64, "mutex reservation too small");
const _: () = assert!(S_XRUNS - S_COND >= 48, "cond reservation too small");
const _: () = assert!(S_RING <= STATE_PAGE, "state bookkeeping exceeds one page");

// The `AudioDevice` record: six word-slots, `String` fields as pointers.
pub(super) const DEVICE_FIELD_ID: usize = 0;
pub(super) const DEVICE_FIELD_NAME: usize = 8;
pub(super) const DEVICE_FIELD_CAN_INPUT: usize = 16;
pub(super) const DEVICE_FIELD_CAN_OUTPUT: usize = 24;
pub(super) const DEVICE_FIELD_IS_DEFAULT_INPUT: usize = 32;
pub(super) const DEVICE_FIELD_IS_DEFAULT_OUTPUT: usize = 40;
pub(super) const DEVICE_RECORD_SIZE: usize = 48;

// Shared emit helpers: `emit_alloc` is the one arena-allocation free function
// (`code/mod.rs`, bug-322); the rest still live in `tls`. Reuse them rather than
// duplicating. `emit_data_address` is re-exported for the AudioQueue phases.
pub(super) use super::emit_alloc;
pub(super) use super::tls::{emit_arena_free, emit_data_address, emit_fail};

// The emitted AudioQueue output callback (macOS): a C-ABI function the OS calls
// on an ordinary internal thread when a played buffer is free. openOutput takes
// its address; mod.rs registers the body when an output program is built.
pub(in crate::target::shared::code) const AUDIO_OUTPUT_CALLBACK_SYMBOL: &str =
    "_mfb_rt_audio_output_callback";
pub(in crate::target::shared::code) const AUDIO_INPUT_CALLBACK_SYMBOL: &str =
    "_mfb_rt_audio_input_callback";

mod alsa;
mod macos;

pub(in crate::target::shared::code) use macos::{
    lower_audio_input_callback, lower_audio_output_callback,
};

/// Dispatch an `audio.*` runtime-helper body to the platform backend.
pub(in crate::target::shared::code) fn lower_audio_helper(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    if platform.target().contains("macos") {
        return macos::lower_audio_macos(call, symbol, platform_imports, platform);
    }
    alsa::lower_audio_alsa(call, symbol, platform_imports, platform)
}

/// C-string data objects (the `libasound.so.2` soname + ALSA symbol names) the
/// Linux backend references for its `dlopen`/`dlsym`.
pub(in crate::target::shared::code) fn alsa_data_objects() -> Vec<CodeDataObject> {
    alsa::data_objects()
}

/// Allocate a `List OF Byte` of `count_off` elements: size the block, write the
/// header, and fill the lookup table with the identity mapping
/// (`valueOffset = i`, `valueLength = 1`). The payload bytes are left
/// uninitialized for the caller to fill.
///
/// One copy, shared by both audio backends (plan-57-B). It existed verbatim in
/// `alsa.rs` and `macos.rs` — the two differed only in label names and
/// comments. A third near-variant that also copies from a source buffer lives at
/// `crypto_ec::emit_build_byte_list`.
///
/// Sharing it is what makes plan-57-D a small edit rather than a sweep: this is
/// one of the places that must stop writing a lookup table once a fixed-width
/// list no longer has one.
///
/// A free function rather than a `CodeBuilder` method because both callers are
/// standalone `CodeFunction` emitters with no builder in scope (plan-57-A §Open
/// Decisions).
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
        abi::move_immediate("%v11", "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers("%v12", "%v10", "%v11"),
        abi::add_immediate("%v12", "%v12", COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), "%v12", "%v10"), // + count payload bytes
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::move_register("%v15", abi::RET[1]),
        abi::store_u64("%v15", abi::stack_pointer(), list_off),
        abi::move_immediate("%v9", "Byte", &byte_list_block_kind().to_string()),
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
    ]);
    // kind 2 has no entry array to fill (plan-57-D), so the ENTIRE loop is
    // skipped — not just its body.
    //
    // plan-57-D guarded only the body, which left `label; cmp i,count;
    // bge done; i++; b loop` behind: a no-op loop that still ran `count` times at
    // RUNTIME. Every audio capture allocation paid it, and it scales with the
    // buffer — a 3-minute stereo 48 kHz read burned ~34 million iterations doing
    // nothing. Correct output, silently linear waste, which is why nothing caught
    // it. The header stores above already set count/capacity/dataLength/
    // dataCapacity, so under kind 2 there is nothing left for the loop to do.
    if byte_list_entry_stride() != 0 {
        instructions.extend([
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
            abi::add_immediate("%v11", "%v11", byte_list_entry_stride()),
            abi::add_immediate("%v13", "%v13", 1),
            abi::branch(&entry_loop),
            abi::label(&entry_done),
        ]);
    }
}
