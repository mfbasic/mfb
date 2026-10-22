//! The module-level counterpart of `an_emit_failure_past_the_first_one_still_propagates`.
//!
//! That sweep drives each registry body on its own. What it never reaches is the
//! module ASSEMBLY around them: `lower_module_for_platform` lowers the program's
//! functions and then emits, one after another, the runtime helpers the program
//! needs — the arena, the stdin broadcast reader, the file drain, the module's
//! resource teardown, the process reaper, the thread runtime, the graphics
//! trampoline, the `--debug` report — and each of those emitters makes platform
//! calls of its own. A failure inside any of them must come back out of
//! `lower_module_for_platform` as the refusal it was, not be dropped on the floor
//! with the helper half-emitted (a program that links but calls into a hole).
//!
//! The same [`FailAt`] wrapper, around the same real platforms, is handed to the
//! whole module lowering: one clean run counts the fallible calls, then each is
//! failed in turn and the module must refuse with the injected message.

use super::*;
use crate::codegen::engine::types::NativeCodePlan;
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::NativePlan;
use crate::target::NativeBuildMode;
use crate::testutil::{nir_for_src, CodeTarget};

/// The real native plan for `module` on `target`, as the backend would build it.
fn native_plan(module: &NirModule, target: CodeTarget) -> NativePlan {
    use crate::os::linux::flavor::LinuxFlavor::Glibc;
    match target {
        CodeTarget::MacosAarch64 => crate::target::macos_aarch64::plan::lower_module(module),
        CodeTarget::LinuxAarch64 => crate::target::linux_aarch64::plan::lower_module(module, Glibc),
        CodeTarget::LinuxX86_64 => crate::target::linux_x86_64::plan::lower_module(module, Glibc),
        CodeTarget::LinuxRiscv64 => crate::target::linux_riscv64::plan::lower_module(module, Glibc),
        CodeTarget::WindowsX86_64 => crate::target::win_x86_64::plan::lower_module(module),
    }
    .unwrap_or_else(|err| panic!("{} native plan: {err}", target.name()))
}

fn lower(
    module: &NirModule,
    plan: &NativePlan,
    platform: &dyn CodegenPlatform,
) -> Option<Result<NativeCodePlan, String>> {
    crate::testutil::silence_panics(|| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::codegen::engine::builder::lower_module_for_platform(module, plan, &[], platform)
        }))
    })
    .ok()
}

/// One program, lowered for one target.
struct Case {
    label: &'static str,
    source: &'static str,
    target: CodeTarget,
    build_mode: NativeBuildMode,
    /// A `--debug` build, which adds the debug report's helpers.
    debug: bool,
}

/// Fail each fallible platform call a clean lowering of `case` makes, one at a time;
/// every one must come back as the injected refusal. Returns the number of calls
/// swept, and appends each one that did not propagate to `swallowed`.
fn sweep(case: &Case, swallowed: &mut Vec<String>) -> usize {
    let at = format!("{} on {}", case.label, case.target.name());
    let mut module = nir_for_src(case.source, case.target, case.build_mode)
        .unwrap_or_else(|err| panic!("{at}: NIR: {err}"));
    module.debug = crate::codegen::debug::DebugOptions {
        enabled: case.debug,
    };
    let plan = native_plan(&module, case.target);
    let platforms = os_seam_platforms();
    let (_, platform) = platforms
        .iter()
        .find(|(name, _)| *name == case.target.name())
        .expect("every CodeTarget has a seam platform");

    let counting = FailAt::counting(platform.as_ref());
    match lower(&module, &plan, &counting) {
        Some(Ok(_)) => {}
        other => panic!(
            "{at}: the clean lowering must succeed before its failures mean anything: {:?}",
            other.map(|result| result.map(|_| ()))
        ),
    }
    let calls = counting.calls();
    for n in 0..calls {
        let failing = FailAt::new(platform.as_ref(), n);
        match lower(&module, &plan, &failing) {
            Some(Err(message)) if message.contains(INJECTED) => {}
            Some(Err(message)) => swallowed.push(format!(
                "{at} call #{n}: a different refusal replaced the injected one: {message}"
            )),
            Some(Ok(_)) => swallowed.push(format!(
                "{at} call #{n}: the emit failed and the module lowered anyway"
            )),
            None => swallowed.push(format!(
                "{at} call #{n}: the module lowering PANICKED on the error path"
            )),
        }
    }
    calls
}

