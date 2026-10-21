//! The `--debug` report is emitted completely on every backend.
//!
//! `codegen/debug/` builds the stderr report `_mfb_shutdown` writes as its last
//! act: a core section, a perf section (a clock read), an arena section, a process
//! section (peak resident size) and, for a program that starts a thread, a thread
//! section. `debug/tests.rs` pins the helpers' contracts against ONE backend
//! (macOS), and the runtime test spawns `mfb`, so the per-OS arms — the Windows
//! `QueryPerformanceCounter` clock and `K32GetProcessMemoryInfo` peak, the arena
//! registry's unregister at thread teardown, the thread section's registry hooks —
//! were reached only on the host that happens to run them.
//!
//! What is asserted is what the report needs from the whole plan: the shutdown
//! helper and every active section's report helper exist, every data object the
//! debug code addresses is defined, and the OS facility each section reads is
//! IMPORTED on that backend. A section whose helper lowered but whose import the
//! plan never declared fails at link time on exactly one OS.

use crate::codegen::debug::{active_features, DebugOptions, DEBUG_SHUTDOWN_SYMBOL};
use crate::codegen::engine::types::NativeCodePlan;
use crate::target::shared::nir::NirModule;
use crate::target::NativeBuildMode;
use crate::testutil::{code_for_nir, nir_for_src, CodeTarget};

const PLAIN: &str = "FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n";

/// A program that starts a thread, so the report carries the thread section and
/// the worker's arena registers with (and unregisters from) the arena registry.
const THREADED: &str = "\
IMPORT thread

ISOLATED FUNC work(w AS ThreadWorker OF Integer TO Integer, n AS Integer) AS Integer
  RETURN n
END FUNC

FUNC main() AS Integer
  RES t AS Thread OF Integer TO Integer = thread::start(work, 1)
  RETURN thread::waitFor(t)
END FUNC
";

/// An app-mode program: the core section names the build `app`, not `console`.
const APP: &str = "\
IMPORT app

FUNC main() AS Integer
  app::setMode(app::Mode.Canvas)
  RETURN 0
END FUNC
";

fn lowered(source: &str, target: CodeTarget, mode: NativeBuildMode) -> (NirModule, NativeCodePlan) {
    let mut module = nir_for_src(source, target, mode)
        .unwrap_or_else(|err| panic!("{} NIR: {err}", target.name()));
    module.debug = DebugOptions { enabled: true };
    let plan = code_for_nir(&module, target)
        .unwrap_or_else(|err| panic!("a --debug build must lower on {}: {err}", target.name()));
    (module, plan)
}

/// The OS imports the report's sections read, by backend family: a clock for the
/// perf section's timings and a peak-resident-size query for the process section.
fn expected_imports(target: CodeTarget) -> &'static [&'static str] {
    match target {
        CodeTarget::WindowsX86_64 => &[
            "QueryPerformanceCounter",
            "QueryPerformanceFrequency",
            "GetCurrentProcess",
            "K32GetProcessMemoryInfo",
        ],
        _ => &["clock_gettime", "getrusage"],
    }
}

/// The imports whose absence the aggregate lowering must refuse: the OS facilities
/// the sections read, plus macOS's `write`, the one platform whose line writer is an
/// import the shutdown helper itself resolves (Linux writes by syscall).
fn refused_without(target: CodeTarget) -> Vec<&'static str> {
    let mut wanted = expected_imports(target).to_vec();
    if target == CodeTarget::MacosAarch64 {
        wanted.push("_write");
    }
    wanted
}

/// The OS calls one section makes itself (not through the shared line writer).
fn section_imports(target: CodeTarget, section: &str) -> &'static [&'static str] {
    match (target, section) {
        (CodeTarget::WindowsX86_64, "process") => &["GetCurrentProcess", "K32GetProcessMemoryInfo"],
        (CodeTarget::MacosAarch64, "process") => &["getrusage", "_write"],
        (CodeTarget::MacosAarch64, "thread") => &["_write"],
        (_, "process") => &["getrusage"],
        _ => &[],
    }
}

