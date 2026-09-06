//! `CodegenPlatform`'s optional hooks: who implements each, and how the rest
//! decline.
//!
//! Thirty-six of the trait's methods carry a default body, and they are the
//! trait's whole extension mechanism: a backend implements the hooks its OS
//! needs and inherits the default for the rest. Declining comes in four shapes,
//! and each one is a different promise:
//!
//!   * `Err(...)` — **refuse**. The operation exists in the language but only
//!     Windows has a primitive for it (`os::getEnv` through
//!     `GetEnvironmentVariableW`, the UTF-8 `argv` build, the wide-string
//!     queries, `emit_restore_blocking`). A non-Windows backend reaching one is
//!     a routing bug, and the error is what turns it into a build failure
//!     instead of a function that emits nothing and returns success.
//!   * `unreachable!(...)` — **cannot happen**. `emit_verify_nofollow` and
//!     `emit_verify_within` are the `fs::openWithin` reparse-point checks, which
//!     the POSIX backends do not route to at all.
//!   * `None` — **this target has no such feature**. Not a failure; it is how a
//!     backend without Metal says so, and the caller falls back.
//!   * `unimplemented!(...)` — **this ISA has no port**. riscv64 has no app
//!     mode, and `AppSupport::require_gtk` hard-stops at the *boundary*,
//!     deliberately: "an unported ISA panics ... rather than after assembling
//!     wrong-convention instructions".
//!
//! The failure this file exists for is `None` quietly swallowing the other
//! three. A hook a backend *needs* but has not overridden returns the default,
//! and the caller reads that as "no app mode here" and carries on — so a macOS
//! build that lost `emit_app_io_write` would link, run, and print nothing, with
//! no diagnostic anywhere, and an rv64 hook that started declining instead of
//! hard-stopping would produce a binary with no app in it. So the table records
//! *which* of the three answers each `(backend, hook)` pair gives, rather than a
//! set of implemented names.
//!
//! Every platform here is the real backend, not a stub. A stub would answer the
//! default for everything and the whole file would be vacuous.

use std::collections::HashMap;

use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::engine::types::{
    AppEntrySpec, CodeInstruction, CodeRelocation, CodegenPlatform, PresentationMode,
};
use crate::os::linux::flavor::LinuxFlavor;
use crate::target::linux_common::code::Platform as LinuxPlatform;
use crate::target::macos_aarch64::code::Platform as MacosPlatform;
use crate::target::win_x86_64::code::Platform as WindowsPlatform;

/// The five real backends, by the name their `target()` reports.
fn platforms() -> Vec<(&'static str, Box<dyn CodegenPlatform>)> {
    vec![
        ("macos-aarch64", Box::new(MacosPlatform)),
        ("windows-x86_64", Box::new(WindowsPlatform)),
        (
            "linux-aarch64",
            Box::new(LinuxPlatform::for_test(
                crate::target::linux_aarch64::code::Aarch64,
                LinuxFlavor::Glibc,
            )),
        ),
        (
            "linux-x86_64",
            Box::new(LinuxPlatform::for_test(
                crate::target::linux_x86_64::code::X86_64,
                LinuxFlavor::Glibc,
            )),
        ),
        (
            "linux-riscv64",
            Box::new(LinuxPlatform::for_test(
                crate::target::linux_riscv64::code::Riscv64,
                LinuxFlavor::Glibc,
            )),
        ),
    ]
}

/// Scratch outputs for a hook call. A declining hook must leave all three empty
/// — that is the difference between "declined" and "emitted half a sequence and
/// then gave up", which the caller cannot tell apart from the return value.
#[derive(Default)]
struct Sink {
    imports: HashMap<String, String>,
    instructions: Vec<CodeInstruction>,
    relocations: Vec<CodeRelocation>,
}

impl Sink {
    fn is_empty(&self) -> bool {
        self.instructions.is_empty() && self.relocations.is_empty()
    }
}

// --- the Windows-only refusing hooks --------------------------------------

