//! Codegen contracts for the `--debug` report helpers.
//!
//! The runtime behavior (the block on stderr, on every exit path) is pinned by
//! `tests/runtime/rt_debug_report.rs`. What is pinned here is what a runtime test
//! cannot see: that the helpers never touch the arena they run after, where the
//! call sits inside `_mfb_shutdown`, and that a normal build gets nothing.

use std::collections::HashMap;

use super::*;
use crate::codegen::engine::mir;
use crate::codegen::engine::tests::test_support::Stream;
use crate::codegen::error::constants::ARENA_STATE_REGISTER;
use crate::codegen::os::process::process_lifecycle::lower_shutdown;
use crate::target::macos_aarch64::code::Platform as MacosPlatform;
use crate::target::NativeBuildMode;
use crate::testutil::CodeTarget;

const SOURCE: &str = "FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n";

/// The imports the debug helpers resolve against on macOS: `_write` for every report
/// line (every entry module carries it, `entry_error_imports`), the mutex pair the
/// arena registry's register helper takes (a real plan attributes those to it through
/// `DebugFeature::lock_helpers`, `plan::symbols::platform_imports`), `_getrusage` for
/// the peak RSS, and `_clock_gettime` for the arena memory series' clock (plan-133-C).
/// A real macOS plan imports `_clock_gettime` for every program entry, for the
/// arena-fill seed (`macos_aarch64/plan.rs`, `entry_imports`).
fn imports() -> HashMap<String, String> {
    [
        "_write",
        "_pthread_mutex_lock",
        "_pthread_mutex_unlock",
        "_getrusage",
        "_clock_gettime",
    ]
    .into_iter()
    .map(|symbol| (symbol.to_string(), "libSystem".to_string()))
    .collect()
}

fn module(debug: bool) -> NirModule {
    let mut module =
        crate::testutil::nir_for_src(SOURCE, CodeTarget::MacosAarch64, NativeBuildMode::Console)
            .unwrap();
    module.debug = DebugOptions { enabled: debug };
    module
}

fn report_functions() -> Vec<CodeFunction> {
    mir::set_backend(MacosPlatform.backend());
    code_functions(&module(true), &imports(), &MacosPlatform).unwrap()
}

/// A build without `--debug` gets no debug function and no debug data object —
/// the byte-identity of every normal build rests on this.
#[test]
fn a_normal_module_gets_no_debug_code_or_data() {
    let normal = module(false);
    mir::set_backend(MacosPlatform.backend());
    assert!(code_functions(&normal, &imports(), &MacosPlatform)
        .unwrap()
        .is_empty());
    assert!(data_objects(&normal).is_empty());
    assert_eq!(active_features(&normal).count(), 0);
}

/// A `--debug` program that starts a thread gets the `thread` section, which reads
/// the thread table that program carries.
#[test]
fn a_threaded_debug_module_gets_the_thread_section() {
    const THREADED: &str = "IMPORT thread\n\
ISOLATED FUNC work(w AS ThreadWorker OF Integer TO Integer, n AS Integer) AS Integer\n  RETURN n\nEND FUNC\n\
FUNC main() AS Integer\n  RES t AS Thread OF Integer TO Integer = thread::start(work, 1)\n  RETURN thread::waitFor(t)\nEND FUNC\n";
    let mut module =
        crate::testutil::nir_for_src(THREADED, CodeTarget::MacosAarch64, NativeBuildMode::Console)
            .unwrap();
    module.debug = DebugOptions { enabled: true };
    mir::set_backend(MacosPlatform.backend());
    let symbols: Vec<String> = code_functions(&module, &imports(), &MacosPlatform)
        .unwrap()
        .into_iter()
        .map(|f| f.symbol)
        .collect();
    assert!(
        symbols.iter().any(|s| s == "_mfb_debug_report_thread"),
        "no thread section in {symbols:?}"
    );
}

