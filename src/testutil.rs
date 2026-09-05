//! Shared `#[cfg(test)]` fixtures for the unit-test suite (plan-12).
//!
//! These build the common source → AST → IR pipeline objects that most
//! front-end and codegen unit tests need, so individual `mod tests` blocks
//! don't each re-derive the same boilerplate. Keep helpers here small and
//! composable; anything file-specific stays in that file's own test module.

#![cfg(test)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::{parse_source, AstFile, AstProject};
use crate::ir::{self, IrProject};

/// Locate a committed test fixture directory by its leaf name, searching
/// recursively under `tests/`. After the tests reorganization fixtures live
/// under `tests/{syntax,rt-error,rt-behavior}/<feature>/<name>` (plus the
/// `tests/acceptance` app and the `tests/byte-identity` gate-coverage tree),
/// and leaf names are unique — so a by-name search
/// keeps unit tests independent of the exact bucket/feature a fixture lives in.
/// Panics if no matching fixture directory (one holding a `project.json`)
/// exists.
pub fn fixture_dir(name: &str) -> PathBuf {
    fn find(dir: &Path, name: &str) -> Option<PathBuf> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if entry.file_name() == *name && path.join("project.json").is_file() {
                return Some(path);
            }
            if let Some(found) = find(&path, name) {
                return Some(found);
            }
        }
        None
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    find(&root, name).unwrap_or_else(|| panic!("test fixture `{name}` not found under tests/"))
}

/// Parse a single `.mfb` source string into an [`AstFile`], panicking on any
/// parse error (tests that want the error should call `parse_source` directly).
pub fn parse_file(source: &str) -> AstFile {
    parse_source(Path::new("main.mfb"), "main.mfb", source).expect("source should parse")
}

/// Wrap a single source string into a one-file [`AstProject`], appending the
/// compiler-owned prelude (`Pair`, `Partition`) exactly as the real project
/// loader does so the front end sees the always-in-scope generic templates.
/// (Named to avoid colliding with [`crate::ast::parse_project`] under a glob
/// import.)
pub fn project_from_src(source: &str) -> AstProject {
    let project = AstProject {
        name: "test".to_string(),
        files: vec![parse_file(source)],
    };
    // Mirror `ast::manifest::parse_project`: append the prelude last so the
    // user's file stays `files[0]`.
    crate::ast::augment_with_prelude(project)
}

/// Parse and lower a single source string into an [`IrProject`], with no entry
/// point and no external (native `LINK`) functions — the common shape for
/// exercising lowering, serialization, and codegen on hand-written programs.
pub fn lower_src(source: &str) -> IrProject {
    let project = project_from_src(source);
    ir::lower_project_with_external_functions(&project, None, &HashMap::new(), &[])
}

/// The build path's front end over a single source string: parse -> inject the
/// builtin package sources -> elaborate -> monomorphize, yielding the
/// **concrete** HIR every pass below `resolve` consumes
/// (`src/cli/build/mod.rs:459-482`).
///
/// Monomorphization is the part [`lower_src`] skips, and skipping it is not a
/// shortcut: it is what rewrites every overloaded and generic call to the
/// mangled symbol of the instantiation the argument types select. A program that
/// reaches a builtin through a generic seam (`encoding::utf8Decode(List OF
/// Byte)`, the `canvas::` companions) does not type-check without it.
///
/// Panics on a lex/parse or monomorphization failure - a test-author error.
pub fn concrete_hir_from_src(source: &str) -> crate::hir::HirProject {
    // Through `project_from_src`, so the compiler-owned prelude (`Pair`,
    // `Partition`) is present exactly as the real project loader leaves it.
    // Without it a program that names a prelude template -- everything
    // `collections::zip` returns is a `Pair` -- monomorphizes to `Unknown` and
    // dies far downstream with "native len does not accept argument type
    // 'Unknown'", which reads as a codegen bug rather than a missing prelude.
    let augmented = crate::resolver::augment_project(&project_from_src(source))
        .expect("builtin augmentation must succeed");
    crate::monomorph::monomorphize_project(Path::new("."), &crate::hir::elaborate(&augmented))
        .expect("test source must monomorphize")
}

