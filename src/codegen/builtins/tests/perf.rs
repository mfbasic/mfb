//! Codegen contracts for the runtime perf-tracking helpers (`perf.rs`).
//!
//! These four bodies are emitted only for a `--debug` build of a macOS entry (the
//! `perf` section of the debug report), so no ordinary whole-program lowering
//! reaches them; `a_normal_build_emits_no_perf_symbol` below pins both halves.
//! They are lowered here directly, against the real
//! [`crate::target::macos_aarch64::code::Platform`] — a stub would only be
//! asserting against this file's own approximation of `emit_arena_map`,
//! `emit_external_call` and `emit_write`.
//!
//! The rules pinned below are the ones whose violation is *quiet*. A profiler
//! that miscounts is a nuisance; a profiler that recurses into the allocator it
//! is timing, or that crashes a program because its own `mmap` failed, or that
//! silently drops samples once its region fills, is a defect that presents as
//! something else entirely.

use std::collections::HashMap;

use crate::arch::ops::CodeOp;
use crate::codegen::builtins::perf::perf::lower_perf_helper;
use crate::codegen::engine::mir;
use crate::codegen::engine::tests::test_support::{Stream, TestPlatform};
use crate::codegen::engine::types::{CodeInstruction, CodegenPlatform};
use crate::target::macos_aarch64::code::Platform as MacosPlatform;

/// Every helper `lower_perf_helper` dispatches, in injection order.
const HELPERS: [&str; 4] = ["perf.init", "perf.start", "perf.done", "perf.end"];

/// The imports the perf bodies resolve against.
///
/// `emit_external_call` refuses a symbol the platform import list does not
/// declare, which is the same rule a real build follows — the plan force-adds
/// these when perf injection is on (`target::shared::plan::symbols`).
fn imports() -> HashMap<String, String> {
    // Keyed by the MANGLED name the emitter asks for (`_clock_gettime`), which
    // is what a macOS plan's import list carries.
    ["_clock_gettime", "_write"]
        .into_iter()
        .map(|symbol| (symbol.to_string(), "libSystem".to_string()))
        .collect()
}

fn lower(call: &str, platform: &dyn CodegenPlatform) -> Vec<CodeInstruction> {
    mir::set_backend(platform.backend());
    let symbol = format!("_mfb_rt_{}", call.replace('.', "_"));
    // `.unwrap()` rather than `unwrap_or_else(|e| panic!(...))`: the closure is
    // a coverage region that a green run never enters, and `Result::unwrap`
    // needs `Debug` only on the ERROR type, which is `String`. The lowering
    // error text names the helper anyway.
    let (_frame, instructions, _relocations, _slots) =
        lower_perf_helper(call, &symbol, &imports(), platform).unwrap();
    instructions
}

fn macos(call: &str) -> Vec<CodeInstruction> {
    lower(call, &MacosPlatform)
}

fn stream<'a>(name: &'a str, instructions: &'a [CodeInstruction]) -> Stream<'a> {
    Stream { name, instructions }
}

/// No perf body may touch the arena.
///
/// This is the file's own load-bearing invariant: plan-67-F wraps the arena hot
/// path itself with `perf.start`/`perf.end`, so a perf body that reached the
/// arena would recurse perf → arena → perf. The recursion is unbounded and shows
/// up as a stack overflow inside the allocator, nowhere near this file — and only
/// in a `--debug` build, which no ordinary golden exercises.
#[test]
fn no_perf_body_calls_an_arena_helper() {
    for call in HELPERS {
        let instructions = macos(call);
        let calls = stream(call, &instructions).calls_between(0, instructions.len());
        let arena: Vec<&String> = calls.iter().filter(|t| t.contains("arena")).collect();
        assert!(
            arena.is_empty(),
            "{call} must be arena-free (plan-67-F wraps the arena with it, so a \
             call here recurses perf -> arena -> perf); it calls {arena:?}"
        );
    }
}

/// A perf region that failed to map leaves the profiler inert, not broken.
///
/// `perf.init` normalizes an `mmap` failure to a negative return and must leave
/// the global at 0; every other body opens with "load the base, and if it is 0,
/// return". Without that pair a profiler that could not get its 16 MiB would
/// dereference null in a program that merely asked to be profiled.
#[test]
fn a_failed_region_map_leaves_every_perf_body_inert() {
    let init = macos("perf.init");
    let s = stream("perf.init", &init);
    let store = s.index_of("the store of the region base into the perf global", |i| {
        i.op == CodeOp::StrU64
    });
    let bail = s.index_of("the mmap-failure branch", |i| i.op == CodeOp::BranchLt);
    assert!(
        bail < store,
        "perf.init must test the mmap result BEFORE storing it, so a failed map \
         leaves the global at 0 (branch at {bail}, store at {store})"
    );

    for call in ["perf.start", "perf.done", "perf.end"] {
        let instructions = macos(call);
        let s = stream(call, &instructions);
        let load = s.index_of("the load of the perf region base", |i| {
            i.op == CodeOp::LdrU64
        });
        let guard = s.index_after(load, "the null-base guard", |i| {
            i.op == CodeOp::CmpImm && Stream::field(i, "rhs") == "0"
        });
        let bail = s.index_after(guard, "the branch over the body on a null base", |i| {
            i.op == CodeOp::BranchEq
        });
        assert!(
            bail - guard <= 1,
            "{call} must branch out immediately on a zero region base; the compare \
             at {guard} is not followed by its branch (found it at {bail})"
        );
    }
}

