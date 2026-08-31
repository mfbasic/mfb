//! A **trapped** runtime call must still be capability-validated.
//!
//! Found while landing plan-98-B Phase 2, and older than that plan: the TRAP
//! desugar turns `pkg::call(x) TRAP(e)` into a `NirValue::CallResult`, not a
//! `NirValue::RuntimeCall`, and `validate_capabilities` walked a `CallResult`'s
//! **arguments only** — never its target. So the identical call was correctly
//! rejected on a backend that does not advertise it when written bare, and silently
//! accepted when wrapped in a TRAP.
//!
//! That is the common case, not a corner: a program almost always traps a fallible
//! call. The consequence was a binary emitted for a backend with no implementation
//! behind the call — the exact situation `validate_capabilities` exists to prevent,
//! reached through the ordinary way of writing the code.
//!
//! The fix collects a `CallResult`'s target when it is package-qualified *and*
//! names a runtime-helper family — the same predicate the sibling pass
//! (`runtime::usage::push_value_helpers`) already used. Both halves matter: the
//! bare-named `general` family (`toString`, `toInt`) also answers to
//! `helper_for_call` but appears in no backend's `runtime_calls`, so collecting it
//! would fail every program that traps a conversion.
//!
//! `windows-x86_64` is the vehicle because it advertises a strict subset of the
//! macOS surface (`process.shell`, `process.spawnEnv`, `os.resourcePath` are the
//! three it lacks), so a real, non-hypothetical gap exists to test against.

mod common;

use std::process::Command;

/// Build for `target` and return `(succeeded, combined output)`.
fn build(name: &str, source: &str, target: &str) -> (bool, String) {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("-target")
        .arg(target)
        .arg(&project)
        .output()
        .expect("run mfb build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_dir_all(&project);
    (output.status.success(), combined)
}

/// A call every backend advertises, trapped. This must still build — the fix must
/// reject *unsupported* calls, not all trapped ones.
#[test]
fn a_trapped_supported_call_still_builds() {
    let (ok, log) = build(
        "trap_gate_supported",
        "IMPORT fs\n\
         FUNC main AS Integer\n\
        \x20 LET text AS String = fs::readText(\"project.json\") TRAP(err)\n\
        \x20   RETURN 1\n\
        \x20 END TRAP\n\
        \x20 RETURN len(text)\n\
         END FUNC\n",
        "windows-x86_64",
    );
    assert!(ok, "a trapped, advertised call must still build:\n{log}");
}

/// Trapping a conversion must not be mistaken for a capability-gated call. The
/// `general` family answers to `helper_for_call` but its members are bare-named and
/// unconditionally available; an over-broad fix rejects this program with
/// "native backend does not implement runtime helper 'general'".
#[test]
fn a_trapped_general_builtin_is_not_capability_gated() {
    let (ok, log) = build(
        "trap_gate_general",
        "FUNC main AS Integer\n\
        \x20 LET n AS Integer = toInt(\"12\") TRAP(err)\n\
        \x20   RETURN 1\n\
        \x20 END TRAP\n\
        \x20 RETURN n - 12\n\
         END FUNC\n",
        "windows-x86_64",
    );
    assert!(
        ok,
        "a trapped general-family builtin is not capability-gated:\n{log}"
    );
}

/// The regression itself. `process::shell` is advertised on macOS but **not** on
/// Windows, so a Windows build has a genuinely unsupported call to aim at. Pairing
/// the bare and trapped forms is the point: before the fix they disagreed, and only
/// the bare one was rejected.
#[test]
fn a_trapped_unsupported_call_is_rejected_like_a_bare_one() {
    const TARGET: &str = "windows-x86_64";
    const BARE: &str = "IMPORT process\n\
         FUNC main AS Integer\n\
        \x20 RES p AS process::Process = process::shell(\"echo hi\")\n\
        \x20 RETURN 0\n\
         END FUNC\n";
    const TRAPPED: &str = "IMPORT process\n\
         FUNC main AS Integer\n\
        \x20 RES p AS process::Process = process::shell(\"echo hi\") TRAP(err)\n\
        \x20   RETURN 1\n\
        \x20 END TRAP\n\
        \x20 RETURN 0\n\
         END FUNC\n";

    let (bare_ok, bare_log) = build("trap_gate_bare", BARE, TARGET);
    let (trapped_ok, trapped_log) = build("trap_gate_trapped", TRAPPED, TARGET);

    // A vacuous pass would be worse than a failure here: if the bare form built,
    // the call is supported and this test proves nothing. Assert the premise.
    assert!(
        !bare_ok,
        "{TARGET} unexpectedly supports process::shell, so this test no longer \
         covers an unsupported call; pick another:\n{bare_log}"
    );
    assert!(
        bare_log.contains("does not support runtime call"),
        "the premise is a CAPABILITY rejection, not some other error:\n{bare_log}"
    );
    assert!(
        !trapped_ok,
        "TRAPPING an unsupported call must not smuggle it past capability \
         validation — the bare form was correctly rejected:\n{trapped_log}"
    );
    assert!(
        trapped_log.contains("does not support runtime call"),
        "the trapped form must fail with the capability diagnostic, not something \
         incidental:\n{trapped_log}"
    );
}