/// Every Windows-only primitive refuses on the four POSIX backends.
///
/// These are reached only through a Windows routing decision, so a POSIX
/// backend arriving here means the routing broke. The default returns `Err`
/// rather than `Ok(())` precisely so that shows up as a build failure instead of
/// a function body that is silently empty — and an empty body for `os::getEnv`
/// returns whatever was in the return register.
#[test]
fn the_windows_only_primitives_refuse_on_every_posix_backend() {
    for (name, platform) in platforms() {
        if name == "windows-x86_64" {
            continue;
        }
        let mut sink = Sink::default();
        let outcomes: Vec<(&str, Result<(), String>)> = vec![
            (
                "emit_build_argv_utf8",
                platform.emit_build_argv_utf8(
                    "_main",
                    &sink.imports,
                    &mut sink.instructions,
                    &mut sink.relocations,
                ),
            ),
            (
                "emit_env_get",
                platform.emit_env_get(
                    "_probe",
                    &sink.imports,
                    &mut sink.instructions,
                    &mut sink.relocations,
                ),
            ),
            (
                "emit_env_set",
                platform.emit_env_set(
                    "_probe",
                    &sink.imports,
                    &mut sink.instructions,
                    &mut sink.relocations,
                ),
            ),
            (
                "emit_os_wide_string",
                platform.emit_os_wide_string(
                    "version",
                    "_probe",
                    &sink.imports,
                    &mut sink.instructions,
                    &mut sink.relocations,
                ),
            ),
            (
                "emit_restore_blocking",
                platform.emit_restore_blocking(
                    0,
                    8,
                    "_probe",
                    &sink.imports,
                    &mut sink.instructions,
                    &mut sink.relocations,
                ),
            ),
        ];
        for (hook, outcome) in outcomes {
            let message = outcome.expect_err(&format!(
                "{name}: `{hook}` is a Windows-only primitive; a POSIX backend \
                 reaching it is a routing bug, and returning Ok would emit a \
                 function whose body is empty and whose result register is \
                 whatever the caller left there"
            ));
            assert!(
                message.contains("windows") || message.contains("Windows"),
                "{name}: `{hook}` refused with {message:?}, which does not say \
                 the operation is Windows-only -- the message is the only thing \
                 that tells whoever hits it where to look"
            );
        }
        assert!(
            sink.is_empty(),
            "{name}: a refusing hook must emit nothing at all; the five left {} \
             instruction(s) and {} relocation(s) in the caller's stream, which \
             the caller keeps even though the call failed",
            sink.instructions.len(),
            sink.relocations.len()
        );
    }
}

/// The two `fs::openWithin` reparse-point checks hard-stop off Windows.
///
/// `emit_verify_nofollow` and `emit_verify_within` default to `unreachable!`
/// rather than to an `Err`, and the difference is deliberate: the other
/// Windows-only primitives are *routed to* on POSIX and refuse, while these two
/// are not routed to at all, so reaching one is not a build that should fail
/// with a diagnostic — it is a claim about the code that has stopped being
/// true.
///
/// Asserting a panic needs `catch_unwind`, which is why it is its own test:
/// folding it into the `Err` sweep above would make every row there pay for the
/// hook swap.
#[test]
fn the_openwithin_reparse_checks_are_unreachable_off_windows() {
    for (name, platform) in platforms() {
        if name == "windows-x86_64" {
            continue;
        }
        for hook in ["emit_verify_nofollow", "emit_verify_within"] {
            let mut sink = Sink::default();
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if hook == "emit_verify_nofollow" {
                    platform.emit_verify_nofollow(
                        "_probe",
                        &sink.imports,
                        &mut sink.instructions,
                        &mut sink.relocations,
                    )
                } else {
                    platform.emit_verify_within(
                        "_probe",
                        &sink.imports,
                        &mut sink.instructions,
                        &mut sink.relocations,
                    )
                }
            }));
            std::panic::set_hook(previous);
            let payload = outcome.err().unwrap_or_else(|| {
                panic!(
                    "{name}: `{hook}` is the Windows-only `fs::openWithin` \
                     reparse-point check and nothing routes to it off Windows, \
                     so its default must hard-stop rather than return"
                )
            });
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_default();
            assert!(
                message.contains("Windows-only"),
                "{name}: `{hook}` hard-stopped with {message:?}, which does not \
                 say the check is Windows-only"
            );
        }
    }
}