/// Table B is keyed by pointer identity — no hash, no byte compare, no key copy.
///
/// Every injection of a name loads the one data object emitted for it, so
/// identical names arrive as the identical pointer and an equality compare is
/// exact. Reaching for a string compare here would be a correctness *and* a
/// re-entrancy bug: the comparison helpers are not arena-free.
#[test]
fn the_name_tables_compare_pointers_and_never_call_out_to_compare() {
    for call in ["perf.start", "perf.end"] {
        let instructions = macos(call);
        let s = stream(call, &instructions);
        assert!(
            instructions
                .iter()
                .any(|i| i.op == CodeOp::Cmp && !Stream::field(i, "rhs").is_empty()),
            "{call} must compare the stored key against the incoming pointer"
        );
        // A membership test, not `a || b`: the right operand of a `||` whose left
        // side always holds is never evaluated, and an unevaluated operand is an
        // uncovered region in this very file.
        let permitted = ["_clock_gettime", "_write"];
        for target in s.calls_between(0, instructions.len()) {
            assert!(
                permitted.contains(&target.as_str()),
                "{call} may only call the clock and the writer; it calls `{target}` \
                 (a key compare or copy helper would break both the pointer-identity \
                 key and the arena-free rule)"
            );
        }
    }
}

/// A full region bumps a visible counter — it never silently drops the sample.
///
/// `perf.end` has three counters that exist only to make loss visible: the log
/// `overflow`, the `mismatch` for an end with no open start, and table A's own
/// capacity stop. A profiler that quietly capped would report a plausible,
/// wrong distribution, which is worse than reporting nothing.
#[test]
fn perf_end_counts_every_way_a_sample_can_be_lost() {
    let instructions = macos("perf.end");
    let s = stream("perf.end", &instructions);
    // Each counter is a load / add 1 / store back at its own header offset.
    let bumped: Vec<String> = instructions
        .windows(3)
        .filter(|w| {
            w[0].op == CodeOp::LdrU64
                && w[1].op == CodeOp::AddImm
                && Stream::field(&w[1], "imm") == "1"
                && w[2].op == CodeOp::StrU64
                && Stream::field(&w[2], "base") == Stream::field(&w[0], "base")
                && Stream::field(&w[2], "offset") == Stream::field(&w[0], "offset")
        })
        .map(|w| Stream::field(&w[0], "offset"))
        .collect();
    for (offset, what) in [
        ("24", "mismatch (an end with no open start)"),
        ("32", "overflow (the sample log is full)"),
    ] {
        assert!(
            bumped.iter().any(|o| o == offset),
            "perf.end must bump the {what} counter at header offset {offset}; \
             it bumps {bumped:?}"
        );
    }
    assert!(
        s.labels().iter().any(|(_, n)| n.ends_with("_ov")),
        "perf.end must have a distinct overflow exit"
    );
    assert!(
        s.labels().iter().any(|(_, n)| n.ends_with("_mm")),
        "perf.end must have a distinct mismatch exit"
    );
}

/// Off macOS every helper lowers to a bare return.
///
/// The dispatch is deliberately total so that a perf symbol referenced on a
/// non-macOS target produces an inert body rather than an error or wrong-ISA
/// code. Nothing references them there today, which is exactly why a regression
/// would go unseen.
#[test]
fn off_macos_every_perf_helper_is_an_inert_body() {
    for call in HELPERS {
        let instructions = lower(call, &TestPlatform);
        let ops: Vec<CodeOp> = instructions.iter().map(|i| i.op).collect();
        // A frame prologue/epilogue is unavoidable (`finalize_vreg_body_with_locals`
        // runs for every arm); what must be absent is any body at all.
        let frame = [CodeOp::Label, CodeOp::Ret, CodeOp::SubSp, CodeOp::AddSp];
        let body: Vec<CodeOp> = ops
            .iter()
            .copied()
            .filter(|op| !frame.contains(op))
            .collect();
        assert!(
            body.is_empty(),
            "{call} on a non-macOS platform must lower to a bare frame and `ret`; \
             it also emitted {body:?}"
        );
    }
}

