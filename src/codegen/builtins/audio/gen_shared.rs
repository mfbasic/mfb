//! Native code generation for the built-in `audio` package (raw interleaved
//! `s16le` PCM). The macOS backend (`macos` submodule) drives Core Audio's
//! `AudioQueue`; the Linux backend (`alsa` submodule) drives ALSA's blocking PCM
//! API through a `dlopen`'d `libasound.so.2`. Neither uses a lock-free ring —
//! this compiler emits no atomics, so all cross-thread sync is pthread
//! mutex/cond (plan-33-A §6).
//!
//! `AudioDevice` is a plain read-only record (pointer-`String` layout, like
//! `net::Address`): six 8-byte field slots.

// Moved to `builder_collection_layout` (plan-58-B) so `link_thunk`'s
// `OUT CBuffer` staging can reach it without depending on `audio`. Re-imported
// here so both backends keep naming it unqualified.
// --- codegen tier imports (migration) ---
use super::gen_common::Query;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::types::ParameterType;
use std::collections::HashMap;

// --- AudioHandle: arena record, pointer-sized reference (plan-33-A §5.1) ------
// Identical layout for both resource types. Shares the canonical plan-80
// header: tag@0, handle (`H_KIND`)@8, closed@16, generic union STATE@24 — then
// the audio-specific tail (sample-rate/…/mmap-`H_STATE` ptr) at 32+.
pub(crate) const H_KIND: usize = RESOURCE_OFFSET_HANDLE; // 1 = input, 2 = output
pub(crate) const H_CLOSED: usize = RESOURCE_OFFSET_CLOSED; // mirror; authoritative `closed` is in state
pub(crate) const H_SAMPLE_RATE: usize = 32;
pub(crate) const H_CHANNELS: usize = 40;
pub(crate) const H_BYTES_PER_FRAME: usize = 48; // channels * 2
pub(crate) const H_BUFFER_FRAMES: usize = 56;
pub(crate) const H_STATE: usize = 64; // -> mmap'd AudioState
pub(crate) const H_RECORD_SIZE: usize = RESOURCE_RECORD_SIZE_BYTES;

// The `closed` mirror is at the canonical resource closed-flag offset
// (plan-38/80): the closed-default (`lower_default_value`) sets exactly this
// byte, and the whole handle record fits inside the shared closed-default
// record so the zeroed default covers it. `S_CLOSED` (in the mmap'd state) is
// the authoritative flag; the guards read this arena-resident mirror.
const _: () = assert!(H_KIND == RESOURCE_OFFSET_HANDLE);
const _: () = assert!(H_CLOSED == RESOURCE_OFFSET_CLOSED);
const _: () = assert!(H_RECORD_SIZE <= RESOURCE_RECORD_SIZE_BYTES);
// The audio-specific tail (mmap `H_STATE` ptr) fits inside the shared envelope,
// clear of the generic union STATE slot at `RESOURCE_OFFSET_STATE`.
const _: () = assert!(H_STATE + 8 <= RESOURCE_RECORD_SIZE_BYTES);
const _: () = assert!(H_SAMPLE_RATE > RESOURCE_OFFSET_STATE);

pub(crate) const KIND_INPUT: &str = "1";
pub(crate) const KIND_OUTPUT: &str = "2";
pub(crate) const NUM_BUFFERS: usize = 4;

// --- AudioState: one mmap'd page, NOT arena (an OS callback thread touches it) -
// pthread_mutex_t (64 B) / pthread_cond_t (48 B) get 128 B each (§5.1). Compile-
// time asserts below guard the reservations against the platform sizes.
pub(crate) const S_MUTEX: usize = 0;
pub(crate) const S_COND: usize = 128;
pub(crate) const S_XRUNS: usize = 256;
pub(crate) const S_CLOSED: usize = 264;
pub(crate) const S_STARTED: usize = 272;
pub(crate) const S_OSOBJECT: usize = 280; // AudioQueueRef (macOS) / snd_pcm_t* (Linux)
pub(crate) const S_FREE_TOP: usize = 288; // count of free output buffers
pub(crate) const S_FREE_BUFS: usize = 296; // [NUM_BUFFERS] AudioQueueBufferRef -> 296..328
pub(crate) const S_RING_CAP: usize = 328;
pub(crate) const S_RING_HEAD: usize = 336; // wrapped write index [0, ringCap)
pub(crate) const S_RING_TAIL: usize = 344; // wrapped read index [0, ringCap)
pub(crate) const S_MAP_SIZE: usize = 352; // total mmap length, for munmap
pub(crate) const S_RING_FILL: usize = 360; // bytes currently buffered