fn assert_complete_report(
    module: &NirModule,
    plan: &NativeCodePlan,
    target: CodeTarget,
    what: &str,
) {
    let name = target.name();
    let symbols: Vec<&str> = plan.functions.iter().map(|f| f.symbol.as_str()).collect();
    assert!(
        symbols.contains(&DEBUG_SHUTDOWN_SYMBOL),
        "{what} on {name}: a --debug plan must define {DEBUG_SHUTDOWN_SYMBOL}"
    );
    for feature in active_features(module) {
        let Some(report) = feature.report_symbol() else {
            continue;
        };
        assert!(
            symbols.contains(&report),
            "{what} on {name}: section `{}` must define {report}",
            feature.name()
        );
    }
    let data: Vec<&str> = plan
        .data_objects
        .iter()
        .map(|d| d.symbol.as_str())
        .collect();
    for function in plan
        .functions
        .iter()
        .filter(|f| f.symbol.starts_with("_mfb_debug"))
    {
        for relocation in function.relocations.iter().filter(|r| r.binding == "data") {
            assert!(
                data.contains(&relocation.to.as_str()),
                "{what} on {name}: {} addresses {} which no data object defines",
                function.symbol,
                relocation.to
            );
        }
    }
    for want in expected_imports(target) {
        assert!(
            plan.imports
                .iter()
                .any(|import| import.symbol.contains(want)),
            "{what} on {name}: the report reads the OS through `{want}`, which the plan \
             never imports; it imports {:?}",
            plan.imports.iter().map(|i| &i.symbol).collect::<Vec<_>>()
        );
    }
}

/// A console `--debug` build carries the whole report on every backend.
#[test]
fn every_backend_carries_the_whole_debug_report() {
    for target in CodeTarget::ALL {
        let (module, plan) = lowered(PLAIN, target, NativeBuildMode::Console);
        assert_complete_report(&module, &plan, target, "plain program");
    }
}

/// A threaded program adds the thread section and the arena registry's
/// register/unregister pair for the worker's arena, on every backend.
#[test]
fn a_threaded_program_adds_the_thread_section_on_every_backend() {
    for target in CodeTarget::ALL {
        let (module, plan) = lowered(THREADED, target, NativeBuildMode::Console);
        assert_complete_report(&module, &plan, target, "threaded program");
        assert!(
            plan.functions
                .iter()
                .any(|f| f.symbol == "_mfb_debug_report_thread"),
            "threaded program on {}: the report must include the thread section",
            target.name()
        );
    }
}

/// An app-mode build reports its build as `app`, on every backend with an app mode.
#[test]
fn an_app_build_lowers_a_complete_report() {
    for target in CodeTarget::ALL {
        let Some(mode) = target.app_mode() else {
            continue;
        };
        let (module, plan) = lowered(APP, target, mode);
        assert_complete_report(&module, &plan, target, "app program");
    }
}

/// The real code-generation platform for `target`, as `registry_bodies` builds them.
fn platform_for(target: CodeTarget) -> Box<dyn crate::codegen::engine::types::CodegenPlatform> {
    use crate::os::linux::flavor::LinuxFlavor::Glibc;
    use crate::target::linux_common::code::Platform as Linux;
    match target {
        CodeTarget::MacosAarch64 => Box::new(crate::target::macos_aarch64::code::Platform),
        CodeTarget::WindowsX86_64 => Box::new(crate::target::win_x86_64::code::Platform),
        CodeTarget::LinuxAarch64 => Box::new(Linux::for_test(
            crate::target::linux_aarch64::code::Aarch64,
            Glibc,
        )),
        CodeTarget::LinuxX86_64 => Box::new(Linux::for_test(
            crate::target::linux_x86_64::code::X86_64,
            Glibc,
        )),
        CodeTarget::LinuxRiscv64 => Box::new(Linux::for_test(
            crate::target::linux_riscv64::code::Riscv64,
            Glibc,
        )),
    }
}

/// A report section whose OS facility the plan did not import is REFUSED, naming the
/// facility, rather than lowered into a call the linker cannot resolve.
///
/// The plan derives its import list from what each section declares it calls
/// (`DebugFeature::os_imports`); the helpers resolve every call against that list as
/// they emit, so a section that grew a call without declaring it fails here, on the
/// backend whose OS symbol it forgot, instead of at link time on one machine.
#[test]
fn a_debug_section_refuses_an_os_call_its_plan_did_not_import() {
    for target in CodeTarget::ALL {
        let (module, plan) = lowered(PLAIN, target, NativeBuildMode::Console);
        let platform = platform_for(target);
        for want in refused_without(target) {
            let imports: std::collections::HashMap<String, String> = plan
                .imports
                .iter()
                .filter(|import| !import.symbol.contains(want))
                .map(|import| (import.symbol.clone(), import.library.clone()))
                .collect();
            crate::codegen::engine::mir::set_backend(platform.backend());
            let err = match crate::codegen::debug::code_functions(&module, &imports, &*platform) {
                Ok(_) => panic!(
                    "{}: lowering the report without `{want}` imported must be refused",
                    target.name()
                ),
                Err(err) => err,
            };
            assert!(
                err.contains(want),
                "{}: the refusal must name the missing import `{want}`; it said {err}",
                target.name()
            );
        }
    }
}