/// A backend that overrides nothing gets the documented defaults.
///
/// Every one of the five real backends overrides the whole app-mode family, so
/// the trait's own default bodies are dead for all of them — and those defaults
/// are exactly what a SIXTH backend inherits on the day it is added. `None`
/// means "this target has no app mode", and it is the answer that lets a new
/// console-only backend lower a program at all instead of tripping over a hook
/// it has never heard of.
///
/// `TestPlatform` is the stub that overrides nothing, so it is the only way to
/// reach them.
#[test]
fn a_backend_that_overrides_nothing_declines_every_optional_hook() {
    let stub = TestPlatform;
    for (hook, answer) in optional_hook_answers(&stub) {
        assert_eq!(
            answer,
            Answer::Declined,
            "a backend that overrides nothing must inherit `None` for `{hook}` \
             -- that is what makes the hook optional, and a default that did \
             anything else would make adding a backend a matter of implementing \
             all of them first"
        );
    }
    // The value-returning defaults, which `optional_hook_answers` cannot reach
    // because they answer with a collection rather than an `Option`.
    assert!(
        stub.app_mode_data_objects("probe").is_empty(),
        "a backend with no app mode has no app-mode data objects"
    );
    assert!(
        stub.app_mode_reconcile_data_objects().is_empty(),
        "a backend with no mode reconcile has no reconcile data objects"
    );
    assert!(
        stub.emit_tls_block_trampolines(true).is_empty(),
        "a backend with no foreign TLS callback boundary emits no trampolines"
    );
}

/// Windows implements all five, and says so with a real emission.
///
/// The mirror of the test above: without it, deleting every Windows override
/// would leave that test passing on five platforms instead of four.
#[test]
fn windows_implements_every_primitive_the_others_refuse() {
    let platform = WindowsPlatform;
    for hook in [
        "emit_env_get",
        "emit_env_set",
        "emit_os_wide_string/hostName",
        "emit_os_wide_string/userName",
        "emit_os_wide_string/executablePath",
    ] {
        let mut sink = Sink::default();
        let outcome = match hook {
            "emit_env_get" => platform.emit_env_get(
                "_probe",
                &sink.imports,
                &mut sink.instructions,
                &mut sink.relocations,
            ),
            "emit_env_set" => platform.emit_env_set(
                "_probe",
                &sink.imports,
                &mut sink.instructions,
                &mut sink.relocations,
            ),
            other => platform.emit_os_wide_string(
                other
                    .split_once('/')
                    .expect("a wide-string row names its query")
                    .1,
                "_probe",
                &sink.imports,
                &mut sink.instructions,
                &mut sink.relocations,
            ),
        };
        outcome.unwrap_or_else(|err| {
            panic!("windows-x86_64: `{hook}` must be implemented, not refused: {err}")
        });
        assert!(
            !sink.instructions.is_empty(),
            "windows-x86_64: `{hook}` returned Ok without emitting anything, so \
             the caller gets an empty body and a result register it never wrote"
        );
    }
}

// --- the optional-feature hooks -------------------------------------------

/// How a platform answered one optional hook.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Answer {
    /// `Some(..)` — the backend has this feature and emitted for it.
    Implemented,
    /// `None` — this OS has no such feature; the caller falls back.
    Declined,
    /// A panic. Not a failure: `AppSupport::require_gtk` hard-stops an ISA with
    /// no app-mode port *at the boundary*, deliberately, "rather than after
    /// assembling wrong-convention instructions".
    NotPorted,
}