/// A console program that reads standard input, owns a module-level file, starts a
/// thread over a recursive message type, detaches a child process and creates a
/// capturing closure: between them they need the stdin broadcast helpers, the file
/// drain, the module resource teardown, the process reaper, the graph copy/drop
/// walkers, the closure environment routine and the thread runtime.
const MANY_HELPERS: &str = "\
IMPORT io
IMPORT fs
IMPORT thread
IMPORT process
IMPORT collections

TYPE Leaf
  v AS Integer
END TYPE

TYPE Node2
  left AS Tree
  right AS Tree
END TYPE

UNION Tree
  Leaf
  Node2
END UNION

RES h AS fs::File = fs::createTempFile()

ISOLATED FUNC spin(t AS ThreadWorker OF Tree TO Integer, n AS Integer) AS Integer
  RETURN n
END FUNC

FUNC main() AS Integer
  MUT s AS String = io::readLine()
  collections::forEach([1, 2], LAMBDA(v AS Integer) -> s = s & \"x\")
  RES f AS fs::File = fs::open(s, \"w\")
  fs::writeAll(f, s)
  fs::close(f)
  fs::writeAll(h, s)
  RES p = process::spawn([\"true\"])
  process::detach(p)
  RES t AS Thread OF Tree TO Integer = thread::start(spin, 7)
  io::print(toString(thread::waitFor(t)))
  RETURN 0
END FUNC
";

/// A program that opens both audio streams: on macOS the module carries the
/// AudioQueue output and input callbacks, which `openOutput`/`openInput` take the
/// address of.
const AUDIO: &str = "\
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

/// Sweep `case` and require every injected failure to come back out. The floor on
/// the call count keeps the sweep honest: a program that stopped reaching the
/// helpers it is for would pass every assertion above with nothing swept.
fn assert_every_failure_propagates(case: Case, at_least: usize) {
    let mut swallowed = Vec::new();
    let calls = sweep(&case, &mut swallowed);
    assert!(
        swallowed.is_empty(),
        "{} of {calls} injected failures did not propagate:\n{}",
        swallowed.len(),
        swallowed.join("\n")
    );
    assert!(
        calls >= at_least,
        "{} on {}: only {calls} fallible platform calls, expected at least {at_least}",
        case.label,
        case.target.name()
    );
}

/// Measured at 186 calls when written.
#[test]
fn a_platform_failure_in_a_console_programs_helpers_propagates() {
    assert_every_failure_propagates(
        Case {
            label: "helpers",
            source: MANY_HELPERS,
            target: CodeTarget::LinuxX86_64,
            build_mode: NativeBuildMode::Console,
            debug: false,
        },
        150,
    );
}

/// The same program as a `--debug` build, which adds `_mfb_debug_shutdown` and its
/// report sections. Measured at 229 calls when written.
#[test]
fn a_platform_failure_in_the_debug_report_propagates() {
    assert_every_failure_propagates(
        Case {
            label: "helpers --debug",
            source: MANY_HELPERS,
            target: CodeTarget::LinuxX86_64,
            build_mode: NativeBuildMode::Console,
            debug: true,
        },
        190,
    );
}

/// Measured at 76 calls when written.
#[test]
fn a_platform_failure_in_the_audio_callbacks_propagates() {
    assert_every_failure_propagates(
        Case {
            label: "audio",
            source: AUDIO,
            target: CodeTarget::MacosAarch64,
            build_mode: NativeBuildMode::Console,
            debug: false,
        },
        60,
    );
}
