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

// The `AudioDevice` record: six word-slots, `String` fields as pointers.
pub(super) const DEVICE_FIELD_ID: usize = 0;
pub(super) const DEVICE_FIELD_NAME: usize = 8;
pub(super) const DEVICE_FIELD_CAN_INPUT: usize = 16;
pub(super) const DEVICE_FIELD_CAN_OUTPUT: usize = 24;
pub(super) const DEVICE_FIELD_IS_DEFAULT_INPUT: usize = 32;
pub(super) const DEVICE_FIELD_IS_DEFAULT_OUTPUT: usize = 40;
pub(super) const DEVICE_RECORD_SIZE: usize = 48;

// Shared emit helpers live in the `tls` module; reuse them rather than
// duplicating. `emit_data_address` is re-exported for the AudioQueue phases.
#[allow(unused_imports)]
pub(super) use super::tls::{emit_alloc, emit_data_address, emit_fail};

mod macos;

/// Dispatch an `audio.*` runtime-helper body to the platform backend.
pub(in crate::target::shared::code) fn lower_audio_helper(
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
    if platform.target().contains("macos") {
        return macos::lower_audio_macos(call, symbol, platform_imports, platform);
    }
    // Linux/ALSA backend lands in plan-33-C.
    Err(format!(
        "native code plan does not emit runtime call '{call}' for {}",
        platform.target()
    ))
}