/// How each optional hook answered, for one platform.
fn optional_hook_answers(platform: &dyn CodegenPlatform) -> Vec<(&'static str, Answer)> {
    // An implemented hook emits real MIR, and the MIR builders read the active
    // backend from a thread-local that only a lowering entry point installs.
    // Without it the first hook that does any work panics for a reason that has
    // nothing to do with the feature under test -- and `NotPorted` would then
    // swallow it.
    crate::codegen::engine::mir::set_backend(platform.backend());

    let mut found = Vec::new();
    let mut sink = Sink::default();

    macro_rules! probe {
        ($name:literal, $call:expr) => {{
            let hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            let answered =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| $call.is_some()));
            std::panic::set_hook(hook);
            found.push((
                $name,
                match answered {
                    Ok(true) => Answer::Implemented,
                    Ok(false) => Answer::Declined,
                    Err(_) => Answer::NotPorted,
                },
            ));
        }};
    }

    probe!(
        "emit_app_io_write",
        platform.emit_app_io_write(
            "_probe",
            false,
            true,
            None,
            &sink.imports,
            &mut sink.instructions,
            &mut sink.relocations,
        )
    );
    probe!(
        "emit_app_io_flush",
        platform.emit_app_io_flush("_probe", &mut sink.instructions, &mut sink.relocations)
    );
    probe!(
        "emit_app_io_input",
        platform.emit_app_io_input("_probe", &mut sink.instructions, &mut sink.relocations)
    );
    probe!(
        "emit_app_raw_input_mode",
        platform.emit_app_raw_input_mode("_probe", &mut sink.instructions, &mut sink.relocations)
    );
    probe!(
        "emit_app_io_is_terminal",
        platform.emit_app_io_is_terminal("_probe", &mut sink.instructions, &mut sink.relocations)
    );
    probe!(
        "emit_app_term_helper",
        platform.emit_app_term_helper(
            "term.on",
            "_probe",
            0,
            &mut sink.instructions,
            &mut sink.relocations,
        )
    );
    probe!(
        "emit_app_mode_reconcile",
        platform.emit_app_mode_reconcile(
            "_probe",
            0,
            &mut sink.instructions,
            &mut sink.relocations
        )
    );
    probe!(
        "emit_canvas_blit",
        platform.emit_canvas_blit("_probe", &mut sink.instructions, &mut sink.relocations)
    );
    probe!(
        "emit_metal_init",
        platform.emit_metal_init("_probe", &mut sink.instructions, &mut sink.relocations)
    );
    probe!(
        "emit_metal_draw",
        platform.emit_metal_draw("_probe", &mut sink.instructions, &mut sink.relocations)
    );
    probe!(
        "emit_app_program_entry",
        platform.emit_app_program_entry(
            &AppEntrySpec {
                language_entry_accepts_args: false,
                uses_term: false,
                initial_mode: PresentationMode::Console,
                uses_canvas: false,
            },
            &sink.imports,
        )
    );

    found
}

/// Every optional hook, on every backend, answered exactly as recorded.
///
/// A full table rather than a set of "implemented" names, because the three
/// answers are three different promises and confusing any pair is a real bug:
///
///   * `Implemented` -> `Declined` is the failure this file exists for. The
///     caller reads `None` as "this target has no app mode" and falls back
///     silently, so a macOS build that lost `emit_app_io_write` would link, run,
///     and print nothing, with no diagnostic anywhere.
///   * `NotPorted` -> `Declined` is the same failure wearing riscv64's clothes.
///     `AppSupport::require_gtk` hard-stops an unported ISA *at the boundary*,
///     deliberately — "rather than after assembling wrong-convention
///     instructions". A hook that started returning `None` instead would let an
///     rv64 `-app` build produce a binary with no app in it.
///   * `Declined` -> `Implemented` is worth a look too, because the fallback it
///     displaces was the tested path.
#[test]
fn every_backend_answers_each_optional_hook_exactly_as_recorded() {
    // Every row, not the first mismatch: the table is a five-row snapshot, and
    // reading it back one failure per run is how a correction to one row hides
    // the next.
    let mut drift = Vec::new();
    for (name, platform) in platforms() {
        for (hook, answer) in optional_hook_answers(platform.as_ref()) {
            let expected = expected_answer(name, hook);
            if answer != expected {
                drift.push(format!(
                    "{name} {hook}: recorded {expected:?}, got {answer:?}"
                ));
            }
        }
    }
    assert!(
        drift.is_empty(),
        "the optional-hook table no longer matches the backends:\n  {}",
        drift.join("\n  ")
    );
}

/// The recorded answer for one `(backend, hook)` pair.
///
/// Read off a measurement, not off the `impl` blocks: grepping which backend
/// *defines* a method is not the same question. `linux_common` defines every app
/// hook for all three Linux ISAs, and two of the three answers it gives are not
/// `Implemented`.
fn expected_answer(target: &str, hook: &str) -> Answer {
    // Metal is the ONLY hook the four app-capable backends disagree about. Every
    // other one -- the transcript I/O, the raw-input mode, the term helper, the
    // mode reconcile, the canvas blit, the app entry -- is implemented by all
    // four, which is the property that makes app mode portable at all.
    let metal = matches!(hook, "emit_metal_init" | "emit_metal_draw");
    match target {
        // AppKit transcript AND Metal: macOS is the only backend with both.
        "macos-aarch64" => Answer::Implemented,
        // Win32 and GTK: everything but Metal.
        "windows-x86_64" | "linux-aarch64" | "linux-x86_64" if metal => Answer::Declined,
        "windows-x86_64" | "linux-aarch64" | "linux-x86_64" => Answer::Implemented,
        // riscv64 has no app-mode port. Every GTK hook hard-stops at the
        // boundary; Metal is not GTK's, so it declines like everywhere else.
        "linux-riscv64" if metal => Answer::Declined,
        "linux-riscv64" => Answer::NotPorted,
        other => panic!("no optional-hook row for backend {other}"),
    }
}

