//! Native plans, one stage below the code plan: the imports each backend declares.
//!
//! The import list of a plan is a contract with the linker -- an import a
//! lowering needs but the plan omits is an unresolved symbol at link time; an
//! import it declares and nothing uses is dead weight in every binary. These
//! tests drive `lower_module` for each backend from a NIR module, with the module
//! shaped to reach the branch under test (no entry, `--debug`, app mode).

use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::{NativePlan, PlatformImport};
use crate::target::NativeBuildMode;
use crate::testutil::{nir_for_src, CodeTarget};

pub(super) const HELLO: &str = "\
IMPORT io

FUNC main() AS Integer
  io::print(\"hi\")
  RETURN 0
END FUNC
";

const TERM_MOUSE: &str = "\
IMPORT term

FUNC main() AS Integer
  term::enableMouse(TRUE)
  LET event AS term::MouseEvent = term::pollMouse()
  RETURN 0
END FUNC
";

const FS_SEEK: &str = "\
IMPORT fs

FUNC main() AS Integer
  RES f = fs::openFile(\"x.txt\")
  fs::seek(f, 7)
  fs::close(f)
  RETURN 0
END FUNC
";

const CANVAS_MOUSE: &str = "\
IMPORT app
IMPORT canvas

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  canvas::enableMouse(TRUE)
  canvas::present([])
  LET event AS canvas::MouseEvent = canvas::pollMouse()
END FUNC
";

pub(super) fn module_for(source: &str, target: CodeTarget, mode: NativeBuildMode) -> NirModule {
    nir_for_src(source, target, mode).expect("the program lowers to NIR")
}

pub(super) fn plan_of(module: &NirModule, target: CodeTarget) -> Result<NativePlan, String> {
    use crate::os::linux::flavor::LinuxFlavor::Glibc;
    match target {
        CodeTarget::MacosAarch64 => crate::target::macos_aarch64::plan::lower_module(module),
        CodeTarget::LinuxAarch64 => crate::target::linux_aarch64::plan::lower_module(module, Glibc),
        CodeTarget::LinuxX86_64 => crate::target::linux_x86_64::plan::lower_module(module, Glibc),
        CodeTarget::LinuxRiscv64 => crate::target::linux_riscv64::plan::lower_module(module, Glibc),
        CodeTarget::WindowsX86_64 => crate::target::win_x86_64::plan::lower_module(module),
    }
}

pub(super) fn symbols<'a>(imports: &'a [PlatformImport], required_by: &str) -> Vec<&'a str> {
    imports
        .iter()
        .filter(|import| import.required_by == required_by)
        .map(|import| import.symbol.as_str())
        .collect()
}

/// The symbols `import`ed on behalf of `required_by`, which must include `wanted`.
fn assert_declares(plan: &NativePlan, target: CodeTarget, required_by: &str, wanted: &[&str]) {
    let declared = symbols(&plan.platform_imports, required_by);
    for symbol in wanted {
        assert!(
            declared.contains(symbol),
            "{}: `{required_by}` calls {symbol}, so the plan must import it; it imports {declared:?}",
            target.name()
        );
    }
}

/// A module with no entry (a library build) takes no entry imports.
///
/// `_exit`, `_write`, `clock_gettime`, the signal and fcntl set are what the
/// program ENTRY calls; a module without one has no `_main` to attribute them to,
/// so declaring them would hand the linker symbols nothing references. Windows
/// attributes its process imports to `_start` unconditionally and is held to
/// that instead.
#[test]
fn a_module_without_an_entry_declares_no_entry_imports() {
    for target in CodeTarget::ALL {
        let mut module = module_for(HELLO, target, NativeBuildMode::Console);
        let with_entry = plan_of(&module, target).expect("the plan lowers");
        module.entry = None;
        let plan = plan_of(&module, target).expect("an entry-less module still plans");
        if target == CodeTarget::WindowsX86_64 {
            // plan-179: the arena memory cap is carried only by an executable
            // (`arena_cap_enabled`), so its RAM probe and map-lock imports ride
            // `_start` only with an entry; compare against the capless plan.
            let mut capless = module_for(HELLO, target, NativeBuildMode::Console);
            capless.arena_memory.disable = true;
            let capless = plan_of(&capless, target).expect("the capless plan lowers");
            assert_eq!(
                symbols(&plan.platform_imports, "_start"),
                symbols(&capless.platform_imports, "_start"),
                "windows imports its `_start` set whether or not the module has an entry"
            );
            for cap_import in [
                "GlobalMemoryStatusEx",
                "AcquireSRWLockExclusive",
                "ReleaseSRWLockExclusive",
            ] {
                assert!(
                    symbols(&with_entry.platform_imports, "_start").contains(&cap_import),
                    "control: an executable carries the cap and imports {cap_import}"
                );
                assert!(
                    !symbols(&plan.platform_imports, "_start").contains(&cap_import),
                    "an entry-less module carries no arena cap, so no {cap_import}"
                );
            }
            continue;
        }
        assert!(
            !symbols(&with_entry.platform_imports, "_main").is_empty(),
            "{}: control: a module with an entry imports for `_main`",
            target.name()
        );
        assert_eq!(
            symbols(&plan.platform_imports, "_main"),
            Vec::<&str>::new(),
            "{}: an entry-less module must declare nothing for `_main`",
            target.name()
        );
    }
}