// Output only: the buffer `write` is still filling, and how many bytes are in
// it. An AudioQueue never finishes a buffer holding less than a full period, so
// a partly-filled buffer must not be enqueued (bug-370) — it is carried here
// until a later `write` fills it or `close` pads it with silence. Only the
// writing thread touches these, so they need no mutex.
pub(crate) const S_PENDING_BUF: usize = 368;
pub(crate) const S_PENDING_FILL: usize = 376;
pub(crate) const S_RING: usize = 384; // input ring payload (page-area)

// `AudioState` bookkeeping fits in the first page; output uses no ring so one
// page suffices. Input sizes the mapping to `S_RING + ringCapacity`.
pub(crate) const STATE_PAGE: usize = 16384;

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
pub(crate) const DEVICE_FIELD_ID: usize = 0;
pub(crate) const DEVICE_FIELD_NAME: usize = 8;
pub(crate) const DEVICE_FIELD_CAN_INPUT: usize = 16;
pub(crate) const DEVICE_FIELD_CAN_OUTPUT: usize = 24;
pub(crate) const DEVICE_FIELD_IS_DEFAULT_INPUT: usize = 32;
pub(crate) const DEVICE_FIELD_IS_DEFAULT_OUTPUT: usize = 40;
pub(crate) const DEVICE_RECORD_SIZE: usize = 48;

// Shared generic emitters, all from `native_helpers` (bug-330): `emit_alloc`
// is the one arena-allocation free function (`code/mod.rs`, bug-322); the rest
// are the package-neutral emitters that used to live in `tls`. Reuse them
// rather than duplicating. `emit_data_address` is re-exported for the
// AudioQueue phases.

// The emitted AudioQueue output callback (macOS): a C-ABI function the OS calls
// on an ordinary internal thread when a played buffer is free. openOutput takes
// its address; mod.rs registers the body when an output program is built.
pub(crate) const AUDIO_OUTPUT_CALLBACK_SYMBOL: &str = "_mfb_rt_audio_output_callback";
pub(crate) const AUDIO_INPUT_CALLBACK_SYMBOL: &str = "_mfb_rt_audio_input_callback";

// Scaffolding both backends share (bug-330); imported here so each backend's
// `use super::*` picks them up.
pub(crate) use super::gen_macos_callbacks::{
    lower_audio_input_callback, lower_audio_output_callback,
};

/// The `(instructions, relocations, stack_size)` an `audio` OS-seam body emits before
/// the `abi_function` wrapper finalizes it — the successor to the finalized
/// [`HelperResult`] tuple (see `fs`'s `FsBodyParts`). `stack_size` is the explicit
/// sp-relative locals region the body reserves; the wrapper passes it to
/// `finalize_vreg_body_with_locals`, byte-identical to the body's former self-finalize.
pub(crate) type AudioBodyParts = (Vec<CodeInstruction>, Vec<CodeRelocation>, usize);

/// The `void` result every device-I/O `audio` member returns from its per-member
/// `abi_function` body: every audio body emits its own fallible ABI, so the wrapper
/// appends no epilogue. `type_` is `Nothing`; `text` carries the runtime-call name.
pub(crate) fn void_result(call: &str) -> ValueResult {
    ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: call.to_string(),
    }
}

/// Family dispatch for the two `open` members (`openInput`/`openOutput`), shared by
/// both because a single caller cannot know its own direction: `is_input` picks the
/// stream direction, `device` selects the named-device overload
/// (`openInputDevice`/`openOutputDevice`). macOS has direction-specific emitters
/// (`lower_open_input`/`lower_open_output`); ALSA and WASAPI take a unified
/// `lower_open(is_input, device)`. Returns the pre-finalize [`AudioBodyParts`].
pub(crate) fn dispatch_open(
    symbol: &str,
    is_input: bool,
    device: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    match platform.family() {
        PlatformFamily::MacOS => {
            if is_input {
                super::gen_macos_stream::lower_open_input(
                    symbol,
                    device,
                    platform_imports,
                    platform,
                )
            } else {
                super::gen_macos_stream::lower_open_output(
                    symbol,
                    device,
                    platform_imports,
                    platform,
                )
            }
        }
        PlatformFamily::Linux => {
            super::gen_alsa_stream::lower_open(symbol, is_input, device, platform_imports, platform)
        }
        PlatformFamily::Windows => {
            super::gen_windows::lower_open(symbol, is_input, device, platform_imports, platform)
        }
    }
}

