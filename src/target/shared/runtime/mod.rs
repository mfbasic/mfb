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
    Process,
    // plan-67-B: internal runtime performance tracking. Unlike every other family
    // it is NOT reachable from MFB source (there is no `perf::` package); its four
    // helpers are invoked only by compiler-injected calls in a `--cfg perf`-built,
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
            RuntimeHelper::Process => "process",
            RuntimeHelper::Perf => "perf",
            RuntimeHelper::Term => "term",
            RuntimeHelper::Thread => "thread",
            RuntimeHelper::Tls => "tls",
        }
    }

    /// The helper family owning package `name` (the inverse of [`name`](Self::name)),
    /// or `None` if `name` is not a runtime-helper package. Maps a derived call's
    /// package prefix (`"process"` from `"process.spawn"`) to its family.
    pub fn from_package_name(name: &str) -> Option<RuntimeHelper> {
        Some(match name {
            "app" => RuntimeHelper::App,
            "audio" => RuntimeHelper::Audio,
            "crypto" => RuntimeHelper::Crypto,
            "datetime" => RuntimeHelper::Datetime,
            "fs" => RuntimeHelper::Fs,
            "general" => RuntimeHelper::General,
            "io" => RuntimeHelper::Io,
            "math" => RuntimeHelper::Math,
            "net" => RuntimeHelper::Net,
            "os" => RuntimeHelper::Os,
            "process" => RuntimeHelper::Process,
            "perf" => RuntimeHelper::Perf,
            "term" => RuntimeHelper::Term,
            "thread" => RuntimeHelper::Thread,
            "tls" => RuntimeHelper::Tls,
            _ => return None,
        })
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

mod catalog;
mod perf_specs;
mod term_specs;
mod thread_specs;
mod usage;

pub(crate) use catalog::{spec_for_call, spec_for_symbol, supported_helper_specs};
pub(crate) use usage::{is_native_direct_call, required_helpers};

use perf_specs::*;
use term_specs::*;
use thread_specs::*;

pub fn helper_for_call(name: &str) -> Option<RuntimeHelper> {
    if crate::codegen::registry::registry().owning_package(name) == Some("app") {
        Some(RuntimeHelper::App)
    } else if name.starts_with("audio.") {
        // Every `audio.*` runtime call routes to the Audio family: the descriptor
        // members plus the IR-level overload-split names (`openInputDevice`/
        // `openOutputDevice`/`readTimeout`/`pollTimeout`/`closeInput`/`closeOutput`) that
        // `audio::runtime_overload_name` synthesizes at IR level, so they exist at NIR.
        // (The source members `audio.render`/`audio.play` are internalized before
        // reaching here.)
        Some(RuntimeHelper::Audio)
    } else if crate::codegen::builtins::crypto::is_native_crypto_call(name) {
        Some(RuntimeHelper::Crypto)
    } else if matches!(
        name,
        "datetime.nowNanos" | "datetime.monotonicNanos" | "datetime.localOffset"
    ) {
        Some(RuntimeHelper::Datetime)
    } else if crate::codegen::registry::registry().owning_package(name) == Some("fs") {
        Some(RuntimeHelper::Fs)
    } else if builtins::general::is_general_call(name) {
        Some(RuntimeHelper::General)
    } else if crate::codegen::registry::registry().owning_package(name) == Some("io") {
        Some(RuntimeHelper::Io)
    } else if crate::codegen::registry::registry().owning_package(name) == Some("math") {
        Some(RuntimeHelper::Math)
    } else if builtins::term::is_term_call(name) {
        Some(RuntimeHelper::Term)
    } else if crate::codegen::builtins::thread::is_thread_runtime_call(name) {
        Some(RuntimeHelper::Thread)
    } else if crate::codegen::registry::registry().owning_package(name) == Some("net") {
        // The `net.connectTcpAddr`/`net.pollList` code forms are `os_aliases`, not
        // descriptor members, so `owning_package` yields `None` for them — they are
        // code-layer-only (`CODE_LAYER_ONLY_CALLS`) and must not classify here.
        Some(RuntimeHelper::Net)
    } else if crate::codegen::registry::registry().owning_package(name) == Some("os") {
        Some(RuntimeHelper::Os)
    } else if crate::codegen::registry::registry().owning_package(name) == Some("process")
        || name == "process.__drop"
    {
        Some(RuntimeHelper::Process)
    } else if crate::codegen::registry::registry().owning_package(name) == Some("tls")
        || name == "tls.closeListener"
    {
        // `tls.closeListener` is the internal listener-shaped scope-drop close body
        // (not a descriptor member), synthesized during IR lowering — routed here like
        // `process.__drop`.
        Some(RuntimeHelper::Tls)
    } else {
        None
    }
}
