//! A runtime helper refuses, by name, an OS import its plan did not declare.
//!
//! Every runtime helper that reaches the OS resolves its calls through the plan's
//! import list as it emits (`CodegenPlatform::emit_external_call`): the list is
//! derived from what the program's calls need, so a helper that grew a call the plan
//! does not declare fails at LOWERING, naming the symbol, instead of at link time on
//! the one OS that lacks it. The alternative to the refusal is a relocation against a
//! symbol nothing imports.
//!
//! The helpers are driven directly (they are `pub(crate)` entry points taking the
//! import map and a platform), against each backend's real platform. For each helper
//! on each backend: lower it with the plan's full import list, collect the external
//! symbols its relocations name, and lower it again with each one withdrawn in turn;
//! the lowering must refuse and the refusal must name the withdrawn symbol.

use std::collections::{BTreeSet, HashMap};

use super::platforms::platform_for;
use crate::codegen::engine::mir;
use crate::codegen::engine::types::{CodeFunction, CodegenPlatform, NativeCodePlan};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, CodeTarget};

/// A program that reads standard input, so the plan imports what the stdin helpers call.
const READS_STDIN: &str = "\
IMPORT io

FUNC main() AS Integer
  LET line = io::readLine()
  io::print(line)
  RETURN 0
END FUNC
";

type Lower =
    dyn Fn(&HashMap<String, String>, &dyn CodegenPlatform) -> Result<Vec<CodeFunction>, String>;

fn imports_of(plan: &NativeCodePlan) -> HashMap<String, String> {
    plan.imports
        .iter()
        .map(|import| (import.symbol.clone(), import.library.clone()))
        .collect()
}

/// The external symbols `functions` call, per their relocations.
fn external_symbols(functions: &[CodeFunction]) -> BTreeSet<String> {
    functions
        .iter()
        .flat_map(|function| function.relocations.iter())
        .filter(|relocation| relocation.library.is_some())
        .map(|relocation| relocation.to.clone())
        .collect()
}

/// Withdraw each external symbol `lower` calls, in turn, on every POSIX backend; the
/// refusal must name it. Returns how many (target, symbol) refusals held.
fn refuses_each_missing_import(label: &str, lower: &Lower) -> usize {
    refuses_each_missing_import_in(READS_STDIN, label, lower)
}

/// [`refuses_each_missing_import`] with the import list of `source`'s plan.
fn refuses_each_missing_import_in(source: &str, label: &str, lower: &Lower) -> usize {
    refuses_each_missing_import_on(&CodeTarget::ALL, source, label, lower)
}

/// [`refuses_each_missing_import_in`] on `targets` only, for a helper one backend owns.
fn refuses_each_missing_import_on(
    targets: &[CodeTarget],
    source: &str,
    label: &str,
    lower: &Lower,
) -> usize {
    let mut refused = 0;
    for &target in targets {
        let plan = code_for_src_cached(source, target, Console);
        let imports = imports_of(plan);
        let platform = platform_for(target);
        mir::set_backend(platform.backend());
        let functions = lower(&imports, &*platform)
            .unwrap_or_else(|err| panic!("{label} must lower on {}: {err}", target.name()));
        // Windows reaches the OS through its own import address table, which
        // `emit_external_call` does not consult by symbol: nothing to withdraw there.
        if target == CodeTarget::WindowsX86_64 {
            continue;
        }
        for symbol in external_symbols(&functions) {
            let mut withdrawn = imports.clone();
            // The import list is keyed by the symbol as the platform spells it; a
            // relocation's `to` is that spelling.
            if withdrawn.remove(&symbol).is_none() {
                continue;
            }
            let err = match lower(&withdrawn, &*platform) {
                Ok(_) => panic!(
                    "{label} on {}: lowering without `{symbol}` imported must be refused",
                    target.name()
                ),
                Err(err) => err,
            };
            assert!(
                err.contains(symbol.trim_start_matches('_')),
                "{label} on {}: the refusal must name `{symbol}`; it said {err}",
                target.name()
            );
            refused += 1;
        }
    }
    refused
}

#[test]
fn the_stdin_next_byte_helper_refuses_an_undeclared_import() {
    let refused = refuses_each_missing_import("stdin next_byte", &|imports, platform| {
        crate::codegen::io::stdin::lower_stdin_next_byte(
            imports,
            platform,
            crate::codegen::error::constants::STDIN_LOG_CAP_DEFAULT,
        )
        .map(|function| vec![function])
    });
    assert!(
        refused > 0,
        "some backend must call the OS from this helper"
    );
}