/// Parse, augment, monomorphize and lower a single source string into an
/// [`IrProject`] — [`lower_src`] plus the monomorphization pass the build path
/// runs. Use this whenever the program under test reaches a builtin through an
/// overloaded or generic call; see [`concrete_hir_from_src`].
///
/// The entry is not cosmetic. `lower_module_for_platform` emits the program
/// entry — and, in an `-app` build, the whole toolkit bootstrap that DEFINES the
/// callbacks the reconcile seam relocates against — only inside
/// `if let Some(entry) = &module.entry` (`src/codegen/engine/builder/mod.rs`).
/// Lowering an app program with `None` therefore fails validation on a dangling
/// `_mfb_gtkapp_reconcile_idle`, which reads as a codegen bug.
pub fn lower_src_concrete(source: &str, entry: Option<crate::ir::EntryPoint>) -> IrProject {
    crate::ir::lower_augmented_project(&concrete_hir_from_src(source), entry, &HashMap::new(), &[])
}

/// The entry the harness declares for `source`.
///
/// `accepts_args` is read off the source rather than hardcoded. It is not
/// cosmetic: it decides whether the program entry captures `argv` at all, and a
/// harness that always said `false` left the whole args-capture path in
/// `engine/function/entry.rs` unreachable -- the load of `argc`/`argv` off the
/// initial stack, the deferred-capture branch, and the copy into the arena.
fn main_entry(source: &str) -> crate::ir::EntryPoint {
    let accepts_args = source
        .lines()
        .filter_map(|line| line.trim().strip_prefix("FUNC main"))
        .any(|rest| rest.starts_with('(') && !rest.starts_with("()"));
    crate::ir::EntryPoint {
        name: "main".to_string(),
        returns: crate::types::ParameterType::Integer,
        accepts_args,
    }
}

/// Run the build path's two checkers over `src` and return the emitted
/// diagnostic rule codes (in stream order). An empty vector means the program
/// is accepted.
pub fn check_src(source: &str) -> Vec<String> {
    // One source-diagnostic oracle for a pipeline-level test (plan-107):
    // `ir::shape` over the concrete HIR, then `ir::verify` over the lowered IR -
    // exactly the build's order.
    let project_dir = Path::new(".");
    let concrete = concrete_hir_from_src(source);
    let no_signatures = HashMap::new();
    let mut diagnostics =
        crate::ir::shape::collect_diagnostics(project_dir, &concrete, &[], &no_signatures, &[]);
    let lowered = crate::ir::lower_augmented_project(&concrete, None, &no_signatures, &[]);
    let link_spans = crate::ir::link_spans(&concrete);
    diagnostics.extend(crate::ir::verify_source_diagnostics(
        &lowered,
        project_dir,
        &[],
        &link_spans,
    ));
    diagnostics.into_iter().map(|d| d.rule).collect()
}

/// True when the pipeline accepts `src` with zero diagnostics.
pub fn accepts(source: &str) -> bool {
    check_src(source).is_empty()
}

// --- in-process native codegen -------------------------------------------
//
// The `tests/` integration harness shells out to a separately built
// `target/release/mfb` (`tests/common/mod.rs:1080`), so nothing it executes is
// observable from this process — an emitter reached only through a builtin call
// therefore has no in-process coverage at all unless a unit test drives the
// compiler itself. These helpers run the real `.ncode` dump pipeline
// (`src/target/<backend>/mod.rs`'s `write_native_code_plan`) minus the file
// write: AST -> IR -> NIR -> native plan -> native code. No linker, no
// subprocess.

/// A backend to lower a test program for.
///
/// Spelled out rather than derived from `BuildTarget` because each backend's
/// `plan`/`code` entry points take different arguments (the Linux ones are
/// flavor-parameterized), and because a test naming a target wants a compile
/// error if that target goes away, not a runtime "unknown target".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeTarget {
    MacosAarch64,
    LinuxAarch64,
    LinuxX86_64,
    LinuxRiscv64,
    WindowsX86_64,
}

