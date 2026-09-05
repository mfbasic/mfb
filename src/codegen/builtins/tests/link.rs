//! The native `LINK` thunk, lowered in process for every backend.
//!
//! `codegen/link/thunk/link_thunk.rs` is 2,277 lines and emits everything a
//! `LINK "sqlite3" AS sql` block turns into: the per-library `dlopen` of a
//! *resolved* filename, the per-symbol `dlsym`, the marshalling of each `ABI`
//! parameter, the `CSTRUCT` field packing, the `RES` handle records and their
//! close ops, and the `FREE` callbacks. Nothing in process reached a line of it,
//! because a `LINK` program cannot even be lowered without a native-library
//! table: resolution comes from the project's `libraries` manifest section, and
//! a logical name with no locator is a hard build error.
//!
//! `testutil::code_for_linking_src` supplies that table directly — a `System`
//! locator per library, which is what a `"type": "system"` manifest entry
//! produces — so the committed `rt-behavior/native` fixtures lower here.
//!
//! What is asserted is what the acceptance matrix cannot say cheaply: that the
//! same declaration produces a *complete* thunk on all five backends. A dangling
//! `dlsym` or a library the plan never opens is a runtime `NULL` call, and the
//! fixture that would catch it only runs where its library exists.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::tests::test_support::Stream;
use crate::codegen::engine::types::NativeCodePlan;
use crate::testutil::{code_for_linking_src, fixture_src, try_code_for_linking_src, CodeTarget};

/// `(fixture, libraries it LINKs)` — every one of which lowers on all five
/// backends.
const LINKING: &[(&str, &[&str])] = &[
    ("native-link-const-64bit-rt", &["sqlite3"]),
    ("native-private-resource-rt", &["sqlite3"]),
    ("native-struct-scalar-rt", &["c"]),
    ("native-cbuffer-read-rt", &["c"]),
];

/// The fixture whose `ABI` block needs five integer argument registers, which
/// Win64 does not have. Kept out of the all-backends sweep and given its own
/// case below, because "this target cannot" is a contract too.
const OVER_WIN64_ARGUMENT_REGISTERS: &[(&str, &[&str])] = &[("native-link-free-rt", &["sqlite3"])];

fn lowered(fixture: &str, libraries: &[&str], target: CodeTarget) -> NativeCodePlan {
    code_for_linking_src(&fixture_src(fixture), target, libraries)
}

/// A `cstring_object`'s hex-encoded, NUL-terminated bytes, as text.
fn decode_cstring(value: &str) -> Option<String> {
    let bytes: Vec<u8> = value
        .as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    let text = bytes.strip_suffix(&[0])?;
    String::from_utf8(text.to_vec()).ok()
}

/// Every `LINK` program lowers on every backend, with a loader call per library.
///
/// A backend that could not emit the thunk at all would be caught by the
/// acceptance matrix; a backend that emitted a thunk which never opens the
/// library would not, because the program only fails at run time, on a machine
/// that has that library.
#[test]
fn every_backend_emits_a_complete_thunk_for_every_link_program() {
    for (fixture, libraries) in LINKING {
        for target in CodeTarget::ALL {
            let plan = lowered(fixture, libraries, target);
            let calls: Vec<String> = plan
                .functions
                .iter()
                .flat_map(|f| {
                    let stream = Stream::of(f);
                    stream.calls_between(0, f.instructions.len())
                })
                .collect();
            // POSIX opens with `dlopen`/`dlsym`; Windows with LoadLibrary /
            // GetProcAddress. Either way the thunk must ask the loader twice:
            // once for the library and once per symbol.
            let opens = calls
                .iter()
                .filter(|t| t.contains("dlopen") || t.contains("LoadLibrary"))
                .count();
            let looks_up = calls
                .iter()
                .filter(|t| t.contains("dlsym") || t.contains("GetProcAddress"))
                .count();
            assert!(
                opens > 0,
                "{fixture} on {}: the thunk must open the library it links; it \
                 called {calls:?}",
                target.name()
            );
            assert!(
                looks_up > 0,
                "{fixture} on {}: the thunk must resolve the symbols it declares",
                target.name()
            );
        }
    }
}

