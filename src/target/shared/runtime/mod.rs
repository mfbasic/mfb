use crate::builtins;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeHelper {
    App,
    Audio,
    Crypto,
    Datetime,
    Fs,
    General,
    Io,
    Math,
    Net,
    Os,
    // plan-67-B: internal runtime performance tracking. Unlike every other family
    // it is NOT reachable from MFB source (there is no `perf::` package); its four
    // helpers are invoked only by compiler-injected calls in a debug-built,
    // macOS-entry program, so its calls are catalogued as code-layer-only (see
    // `catalog::tests::CODE_LAYER_ONLY_CALLS`) and forced into the emitted symbol
    // set in `plan::symbols::runtime_symbols` rather than routed by
    // `helper_for_call`/`required_helpers`.
    Perf,
    Term,
    Thread,
    Tls,
}

impl RuntimeHelper {
    pub fn name(self) -> &'static str {
        match self {
            RuntimeHelper::App => "app",
            RuntimeHelper::Audio => "audio",
            RuntimeHelper::Crypto => "crypto",
            RuntimeHelper::Datetime => "datetime",
            RuntimeHelper::Fs => "fs",
            RuntimeHelper::General => "general",
            RuntimeHelper::Io => "io",
            RuntimeHelper::Math => "math",
            RuntimeHelper::Net => "net",
            RuntimeHelper::Os => "os",
            RuntimeHelper::Perf => "perf",
            RuntimeHelper::Term => "term",
            RuntimeHelper::Thread => "thread",
            RuntimeHelper::Tls => "tls",
        }
    }
}

pub fn symbol_for_call(helper: RuntimeHelper, target: &str) -> String {
    format!(
        "_mfb_rt_{}_{}",
        helper.name(),
        target
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>()
    )
}

#[derive(Clone, Copy)]
pub(crate) struct RuntimeHelperSpec {
    pub(crate) helper: RuntimeHelper,
    pub(crate) call: &'static str,
    // No `symbol` field: the helper symbol is always derived via
    // `symbol_for_call(helper, call)` (bug-329). `catalog::tests` proved the
    // transcribed copies were byte-identical for every spec before the field
    // was deleted; if a future spec ever needs a non-derivable symbol, the
    // field must come back rather than special-casing `symbol_for_call`.
    pub(crate) abi: RuntimeHelperAbi,
}

/// The machine-read half of a helper's calling contract. `returns` is the one
/// field code planning consumes (it types each helper's `CodeFunction`).
///
/// There are deliberately no `params`/`clobbers` fields (bug-329): the former
/// transcribed argument names/types/registers that nothing read — the front-end
/// tables in `src/builtins/` own argument shapes, and the copies here had
/// already drifted from them — and the latter repeated one constant at every
/// spec while the register allocator models call clobbering independently
/// (every internal `bl _mfb_*` destroys all of `x0`–`x17`; see
/// `regalloc/analysis.rs` call-clobber masks and `.ai/compiler.md`).
#[derive(Clone, Copy)]
pub(crate) struct RuntimeHelperAbi {
    pub(crate) returns: &'static str,
}

mod app_specs;
mod audio_specs;
mod catalog;
mod crypto_specs;
mod datetime_specs;
mod fs_specs;
mod io_specs;
mod net_specs;
mod os_specs;
mod perf_specs;
mod term_specs;
mod thread_specs;
mod tls_specs;
mod usage;

pub(crate) use catalog::{spec_for_call, spec_for_symbol, supported_helper_specs};
pub(crate) use usage::{is_native_direct_call, required_helpers};

use app_specs::*;
use audio_specs::*;
use crypto_specs::*;
use datetime_specs::*;
use fs_specs::*;
use io_specs::*;
use net_specs::*;
use os_specs::*;
use perf_specs::*;
use term_specs::*;
use thread_specs::*;
use tls_specs::*;

pub fn helper_for_call(name: &str) -> Option<RuntimeHelper> {
    if builtins::app::is_app_call(name) {
        Some(RuntimeHelper::App)
    } else if builtins::audio::is_audio_runtime_call(name) {
        Some(RuntimeHelper::Audio)
    } else if builtins::crypto::is_native_crypto_call(name) {
        Some(RuntimeHelper::Crypto)
    } else if matches!(
        name,
        "datetime.nowNanos" | "datetime.monotonicNanos" | "datetime.localOffset"
    ) {
        Some(RuntimeHelper::Datetime)
    } else if builtins::fs::is_fs_call(name) {
        Some(RuntimeHelper::Fs)
    } else if builtins::general::is_general_call(name) {
        Some(RuntimeHelper::General)
    } else if builtins::io::is_io_call(name) {
        Some(RuntimeHelper::Io)
    } else if builtins::math::is_math_call(name) {
        Some(RuntimeHelper::Math)
    } else if builtins::term::is_term_call(name) {
        Some(RuntimeHelper::Term)
    } else if builtins::thread::is_thread_runtime_call(name) {
        Some(RuntimeHelper::Thread)
    } else if builtins::net::is_net_call(name) {
        Some(RuntimeHelper::Net)
    } else if builtins::os::is_os_call(name) {
        Some(RuntimeHelper::Os)
    } else if builtins::tls::is_tls_runtime_call(name) {
        Some(RuntimeHelper::Tls)
    } else {
        None
    }
}
