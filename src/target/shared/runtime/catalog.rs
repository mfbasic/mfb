use super::*;
use std::sync::OnceLock;

/// The hand-written specs for packages not yet migrated onto the registry. Merged
/// with the registry-derived specs into the single frozen table by
/// [`supported_helper_specs`], which owns the `ptr::eq` identity contract (bug-382).
/// As each package migrates, its rows delete from here and it joins the derivation.
static LEGACY_HELPER_SPECS: &[RuntimeHelperSpec] = &[
    APP_GET_MODE_SPEC,
    APP_SET_MODE_SPEC,
    AUDIO_DEVICES_SPEC,
    AUDIO_OPEN_INPUT_SPEC,
    AUDIO_OPEN_INPUT_DEVICE_SPEC,
    AUDIO_OPEN_OUTPUT_SPEC,
    AUDIO_OPEN_OUTPUT_DEVICE_SPEC,
    AUDIO_READ_SPEC,
    AUDIO_READ_TIMEOUT_SPEC,
    AUDIO_WRITE_SPEC,
    AUDIO_POLL_SPEC,
    AUDIO_POLL_TIMEOUT_SPEC,
    AUDIO_AVAILABLE_SPEC,
    AUDIO_XRUNS_SPEC,
    AUDIO_CLOSE_INPUT_SPEC,
    AUDIO_CLOSE_OUTPUT_SPEC,
    // `crypto` is migrated: its ten native runtime helpers (`randomBytes`, the
    // NIST-EC `generateP*Raw` / `p{256,384,521}{Sign,Verify}`) are DERIVED from the
    // registry (`registry::runtime_specs`) and merged in by `supported_helper_specs`,
    // so no hand-written `CRYPTO_*_SPEC` rows live here.
    // `datetime` is migrated: its three OS-seam intrinsics (`nowNanos`,
    // `monotonicNanos`, `localOffset`) are DERIVED from the registry
    // (`registry::runtime_specs`), so no hand-written `DATETIME_*_SPEC` rows here.
    // `io` is migrated: its specs are DERIVED from the registry
    // (`registry::runtime_specs`) and merged in by `supported_helper_specs`, so no
    // hand-written `IO_*_SPEC` rows live here.
    TERM_ON_SPEC,
    TERM_OFF_SPEC,
    TERM_IS_ON_SPEC,
    TERM_SET_FOREGROUND_SPEC,
    TERM_SET_BACKGROUND_SPEC,
    TERM_SET_BOLD_SPEC,
    TERM_SET_UNDERLINE_SPEC,
    TERM_SHOW_CURSOR_SPEC,
    TERM_HIDE_CURSOR_SPEC,
    TERM_CLEAR_SPEC,
    TERM_SYNC_SPEC,
    TERM_MOVE_TO_SPEC,
    TERM_DRAW_HLINE_SPEC,
    TERM_DRAW_VLINE_SPEC,
    TERM_DRAW_BOX_SPEC,
    TERM_FILL_RECT_SPEC,
    TERM_DRAW_TEXT_SPEC,
    TERM_DRAW_GLYPH_SPEC,
    TERM_GET_FOREGROUND_SPEC,
    TERM_GET_BACKGROUND_SPEC,
    TERM_GET_BOLD_SPEC,
    TERM_GET_UNDERLINE_SPEC,
    TERM_TERMINAL_SIZE_SPEC,
    TERM_DID_RESIZE_SPEC,
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
                let helper = RuntimeHelper::from_package_name(pkg)
                    .unwrap_or_else(|| panic!("no RuntimeHelper for package `{pkg}`"));
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
            "net.connectTcpAddr",
            // plan-90-A: `process.spawn(args, cwd, env, envReplace)` is rewritten to
            // `process.spawnEnv` in the code layer (`builder_values`), so it never
            // exists at the NIR level and `helper_for_call` must not classify it.
            // (`process.__drop` IS routed — like audio's close ops — via
            // `is_process_runtime_call`, so it is deliberately NOT listed here.)
            "process.spawnEnv",
            "process.sendTimeout",
            "process.sendBytesTimeout",
            "process.pollFrom",
            "process.receiveFrom",
            "process.receiveBytesFrom",
            // plan-76-A: `net.poll(List OF RES Socket)` is rewritten to `net.pollList`
            // in the code layer (`builder_values`), so it never exists at the NIR
            // level and `helper_for_call` must not classify it.
            "net.pollList",
            // plan-76-C: same for `tls.poll(List OF RES TlsSocket)` → `tls.pollList`.
            "tls.pollList",
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
            RuntimeHelper::App,
            RuntimeHelper::Audio,
            RuntimeHelper::Crypto,
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