/// The resolved filename is a data object, not the logical name.
///
/// `plan-46-C`: the `dlopen` filename is the binding author's declared `source`
/// for this build's exact (os, arch, libc), resolved from the locator table.
/// Synthesizing a soname from the logical name — `lib{logical}.so.0` — does not
/// consult the manifest and misses every unversioned `.so`, `.so.3`,
/// non-`lib`-prefixed, and per-arch variant, which is why the emitter's own
/// comment forbids reintroducing it. The evidence is that the emitted data
/// carries the RESOLVED name.
#[test]
fn the_thunk_opens_the_resolved_filename_not_the_logical_name() {
    for (fixture, libraries) in LINKING {
        let plan = lowered(fixture, libraries, CodeTarget::LinuxX86_64);
        for library in *libraries {
            let resolved = format!("lib{library}.so");
            // The object is a hex-encoded C string (`cstring_object`), so the
            // filename is compared after decoding rather than by substring --
            // matching the hex would pass for a name that merely contains it.
            let carried = plan
                .data_objects
                .iter()
                .filter(|object| object.symbol.starts_with("_mfb_linker_lib_"))
                .filter_map(|object| decode_cstring(&object.value))
                .any(|name| name == resolved);
            assert!(
                carried,
                "{fixture}: the thunk must carry the RESOLVED filename \
                 `{resolved}` as its `_mfb_linker_lib_*` data object; it carries \
                 {:?}",
                plan.data_objects
                    .iter()
                    .filter(|o| o.symbol.starts_with("_mfb_linker_lib_"))
                    .filter_map(|o| decode_cstring(&o.value))
                    .collect::<Vec<_>>()
            );
        }
    }
}

/// A failed `dlopen`/`dlsym` is branched on, never used.
///
/// The loader returns NULL for a library that is not installed or a symbol that
/// is not exported, and both are ordinary on a user's machine. A thunk that
/// called through the result without testing it would segfault instead of
/// raising, and no fixture can show that on a machine where the library IS
/// present — which is every machine the fixture runs on.
#[test]
fn every_loader_result_is_tested_before_it_is_called() {
    for (fixture, libraries) in LINKING {
        let plan = lowered(fixture, libraries, CodeTarget::LinuxX86_64);
        for function in &plan.functions {
            let stream = Stream::of(function);
            for (at, instruction) in function.instructions.iter().enumerate() {
                let target = instruction.get("target").unwrap_or_default();
                if instruction.op != CodeOp::BranchLink
                    || !(target.contains("dlopen") || target.contains("dlsym"))
                {
                    continue;
                }
                // Within a short window after the call, the result must be
                // compared and branched on. The window is generous because the
                // backend may park the result first.
                let end = (at + 12).min(function.instructions.len());
                let tested = function.instructions[at + 1..end]
                    .iter()
                    .any(|i| matches!(i.op, CodeOp::CmpImm | CodeOp::Cmp | CodeOp::RvBr));
                assert!(
                    tested,
                    "{fixture}: `{}` calls `{target}` at {at} and does not test the \
                     result before using it — a missing library or symbol would be \
                     called through as NULL. Stream: {:?}",
                    stream.name,
                    &function.instructions[at..end]
                        .iter()
                        .map(|i| i.op)
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}

/// A declaration needing more argument registers than the target has is
/// REJECTED, by name, at compile time.
///
/// Win64 passes four integer arguments in registers and the thunk emitter does
/// not stage the rest on the stack. The alternative to this error is silently
/// dropping the fifth argument, which the callee would read as whatever was left
/// in the register - a wrong value handed to C with no diagnostic anywhere. The
/// message must also name the function, because a project with a dozen `ABI`
/// blocks cannot act on "some declaration is too wide".
#[test]
fn a_declaration_too_wide_for_the_target_is_rejected_by_name() {
    for (fixture, libraries) in OVER_WIN64_ARGUMENT_REGISTERS {
        let source = fixture_src(fixture);

        // It lowers everywhere the registers exist...
        for target in [
            CodeTarget::MacosAarch64,
            CodeTarget::LinuxAarch64,
            CodeTarget::LinuxX86_64,
            CodeTarget::LinuxRiscv64,
        ] {
            let plan = try_code_for_linking_src(&source, target, libraries)
                .unwrap_or_else(|err| panic!("{fixture} must lower on {}: {err}", target.name()));
            assert!(!plan.functions.is_empty());
        }

        // ...and is refused, with the function named, where they do not.
        let refusal = try_code_for_linking_src(&source, CodeTarget::WindowsX86_64, libraries)
            .err()
            .unwrap_or_default();
        for want in ["integer ABI slots", "argument registers"] {
            assert!(
                refusal.contains(want),
                "{fixture}: the Win64 rejection must mention {want:?} so the author \
                 knows what to narrow; it said {refusal:?}"
            );
        }
        assert!(
            refusal.contains("native function `"),
            "{fixture}: the Win64 rejection must NAME the declaration; it said \
             {refusal:?}"
        );
    }
}