impl CodeTarget {
    /// Every backend, for a test that asserts a property must hold on all of them.
    pub const ALL: [Self; 5] = [
        Self::MacosAarch64,
        Self::LinuxAarch64,
        Self::LinuxX86_64,
        Self::LinuxRiscv64,
        Self::WindowsX86_64,
    ];

    /// The `os-arch` name `lower_project` stamps into the NIR module.
    pub fn name(self) -> &'static str {
        match self {
            Self::MacosAarch64 => "macos-aarch64",
            Self::LinuxAarch64 => "linux-aarch64",
            Self::LinuxX86_64 => "linux-x86_64",
            Self::LinuxRiscv64 => "linux-riscv64",
            Self::WindowsX86_64 => "windows-x86_64",
        }
    }

    /// This backend's `-app` build mode, or `None` for a console-only backend.
    ///
    /// A program that calls `app::setMode` needs it: the mode switch lowers to
    /// the host toolkit's event loop (`g_idle_add` on Linux), and only the app
    /// build mode declares that import - a console plan rejects the call with
    /// "the platform import list does not declare", which reads as a codegen bug
    /// rather than as the wrong build mode.
    pub fn app_mode(self) -> Option<crate::target::NativeBuildMode> {
        use crate::target::NativeBuildMode::*;
        match self {
            Self::MacosAarch64 => Some(MacApp),
            Self::LinuxAarch64 | Self::LinuxX86_64 => Some(LinuxApp),
            Self::WindowsX86_64 => Some(WindowsApp),
            // rv64 is console-only: the GTK4 toolkit is not ported
            // (`src/target/linux_riscv64/code.rs`'s `APP_MODE_UNPORTED`).
            Self::LinuxRiscv64 => None,
        }
    }
}

/// Lower `source` to native code for `target`, in process.
///
/// Returns the same `NativeCodePlan` the `-ncode` dump writes, so a test can
/// assert on the exact instruction stream, relocations and data objects the
/// compiler emits for a whole program. Panics with the compiler's own message if
/// any stage rejects the program — a test-author error, not a product failure.
pub fn code_for_src_on(
    source: &str,
    target: CodeTarget,
) -> crate::codegen::engine::types::NativeCodePlan {
    code_for_src_mode(source, target, crate::target::NativeBuildMode::Console)
}

/// [`code_for_src_on`] with an explicit build mode.
pub fn code_for_src_mode(
    source: &str,
    target: CodeTarget,
    build_mode: crate::target::NativeBuildMode,
) -> crate::codegen::engine::types::NativeCodePlan {
    code_for_src_with(source, target, build_mode, Default::default())
}

/// [`code_for_src_mode`], but returning the compiler's message instead of
/// panicking.
///
/// A corpus sweep needs this: the panic a failed lowering raises carries the
/// compiler's message but not the FIXTURE, and "some program does not lower on
/// macos-aarch64" is not actionable across four hundred of them.
pub fn try_code_for_src(
    source: &str,
    target: CodeTarget,
    build_mode: crate::target::NativeBuildMode,
) -> Result<crate::codegen::engine::types::NativeCodePlan, String> {
    try_code_for_src_with(source, target, build_mode, Default::default())
}

/// [`code_for_linking_src`], but returning the compiler's message instead of
/// panicking.
///
/// A refusal is a contract in its own right — "this declaration needs more
/// argument registers than this target has" is the alternative to silently
/// dropping an argument — and asserting on it by catching a panic would be
/// asserting on how the harness formats its payload.
pub fn try_code_for_linking_src(
    source: &str,
    target: CodeTarget,
    libraries: &[&str],
) -> Result<crate::codegen::engine::types::NativeCodePlan, String> {
    try_code_for_src_with(
        source,
        target,
        crate::target::NativeBuildMode::Console,
        link_library_table(target, libraries),
    )
}

