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
    // variant), so they never exist at the NIR level where `helper_for_call`
    // routes calls. They are catalogued only so `spec_for_call`/`spec_for_symbol`
    // resolve them during code emission and object planning.
    const CODE_LAYER_ONLY_CALLS: &[&str] = &[
        // `thread`'s worker/parent + resource-plane code forms (`emit`/`read`/
        // `transferResource`/`acceptResource`/`emitResource`/
        // `readResource`) and the `drop` scope-cleanup op are NOT listed: since the
        // migration to `Body::abi_function`/`abi_function_aliased` they are registered
        // `os_aliases` of an `abi_function` member (`emit`→`send`, `read`→`receive`,
        // `sleepWorker`→`sleep`, the `*Resource` forms→`transfer`/`accept`,
        // `drop`→`cancel`), so `is_abi_function_call`/`abi_function_lower` classify them
        // to the `Thread` family (like net's/tls's aliases). They are still synthesized
        // in the code layer (`builder_values` rewrites the direction split; `drop` is
        // the codegen-emitted handle-cleanup op), so they never reach `helper_for_call`
        // at the NIR level in practice — but were they to, the family answer is now
        // correct rather than `None`.
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
        // plan-98-B: `canvas.present` (and, from Phase 4, the `Image`/`Font`
        // close ops) — an `AbiFunction` family that keeps its own package name
        // rather than falling back to the shared `Abi` family.
        RuntimeHelper::Canvas,
        RuntimeHelper::Datetime,
        RuntimeHelper::Fs,
        RuntimeHelper::Io,
        RuntimeHelper::Net,
        RuntimeHelper::Os,
        RuntimeHelper::Process,
        // plan-67-B: catalogued (four `perf.*` specs) though code-layer-only.
        RuntimeHelper::Perf,
        // plan-110-B: `tcp` is its own family rather than riding the shared
        // `Abi` one, so its helpers keep `_mfb_rt_tcp_tcp_*` symbols that name
        // the package — matching `net`/`tls` and keeping a stack trace legible.
        RuntimeHelper::Tcp,
        RuntimeHelper::Term,
        RuntimeHelper::Thread,
        RuntimeHelper::Tls,
        // plan-110-C: `udp` is likewise its own family, for the same reason.
        RuntimeHelper::Udp,
    ] {
        assert!(
            families.contains(&helper),
            "family {} has no catalogued spec",
            helper.name()
        );
    }
    assert_eq!(families.len(), 16, "unexpected extra catalogued family");
}