/// `--debug` adds the report's imports: the peak resident set size, the clock for
/// the arena series, the lock the report takes; without it none are declared.
#[test]
fn debug_adds_the_peak_rss_and_clock_imports_and_nothing_else_does() {
    for target in CodeTarget::ALL {
        let off = module_for(HELLO, target, NativeBuildMode::Console);
        let plain = plan_of(&off, target).expect("plans");
        let mut module = off;
        module.debug.enabled = true;
        let debug = plan_of(&module, target).expect("a debug module plans");
        let (owner, wanted): (&str, &[&str]) = match target {
            CodeTarget::WindowsX86_64 => (
                "_start",
                &[
                    "GetCurrentProcess",
                    "K32GetProcessMemoryInfo",
                    "QueryPerformanceCounter",
                    "QueryPerformanceFrequency",
                ],
            ),
            CodeTarget::MacosAarch64 => ("_main", &["_getrusage"]),
            _ => ("_main", &["getrusage"]),
        };
        assert_declares(&debug, target, owner, wanted);
        let before = symbols(&plain.platform_imports, owner);
        for symbol in wanted {
            assert!(
                !before.contains(symbol),
                "{}: {symbol} belongs to the debug report only",
                target.name()
            );
        }
    }
}

/// A `--debug` app build adds the toolkit's test hooks: macOS hops to the main
/// thread to read the transcript back, GTK reads the text buffer; Windows has none.
#[test]
fn a_debug_app_build_declares_the_toolkits_readback_imports() {
    for target in CodeTarget::ALL {
        let Some(mode) = target.app_mode() else {
            continue;
        };
        let mut module = module_for(HELLO, target, mode);
        module.debug.enabled = true;
        let plan = plan_of(&module, target).expect("a debug app module plans");
        let wanted: &[&str] = match target {
            CodeTarget::MacosAarch64 => &["_dispatch_sync_f", "__dispatch_main_q"],
            CodeTarget::WindowsX86_64 => &[],
            _ => &[
                "gtk_text_buffer_get_bounds",
                "gtk_text_buffer_get_text",
                "strlen",
            ],
        };
        assert_declares(&plan, target, "_main", wanted);
        let plain = plan_of(&module_for(HELLO, target, mode), target).expect("plans");
        for symbol in wanted {
            assert!(
                !symbols(&plain.platform_imports, "_main").contains(symbol),
                "{}: {symbol} is for --debug builds only",
                target.name()
            );
        }
    }
}

/// The terminal mouse members and `fs::seek` import what their helpers call, on
/// every backend: the helper's own symbols, not the entry's.
#[test]
fn the_mouse_and_seek_helpers_import_what_they_call_on_every_backend() {
    for target in CodeTarget::ALL {
        let mouse = module_for(TERM_MOUSE, target, NativeBuildMode::Console);
        let plan = plan_of(&mouse, target).expect("plans");
        let (enable, poll, seek): (&[&str], &[&str], &[&str]) = match target {
            CodeTarget::WindowsX86_64 => (
                &["GetStdHandle", "WriteFile", "QueryPerformanceCounter"],
                &["ReadFile", "PeekNamedPipe", "QueryPerformanceFrequency"],
                &["SetFilePointerEx", "WriteFile", "GetLastError"],
            ),
            CodeTarget::MacosAarch64 => (
                &["_write", "_clock_gettime", "_getenv", "_poll"],
                &["_clock_gettime", "_poll", "_read", "___error"],
                &["_lseek", "_fsync", "___error"],
            ),
            _ => (
                &["write", "clock_gettime", "getenv", "poll"],
                &["clock_gettime", "poll", "read", "__errno_location"],
                &["lseek", "fsync", "__errno_location"],
            ),
        };
        assert_declares(&plan, target, "_mfb_rt_term_term_enableMouse", enable);
        assert_declares(&plan, target, "_mfb_rt_term_term_pollMouse", poll);
        let seeking = module_for(FS_SEEK, target, NativeBuildMode::Console);
        let plan = plan_of(&seeking, target).expect("plans");
        assert_declares(&plan, target, "_mfb_rt_fs_fs_seek", seek);
    }
}

/// The canvas mouse members: `enableMouse` reads the clock and the inject
/// affordance's environment variable, `pollMouseRaw` reads what is waiting.
#[test]
fn the_canvas_mouse_members_import_the_clock_and_the_readers_source() {
    for target in CodeTarget::ALL {
        let Some(mode) = target.app_mode() else {
            continue;
        };
        let module = module_for(CANVAS_MOUSE, target, mode);
        let plan = plan_of(&module, target).expect("plans");
        let (enable, poll): (&[&str], &[&str]) = match target {
            // `MFB_MOUSE_INJECT` is POSIX-only, so Windows declares no enable imports.
            CodeTarget::WindowsX86_64 => (
                &[],
                &[
                    "PeekNamedPipe",
                    "GetFileType",
                    "WaitForSingleObject",
                    "Sleep",
                ],
            ),
            CodeTarget::MacosAarch64 => (
                &["_clock_gettime", "_getenv", "_poll"],
                &["_clock_gettime", "_poll", "_read", "___error"],
            ),
            _ => (
                &["clock_gettime", "getenv", "poll"],
                &["clock_gettime", "poll", "read", "__errno_location"],
            ),
        };
        assert_declares(&plan, target, "_mfb_rt_canvas_canvas_enableMouse", enable);
        assert_declares(&plan, target, "_mfb_rt_canvas_canvas_pollMouseRaw", poll);
    }
}
