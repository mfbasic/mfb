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
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
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

mod alsa;
mod common;
mod macos;
mod windows;

// Scaffolding both backends share (bug-330); imported here so each backend's
// `use super::*` picks them up.
use common::{emit_validate_open, Query, READ_FRAMES_MAX};
pub(crate) use macos::{lower_audio_input_callback, lower_audio_output_callback};

/// The `(instructions, relocations, stack_size)` an `audio` OS-seam body emits before
/// the `abi_function` wrapper finalizes it — the successor to the finalized
/// [`HelperResult`] tuple (see `fs`'s `FsBodyParts`). `stack_size` is the explicit
/// sp-relative locals region the body reserves; the wrapper passes it to
/// `finalize_vreg_body_with_locals`, byte-identical to the body's former self-finalize.
pub(crate) type AudioBodyParts = (Vec<CodeInstruction>, Vec<CodeRelocation>, usize);

/// The `abi_function` body shared by every device-I/O `audio` member (crypto/io's
/// clean-room shape). The `abi_function` wrapper seeds the entry label, binds the
/// incoming ABI argument registers, and finalizes; this body dispatches to the
/// family-generic [`lower_audio_helper`] by the runtime-call name in
/// [`AbiCtx::call`](crate::codegen::registry::AbiCtx) — which is the member's own name
/// OR one of its IR-level overload-split code forms (`openInputDevice`/`readTimeout`/
/// `closeInput`/…) — and appends its instructions/relocations. All device-I/O members
/// register this one body; the aux→primary routing lives in `abi_function_lower`.
pub(crate) fn lower_audio_os_seam(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &crate::codegen::registry::AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        lower_audio_helper(ctx.call, &symbol, ctx.platform_imports, ctx.platform)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    // A `void` location: every audio body emits its own fallible ABI, so the wrapper
    // appends no epilogue.
    Ok(ValueResult {
        type_: "Nothing".to_string(),
        location: Operand::from("void"),
        text: ctx.call.to_string(),
    })
}

/// Dispatch an `audio.*` runtime-helper body to the platform backend, picked by
/// `platform.family()`. Reached from the shared [`lower_audio_os_seam`]
/// `abi_function` body; each backend dispatcher returns the pre-finalize
/// [`AudioBodyParts`] the wrapper finalizes.
pub(crate) fn lower_audio_helper(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    match platform.family() {
        PlatformFamily::MacOS => macos::lower_audio_macos(call, symbol, platform_imports, platform),
        PlatformFamily::Linux => alsa::lower_audio_alsa(call, symbol, platform_imports, platform),
        // Windows drives WASAPI over COM (plan-66 G+H): the `windows` submodule.
        PlatformFamily::Windows => {
            windows::lower_audio_windows(call, symbol, platform_imports, platform)
        }
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
                    windows::data_objects()
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
