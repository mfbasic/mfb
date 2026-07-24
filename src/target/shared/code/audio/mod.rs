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
// Moved to `builder_collection_layout` (plan-58-B) so `link_thunk`'s
// `OUT CBuffer` staging can reach it without depending on `audio`. Re-imported
// here so both backends keep naming it unqualified.
use super::builder_collection_layout::emit_alloc_byte_list;

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

// Output only: the buffer `write` is still filling, and how many bytes are in
// it. An AudioQueue never finishes a buffer holding less than a full period, so
// a partly-filled buffer must not be enqueued (bug-370) — it is carried here
// until a later `write` fills it or `close` pads it with silence. Only the
// writing thread touches these, so they need no mutex.
pub(super) const S_PENDING_BUF: usize = 368;
pub(super) const S_PENDING_FILL: usize = 376;
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
const _: () = assert!(
    S_PENDING_FILL < S_RING,
    "pending-buffer slots overlap the ring"
);

// The `AudioDevice` record: six word-slots, `String` fields as pointers.
pub(super) const DEVICE_FIELD_ID: usize = 0;
pub(super) const DEVICE_FIELD_NAME: usize = 8;
pub(super) const DEVICE_FIELD_CAN_INPUT: usize = 16;
pub(super) const DEVICE_FIELD_CAN_OUTPUT: usize = 24;
pub(super) const DEVICE_FIELD_IS_DEFAULT_INPUT: usize = 32;
pub(super) const DEVICE_FIELD_IS_DEFAULT_OUTPUT: usize = 40;
pub(super) const DEVICE_RECORD_SIZE: usize = 48;

// Shared generic emitters, all from `native_helpers` (bug-330): `emit_alloc`
// is the one arena-allocation free function (`code/mod.rs`, bug-322); the rest
// are the package-neutral emitters that used to live in `tls`. Reuse them
// rather than duplicating. `emit_data_address` is re-exported for the
// AudioQueue phases.
pub(super) use super::emit_alloc;
pub(super) use super::native_helpers::{
    emit_arena_free, emit_data_address, emit_fail, hex_encode_cstring,
};

// The emitted AudioQueue output callback (macOS): a C-ABI function the OS calls
// on an ordinary internal thread when a played buffer is free. openOutput takes
// its address; mod.rs registers the body when an output program is built.
pub(in crate::target::shared::code) const AUDIO_OUTPUT_CALLBACK_SYMBOL: &str =
    "_mfb_rt_audio_output_callback";
pub(in crate::target::shared::code) const AUDIO_INPUT_CALLBACK_SYMBOL: &str =
    "_mfb_rt_audio_input_callback";

mod alsa;
mod common;
mod macos;

// Scaffolding both backends share (bug-330); imported here so each backend's
// `use super::*` picks them up.
use common::{emit_validate_open, Query, READ_FRAMES_MAX};

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
    match platform.family() {
        PlatformFamily::MacOS => macos::lower_audio_macos(call, symbol, platform_imports, platform),
        PlatformFamily::Linux => alsa::lower_audio_alsa(call, symbol, platform_imports, platform),
        // No Windows audio backend is in plan-47's scope. Falling through to ALSA
        // would bake libasound sonames into a Windows binary (§3.2), so reject
        // loudly until a Windows audio sub-plan owns this.
        PlatformFamily::Windows => unreachable!("no Windows audio backend (plan-47 non-goal)"),
    }
}

/// C-string data objects (the `libasound.so.2` soname + ALSA symbol names) the
/// Linux backend references for its `dlopen`/`dlsym`.
fn alsa_data_objects() -> Vec<CodeDataObject> {
    alsa::data_objects()
}

/// The selected audio backend for a build. Owns the two whole-plan decisions
/// that `code/mod.rs` used to re-derive from `platform.target()` plus
/// hand-maintained literal symbol lists (bug-330 cause #3): which read-only data
/// objects the audio backend needs, and which AudioQueue callbacks to emit.
pub(in crate::target::shared::code) enum AudioBackend {
    CoreAudio,
    Alsa,
    /// Platforms with no audio surface (console Windows — plan-47 non-goal).
    /// `AudioBackend::select` is called for EVERY build to compute the audio data
    /// objects, not only audio-using ones, so this must be a real "no audio"
    /// answer rather than `unreachable!`: it emits no data objects and no
    /// callbacks. An audio-using Windows program is rejected earlier, at the
    /// `runtime_calls` capability gate (audio.* is not advertised).
    NoAudio,
}

impl AudioBackend {
    /// Select the backend for `platform`. The single place the audio macOS/Linux
    /// decision is made.
    pub(in crate::target::shared::code) fn select(platform: &dyn CodegenPlatform) -> Self {
        match platform.family() {
            PlatformFamily::MacOS => AudioBackend::CoreAudio,
            PlatformFamily::Linux => AudioBackend::Alsa,
            PlatformFamily::Windows => AudioBackend::NoAudio,
        }
    }

    /// Read-only data objects the backend references, given the plan's runtime
    /// symbols. CoreAudio links AudioToolbox directly and needs none; ALSA emits
    /// its `dlopen`/`dlsym` C strings only when the plan uses an audio helper.
    pub(in crate::target::shared::code) fn data_objects(
        &self,
        runtime_symbols: &[String],
    ) -> Vec<CodeDataObject> {
        match self {
            AudioBackend::CoreAudio | AudioBackend::NoAudio => Vec::new(),
            AudioBackend::Alsa => {
                if runtime_symbols
                    .iter()
                    .any(|symbol| symbol.starts_with("_mfb_rt_audio_"))
                {
                    alsa_data_objects()
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// The AudioQueue callback functions to emit (macOS only): the output
    /// callback when the plan builds an output stream, the input callback when it
    /// builds an input stream. `openOutput`/`openInput` take these addresses.
    pub(in crate::target::shared::code) fn callback_functions(
        &self,
        platform_imports: &HashMap<String, String>,
        platform: &dyn CodegenPlatform,
        runtime_symbols: &[String],
    ) -> Result<Vec<CodeFunction>, String> {
        let mut functions = Vec::new();
        if let AudioBackend::CoreAudio = self {
            let uses = |list: &[&str]| {
                runtime_symbols
                    .iter()
                    .any(|symbol| list.contains(&symbol.as_str()))
            };
            if uses(macos::OUTPUT_CALLBACK_SYMBOLS) {
                functions.push(lower_audio_output_callback(platform_imports, platform)?);
            }
            if uses(macos::INPUT_CALLBACK_SYMBOLS) {
                functions.push(lower_audio_input_callback(platform_imports, platform)?);
            }
        }
        Ok(functions)
    }
}