/// The fallible lowering both `try_*` entry points share.
fn try_code_for_src_with(
    source: &str,
    target: CodeTarget,
    build_mode: crate::target::NativeBuildMode,
    table: crate::binary_repr::NativeLibraryTable,
) -> Result<crate::codegen::engine::types::NativeCodePlan, String> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .name(format!("try_code_for_src({})", target.name()))
        .spawn(move || {
            let hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            let lowered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                code_for_src_inner(&source, target, build_mode, table)
            }));
            std::panic::set_hook(hook);
            lowered.map_err(|payload| {
                payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
                    .unwrap_or_else(|| "lowering panicked with no message".to_string())
            })
        })
        .expect("spawn the lowering thread")
        .join()
        .unwrap_or_else(|_| Err("the lowering thread died".to_string()))
}

/// [`code_for_src_mode`] with an explicit native-library table (see
/// [`code_for_linking_src`], which is the only caller that needs a non-empty
/// one).
pub fn code_for_src_with(
    source: &str,
    target: CodeTarget,
    build_mode: crate::target::NativeBuildMode,
    libraries: crate::binary_repr::NativeLibraryTable,
) -> crate::codegen::engine::types::NativeCodePlan {
    // On a big enough stack. libtest gives each case a 2 MiB thread and the
    // unoptimized front end recurses deeply over the injected builtin package
    // sources (a program importing `canvas` monomorphizes several thousand
    // functions), so running this on the test's own thread aborts the whole
    // binary with `has overflowed its stack` - which reads as a product crash,
    // not as a harness limit. A real build never hits it: `mfb` compiles on the
    // process main thread, whose stack is 8 MiB.
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .name(format!("code_for_src({})", target.name()))
        .spawn(move || code_for_src_inner(&source, target, build_mode, libraries))
        .expect("spawn the lowering thread")
        .join()
        // Re-panic with the INNER message. `join().expect(..)` would report
        // `Any { .. }` and throw away the compiler's own error, which is the
        // only thing that says which program, which target, and why.
        .unwrap_or_else(|payload| {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_else(|| "the lowering thread panicked".to_string());
            panic!("{message}")
        })
}

fn code_for_src_inner(
    source: &str,
    target: CodeTarget,
    build_mode: crate::target::NativeBuildMode,
    libraries: crate::binary_repr::NativeLibraryTable,
) -> crate::codegen::engine::types::NativeCodePlan {
    use crate::os::linux::flavor::LinuxFlavor::Glibc;
    use crate::target::shared::lower;

    let mut ir = lower_src_concrete(source, Some(main_entry(source)));
    ir.native_libraries = libraries;
    let module = lower::lower_project(&ir, target.name().to_string(), &[], build_mode, None)
        .expect("test source must lower to NIR");
    match target {
        CodeTarget::MacosAarch64 => {
            let plan = crate::target::macos_aarch64::plan::lower_module(&module)
                .expect("native plan (macos-aarch64)");
            crate::target::macos_aarch64::code::lower_module(&module, &plan, &[])
        }
        CodeTarget::LinuxAarch64 => {
            let plan = crate::target::linux_aarch64::plan::lower_module(&module, Glibc)
                .expect("native plan (linux-aarch64)");
            crate::target::linux_aarch64::code::lower_module(&module, &plan, &[], Glibc)
        }
        CodeTarget::LinuxX86_64 => {
            let plan = crate::target::linux_x86_64::plan::lower_module(&module, Glibc)
                .expect("native plan (linux-x86_64)");
            crate::target::linux_x86_64::code::lower_module(&module, &plan, &[], Glibc)
        }
        CodeTarget::LinuxRiscv64 => {
            let plan = crate::target::linux_riscv64::plan::lower_module(&module, Glibc)
                .expect("native plan (linux-riscv64)");
            crate::target::linux_riscv64::code::lower_module(&module, &plan, &[], Glibc)
        }
        CodeTarget::WindowsX86_64 => {
            let plan = crate::target::win_x86_64::plan::lower_module(&module)
                .expect("native plan (windows-x86_64)");
            crate::target::win_x86_64::code::lower_module(&module, &plan, &[])
        }
    }
    .expect("test source must lower to native code")
}

