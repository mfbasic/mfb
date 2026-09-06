use super::*;
use std::sync::OnceLock;

/// The hand-written specs for packages not yet migrated onto the registry. Merged
/// with the registry-derived specs into the single frozen table by
/// [`supported_helper_specs`], which owns the `ptr::eq` identity contract (bug-382).
/// As each package migrates, its rows delete from here and it joins the derivation.
static LEGACY_HELPER_SPECS: &[RuntimeHelperSpec] = &[
    // `app` is migrated: its two presentation-mode helpers (`getMode`, `setMode`)
    // are DERIVED from the registry (`registry::runtime_specs`) and merged in by
    // `supported_helper_specs`, so no hand-written `APP_*_SPEC` rows live here.
    // `audio` is migrated: its specs (including the two per-direction resource close
    // ops and the openInputDevice/openOutputDevice/readTimeout/pollTimeout code forms)
    // are DERIVED from the registry (`registry::runtime_specs`) and merged in by
    // `supported_helper_specs`, so no hand-written `AUDIO_*_SPEC` rows live here.
    // `crypto` is migrated: every crypto runtime call is a clean-room `AbiFunction`
    // (`generate`/`sign`/`verify`/`hash`/`seal`, and the OS-seam `randomBytes`), so it
    // routes through the shared `RuntimeHelper::Abi` family and is DERIVED from the
    // registry (`registry::runtime_specs`). There is no `RuntimeHelper::Crypto` family
    // and no hand-written `CRYPTO_*_SPEC` rows.
    // `datetime` is migrated: its three OS-seam intrinsics (`nowNanos`,
    // `monotonicNanos`, `localOffset`) are DERIVED from the registry
    // (`registry::runtime_specs`), so no hand-written `DATETIME_*_SPEC` rows here.
    // `io` is migrated: its specs are DERIVED from the registry
    // (`registry::runtime_specs`) and merged in by `supported_helper_specs`, so no
    // hand-written `IO_*_SPEC` rows live here.
    // `term` is migrated: its 24 native OS-seam helpers (the mode toggle, colors,
    // attributes, cursor, clear/sync, box-drawing, text/glyph, size/resize) are
    // DERIVED from the registry (`registry::runtime_specs`) and merged in by
    // `supported_helper_specs`, so no hand-written `TERM_*_SPEC` rows live here.
    // `fs` is migrated: its specs are DERIVED from the registry
    // (`registry::runtime_specs`, including the `File` resource close op) and merged
    // in by `supported_helper_specs`, so no hand-written `FS_*_SPEC` rows live here.
    // `os` is migrated: its specs are DERIVED from the registry
    // (`registry::runtime_specs`) and merged in by `supported_helper_specs`, so no
    // hand-written `OS_*_SPEC` rows live here.
    // `process` is migrated: its specs are DERIVED from the registry
    // (`registry::runtime_specs`) and merged in by `supported_helper_specs`, so no
    // hand-written `PROCESS_*_SPEC` rows live here.
    // plan-67-B: internal perf-tracking helpers. Catalogued (so `spec_for_symbol`
    // resolves the injected `_mfb_rt_perf_*` calls during emission/object
    // planning) but never routed by `helper_for_call` — they are code-layer-only
    // (see `CODE_LAYER_ONLY_CALLS`).
    PERF_INIT_SPEC,
    PERF_START_SPEC,
    PERF_END_SPEC,
    PERF_DONE_SPEC,
    // No `strings::` row: those ops are all native-direct (lowered inline; no
    // `_mfb_rt_strings_*` helper is ever emitted, bug-120.1). The dead spec
    // table that used to sit beside this comment is gone (bug-326-A1).
    // `thread` is migrated: its specs (the 13 descriptor members plus the
    // `emit`/`read`/`*Resource`/`drop` `os_aliases`) are DERIVED from
    // the registry (`registry::runtime_specs`) and merged in by
    // `supported_helper_specs`, so no hand-written `THREAD_*_SPEC` rows here.
    // `net` is migrated: its specs (including the `connectTcpAddr`/`pollList` code
    // forms and the three resource close ops) are DERIVED from the registry
    // (`registry::runtime_specs`) and merged in by `supported_helper_specs`, so no
    // hand-written `NET_*_SPEC` rows live here.
    // `tls` is migrated: its specs (including the two resource close ops and the
    // `pollList`/`closeListener` code forms) are DERIVED from the registry
    // (`registry::runtime_specs`) and merged in by `supported_helper_specs`, so no
    // hand-written `TLS_*_SPEC` rows live here.
];

/// The one catalog: the still-hand-written [`LEGACY_HELPER_SPECS`] for packages not
/// yet on the registry, plus the specs DERIVED from the registry for every migrated
/// native package ([`crate::codegen::registry::runtime_specs`]) — so a migrated
/// package carries no parallel `*_specs.rs`. Frozen once into a `OnceLock<Vec<_>>` so
/// the table has a single stable address: `spec_for_call`/`spec_for_symbol` callers
/// compare specs with `std::ptr::eq`, which needs one canonical spec per call (bug-382).
pub(crate) fn supported_helper_specs() -> &'static [RuntimeHelperSpec] {
    static MERGED: OnceLock<Vec<RuntimeHelperSpec>> = OnceLock::new();
    MERGED
        .get_or_init(|| {
            let mut specs = LEGACY_HELPER_SPECS.to_vec();
            for call in crate::codegen::registry::runtime_specs() {
                let pkg = call
                    .name
                    .split_once('.')
                    .expect("derived runtime call is package-qualified")
                    .0;
                // An `AbiFunction` member keeps its owning package's family when it
                // has one (plan-101: `io` stays `Io`), else the shared `Abi` family
                // (crypto) — via the same `abi_function_family` `helper_for_call`
                // uses, so its symbol and catalog spec agree.
                let helper = if crate::codegen::registry::is_abi_function_call(call.name) {
                    super::abi_function_family(call.name)
                } else {
                    RuntimeHelper::from_package_name(pkg)
                        .unwrap_or_else(|| panic!("no RuntimeHelper for package `{pkg}`"))
                };
                specs.push(RuntimeHelperSpec {
                    helper,
                    call: call.name,
                    abi: RuntimeHelperAbi {
                        returns: abi_return_name(&call.return_type),
                    },
                });
            }
            specs
        })
        .as_slice()
}

/// The base (unqualified) name of a derived call's return type, matching the spelling
/// the hand-written specs used: a resource handle `process.Process` renders `"Process"`
/// (the ABI type name is bare), while primitives/containers render verbatim.
fn abi_return_name(ty: &crate::types::ParameterType) -> &'static str {
    match ty.name() {
        std::borrow::Cow::Borrowed(name) => name.rsplit('.').next().unwrap_or(name),
        std::borrow::Cow::Owned(name) => Box::leak(name.into_boxed_str()),
    }
}

pub(crate) fn spec_for_symbol(symbol: &str) -> Option<&'static RuntimeHelperSpec> {
    supported_helper_specs()
        .iter()
        .find(|spec| symbol_for_call(spec.helper, spec.call) == symbol)
}

pub(crate) fn spec_for_call(target: &str) -> Option<&'static RuntimeHelperSpec> {
    supported_helper_specs()
        .iter()
        .find(|spec| spec.call == target)
}

#[cfg(test)]
mod tests;
