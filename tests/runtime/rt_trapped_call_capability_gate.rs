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
//! **The rejection half of this test no longer lives here.** `windows-x86_64`
//! used to advertise a strict subset of the macOS surface, so a real gap existed
//! to aim at: `process.shell` until plan-119-B implemented it, then
//! `os.resourcePath`. bug-454 implemented that one too, and it was the LAST —
//! `windows-x86_64` now advertises a superset of `macos-aarch64` and `linux-*`,
//! so **no** call reachable from MFB source is refused by any shipping backend's
//! list. The premise assertions here were written to fail loudly rather than
//! pass vacuously when that happened, and they did.
//!
//! Rather than weaken them, the rejection case moved to
//! `validate::tests::a_trapped_runtime_call_is_capability_checked_like_a_bare_one`,
//! which builds the capability set by hand and so does not depend on a backend
//! having a gap. If a future target ever ships with one again, a build-level
//! case belongs back here.
//!
//! What stays here is the half that still needs a real build: a trapped call the
//! backend DOES advertise must build, and a trapped bare-named `general` builtin
//! must not be capability-gated at all. Both are the over-broad-fix guards — the
//! `general` family answers to `helper_for_call` but appears in no backend's
//! `runtime_calls`, so collecting it would fail every program that traps a
//! conversion.

#[path = "../common/mod.rs"]
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