/// [`code_for_src_mode`], memoized for the whole test binary.
///
/// Lowering a program that `IMPORT canvas` takes ~15s in an unoptimized build -
/// the injected package source alone is several hundred functions - so a suite
/// that wants twenty assertions about one program must not compile it twenty
/// times. The result is leaked and shared; nothing mutates a `NativeCodePlan`.
pub fn code_for_src_cached(
    source: &str,
    target: CodeTarget,
    build_mode: crate::target::NativeBuildMode,
) -> &'static crate::codegen::engine::types::NativeCodePlan {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    type Key = (String, &'static str, &'static str);
    static CACHE: OnceLock<
        Mutex<HashMap<Key, &'static crate::codegen::engine::types::NativeCodePlan>>,
    > = OnceLock::new();

    let key: Key = (source.to_string(), target.name(), build_mode.as_str());
    // Compiled OUTSIDE the lock: two cases wanting different programs must not
    // serialize behind each other, and a 15s compile under a global mutex would
    // turn a parallel suite back into a sequential one. A duplicate compile of
    // the same key is possible and harmless (the first insert wins).
    if let Some(hit) = CACHE
        .get_or_init(Default::default)
        .lock()
        .expect("coverage harness cache")
        .get(&key)
    {
        return hit;
    }
    let built: &'static _ = Box::leak(Box::new(code_for_src_mode(source, target, build_mode)));
    *CACHE
        .get_or_init(Default::default)
        .lock()
        .expect("coverage harness cache")
        .entry(key)
        .or_insert(built)
}

/// [`code_for_src_cached`] in `target`'s `-app` build mode.
pub fn app_code_cached(
    source: &str,
    target: CodeTarget,
) -> &'static crate::codegen::engine::types::NativeCodePlan {
    let mode = target
        .app_mode()
        .unwrap_or_else(|| panic!("{} has no -app build mode", target.name()));
    code_for_src_cached(source, target, mode)
}

/// The `src/main.mfb` of a committed single-file fixture.
///
/// Hand-writing a test program that reaches a builtin's NATIVE fast path is
/// harder than it looks — the fast paths are keyed on exact instantiations
/// (`#collections_groupBy$String$Integer$String`), and a program that misses one
/// silently exercises the interpreted `.mfb` body instead, so the suite passes
/// while measuring nothing. The `tests/rt-behavior/**` fixtures were written
/// against those instantiations and are proven to compile and run, which makes
/// them the right source for a codegen suite that wants the fast path.
///
/// Panics if the fixture is missing or has no `src/main.mfb` — both are
/// test-author errors, and a silently-skipped fixture is worse than a failure.
pub fn fixture_src(name: &str) -> String {
    let path = fixture_dir(name).join("src").join("main.mfb");
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

/// [`code_for_src_cached`] over a committed fixture's `src/main.mfb`.
pub fn code_for_fixture(
    name: &str,
    target: CodeTarget,
) -> &'static crate::codegen::engine::types::NativeCodePlan {
    code_for_src_cached(
        &fixture_src(name),
        target,
        crate::target::NativeBuildMode::Console,
    )
}

/// [`code_for_src_on`] for a program that declares a native `LINK` block.
///
/// A `LINK "sqlite3"` lowers to a `dlopen` of a *resolved* filename, and the
/// resolution comes from the project's `libraries` manifest section by way of
/// `IrProject::native_libraries` — not from the logical name. Without a table
/// entry the whole thunk emitter refuses ("cannot resolve native library"), so
/// no `LINK` program can be lowered by the plain harness at all, and
/// `codegen/link/thunk/link_thunk.rs` (2,277 lines) has no in-process coverage.
///
/// Each `library` is declared as a `System` locator for every OS, with no arch
/// or libc constraint, which is exactly what a `"type": "system"` manifest entry
/// produces and what makes the same program lower for all five backends.
pub fn code_for_linking_src(
    source: &str,
    target: CodeTarget,
    libraries: &[&str],
) -> crate::codegen::engine::types::NativeCodePlan {
    code_for_src_with(
        source,
        target,
        crate::target::NativeBuildMode::Console,
        link_library_table(target, libraries),
    )
}