/// A `--debug` module gets `_mfb_debug_shutdown` plus one report section per
/// registry entry that applies to it, and every symbol those functions address as
/// data exists. Only the `thread` section depends on the program (it reads the
/// thread table, which exists only when the program starts a thread), and this
/// program starts none; `a_threaded_debug_module_gets_the_thread_section` covers it.
#[test]
fn a_debug_module_gets_the_report_helper_and_each_section() {
    let functions = report_functions();
    let symbols: Vec<&str> = functions.iter().map(|f| f.symbol.as_str()).collect();
    assert_eq!(symbols.first(), Some(&DEBUG_SHUTDOWN_SYMBOL));
    let debug_module = module(true);
    for feature in DEBUG_FEATURES {
        if !feature.applies(&debug_module) {
            assert_eq!(feature.name(), thread::THREAD_SECTION, "only the thread section is conditional");
            continue;
        }
        let report = feature.report_symbol().expect("every section reports");
        assert!(
            symbols.contains(&report),
            "{report} missing from {symbols:?}"
        );
    }
    let data: Vec<String> = data_objects(&module(true))
        .into_iter()
        .map(|object| object.symbol)
        .collect();
    for function in &functions {
        for relocation in &function.relocations {
            if relocation.binding == "data" {
                assert!(
                    data.contains(&relocation.to),
                    "{} addresses {} which no debug data object defines",
                    function.symbol,
                    relocation.to
                );
            }
        }
    }
}

/// No debug helper may reach the arena.
///
/// `_mfb_shutdown` calls the report after `_mfb_arena_destroy` has unmapped every
/// block, and a signal can deliver the call with the arena register holding
/// anything at all. A helper that allocated or read through that register would
/// fault only in `--debug` builds, and only at exit. The check is on the allocator's
/// own symbols (`_mfb_arena_*`) and the main-arena global that points at its state,
/// not the word "arena": the arena registry's globals
/// (`_mfb_rt_debug_arena_*`) are debug-owned system memory, never arena blocks.
#[test]
fn debug_helpers_reference_no_arena_symbol() {
    for function in report_functions() {
        for relocation in &function.relocations {
            assert!(
                !relocation.to.starts_with("_mfb_arena_")
                    && relocation.to != crate::codegen::error::constants::MAIN_ARENA_GLOBAL_SYMBOL,
                "{} must not call the arena allocator; it relocates against {}",
                function.symbol,
                relocation.to
            );
        }
        for instruction in &function.instructions {
            for (field, _) in &instruction.fields {
                assert_ne!(
                    instruction.get(field).as_deref(),
                    Some(ARENA_STATE_REGISTER),
                    "{} names the arena register in `{field}` of {:?}",
                    function.symbol,
                    instruction.op
                );
            }
        }
    }
}

/// `_mfb_shutdown` reaches the report after `shutdown_done` — the label both its
/// full teardown and its already-shut-down early return arrive at — and before it
/// returns; a normal build's `_mfb_shutdown` does not call it at all.
#[test]
fn shutdown_calls_debug_report_after_the_done_label() {
    mir::set_backend(MacosPlatform.backend());
    let with =
        lower_shutdown(false, false, false, false, &imports(), &MacosPlatform, true).unwrap();
    let stream = Stream::of(&with);
    let done = stream.label_at("shutdown_done");
    let call = stream.index_of("the call to the debug report", |instruction| {
        instruction.get("target").as_deref() == Some(DEBUG_SHUTDOWN_SYMBOL)
    });
    assert!(
        call > done,
        "the report call (at {call}) must follow `shutdown_done` (at {done}) so the \
         early-return path reaches it too"
    );
    assert!(
        !stream.branches_to("shutdown_done") || call > done,
        "every branch to `shutdown_done` must land before the report call"
    );

    let without = lower_shutdown(
        false,
        false,
        false,
        false,
        &imports(),
        &MacosPlatform,
        false,
    )
    .unwrap();
    let calls = Stream::of(&without).calls_between(0, without.instructions.len());
    assert!(
        !calls.iter().any(|target| target == DEBUG_SHUTDOWN_SYMBOL),
        "a normal build's _mfb_shutdown must not call the report: {calls:?}"
    );
}