/// An unknown perf call is rejected, not silently lowered to a return.
///
/// The macOS arm's `other =>` is the only place that can catch a mistyped
/// injection name; without it a wrong name would emit an empty body and the span
/// it was supposed to time would simply never appear in the report.
#[test]
fn an_unknown_perf_call_is_an_error_on_macos() {
    mir::set_backend(MacosPlatform.backend());
    // `.err()` rather than a `let ... else`: the else arm is a region a green run
    // never enters. `HelperBody` has no `Debug`, so `expect_err` is unavailable.
    let err = lower_perf_helper("perf.nope", "_mfb_rt_perf_nope", &imports(), &MacosPlatform)
        .err()
        .unwrap_or_default();
    assert!(
        err.contains("perf.nope"),
        "an unknown perf call must be rejected by name, so a mistyped injection \
         cannot lower to an empty body and silently drop the span it was meant to \
         time; got {err:?}"
    );
}

/// Every body refuses to emit a call the platform import list does not declare.
///
/// A perf helper reaches libc twice — `clock_gettime` for the timestamp and
/// `write` for the report — and `emit_external_call`/`emit_write` fail closed
/// when the plan has not declared the symbol. That failure is the only thing
/// standing between a mis-specified plan and an executable carrying a call to a
/// symbol nothing imports; the perf symbols are force-added by
/// `target::shared::plan::symbols` precisely so it never fires in a real build,
/// which is why nothing else exercises it.
#[test]
fn a_perf_body_refuses_to_call_an_undeclared_import() {
    let none = HashMap::new();
    mir::set_backend(MacosPlatform.backend());
    for call in ["perf.start", "perf.done", "perf.end"] {
        let symbol = format!("_mfb_rt_{}", call.replace('.', "_"));
        let err = lower_perf_helper(call, &symbol, &none, &MacosPlatform)
            .err()
            .unwrap_or_default();
        assert!(
            err.contains("import"),
            "{call} must refuse to lower when the platform declares no libc \
             import for it, rather than emitting a call nothing resolves; got {err:?}"
        );
    }
    // `perf.init` is the exception and must NOT fail: it only mmaps, through the
    // platform's own syscall seam, so it needs no import at all. A program whose
    // profiler could not be initialized still has to build.
    assert!(
        lower_perf_helper("perf.init", "_mfb_rt_perf_init", &none, &MacosPlatform).is_ok(),
        "perf.init reaches libc for nothing and must lower with no imports declared"
    );
}

/// Every symbol a code plan defines or references: data objects, functions, and
/// relocation targets.
fn plan_names(plan: &crate::codegen::engine::types::NativeCodePlan) -> Vec<String> {
    let mut names: Vec<String> = plan
        .data_objects
        .iter()
        .map(|object| object.symbol.clone())
        .collect();
    for function in &plan.functions {
        names.push(function.symbol.clone());
        names.extend(function.relocations.iter().map(|r| r.to.clone()));
    }
    names
}

/// plan-130-B: perf is a `--debug` feature, not a compiler build flag.
///
/// A normal macOS build emits no perf helper, region object, or injected call, so
/// the timings cannot leak into an ordinary program. The same program lowered with
/// `--debug` carries all four helpers, the region state, and the `perf` report
/// section — so turning them on needs no special compiler — while a `--debug`
/// build for any other target still carries none (the helpers are macOS-only).
#[test]
fn a_normal_build_emits_no_perf_symbol() {
    use crate::codegen::debug::DebugOptions;
    use crate::target::shared::runtime::{symbol_for_call, RuntimeHelper};
    use crate::target::NativeBuildMode;
    use crate::testutil::CodeTarget;

    const SOURCE: &str = "FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n";
    let is_perf = |name: &String| name.starts_with("_mfb_rt_perf_") || name.contains("report_perf");

    let normal = crate::testutil::code_for_src_on(SOURCE, CodeTarget::MacosAarch64);
    let leaked: Vec<String> = plan_names(&normal).into_iter().filter(is_perf).collect();
    assert!(
        leaked.is_empty(),
        "a build without --debug must emit nothing of perf, found {leaked:?}"
    );

    let lower_debug = |target: CodeTarget| {
        let mut module =
            crate::testutil::nir_for_src(SOURCE, target, NativeBuildMode::Console).unwrap();
        module.debug = DebugOptions { enabled: true };
        plan_names(&crate::testutil::code_for_nir(&module, target).unwrap())
    };

    let macos = lower_debug(CodeTarget::MacosAarch64);
    for call in HELPERS {
        let symbol = symbol_for_call(RuntimeHelper::Perf, call);
        assert!(
            macos.contains(&symbol),
            "a macOS --debug build must emit {symbol}"
        );
    }
    for expected in ["_mfb_rt_perf_state", "_mfb_debug_report_perf"] {
        assert!(
            macos.iter().any(|name| name == expected),
            "a macOS --debug build must carry {expected}"
        );
    }

    let linux = lower_debug(CodeTarget::LinuxAarch64);
    let linux_perf: Vec<String> = linux.into_iter().filter(is_perf).collect();
    assert!(
        linux_perf.is_empty(),
        "perf is macOS-only; a linux-aarch64 --debug build emitted {linux_perf:?}"
    );
}