#[test]
fn the_stdin_subscription_helpers_refuse_an_undeclared_import() {
    let mut refused = refuses_each_missing_import("stdin recompute_base", &|imports, platform| {
        crate::codegen::io::stdin::lower_stdin_recompute_base(imports, platform)
            .map(|function| vec![function])
    });
    refused += refuses_each_missing_import("stdin subscribe", &|imports, platform| {
        crate::codegen::io::stdin::lower_stdin_subscribe(imports, platform)
            .map(|function| vec![function])
    });
    refused += refuses_each_missing_import("stdin unsubscribe", &|imports, platform| {
        crate::codegen::io::stdin::lower_stdin_unsubscribe(imports, platform)
            .map(|function| vec![function])
    });
    assert!(
        refused > 0,
        "some backend must call the OS from these helpers"
    );
}

/// A program that starts a thread, so the plan imports what the thread table and the
/// control-block helpers call.
const STARTS_A_THREAD: &str = "\
IMPORT thread

ISOLATED FUNC work(t AS ThreadWorker OF Nothing TO Integer, seed AS Integer) AS Integer
  RETURN seed + 1
END FUNC

FUNC main() AS Integer
  RES a AS Thread OF Nothing TO Integer = thread::start(work, 1)
  RETURN thread::waitFor(a)
END FUNC
";

/// The thread table and control-block helpers (`runtime/thread/control_block.rs`):
/// each lock, unlock, allocation and arena call is resolved through the plan's imports,
/// with and without the `--debug` arena registry.
#[test]
fn the_thread_control_block_helpers_refuse_an_undeclared_import() {
    use crate::codegen::runtime::thread::{
        lower_thread_block_arena_create, lower_thread_block_arena_destroy,
        lower_thread_block_release, lower_thread_table_acquire, lower_thread_table_release,
    };
    let mut refused = refuses_each_missing_import_in(
        STARTS_A_THREAD,
        "thread table acquire",
        &|imports, platform| lower_thread_table_acquire(imports, platform).map(|f| vec![f]),
    );
    refused += refuses_each_missing_import_in(
        STARTS_A_THREAD,
        "thread table release",
        &|imports, platform| lower_thread_table_release(imports, platform).map(|f| vec![f]),
    );
    for debug in [false, true] {
        refused += refuses_each_missing_import_in(
            STARTS_A_THREAD,
            "thread block arena create",
            &move |imports, platform| {
                lower_thread_block_arena_create(debug, imports, platform).map(|f| vec![f])
            },
        );
        refused += refuses_each_missing_import_in(
            STARTS_A_THREAD,
            "thread block arena destroy",
            &move |imports, platform| {
                lower_thread_block_arena_destroy(debug, imports, platform).map(|f| vec![f])
            },
        );
        refused += refuses_each_missing_import_in(
            STARTS_A_THREAD,
            "thread block release",
            &move |imports, platform| {
                lower_thread_block_release(debug, imports, platform).map(|f| vec![f])
            },
        );
    }
    assert!(
        refused > 0,
        "some backend must call the OS from these helpers"
    );
}

/// A program that records and plays, so a macOS plan imports what the AudioQueue
/// callbacks call.
const RECORDS_AND_PLAYS: &str = "\
IMPORT audio

FUNC main() AS Integer
  RES mic AS audio::AudioInput = audio::openInput(48000, 1, 512)
  LET pcm = audio::read(mic, 256)
  RES spk AS audio::AudioOutput = audio::openOutput(48000, 1, 512)
  audio::write(spk, pcm)
  audio::close(spk)
  audio::close(mic)
  RETURN 0
END FUNC
";

/// The macOS AudioQueue output and input callbacks (`audio/gen_macos_callbacks.rs`):
/// the mutex, the condition signal and the buffer re-enqueue are each resolved through
/// the plan's imports.
#[test]
fn the_macos_audio_callbacks_refuse_an_undeclared_import() {
    use crate::codegen::builtins::audio::gen_macos_callbacks::{
        lower_audio_input_callback, lower_audio_output_callback,
    };
    let output = refuses_each_missing_import_on(
        &[CodeTarget::MacosAarch64],
        RECORDS_AND_PLAYS,
        "audio output callback",
        &|imports, platform| lower_audio_output_callback(imports, platform).map(|f| vec![f]),
    );
    let input = refuses_each_missing_import_on(
        &[CodeTarget::MacosAarch64],
        RECORDS_AND_PLAYS,
        "audio input callback",
        &|imports, platform| lower_audio_input_callback(imports, platform).map(|f| vec![f]),
    );
    // Lock, signal and unlock; the input callback also re-enqueues its buffer.
    assert_eq!(output, 3, "the output callback's three OS calls");
    assert_eq!(input, 4, "the input callback's four OS calls");
}