/// Each section, lowered alone, refuses exactly the imports it calls.
///
/// The aggregate lowering above stops at the first section that cannot lower, so it
/// can only prove the FIRST section's imports. Here every active section is lowered
/// by itself with each import of the plan taken away in turn: a refusal must name
/// the import that was removed (never fail for another reason), the process section
/// must refuse the calls that read its peak resident size.
#[test]
fn each_debug_section_refuses_the_imports_it_calls() {
    for target in CodeTarget::ALL {
        let (module, plan) = lowered(THREADED, target, NativeBuildMode::Console);
        let platform = platform_for(target);
        crate::codegen::engine::mir::set_backend(platform.backend());
        for import in &plan.imports {
            let imports: std::collections::HashMap<String, String> = plan
                .imports
                .iter()
                .filter(|other| other.symbol != import.symbol)
                .map(|other| (other.symbol.clone(), other.library.clone()))
                .collect();
            for feature in active_features(&module) {
                let refused = match feature.code_functions(&module, &imports, &*platform) {
                    Ok(_) => false,
                    Err(err) => {
                        assert!(
                            err.contains(&import.symbol),
                            "{} section on {}: lowering without `{}` failed for another \
                             reason: {err}",
                            feature.name(),
                            target.name(),
                            import.symbol
                        );
                        true
                    }
                };
                if let Some(wanted) = section_imports(target, feature.name())
                    .iter()
                    .find(|want| import.symbol.contains(*want))
                {
                    assert!(
                        refused,
                        "{} section on {}: the call through `{wanted}` must be refused when it \
                         is not imported",
                        feature.name(),
                        target.name()
                    );
                }
            }
        }
    }
}

/// A canvas program: the graphics thread owns an arena of its own.
const CANVAS: &str = "\
IMPORT app
IMPORT canvas
IMPORT io

FUNC main() AS Integer
  app::setMode(app::Mode.Canvas)
  LET size = canvas::getSize()
  LET word = canvas::profileWord(0)
  io::print(toString(word))
  RETURN 0
END FUNC
";

/// The same program without the internal profile read, which only a `--debug` build can
/// lower (`canvas::profileWord` names the profile block, which a normal build omits).
const CANVAS_PLAIN: &str = "\
IMPORT app
IMPORT canvas
IMPORT io

FUNC main() AS Integer
  app::setMode(app::Mode.Canvas)
  LET size = canvas::getSize()
  io::print(\"x\")
  RETURN 0
END FUNC
";

/// A `--debug` canvas build carries the render profile block and registers the graphics
/// thread's arena with the arena registry; the same program without `--debug` carries
/// neither.
#[test]
fn a_debug_canvas_build_profiles_the_renderer_and_registers_the_graphics_arena() {
    use crate::codegen::debug::DEBUG_ARENA_REGISTER_SYMBOL;
    // The profile block's symbol (`runtime::canvas::entry::CANVAS_PROFILE_SYMBOL`).
    const CANVAS_PROFILE_SYMBOL: &str = "_mfb_rt_canvas_profile";
    // Two backends, not five: lowering a canvas program injects the whole canvas
    // package, which is the slow part of this suite.
    for target in [CodeTarget::MacosAarch64, CodeTarget::LinuxX86_64] {
        let mode = target.app_mode().expect("both have an app mode");
        let (_, debug) = lowered(CANVAS, target, mode);
        let registers = |plan: &NativeCodePlan| {
            plan.functions.iter().any(|function| {
                function
                    .relocations
                    .iter()
                    .any(|relocation| relocation.to == DEBUG_ARENA_REGISTER_SYMBOL)
            })
        };
        assert!(
            debug
                .data_objects
                .iter()
                .any(|object| object.symbol == CANVAS_PROFILE_SYMBOL),
            "{}: a --debug canvas build carries the profile block",
            target.name()
        );
        assert!(
            registers(&debug),
            "{}: the graphics thread's arena is registered with the --debug registry",
            target.name()
        );
    }
    let normal = crate::testutil::app_code_cached(CANVAS_PLAIN, CodeTarget::LinuxX86_64);
    assert!(
        !normal
            .data_objects
            .iter()
            .any(|object| object.symbol == CANVAS_PROFILE_SYMBOL)
            && !normal.functions.iter().any(|function| function
                .relocations
                .iter()
                .any(|relocation| relocation.to == DEBUG_ARENA_REGISTER_SYMBOL)),
        "a build without --debug carries neither"
    );
}
