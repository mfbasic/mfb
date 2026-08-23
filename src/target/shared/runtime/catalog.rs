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
    THREAD_START_SPEC,
    THREAD_IS_RUNNING_SPEC,
    THREAD_WAIT_FOR_SPEC,
    THREAD_CANCEL_SPEC,
    THREAD_DROP_SPEC,
    THREAD_SEND_SPEC,
    THREAD_POLL_SPEC,
    THREAD_SLEEP_SPEC,
    THREAD_READ_SPEC,
    THREAD_RECEIVE_SPEC,
    THREAD_EMIT_SPEC,
    THREAD_SLEEP_WORKER_SPEC,
    THREAD_TRANSFER_SPEC,
    THREAD_ACCEPT_SPEC,
    THREAD_EMIT_RESOURCE_SPEC,
    THREAD_READ_RESOURCE_SPEC,
    THREAD_IS_CANCELLED_SPEC,
    THREAD_OPEN_STD_IN_SPEC,
    THREAD_CLOSE_STD_IN_SPEC,
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
mod tests {
    use super::*;
    use crate::target::shared::runtime::{helper_for_call, symbol_for_call};
    use std::collections::HashSet;

    // One table-driven parity test over the catalog itself (bug-329), replacing
    // the hand-copied per-family call arrays that used to live in
    // audio_specs.rs/os_specs.rs: a new spec is covered the moment it is added,
    // because there is no second list to maintain.
    #[test]
    fn catalog_is_consistent() {
        let specs = supported_helper_specs();
        let mut seen_symbols = HashSet::new();
        let mut families = HashSet::new();
        // Catalogued calls that `helper_for_call` must NOT classify: these are
        // synthesized inside the code layer (`builder_values` rewrites the
        // user-facing call into the direction/overload-specific queue or addr
        // variant; `thread.drop` is the handle-cleanup helper emitted by
        // codegen primitives), so they never exist at the NIR level where
        // `helper_for_call` routes calls. They are catalogued only so
        // `spec_for_call`/`spec_for_symbol` resolve them during code emission
        // and object planning.
        const CODE_LAYER_ONLY_CALLS: &[&str] = &[
            "thread.drop",
            "thread.read",
            "thread.emit",
            // plan-91-B: worker-side sleep is synthesized from `thread.sleep` in
            // the code layer (builder_values), so it never exists at NIR level.
            "thread.sleepWorker",
            // `process`'s `spawnEnv`/`sendTimeout`/`sendBytesTimeout`/`pollFrom`/
            // `receiveFrom`/`receiveBytesFrom` code-form aliases are NOT listed: since
            // the migration to `Body::abi_function_aliased` they are registered
            // `os_aliases` of an `abi_function` member, so `is_abi_function_call` /
            // `abi_function_lower` classify them to the `Process` family (like net's
            // aliases). They are still synthesized in the code layer (`builder_values`
            // rewrites `process.spawn(4 args)`/`process.send(timeout)`/…), so they never
            // reach `helper_for_call` at the NIR level in practice — but were they to,
            // the family answer is now correct rather than `None`. (`process.__drop` IS
            // routed via `is_process_runtime_call`, so it too is not listed here.)
            // audio's overload-split runtime calls (`openInputDevice`/`openOutputDevice`/
            // `readTimeout`/`pollTimeout`/`closeInput`/`closeOutput`) are rewritten at IR
            // level (`audio::runtime_overload_name`), so they DO exist at the NIR level
            // and `helper_for_call` classifies them — they are deliberately NOT listed
            // here (the `audio.close` base member, which always rewrites away, never
            // reaches a runtime symbol but is still classified by `owning_package`).
            // `net`'s `connectTcpAddr`/`pollList` code-form aliases are NOT listed:
            // since the migration to `Body::abi_function_aliased` they are registered
            // `os_aliases` of an `abi_function` member, so `is_abi_function_call` /
            // `abi_function_lower` classify them to the `Net` family (like audio's
            // overload-split forms). They are still synthesized in the code layer
            // (`builder_values` rewrites `net.poll`/`net.connectTcp`), so they never
            // reach `helper_for_call` at the NIR level in practice — but where they to,
            // the family answer is now correct rather than `None`.
            // `tls`'s `pollList`/`closeListener` code-form aliases are NOT listed: since
            // the migration to `Body::abi_function_aliased` they are registered
            // `os_aliases` of an `abi_function` member, so `is_abi_function_call` /
            // `abi_function_lower` classify them to the `Tls` family (like net's aliases).
            // They are still synthesized in the code layer (`builder_values` rewrites
            // `tls.poll(List …)` / listener scope-drop), so they never reach
            // `helper_for_call` at the NIR level in practice.
            // plan-67-B: perf helpers are injected by the code layer (program
            // entry/exit + arena-region wrapping), never present at the NIR level,
            // so `helper_for_call` must NOT classify them.
            "perf.init",
            "perf.start",
            "perf.end",
            "perf.done",
        ];
        // Family round-trip: the front end routes each call to its helper
        // (except the code-layer-synthesized calls, which must stay invisible
        // to the NIR-level classifier). Collected so one failure reports the
        // whole set.
        let misrouted: Vec<String> = specs
            .iter()
            .filter_map(|spec| {
                let expected = if CODE_LAYER_ONLY_CALLS.contains(&spec.call) {
                    None
                } else {
                    Some(spec.helper)
                };
                let actual = helper_for_call(spec.call);
                (actual != expected)
                    .then(|| format!("{}: {:?} (expected {:?})", spec.call, actual, expected))
            })
            .collect();
        assert!(misrouted.is_empty(), "misrouted calls: {misrouted:#?}");
        for spec in specs {
            // Call round-trip (also proves call strings are unique: a duplicate
            // would resolve to the first entry and fail here for the second).
            assert!(
                std::ptr::eq(spec_for_call(spec.call).unwrap(), spec),
                "spec_for_call {}",
                spec.call
            );
            // Symbol round-trip + uniqueness. This is the surviving form of
            // the pre-deletion `every_spec_symbol_is_derivable` gate: the
            // derived symbol must resolve back to exactly this spec.
            let symbol = symbol_for_call(spec.helper, spec.call);
            assert!(
                std::ptr::eq(spec_for_symbol(&symbol).unwrap(), spec),
                "spec_for_symbol {symbol}"
            );
            assert!(
                seen_symbols.insert(symbol),
                "duplicate symbol for {}",
                spec.call
            );
            // `returns` is the load-bearing abi field; every code-plan consumer
            // reads it.
            assert!(!spec.abi.returns.is_empty(), "{} returns", spec.call);
            families.insert(spec.helper);
        }
        // Every RuntimeHelper family is catalogued except General and Math,
        // which are fully native-direct (lowered inline; no `_mfb_rt_*` helper
        // is ever emitted for them). A variant missing here with no catalogued
        // spec is the dead-catalog situation bug-326 removed for `strings`.
        for helper in [
            // The clean-room `AbiFunction` family (e.g. `crypto.generate`).
            RuntimeHelper::Abi,
            RuntimeHelper::App,
            RuntimeHelper::Audio,
            RuntimeHelper::Datetime,
            RuntimeHelper::Fs,
            RuntimeHelper::Io,
            RuntimeHelper::Net,
            RuntimeHelper::Os,
            RuntimeHelper::Process,
            // plan-67-B: catalogued (four `perf.*` specs) though code-layer-only.
            RuntimeHelper::Perf,
            RuntimeHelper::Term,
            RuntimeHelper::Thread,
            RuntimeHelper::Tls,
        ] {
            assert!(
                families.contains(&helper),
                "family {} has no catalogued spec",
                helper.name()
            );
        }
        assert_eq!(families.len(), 13, "unexpected extra catalogued family");
    }
}