/// Every backend agrees with itself about which OS it is.
///
/// `family()` defaults to `platform_family(self.target())`, a string lookup, so
/// a backend whose `target()` and overridden `family()` disagreed would route
/// every family-keyed decision one way and every name-keyed one the other.
#[test]
fn every_backend_reports_a_family_consistent_with_its_target() {
    for (name, platform) in platforms() {
        assert_eq!(
            platform.target(),
            name,
            "the platform registered as {name} reports target {:?}",
            platform.target()
        );
        assert_eq!(
            platform.family(),
            crate::codegen::engine::types::platform_family(name),
            "{name}: `family()` and `platform_family(target())` disagree, so a \
             family-keyed lowering and a name-keyed one route differently"
        );
    }
}

/// A backend with no app mode REFUSES an app-mode build of `io::is*Terminal`.
///
/// The counterpart to
/// [`a_backend_that_overrides_nothing_declines_every_optional_hook`], one level
/// up: that test proves the hook answers `None`, this one proves what the caller
/// does with the `None`. `lower_is_terminal` branches on
/// `ctx.build_mode.is_app()` before it looks at the platform, so a target
/// without the hook has two things it could do with an app-mode build — refuse,
/// or fall through to the console `isatty(fd)` body — and only one is right.
/// `isatty` on a descriptor an app-mode program never opened answers FALSE, so
/// the fall-through is a program whose `io::isInputTerminal()` is quietly wrong
/// rather than a build that stopped.
///
/// Neither the refusal nor its message had a test: `io::app_unsupported` is the
/// whole of `builtins/io/mod.rs`'s uncovered code, and the `ok_or_else` that
/// calls it is the whole of `gen_is_terminal.rs`'s.
///
/// Driven through the REGISTRY rather than by naming the body, both because
/// `gen_is_terminal` is private to its package and because the registry is how
/// production reaches it — a member re-pointed at a different body would still
/// be tested here. All three descriptors, because the three members share one
/// body through `fd` and `text`, and a refusal wired to one would leave the
/// other two falling through.
#[test]
fn an_app_build_on_a_backend_with_no_app_mode_refuses_the_terminal_predicates() {
    use crate::codegen::registry::{registry, Body};

    let platform = TestPlatform;
    let harness = crate::codegen::engine::tests::test_support::BuilderHarness {
        build_mode: crate::target::NativeBuildMode::MacApp,
        ..Default::default()
    };
    let ctx = harness.abi_ctx(&platform);

    let package = registry()
        .packages()
        .iter()
        .find(|package| package.import_name() == "io")
        .expect("the io package is registered");

    let mut checked = 0;
    for name in ["isInputTerminal", "isOutputTerminal", "isErrorTerminal"] {
        let function = package
            .functions()
            .iter()
            .find(|function| function.name == name)
            .unwrap_or_else(|| panic!("io::{name} is registered"));
        for implementation in function.implementations() {
            let Body::AbiFunction { lower, .. } = implementation.body else {
                panic!("io::{name} lowers through an abi_function body");
            };
            let mut builder = harness.builder("_mfb_rt_probe", &platform);
            let message = match lower(&mut builder, &[], &ctx) {
                Err(message) => message,
                Ok(_) => panic!(
                    "io::{name}: an app-mode build lowered on a backend with no \
                     app-mode io hook. What it emitted is the console \
                     `isatty` body, which answers FALSE for the window the \
                     program is actually running in -- a wrong answer where a \
                     refused build belongs"
                ),
            };
            assert!(
                message.contains("does not support app-mode io helpers"),
                "io::{name}: the refusal must be the app-mode one; got {message:?}"
            );
            assert!(
                message.contains(platform.target()),
                "io::{name}: the refusal must NAME the target that cannot do it, \
                 because that is the only part the author can act on; got \
                 {message:?}"
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked, 3,
        "all three terminal predicates must have been driven; a member that \
         stopped being an abi_function would otherwise skip silently"
    );
}