/// Family dispatch for the three stream-query members (`poll`/`available`/`xruns`),
/// shared because all three funnel to each backend's one `lower_query` emitter with
/// a different [`Query`] discriminant (`poll` additionally passing `PollTimeout` for
/// its timed overload). Returns the pre-finalize [`AudioBodyParts`].
pub(crate) fn dispatch_query(
    symbol: &str,
    query: Query,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    match platform.family() {
        PlatformFamily::MacOS => {
            super::gen_macos_io::lower_query(symbol, query, platform_imports, platform)
        }
        PlatformFamily::Linux => {
            super::gen_alsa_io::lower_query(symbol, query, platform_imports, platform)
        }
        PlatformFamily::Windows => {
            super::gen_windows::lower_query(symbol, query, platform_imports, platform)
        }
    }
}

/// C-string data objects (the `libasound.so.2` soname + ALSA symbol names) the
/// Linux backend references for its `dlopen`/`dlsym`.
fn alsa_data_objects() -> Vec<CodeDataObject> {
    super::gen_alsa_shared::data_objects()
}

/// The selected audio backend for a build. Owns the two whole-plan decisions
/// that `code/mod.rs` used to re-derive from `platform.target()` plus
/// hand-maintained literal symbol lists (bug-330 cause #3): which read-only data
/// objects the audio backend needs, and which AudioQueue callbacks to emit.
pub(crate) enum AudioBackend {
    CoreAudio,
    Alsa,
    /// Windows WASAPI over COM (plan-66 G+H). Emits the read-only GUID/CLSID/IID
    /// data objects the COM `CoCreateInstance`/`Activate`/`GetService` calls
    /// reference; needs no OS callback thread (WASAPI is event-driven).
    Wasapi,
    /// Platforms with no audio surface. `AudioBackend::select` is called for EVERY
    /// build to compute the audio data objects, not only audio-using ones, so this
    /// must be a real "no audio" answer rather than `unreachable!`: it emits no
    /// data objects and no callbacks.
    #[allow(dead_code)]
    NoAudio,
}

impl AudioBackend {
    /// Select the backend for `platform`. The single place the audio macOS/Linux
    /// decision is made.
    pub(crate) fn select(platform: &dyn CodegenPlatform) -> Self {
        match platform.family() {
            PlatformFamily::MacOS => AudioBackend::CoreAudio,
            PlatformFamily::Linux => AudioBackend::Alsa,
            PlatformFamily::Windows => AudioBackend::Wasapi,
        }
    }

    /// Read-only data objects the backend references, given the plan's runtime
    /// symbols. CoreAudio links AudioToolbox directly and needs none; ALSA emits
    /// its `dlopen`/`dlsym` C strings only when the plan uses an audio helper.
    pub(crate) fn data_objects(&self, runtime_symbols: &[String]) -> Vec<CodeDataObject> {
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
            AudioBackend::Wasapi => {
                if runtime_symbols
                    .iter()
                    .any(|symbol| symbol.starts_with("_mfb_rt_audio_"))
                {
                    super::gen_windows::data_objects()
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// The AudioQueue callback functions to emit (macOS only): the output
    /// callback when the plan builds an output stream, the input callback when it
    /// builds an input stream. `openOutput`/`openInput` take these addresses.
    pub(crate) fn callback_functions(
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
            if uses(super::gen_macos_callbacks::OUTPUT_CALLBACK_SYMBOLS) {
                functions.push(lower_audio_output_callback(platform_imports, platform)?);
            }
            if uses(super::gen_macos_callbacks::INPUT_CALLBACK_SYMBOLS) {
                functions.push(lower_audio_input_callback(platform_imports, platform)?);
            }
        }
        Ok(functions)
    }
}