/// One `System` locator per library, for `target`'s OS and any arch/libc.
fn link_library_table(
    target: CodeTarget,
    libraries: &[&str],
) -> crate::binary_repr::NativeLibraryTable {
    use crate::binary_repr::{NativeLibraryEntry, NativeLibraryLocator, NativeLibraryTable};
    use crate::manifest::libraries::LibType;

    let os = target
        .name()
        .split_once('-')
        .map(|(os, _)| os.to_string())
        .expect("a target name is `<os>-<arch>`");
    NativeLibraryTable {
        entries: libraries
            .iter()
            .map(|logical| NativeLibraryEntry {
                logical: (*logical).to_string(),
                locators: vec![NativeLibraryLocator {
                    os: os.clone(),
                    arch: None,
                    libc: None,
                    lib_type: LibType::System,
                    source: format!("lib{logical}.so"),
                    hash: None,
                }],
            })
            .collect(),
    }
}

/// The lowered function whose `name` matches, panicking with the available names
/// when it does not exist (a renamed symbol otherwise reads as an empty stream).
pub fn code_function<'a>(
    code: &'a crate::codegen::engine::types::NativeCodePlan,
    name: &str,
) -> &'a crate::codegen::engine::types::CodeFunction {
    code.functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| {
            let names: Vec<&str> = code.functions.iter().map(|f| f.name.as_str()).collect();
            panic!("no lowered function named `{name}`; have: {names:?}")
        })
}

#[cfg(test)]
mod code_harness_tests {
    use super::*;

    const HELLO: &str = "\
FUNC main() AS Integer
  LET n AS Integer = 40
  RETURN n + 2
END FUNC
";

    /// Every backend lowers the same program, from whichever host runs the test.
    ///
    /// This is what lets a test for a platform-specific emitter cover it on CI:
    /// the coverage job is ubuntu/x86_64 and this machine is macOS/arm64, so a
    /// macOS emitter reached only "because we are on a Mac" is 0% there and
    /// nobody notices (`tls/gen_macos/timeout.rs` is exactly that today). Naming
    /// the target makes the coverage host-independent — and cross-compilation is
    /// a shipped feature, so lowering all five is a real contract, not a trick.
    #[test]
    fn every_backend_lowers_a_program_in_process() {
        for target in CodeTarget::ALL {
            let code = code_for_src_on(HELLO, target);
            assert_eq!(
                code.target,
                target.name(),
                "the code plan must record the backend it was lowered for"
            );
            let main = code_function(&code, "main");
            assert!(
                !main.instructions.is_empty(),
                "{}: main must lower to a non-empty instruction stream",
                target.name()
            );
            assert!(
                code.entry_symbol.is_some(),
                "{}: a program with an entry point must name its entry symbol",
                target.name()
            );
        }
    }

    /// `app_mode()` answers for exactly the backends that ship `-app`.
    ///
    /// rv64 is console-only and must stay that way here: handing it an app mode
    /// would make `app_code_cached` lower GTK bodies for an ISA with no GTK
    /// entry point, which `AppSupport::Unsupported` panics on far from the cause.
    #[test]
    fn only_the_app_capable_backends_report_an_app_mode() {
        assert_eq!(CodeTarget::LinuxRiscv64.app_mode(), None);
        for target in CodeTarget::ALL {
            if target != CodeTarget::LinuxRiscv64 {
                assert!(
                    target.app_mode().is_some(),
                    "{} ships -app and must report a build mode",
                    target.name()
                );
            }
        }
    }

    /// The cache returns the same lowering rather than recompiling it.
    #[test]
    fn the_cache_hands_back_one_lowering_per_key() {
        use crate::target::NativeBuildMode::Console;
        let first = code_for_src_cached(HELLO, CodeTarget::LinuxX86_64, Console);
        let again = code_for_src_cached(HELLO, CodeTarget::LinuxX86_64, Console);
        assert!(
            std::ptr::eq(first, again),
            "a repeated (source, target, mode) must not recompile"
        );
    }
}
